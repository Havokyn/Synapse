//! Suggestion-engine MCP tools (#858) — own router, merged in `server.rs`.
//!
//! `suggestion_tick` runs one decision pass (expire/abandon + gated creation);
//! `suggestion_list` reads the persisted suggestion rows. Thin wrappers around
//! [`crate::m3::suggestions`], which owns the CF_KV truth and the anti-Clippy
//! gates.

use rmcp::{RoleServer, service::RequestContext};
use serde_json::{Value, json};
use synapse_core::{error_codes, types::RoutineFeedbackOutcome};

use super::{ErrorData, Json, Parameters, SynapseService, tool, tool_router};

use crate::m1::CdpOpenTabParams;
use crate::m3::episodes::now_ts_ns;
use crate::m3::plan::{
    PlanBackend, PlanDocument, PlanStep, Postcondition, RoutineCompilePlanParams,
    compile_routine_plan, load_plan,
};
use crate::m3::plan_execution::{
    PlanStepExecutionReport, PlanStepExecutionStatus, build_plan_execution_record,
    plan_execution_id, write_plan_execution,
};
use crate::m3::suggestions::{
    SuggestionAcceptParams, SuggestionAcceptResponse, SuggestionListParams, SuggestionListResponse,
    SuggestionTickParams, SuggestionTickResponse, accept_suggestion_for_execution,
    list_suggestions, load_suggestion_by_id, record_suggestion_execution_feedback,
    required_permissions_accept, required_permissions_list, required_permissions_tick,
    suggestion_tick,
};
use crate::m4::{ActLaunchParams, LaunchWindowState};

const PLAN_REF_PREFIX: &str = "plan/v1/";
const DEFAULT_EXECUTION_LAUNCH_TIMEOUT_MS: u64 = 10_000;

#[tool_router(router = suggestions_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(
        description = "Run ONE suggestion-engine pass (#858): expire timed-out live suggestions (→ ignored_timeout feedback), abandon ones whose routine left the live intent set (→ abandoned feedback), then create suggestions for the routines the operator appears to be executing now that pass EVERY anti-Clippy gate — confidence threshold, #856 decline cooldown, quiet hours, dedup (one live per routine), per-routine frequency cap, and global frequency cap. Disabled/archived routines never surface. Truth is persisted in CF_KV (suggestion/v1/), so caps/dedup survive a daemon restart. Returns every candidate's gate decision (created or the precise suppression reason), plus the created/expired/abandoned ids and the active config. Pass now_ts_ns to evaluate a past instant (replay), or dry_run to compute decisions without persisting."
    )]
    pub async fn suggestion_tick(
        &self,
        params: Parameters<SuggestionTickParams>,
    ) -> Result<Json<SuggestionTickResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "suggestion_tick",
            now_ts_ns = params.0.now_ts_ns,
            dry_run = params.0.dry_run,
            "tool.invocation kind=suggestion_tick"
        );
        self.require_m3_permissions("suggestion_tick", &required_permissions_tick(&params.0))?;
        let db = self.m3_storage()?;
        suggestion_tick(&db, &params.0).map(Json)
    }

    #[tool(
        description = "List surfaced suggestions (#858) from CF_KV, newest first, optionally filtered by status (live/accepted/declined/expired/abandoned) and/or routine_id. Read-only — the operator-facing view of what the suggestion engine has produced and how each resolved."
    )]
    pub async fn suggestion_list(
        &self,
        params: Parameters<SuggestionListParams>,
    ) -> Result<Json<SuggestionListResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "suggestion_list",
            status = ?params.0.status,
            routine_id = params.0.routine_id.as_deref(),
            "tool.invocation kind=suggestion_list"
        );
        self.require_m3_permissions("suggestion_list", &required_permissions_list(&params.0))?;
        let db = self.m3_storage()?;
        list_suggestions(&db, &params.0).map(Json)
    }

    #[tool(
        description = "Accept one live suggestion and execute its compiled setup plan (#860). Loads or compiles the routine plan, marks the durable suggestion/v1 row accepted, runs supported steps through background-first routes (act_launch for apps, cdp_open_tab for browser hosts), refuses unsupported/ambiguous steps loudly, verifies each step's postcondition, persists a plan_execution/v1 report, and records routine feedback with the execution outcome. dry_run returns the same routing report without mutating storage or launching/opening anything."
    )]
    pub async fn suggestion_accept(
        &self,
        params: Parameters<SuggestionAcceptParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<SuggestionAcceptResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "suggestion_accept",
            suggestion_id = %params.0.suggestion_id,
            dry_run = params.0.dry_run,
            "tool.invocation kind=suggestion_accept"
        );
        self.require_m3_permissions("suggestion_accept", &required_permissions_accept(&params.0))?;
        let session_id = super::context::mcp_session_id_from_request_context(&request_context)?;
        self.suggestion_accept_impl(params.0, session_id)
            .await
            .map(Json)
    }
}

impl SynapseService {
    async fn suggestion_accept_impl(
        &self,
        params: SuggestionAcceptParams,
        session_id: Option<String>,
    ) -> Result<SuggestionAcceptResponse, ErrorData> {
        validate_suggestion_accept_params(&params)?;
        let db = self.m3_storage()?;
        let Some(existing) = load_suggestion_by_id(&db, &params.suggestion_id)? else {
            return Err(crate::m1::mcp_error(
                error_codes::STORAGE_READ_FAILED,
                format!(
                    "SUGGESTION_NOT_FOUND: suggestion_id {} is not in CF_KV",
                    params.suggestion_id
                ),
            ));
        };
        let plan = load_or_compile_plan(&db, &existing.routine_id, !params.dry_run)?;
        let accepted_ts_ns = now_ts_ns();
        let started_ts_ns = accepted_ts_ns;
        let execution_id = plan_execution_id(&existing.suggestion_id, started_ts_ns);
        let plan_ref = format!("{PLAN_REF_PREFIX}{}", plan.routine_id);
        let accepted = accept_suggestion_for_execution(
            &db,
            &existing.suggestion_id,
            accepted_ts_ns,
            &plan_ref,
            &execution_id,
            params.dry_run,
        )?;

        let steps = self
            .execute_accepted_plan_steps(&plan, &params, session_id.as_deref())
            .await;
        let completed_ts_ns = now_ts_ns();
        let execution = build_plan_execution_record(
            execution_id,
            accepted.suggestion_id.clone(),
            session_id,
            accepted_ts_ns,
            started_ts_ns,
            completed_ts_ns,
            params.dry_run,
            plan.clone(),
            steps,
        );
        if !params.dry_run {
            write_plan_execution(&db, &execution)?;
            let feedback_note = format!(
                "suggestion_accept execution_status={} execution_id={} succeeded_steps={} failed_or_refused_steps={}",
                execution.status.as_str(),
                execution.execution_id,
                execution
                    .steps
                    .iter()
                    .filter(|step| step.status == PlanStepExecutionStatus::Succeeded)
                    .count(),
                execution
                    .steps
                    .iter()
                    .filter(|step| step.status.is_terminal_failure())
                    .count()
            );
            record_suggestion_execution_feedback(
                &db,
                &accepted.routine_id,
                RoutineFeedbackOutcome::Accepted,
                completed_ts_ns,
                &feedback_note,
            )?;
        }
        Ok(SuggestionAcceptResponse {
            suggestion: accepted,
            plan,
            execution,
        })
    }

    async fn execute_accepted_plan_steps(
        &self,
        plan: &PlanDocument,
        params: &SuggestionAcceptParams,
        session_id: Option<&str>,
    ) -> Vec<PlanStepExecutionReport> {
        let mut reports = Vec::with_capacity(plan.steps.len());
        let mut aborted = false;
        for step in &plan.steps {
            let report = if aborted {
                skipped_step_report(
                    step,
                    "previous step failed or was refused; execution aborted",
                )
            } else if params.dry_run {
                dry_run_step_report(step, params)
            } else {
                self.execute_plan_step(step, params, session_id).await
            };
            if report.status.is_terminal_failure() {
                aborted = true;
            }
            reports.push(report);
        }
        reports
    }

    async fn execute_plan_step(
        &self,
        step: &PlanStep,
        params: &SuggestionAcceptParams,
        session_id: Option<&str>,
    ) -> PlanStepExecutionReport {
        match step.backend {
            PlanBackend::ActLaunch => self.execute_launch_step(step, params, session_id).await,
            PlanBackend::CdpOpenTab => {
                self.execute_cdp_open_tab_step(step, params, session_id)
                    .await
            }
            PlanBackend::ShellOpen => refused_step_report(
                step,
                "PLAN_EXECUTOR_BACKEND_UNSUPPORTED",
                "shell_open execution is not implemented yet; refusing instead of silently opening an unverified document",
                json!({
                    "source_app": &step.source_app,
                    "source_document": &step.source_document,
                }),
            ),
            PlanBackend::AgentTask => refused_step_report(
                step,
                "PLAN_EXECUTOR_AGENT_TASK_REQUIRED",
                step.agent_task_reason.as_deref().unwrap_or(
                    "plan step requires agent judgment; no agent was spawned by suggestion_accept",
                ),
                json!({
                    "agent_task_reason": &step.agent_task_reason,
                    "source_app": &step.source_app,
                    "source_document": &step.source_document,
                }),
            ),
        }
    }

    async fn execute_launch_step(
        &self,
        step: &PlanStep,
        params: &SuggestionAcceptParams,
        session_id: Option<&str>,
    ) -> PlanStepExecutionReport {
        let started = now_ts_ns();
        if let Err(error) = self.ensure_supported_use_allows_action("act_launch") {
            return error_step_report(started, step, &error);
        }
        let launch = ActLaunchParams {
            target: step.source_app.clone(),
            args: Vec::new(),
            working_dir: None,
            env: Default::default(),
            wait_for_window_title_regex: Some(".*".to_owned()),
            timeout_ms: params
                .launch_timeout_ms
                .unwrap_or(DEFAULT_EXECUTION_LAUNCH_TIMEOUT_MS),
            idempotency_key: Some(format!(
                "suggestion_accept:{}:step:{}",
                step.source_app, step.index
            )),
            cdp_debug: Some(false),
            force_renderer_accessibility: None,
            windows_console_window_state: Some(LaunchWindowState::Hidden),
            desktop: session_id.map(|_| "agent:session".to_owned()),
        };
        let result = self
            .act_launch_for_session_id(launch, session_id.map(ToOwned::to_owned))
            .await;
        match result {
            Ok(response) => {
                let evidence = json!({ "act_launch": response });
                match &step.postcondition {
                    Postcondition::WindowForProcessExists { .. } if response.hwnd.is_some() => {
                        step_report(
                            started,
                            step,
                            PlanStepExecutionStatus::Succeeded,
                            evidence,
                            None,
                            None,
                        )
                    }
                    Postcondition::WindowForProcessExists { process } => step_report(
                        started,
                        step,
                        PlanStepExecutionStatus::Failed,
                        evidence,
                        Some(error_codes::ACTION_POSTCONDITION_FAILED),
                        Some(format!(
                            "act_launch returned without a window for expected process {process}"
                        )),
                    ),
                    other => step_report(
                        started,
                        step,
                        PlanStepExecutionStatus::Failed,
                        evidence,
                        Some(error_codes::ACTION_POSTCONDITION_FAILED),
                        Some(format!(
                            "act_launch step produced a launch response but cannot verify postcondition {other:?}"
                        )),
                    ),
                }
            }
            Err(error) => error_step_report(started, step, &error),
        }
    }

    async fn execute_cdp_open_tab_step(
        &self,
        step: &PlanStep,
        params: &SuggestionAcceptParams,
        session_id: Option<&str>,
    ) -> PlanStepExecutionReport {
        let started = now_ts_ns();
        let Some(session_id) = session_id else {
            return step_report(
                started,
                step,
                PlanStepExecutionStatus::Refused,
                json!({
                    "browser_window_hwnd": params.browser_window_hwnd,
                    "source_document": &step.source_document,
                }),
                Some(error_codes::HTTP_SESSION_INVALID),
                Some("cdp_open_tab plan steps require an MCP session id; refusing to use the human foreground browser implicitly".to_owned()),
            );
        };
        let Some(host) = browser_host_for_step(step) else {
            return refused_step_report(
                step,
                "PLAN_EXECUTOR_BROWSER_HOST_MISSING",
                "cdp_open_tab step did not carry a BrowserTabAtHost postcondition or source document host",
                json!({
                    "postcondition": &step.postcondition,
                    "source_document": &step.source_document,
                }),
            );
        };
        let requested_url = format!("https://{host}");
        let result = self
            .cdp_open_tab_for_session(
                CdpOpenTabParams {
                    window_hwnd: params.browser_window_hwnd,
                    url: requested_url.clone(),
                },
                session_id,
            )
            .await;
        match result {
            Ok(response) => {
                let host_matches = url_host_matches(&response.target_url, &host);
                let evidence = json!({
                    "requested_host": host,
                    "requested_url": requested_url,
                    "cdp_open_tab": response,
                    "host_matches": host_matches,
                });
                if host_matches {
                    step_report(
                        started,
                        step,
                        PlanStepExecutionStatus::Succeeded,
                        evidence,
                        None,
                        None,
                    )
                } else {
                    step_report(
                        started,
                        step,
                        PlanStepExecutionStatus::Failed,
                        evidence,
                        Some(error_codes::ACTION_POSTCONDITION_FAILED),
                        Some(
                            "cdp_open_tab target_url host did not match the plan postcondition"
                                .to_owned(),
                        ),
                    )
                }
            }
            Err(error) => error_step_report(started, step, &error),
        }
    }
}

fn validate_suggestion_accept_params(params: &SuggestionAcceptParams) -> Result<(), ErrorData> {
    if params.suggestion_id.trim().is_empty() {
        return Err(crate::m1::mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            "suggestion_accept suggestion_id must not be empty",
        ));
    }
    if let Some(timeout_ms) = params.launch_timeout_ms
        && timeout_ms == 0
    {
        return Err(crate::m1::mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            "suggestion_accept launch_timeout_ms must be >= 1",
        ));
    }
    Ok(())
}

fn load_or_compile_plan(
    db: &std::sync::Arc<synapse_storage::Db>,
    routine_id: &str,
    store: bool,
) -> Result<PlanDocument, ErrorData> {
    if let Some(plan) = load_plan(db, routine_id)? {
        return Ok(plan);
    }
    Ok(compile_routine_plan(
        db,
        &RoutineCompilePlanParams {
            routine_id: routine_id.to_owned(),
            store,
        },
    )?
    .plan)
}

fn browser_host_for_step(step: &PlanStep) -> Option<String> {
    match &step.postcondition {
        Postcondition::BrowserTabAtHost { host } => Some(host.clone()),
        _ => step.source_document.clone(),
    }
}

fn dry_run_step_report(
    step: &PlanStep,
    params: &SuggestionAcceptParams,
) -> PlanStepExecutionReport {
    let started = now_ts_ns();
    step_report(
        started,
        step,
        PlanStepExecutionStatus::DryRun,
        json!({
            "dry_run": true,
            "backend": step.backend,
            "browser_window_hwnd": params.browser_window_hwnd,
            "launch_timeout_ms": params.launch_timeout_ms.unwrap_or(DEFAULT_EXECUTION_LAUNCH_TIMEOUT_MS),
        }),
        None,
        None,
    )
}

fn skipped_step_report(step: &PlanStep, reason: &str) -> PlanStepExecutionReport {
    let started = now_ts_ns();
    step_report(
        started,
        step,
        PlanStepExecutionStatus::Skipped,
        json!({ "reason": reason }),
        Some("PLAN_EXECUTOR_STEP_SKIPPED"),
        Some(reason.to_owned()),
    )
}

fn refused_step_report(
    step: &PlanStep,
    code: &'static str,
    reason: &str,
    evidence: Value,
) -> PlanStepExecutionReport {
    let started = now_ts_ns();
    step_report(
        started,
        step,
        PlanStepExecutionStatus::Refused,
        evidence,
        Some(code),
        Some(reason.to_owned()),
    )
}

fn error_step_report(started: u64, step: &PlanStep, error: &ErrorData) -> PlanStepExecutionReport {
    let code = error_data_code(error);
    let status = if is_refusal_code(code.as_deref()) {
        PlanStepExecutionStatus::Refused
    } else {
        PlanStepExecutionStatus::Failed
    };
    step_report(
        started,
        step,
        status,
        json!({
            "error_data": &error.data,
        }),
        code.as_deref(),
        Some(error.message.to_string()),
    )
}

fn step_report(
    started: u64,
    step: &PlanStep,
    status: PlanStepExecutionStatus,
    evidence: Value,
    error_code: Option<&str>,
    error: Option<String>,
) -> PlanStepExecutionReport {
    let completed = now_ts_ns();
    PlanStepExecutionReport {
        index: step.index,
        backend: step.backend,
        action: step.action.clone(),
        postcondition: step.postcondition.clone(),
        status,
        started_ts_ns: started,
        completed_ts_ns: completed,
        duration_ns: completed.saturating_sub(started),
        evidence,
        error_code: error_code.map(ToOwned::to_owned),
        error,
    }
}

fn error_data_code(error: &ErrorData) -> Option<String> {
    error
        .data
        .as_ref()
        .and_then(|data| data.get("code"))
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn is_refusal_code(code: Option<&str>) -> bool {
    matches!(
        code,
        Some(error_codes::SAFETY_PERMISSION_DENIED)
            | Some(error_codes::SAFETY_PROFILE_ACTION_DENIED)
            | Some(error_codes::SAFETY_LAUNCH_DENIED_BY_POLICY)
            | Some(error_codes::HTTP_SESSION_INVALID)
            | Some(error_codes::TOOL_PARAMS_INVALID)
    )
}

fn url_host_matches(url: &str, expected_host: &str) -> bool {
    let expected = expected_host
        .trim()
        .trim_end_matches('/')
        .to_ascii_lowercase();
    if expected.is_empty() {
        return false;
    }
    reqwest::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_ascii_lowercase))
        .is_some_and(|host| host == expected)
}
