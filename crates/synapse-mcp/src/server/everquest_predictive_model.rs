#![allow(clippy::derive_partial_eq_without_eq)]

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use rmcp::{ErrorData, schemars::JsonSchema};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use synapse_core::error_codes;
use synapse_storage::cf;

use super::{
    Json, Parameters, SynapseService,
    everquest_domain::{
        EverQuestDynamicJepaActionRow, EverQuestDynamicJepaOutcomeRow,
        EverQuestDynamicJepaStateRow, EverQuestDynamicJepaTransitionRow,
    },
    everquest_log::EVERQUEST_PROFILE_ID,
    everquest_trajectory::EverQuestTrajectoryRow,
    tool, tool_router,
};
use crate::m1::mcp_error;

const FIT_TOOL: &str = "everquest_predictive_model_fit";
const PREDICT_TOOL: &str = "everquest_predictive_model_predict";
const SCHEMA_VERSION: u32 = 1;
const MODEL_ROW_PREFIX: &str = "everquest/predictive_model/v1";
const PREDICTION_ROW_PREFIX: &str = "everquest/prediction/v1";
const TRAJECTORY_ROW_PREFIX: &str = "everquest/trajectory/v1";
const MAX_ID_BYTES: usize = 128;
const MAX_TEXT_BYTES: usize = 512;
const MAX_TRAJECTORIES: u32 = 128;
const MAX_CANDIDATE_ACTIONS: usize = 16;
const MAX_SOURCE_REFS: usize = 32;
const MAX_LIMITATIONS: usize = 32;
const DEFAULT_MAX_TRAJECTORIES: u32 = 64;
const DEFAULT_MIN_TRANSITION_SUPPORT: u32 = 1;
const DEFAULT_MIN_CONFIDENCE: f32 = 0.60;

const ACTION_KINDS: &[&str] = &[
    "loc_probe",
    "target_consider",
    "bounded_move",
    "combat_spell",
    "sit_rest",
    "inventory_read",
    "map_read",
    "denied_unsafe",
];

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EverQuestPredictiveModelFitParams {
    pub model_id: String,
    #[serde(default = "default_profile_id")]
    pub profile_id: String,
    #[serde(default)]
    pub trajectory_row_keys: Vec<String>,
    #[serde(default = "default_max_trajectories")]
    pub max_trajectories: u32,
    #[serde(default = "default_min_transition_support")]
    pub min_transition_support: u32,
    #[serde(default = "default_min_confidence")]
    pub min_confidence: f32,
    #[serde(default)]
    pub source_refs: Vec<EverQuestPredictiveSourceRef>,
    #[serde(default)]
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EverQuestPredictiveModelPredictParams {
    pub prediction_id: String,
    #[serde(default = "default_profile_id")]
    pub profile_id: String,
    pub model_id: String,
    pub state_row_key: String,
    pub candidate_actions: Vec<EverQuestPredictiveCandidateAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_model_hash: Option<String>,
    #[serde(default = "default_min_transition_support")]
    pub min_transition_support: u32,
    #[serde(default = "default_min_confidence")]
    pub min_confidence: f32,
    #[serde(default)]
    pub source_refs: Vec<EverQuestPredictiveSourceRef>,
    #[serde(default)]
    pub limitations: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestPredictiveModelFitResponse {
    pub ok: bool,
    pub row_key: String,
    pub stored_value_len_bytes: u64,
    pub model: EverQuestPredictiveModelRow,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestPredictiveModelPredictResponse {
    pub ok: bool,
    pub row_key: String,
    pub stored_value_len_bytes: u64,
    pub prediction: EverQuestPredictivePredictionRow,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestPredictiveSourceRef {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_offset: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_offset: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestPredictiveCandidateAction {
    pub action_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestPredictiveModelRow {
    pub schema_version: u32,
    pub row_kind: String,
    pub profile_id: String,
    pub model_id: String,
    pub row_key: String,
    pub trained_at: DateTime<Utc>,
    pub algorithm: String,
    pub status: String,
    pub model_hash: String,
    pub training: EverQuestPredictiveTrainingSummary,
    pub source_trajectory_keys: Vec<String>,
    pub source_transition_keys: Vec<String>,
    pub entries: Vec<EverQuestPredictiveModelEntry>,
    pub action_fallback_entries: Vec<EverQuestPredictiveModelEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub global_fallback: Option<EverQuestPredictiveModelEntry>,
    pub source_refs: Vec<EverQuestPredictiveSourceRef>,
    pub limitations: Vec<String>,
    pub evidence_boundary: EverQuestPredictiveEvidenceBoundary,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestPredictiveTrainingSummary {
    pub trajectory_count: u32,
    pub transition_count: u32,
    pub accepted_transition_count: u32,
    pub rejected_transition_count: u32,
    pub scan_truncated: bool,
    pub min_transition_support: u32,
    pub min_confidence: f32,
    pub competence_floor: f32,
    pub stretch_target: f32,
    pub conflicting_bucket_count: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestPredictiveModelEntry {
    pub scope: String,
    pub state_signature: String,
    pub action_kind: String,
    pub sample_count: u32,
    pub winning_count: u32,
    pub confidence: f32,
    pub target: EverQuestPredictiveTarget,
    pub source_transition_ids: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestPredictiveTarget {
    pub outcome_kind: String,
    pub next_zone_short_name: String,
    pub next_coord_bucket: String,
    pub log_event_kind: String,
    pub con_delta: String,
    pub surprise: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestPredictivePredictionRow {
    pub schema_version: u32,
    pub row_kind: String,
    pub profile_id: String,
    pub prediction_id: String,
    pub row_key: String,
    pub predicted_at: DateTime<Utc>,
    pub model_row_key: String,
    pub model_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_model_hash: Option<String>,
    pub state_row_key: String,
    pub state_signature: String,
    pub candidate_actions: Vec<EverQuestPredictiveCandidateAction>,
    pub evaluated_candidates: Vec<EverQuestPredictiveCandidateEvaluation>,
    pub decision: String,
    pub abstain: bool,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<EverQuestPredictiveCandidateEvaluation>,
    pub source_refs: Vec<EverQuestPredictiveSourceRef>,
    pub limitations: Vec<String>,
    pub evidence_boundary: EverQuestPredictiveEvidenceBoundary,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestPredictiveCandidateEvaluation {
    pub action_kind: String,
    pub source_scope: String,
    pub sample_count: u32,
    pub confidence: f32,
    pub target: EverQuestPredictiveTarget,
    pub reason: String,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EverQuestPredictiveEvidenceBoundary {
    pub supports_planning_quality: bool,
    pub manual_fsv_required_for_runtime: bool,
    pub is_fsv: bool,
    pub redacted: bool,
    pub no_game_input_executed: bool,
    pub note: String,
}

#[derive(Clone, Debug)]
struct NormalizedFitParams {
    model_id: String,
    profile_id: String,
    trajectory_row_keys: Vec<String>,
    max_trajectories: u32,
    min_transition_support: u32,
    min_confidence: f32,
    source_refs: Vec<EverQuestPredictiveSourceRef>,
    limitations: Vec<String>,
}

#[derive(Clone, Debug)]
struct NormalizedPredictParams {
    prediction_id: String,
    profile_id: String,
    model_id: String,
    state_row_key: String,
    candidate_actions: Vec<EverQuestPredictiveCandidateAction>,
    expected_model_hash: Option<String>,
    min_transition_support: u32,
    min_confidence: f32,
    source_refs: Vec<EverQuestPredictiveSourceRef>,
    limitations: Vec<String>,
}

#[derive(Clone, Debug)]
struct ReadRow<T> {
    key: String,
    row: T,
}

#[derive(Clone, Debug)]
struct TrainingExample {
    state_signature: String,
    action_kind: String,
    target: EverQuestPredictiveTarget,
    transition_id: String,
    domain_transition_key: String,
}

#[derive(Clone, Debug, Default)]
struct TargetStats {
    target: Option<EverQuestPredictiveTarget>,
    count: u32,
    source_transition_ids: BTreeSet<String>,
}

#[derive(Clone, Debug, Default)]
struct BucketStats {
    targets: BTreeMap<String, TargetStats>,
    total: u32,
}

#[tool_router(router = everquest_predictive_model_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(
        description = "Fit a transparent EverQuest action-conditioned predictive baseline from verified trajectory/domain rows with exact CF_KV readback"
    )]
    pub async fn everquest_predictive_model_fit(
        &self,
        params: Parameters<EverQuestPredictiveModelFitParams>,
    ) -> Result<Json<EverQuestPredictiveModelFitResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = FIT_TOOL,
            "tool.invocation kind=everquest_predictive_model_fit"
        );
        let params = normalize_fit_params(params.0)?;
        let response = self.fit_predictive_model(params)?;
        Ok(Json(response))
    }

    #[tool(
        description = "Persist one EverQuest predictive-model next-outcome prediction row with calibrated abstention and exact CF_KV readback"
    )]
    pub async fn everquest_predictive_model_predict(
        &self,
        params: Parameters<EverQuestPredictiveModelPredictParams>,
    ) -> Result<Json<EverQuestPredictiveModelPredictResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = PREDICT_TOOL,
            "tool.invocation kind=everquest_predictive_model_predict"
        );
        let params = normalize_predict_params(params.0)?;
        let response = self.predict_with_model(params)?;
        Ok(Json(response))
    }
}

impl SynapseService {
    fn fit_predictive_model(
        &self,
        params: NormalizedFitParams,
    ) -> Result<EverQuestPredictiveModelFitResponse, ErrorData> {
        let (trajectories, scan_truncated) = self.read_training_trajectories(&params)?;
        let examples = self.training_examples(&params.profile_id, &trajectories)?;
        let row = build_model_row(params, &trajectories, &examples, scan_truncated);
        let key = row.row_key.clone();
        let (model, stored_value_len_bytes) =
            self.persist_predictive_kv_json(&key, &row, "EverQuest predictive model row")?;
        Ok(EverQuestPredictiveModelFitResponse {
            ok: true,
            row_key: key,
            stored_value_len_bytes,
            model,
        })
    }

    fn predict_with_model(
        &self,
        params: NormalizedPredictParams,
    ) -> Result<EverQuestPredictiveModelPredictResponse, ErrorData> {
        let model_key = model_row_key(&params.profile_id, &params.model_id);
        let model = self.read_kv_json::<EverQuestPredictiveModelRow>(
            &model_key,
            "EverQuest predictive model row",
        )?;
        validate_model_row(&model, &params.profile_id, &params.model_id, &model_key)?;
        let state = self.read_kv_json::<EverQuestDynamicJepaStateRow>(
            &params.state_row_key,
            "EverQuest DynamicJEPA state row",
        )?;
        validate_state_row(&state, &params.profile_id, &params.state_row_key)?;
        let state_signature = state_signature(&state);
        let row = build_prediction_row(params, model, state_signature);
        let key = row.row_key.clone();
        let (prediction, stored_value_len_bytes) =
            self.persist_predictive_kv_json(&key, &row, "EverQuest predictive prediction row")?;
        Ok(EverQuestPredictiveModelPredictResponse {
            ok: true,
            row_key: key,
            stored_value_len_bytes,
            prediction,
        })
    }

    fn read_training_trajectories(
        &self,
        params: &NormalizedFitParams,
    ) -> Result<(Vec<ReadRow<EverQuestTrajectoryRow>>, bool), ErrorData> {
        let runtime = self.reflex_runtime()?;
        let runtime = runtime.lock().map_err(|_| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                "reflex runtime lock poisoned while reading EverQuest trajectories",
            )
        })?;
        if params.trajectory_row_keys.is_empty() {
            let prefix = trajectory_prefix(&params.profile_id);
            let limit = params.max_trajectories.saturating_add(1) as usize;
            let rows = runtime
                .storage_cf_prefix_rows(cf::CF_KV, prefix.as_bytes(), limit)
                .map_err(|error| mcp_error(error.code(), error.to_string()))?;
            let scan_truncated = rows.len() > params.max_trajectories as usize;
            drop(runtime);
            let trajectories = rows
                .into_iter()
                .take(params.max_trajectories as usize)
                .map(|(key, value)| {
                    let key = String::from_utf8_lossy(&key).to_string();
                    decode_trajectory_row(&key, &value, &params.profile_id)
                })
                .collect::<Result<Vec<_>, ErrorData>>()?;
            return Ok((trajectories, scan_truncated));
        }
        let rows = params
            .trajectory_row_keys
            .iter()
            .map(|key| {
                let value = runtime
                    .storage_kv_row(key.as_bytes())
                    .map_err(|error| mcp_error(error.code(), error.to_string()))?
                    .ok_or_else(|| {
                        mcp_error(
                            error_codes::STORAGE_READ_FAILED,
                            format!("EverQuest trajectory row missing: {key}"),
                        )
                    })?;
                Ok((key.clone(), value))
            })
            .collect::<Result<Vec<_>, ErrorData>>()?;
        drop(runtime);
        let trajectories = rows
            .into_iter()
            .map(|(key, value)| decode_trajectory_row(&key, &value, &params.profile_id))
            .collect::<Result<Vec<_>, ErrorData>>()?;
        Ok((trajectories, false))
    }

    fn training_examples(
        &self,
        profile_id: &str,
        trajectories: &[ReadRow<EverQuestTrajectoryRow>],
    ) -> Result<Vec<TrainingExample>, ErrorData> {
        let runtime = self.reflex_runtime()?;
        let runtime = runtime.lock().map_err(|_| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                "reflex runtime lock poisoned while reading EverQuest domain rows",
            )
        })?;
        let mut examples = Vec::new();
        for trajectory in trajectories {
            for transition in &trajectory.row.transitions {
                let Some(domain_key) = transition.domain_transition_row_key.as_deref() else {
                    continue;
                };
                let domain = read_required_json_row::<EverQuestDynamicJepaTransitionRow>(
                    &runtime, domain_key,
                )?;
                validate_domain_transition(&domain, profile_id, domain_key)?;
                let state = read_required_json_row::<EverQuestDynamicJepaStateRow>(
                    &runtime,
                    &domain.row.state_row_key,
                )?;
                let action = read_required_json_row::<EverQuestDynamicJepaActionRow>(
                    &runtime,
                    &domain.row.action_row_key,
                )?;
                let outcome = read_required_json_row::<EverQuestDynamicJepaOutcomeRow>(
                    &runtime,
                    &domain.row.outcome_row_key,
                )?;
                validate_domain_links(&domain, &state, &action, &outcome)?;
                examples.push(TrainingExample {
                    state_signature: state_signature(&state.row),
                    action_kind: enum_string(&action.row.fields.action_kind),
                    target: target_from_outcome(&outcome.row),
                    transition_id: domain.row.transition_id.clone(),
                    domain_transition_key: domain.key,
                });
            }
        }
        drop(runtime);
        Ok(examples)
    }

    fn read_kv_json<T>(&self, key: &str, label: &str) -> Result<T, ErrorData>
    where
        T: DeserializeOwned,
    {
        let runtime = self.reflex_runtime()?;
        let runtime = runtime.lock().map_err(|_| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                format!("reflex runtime lock poisoned while reading {label}"),
            )
        })?;
        let value = runtime
            .storage_kv_row(key.as_bytes())
            .map_err(|error| mcp_error(error.code(), error.to_string()))?
            .ok_or_else(|| {
                mcp_error(
                    error_codes::STORAGE_READ_FAILED,
                    format!("{label} missing: {key}"),
                )
            })?;
        drop(runtime);
        serde_json::from_slice::<T>(&value).map_err(|error| {
            mcp_error(
                error_codes::STORAGE_CORRUPTED,
                format!("decode {label} {key}: {error}"),
            )
        })
    }

    fn persist_predictive_kv_json<T>(
        &self,
        key: &str,
        row: &T,
        label: &str,
    ) -> Result<(T, u64), ErrorData>
    where
        T: DeserializeOwned + Serialize,
    {
        let encoded = serde_json::to_vec(row).map_err(|error| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                format!("encode {label}: {error}"),
            )
        })?;
        let runtime = self.reflex_runtime()?;
        let runtime = runtime.lock().map_err(|_| {
            mcp_error(
                error_codes::TOOL_INTERNAL_ERROR,
                format!("reflex runtime lock poisoned while writing {label}"),
            )
        })?;
        runtime
            .storage_put_kv_rows(vec![(key.as_bytes().to_vec(), encoded)])
            .map_err(|error| {
                mcp_error(
                    error_codes::STORAGE_WRITE_FAILED,
                    format!("write {label}: {error}"),
                )
            })?;
        let stored = runtime
            .storage_kv_row(key.as_bytes())
            .map_err(|error| {
                mcp_error(
                    error_codes::STORAGE_READ_FAILED,
                    format!("read {label} after write: {error}"),
                )
            })?
            .ok_or_else(|| {
                mcp_error(
                    error_codes::STORAGE_READ_FAILED,
                    format!("{label} missing after write: {key}"),
                )
            })?;
        drop(runtime);
        let readback = serde_json::from_slice::<T>(&stored).map_err(|error| {
            mcp_error(
                error_codes::STORAGE_CORRUPTED,
                format!("decode {label} after write: {error}"),
            )
        })?;
        Ok((readback, len_to_u64(stored.len())))
    }
}

fn build_model_row(
    params: NormalizedFitParams,
    trajectories: &[ReadRow<EverQuestTrajectoryRow>],
    examples: &[TrainingExample],
    scan_truncated: bool,
) -> EverQuestPredictiveModelRow {
    let mut state_action = BTreeMap::<String, BucketStats>::new();
    let mut action_fallback = BTreeMap::<String, BucketStats>::new();
    let mut global = BucketStats::default();
    for example in examples {
        let state_action_key = format!("{}\u{1f}{}", example.state_signature, example.action_kind);
        add_example(state_action.entry(state_action_key).or_default(), example);
        add_example(
            action_fallback
                .entry(example.action_kind.clone())
                .or_default(),
            example,
        );
        add_example(&mut global, example);
    }
    let mut conflicting_bucket_count = 0_u32;
    let entries = state_action
        .into_iter()
        .filter_map(|(key, bucket)| {
            if bucket.targets.len() > 1 {
                conflicting_bucket_count = conflicting_bucket_count.saturating_add(1);
            }
            let (state_signature, action_kind) = split_state_action_key(&key);
            best_entry("state_action", state_signature, action_kind, bucket)
        })
        .collect::<Vec<_>>();
    let action_fallback_entries = action_fallback
        .into_iter()
        .filter_map(|(action_kind, bucket)| {
            best_entry("action_fallback", "*".to_owned(), action_kind, bucket)
        })
        .collect::<Vec<_>>();
    let global_fallback = best_entry("global_fallback", "*".to_owned(), "*".to_owned(), global);
    let source_trajectory_keys = trajectories
        .iter()
        .map(|row| row.key.clone())
        .collect::<Vec<_>>();
    let source_transition_keys = examples
        .iter()
        .map(|example| example.domain_transition_key.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let limitations = model_limitations(
        params.limitations.clone(),
        conflicting_bucket_count,
        examples.is_empty(),
        scan_truncated,
    );
    let status = model_status(
        examples.len(),
        &entries,
        &action_fallback_entries,
        global_fallback.as_ref(),
        params.min_transition_support,
    );
    let row_key = model_row_key(&params.profile_id, &params.model_id);
    let mut row = EverQuestPredictiveModelRow {
        schema_version: SCHEMA_VERSION,
        row_kind: "everquest_predictive_model".to_owned(),
        profile_id: params.profile_id,
        model_id: params.model_id,
        row_key,
        trained_at: Utc::now(),
        algorithm: "action_conditioned_markov_baseline_v1".to_owned(),
        status,
        model_hash: String::new(),
        training: EverQuestPredictiveTrainingSummary {
            trajectory_count: len_to_u32(source_trajectory_keys.len()),
            transition_count: len_to_u32(
                trajectories
                    .iter()
                    .map(|row| row.row.transitions.len())
                    .sum::<usize>(),
            ),
            accepted_transition_count: len_to_u32(examples.len()),
            rejected_transition_count: len_to_u32(
                trajectories
                    .iter()
                    .map(|row| row.row.transitions.len())
                    .sum::<usize>()
                    .saturating_sub(examples.len()),
            ),
            scan_truncated,
            min_transition_support: params.min_transition_support,
            min_confidence: params.min_confidence,
            competence_floor: 0.60,
            stretch_target: 0.80,
            conflicting_bucket_count,
        },
        source_trajectory_keys,
        source_transition_keys,
        entries,
        action_fallback_entries,
        global_fallback,
        source_refs: params.source_refs,
        limitations,
        evidence_boundary: evidence_boundary(),
    };
    row.model_hash = model_hash(&row);
    row
}

fn build_prediction_row(
    params: NormalizedPredictParams,
    model: EverQuestPredictiveModelRow,
    state_signature: String,
) -> EverQuestPredictivePredictionRow {
    let expected_model_hash = params.expected_model_hash.clone();
    let stale_hash = expected_model_hash
        .as_ref()
        .is_some_and(|expected| expected != &model.model_hash);
    let mut evaluated_candidates = if stale_hash {
        Vec::new()
    } else {
        evaluate_candidates(&model, &state_signature, &params.candidate_actions)
    };
    evaluated_candidates.sort_by(|left, right| {
        scope_rank(&left.source_scope)
            .cmp(&scope_rank(&right.source_scope))
            .then_with(|| {
                right
                    .confidence
                    .partial_cmp(&left.confidence)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| right.sample_count.cmp(&left.sample_count))
            .then_with(|| left.action_kind.cmp(&right.action_kind))
    });
    let (decision, abstain, reason, selected) = prediction_decision(
        stale_hash,
        &model,
        &evaluated_candidates,
        params.min_transition_support,
        params.min_confidence,
        params.candidate_actions.is_empty(),
    );
    EverQuestPredictivePredictionRow {
        schema_version: SCHEMA_VERSION,
        row_kind: "everquest_predictive_prediction".to_owned(),
        profile_id: params.profile_id.clone(),
        prediction_id: params.prediction_id.clone(),
        row_key: prediction_row_key(&params.profile_id, &params.prediction_id),
        predicted_at: Utc::now(),
        model_row_key: model.row_key,
        model_hash: model.model_hash,
        expected_model_hash,
        state_row_key: params.state_row_key,
        state_signature,
        candidate_actions: params.candidate_actions,
        evaluated_candidates,
        decision,
        abstain,
        reason,
        selected,
        source_refs: params.source_refs,
        limitations: params.limitations,
        evidence_boundary: evidence_boundary(),
    }
}

fn add_example(bucket: &mut BucketStats, example: &TrainingExample) {
    let key = target_key(&example.target);
    let stats = bucket.targets.entry(key).or_default();
    stats.target = Some(example.target.clone());
    stats.count = stats.count.saturating_add(1);
    stats
        .source_transition_ids
        .insert(example.transition_id.clone());
    bucket.total = bucket.total.saturating_add(1);
}

fn model_limitations(
    mut limitations: Vec<String>,
    conflicting_bucket_count: u32,
    no_examples: bool,
    scan_truncated: bool,
) -> Vec<String> {
    if conflicting_bucket_count > 0 {
        limitations.push("conflicting_outcomes_detected".to_owned());
    }
    if no_examples {
        limitations.push("no_verified_trajectories_available".to_owned());
    }
    if scan_truncated {
        limitations.push("trajectory_scan_truncated".to_owned());
    }
    dedupe_strings(limitations)
}

fn best_entry(
    scope: &str,
    state_signature: String,
    action_kind: String,
    bucket: BucketStats,
) -> Option<EverQuestPredictiveModelEntry> {
    if bucket.total == 0 {
        return None;
    }
    let (_, stats) = bucket.targets.into_iter().max_by(|left, right| {
        left.1
            .count
            .cmp(&right.1.count)
            .then_with(|| right.0.cmp(&left.0))
    })?;
    let target = stats.target?;
    Some(EverQuestPredictiveModelEntry {
        scope: scope.to_owned(),
        state_signature,
        action_kind,
        sample_count: bucket.total,
        winning_count: stats.count,
        confidence: ratio(stats.count, bucket.total),
        target,
        source_transition_ids: stats.source_transition_ids.into_iter().collect(),
    })
}

fn evaluate_candidates(
    model: &EverQuestPredictiveModelRow,
    state_signature: &str,
    candidates: &[EverQuestPredictiveCandidateAction],
) -> Vec<EverQuestPredictiveCandidateEvaluation> {
    candidates
        .iter()
        .filter_map(|candidate| {
            find_entry(model, state_signature, &candidate.action_kind).map(|entry| {
                EverQuestPredictiveCandidateEvaluation {
                    action_kind: candidate.action_kind.clone(),
                    source_scope: entry.scope.clone(),
                    sample_count: entry.sample_count,
                    confidence: entry.confidence,
                    target: entry.target.clone(),
                    reason: format!(
                        "{} sample_count={} confidence={:.3}",
                        entry.scope, entry.sample_count, entry.confidence
                    ),
                }
            })
        })
        .collect()
}

fn find_entry<'a>(
    model: &'a EverQuestPredictiveModelRow,
    state_signature: &str,
    action_kind: &str,
) -> Option<&'a EverQuestPredictiveModelEntry> {
    model
        .entries
        .iter()
        .find(|entry| entry.state_signature == state_signature && entry.action_kind == action_kind)
        .or_else(|| {
            model
                .action_fallback_entries
                .iter()
                .find(|entry| entry.action_kind == action_kind)
        })
        .or(model.global_fallback.as_ref())
}

fn scope_rank(scope: &str) -> u8 {
    match scope {
        "state_action" => 0,
        "action_fallback" => 1,
        "global_fallback" => 2,
        _ => u8::MAX,
    }
}

fn prediction_decision(
    stale_hash: bool,
    model: &EverQuestPredictiveModelRow,
    evaluated: &[EverQuestPredictiveCandidateEvaluation],
    min_transition_support: u32,
    min_confidence: f32,
    no_candidates: bool,
) -> (
    String,
    bool,
    String,
    Option<EverQuestPredictiveCandidateEvaluation>,
) {
    if stale_hash {
        return (
            "abstain_stale_model_hash".to_owned(),
            true,
            "expected_model_hash did not match stored model_hash".to_owned(),
            None,
        );
    }
    if model.status == "no_verified_trajectories" {
        return (
            "abstain_no_verified_trajectories".to_owned(),
            true,
            "model has no verified trajectory examples".to_owned(),
            None,
        );
    }
    if no_candidates {
        return (
            "abstain_no_candidate_actions".to_owned(),
            true,
            "candidate_actions must include at least one action to rank".to_owned(),
            None,
        );
    }
    let Some(best) = evaluated.first().cloned() else {
        return (
            "abstain_no_matching_model_entry".to_owned(),
            true,
            "no model entry matched the candidate actions".to_owned(),
            None,
        );
    };
    if best.sample_count < min_transition_support {
        return (
            "abstain_insufficient_transition_support".to_owned(),
            true,
            format!(
                "best sample_count {} below min_transition_support {}",
                best.sample_count, min_transition_support
            ),
            Some(best),
        );
    }
    if best.confidence < min_confidence {
        return (
            "abstain_uncertain_prediction".to_owned(),
            true,
            format!(
                "best confidence {:.3} below min_confidence {:.3}",
                best.confidence, min_confidence
            ),
            Some(best),
        );
    }
    (
        "predict".to_owned(),
        false,
        "selected highest-confidence candidate with sufficient support".to_owned(),
        Some(best),
    )
}

fn decode_trajectory_row(
    key: &str,
    value: &[u8],
    profile_id: &str,
) -> Result<ReadRow<EverQuestTrajectoryRow>, ErrorData> {
    let row = serde_json::from_slice::<EverQuestTrajectoryRow>(value).map_err(|error| {
        mcp_error(
            error_codes::STORAGE_CORRUPTED,
            format!("decode EverQuest trajectory row {key}: {error}"),
        )
    })?;
    if row.schema_version != SCHEMA_VERSION || row.row_kind != "everquest_trajectory" {
        return Err(mcp_error(
            error_codes::STORAGE_CORRUPTED,
            format!("EverQuest trajectory row has invalid schema or row_kind: {key}"),
        ));
    }
    if row.profile_id != profile_id || row.row_key != key {
        return Err(mcp_error(
            error_codes::STORAGE_CORRUPTED,
            format!("EverQuest trajectory row key/body mismatch: {key}"),
        ));
    }
    if row.redaction.raw_chat_body_persisted
        || row.redaction.raw_target_names_persisted
        || !row.redaction.compact_redacted
        || !row.redaction.all_log_refs_marked_redacted
    {
        return Err(params_error(format!(
            "EverQuest trajectory row is not model-safe because redaction is incomplete: {key}"
        )));
    }
    Ok(ReadRow {
        key: key.to_owned(),
        row,
    })
}

fn read_required_json_row<T>(
    runtime: &synapse_reflex::ReflexRuntime,
    key: &str,
) -> Result<ReadRow<T>, ErrorData>
where
    T: DeserializeOwned,
{
    let value = runtime
        .storage_kv_row(key.as_bytes())
        .map_err(|error| mcp_error(error.code(), error.to_string()))?
        .ok_or_else(|| {
            mcp_error(
                error_codes::STORAGE_READ_FAILED,
                format!("required CF_KV row missing: {key}"),
            )
        })?;
    let row = serde_json::from_slice::<T>(&value).map_err(|error| {
        mcp_error(
            error_codes::STORAGE_CORRUPTED,
            format!("decode CF_KV row {key}: {error}"),
        )
    })?;
    Ok(ReadRow {
        key: key.to_owned(),
        row,
    })
}

fn validate_domain_transition(
    row: &ReadRow<EverQuestDynamicJepaTransitionRow>,
    profile_id: &str,
    key: &str,
) -> Result<(), ErrorData> {
    if row.row.schema_version != SCHEMA_VERSION
        || row.row.row_kind != "everquest_dynamicjepa_transition"
        || row.row.profile_id != profile_id
        || row.row.row_key != key
    {
        return Err(mcp_error(
            error_codes::STORAGE_CORRUPTED,
            format!("EverQuest DynamicJEPA transition row invalid: {key}"),
        ));
    }
    if row.row.evidence_boundary.raw_chat_body_persisted
        || !row.row.evidence_boundary.compact_redacted
    {
        return Err(params_error(format!(
            "EverQuest DynamicJEPA transition row is not model-safe because redaction is incomplete: {key}"
        )));
    }
    Ok(())
}

fn validate_domain_links(
    transition: &ReadRow<EverQuestDynamicJepaTransitionRow>,
    state: &ReadRow<EverQuestDynamicJepaStateRow>,
    action: &ReadRow<EverQuestDynamicJepaActionRow>,
    outcome: &ReadRow<EverQuestDynamicJepaOutcomeRow>,
) -> Result<(), ErrorData> {
    if state.row.row_key != transition.row.state_row_key
        || action.row.row_key != transition.row.action_row_key
        || outcome.row.row_key != transition.row.outcome_row_key
        || state.row.transition_id != transition.row.transition_id
        || action.row.transition_id != transition.row.transition_id
        || outcome.row.transition_id != transition.row.transition_id
    {
        return Err(mcp_error(
            error_codes::STORAGE_CORRUPTED,
            "DynamicJEPA state/action/outcome linkage mismatch",
        ));
    }
    Ok(())
}

fn validate_model_row(
    model: &EverQuestPredictiveModelRow,
    profile_id: &str,
    model_id: &str,
    row_key: &str,
) -> Result<(), ErrorData> {
    if model.schema_version != SCHEMA_VERSION
        || model.row_kind != "everquest_predictive_model"
        || model.profile_id != profile_id
        || model.model_id != model_id
        || model.row_key != row_key
    {
        return Err(mcp_error(
            error_codes::STORAGE_CORRUPTED,
            format!("EverQuest predictive model row key/body mismatch: {row_key}"),
        ));
    }
    let actual_hash = model_hash(model);
    if actual_hash != model.model_hash {
        return Err(mcp_error(
            error_codes::STORAGE_CORRUPTED,
            format!("EverQuest predictive model hash mismatch: {row_key}"),
        ));
    }
    Ok(())
}

fn validate_state_row(
    state: &EverQuestDynamicJepaStateRow,
    profile_id: &str,
    row_key: &str,
) -> Result<(), ErrorData> {
    if state.schema_version != SCHEMA_VERSION
        || state.row_kind != "everquest_dynamicjepa_state"
        || state.profile_id != profile_id
        || state.row_key != row_key
    {
        return Err(mcp_error(
            error_codes::STORAGE_CORRUPTED,
            format!("EverQuest DynamicJEPA state row key/body mismatch: {row_key}"),
        ));
    }
    Ok(())
}

fn state_signature(state: &EverQuestDynamicJepaStateRow) -> String {
    format!(
        "zone={}|coord={}|target={}|con={}|level={}|focus={}",
        state.fields.zone_short_name,
        state.fields.coord_bucket,
        enum_string(&state.fields.target_kind),
        enum_string(&state.fields.con_bucket),
        enum_string(&state.fields.level_bucket),
        enum_string(&state.fields.ui_focus_bucket)
    )
}

fn target_from_outcome(outcome: &EverQuestDynamicJepaOutcomeRow) -> EverQuestPredictiveTarget {
    EverQuestPredictiveTarget {
        outcome_kind: enum_string(&outcome.fields.outcome_kind),
        next_zone_short_name: outcome.fields.next_zone_short_name.clone(),
        next_coord_bucket: outcome.fields.next_coord_bucket.clone(),
        log_event_kind: enum_string(&outcome.fields.log_event_kind),
        con_delta: enum_string(&outcome.fields.con_delta),
        surprise: outcome.fields.surprise,
    }
}

fn target_key(target: &EverQuestPredictiveTarget) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}",
        target.outcome_kind,
        target.next_zone_short_name,
        target.next_coord_bucket,
        target.log_event_kind,
        target.con_delta,
        target.surprise
    )
}

fn split_state_action_key(key: &str) -> (String, String) {
    key.split_once('\u{1f}').map_or_else(
        || ("*".to_owned(), key.to_owned()),
        |(state, action)| (state.to_owned(), action.to_owned()),
    )
}

fn model_status(
    example_count: usize,
    entries: &[EverQuestPredictiveModelEntry],
    action_fallback_entries: &[EverQuestPredictiveModelEntry],
    global_fallback: Option<&EverQuestPredictiveModelEntry>,
    min_transition_support: u32,
) -> String {
    if example_count == 0 {
        "no_verified_trajectories".to_owned()
    } else if !entries
        .iter()
        .chain(action_fallback_entries)
        .chain(global_fallback)
        .any(|entry| entry.sample_count >= min_transition_support)
    {
        "insufficient_transition_support".to_owned()
    } else {
        "trained".to_owned()
    }
}

fn normalize_fit_params(
    params: EverQuestPredictiveModelFitParams,
) -> Result<NormalizedFitParams, ErrorData> {
    let model_id = validate_id("model_id", &params.model_id)?;
    let profile_id = validate_profile_id(&params.profile_id)?;
    let trajectory_row_keys = normalize_trajectory_keys(params.trajectory_row_keys)?;
    if params.max_trajectories == 0 || params.max_trajectories > MAX_TRAJECTORIES {
        return Err(params_error(format!(
            "max_trajectories must be between 1 and {MAX_TRAJECTORIES}"
        )));
    }
    let min_confidence = validate_probability("min_confidence", params.min_confidence)?;
    if params.min_transition_support == 0 {
        return Err(params_error("min_transition_support must be >= 1"));
    }
    Ok(NormalizedFitParams {
        model_id,
        profile_id,
        trajectory_row_keys,
        max_trajectories: params.max_trajectories,
        min_transition_support: params.min_transition_support,
        min_confidence,
        source_refs: normalize_source_refs(params.source_refs)?,
        limitations: normalize_text_vec("limitations", params.limitations, MAX_LIMITATIONS)?,
    })
}

fn normalize_predict_params(
    params: EverQuestPredictiveModelPredictParams,
) -> Result<NormalizedPredictParams, ErrorData> {
    let prediction_id = validate_id("prediction_id", &params.prediction_id)?;
    let profile_id = validate_profile_id(&params.profile_id)?;
    let model_id = validate_id("model_id", &params.model_id)?;
    let state_row_key = normalize_required_text("state_row_key", &params.state_row_key)?;
    if params.candidate_actions.len() > MAX_CANDIDATE_ACTIONS {
        return Err(params_error(format!(
            "candidate_actions must contain <= {MAX_CANDIDATE_ACTIONS} values"
        )));
    }
    if params.min_transition_support == 0 {
        return Err(params_error("min_transition_support must be >= 1"));
    }
    let expected_model_hash = params
        .expected_model_hash
        .map(|value| normalize_required_text("expected_model_hash", &value))
        .transpose()?;
    Ok(NormalizedPredictParams {
        prediction_id,
        profile_id,
        model_id,
        state_row_key,
        candidate_actions: normalize_candidate_actions(params.candidate_actions)?,
        expected_model_hash,
        min_transition_support: params.min_transition_support,
        min_confidence: validate_probability("min_confidence", params.min_confidence)?,
        source_refs: normalize_source_refs(params.source_refs)?,
        limitations: normalize_text_vec("limitations", params.limitations, MAX_LIMITATIONS)?,
    })
}

fn normalize_trajectory_keys(values: Vec<String>) -> Result<Vec<String>, ErrorData> {
    let mut seen = BTreeSet::new();
    values
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let key = normalize_required_text(&format!("trajectory_row_keys[{index}]"), &value)?;
            if !seen.insert(key.clone()) {
                return Err(params_error(format!("duplicate trajectory row key: {key}")));
            }
            Ok(key)
        })
        .collect()
}

fn normalize_candidate_actions(
    values: Vec<EverQuestPredictiveCandidateAction>,
) -> Result<Vec<EverQuestPredictiveCandidateAction>, ErrorData> {
    values
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let action_kind = normalize_action_kind(
                &format!("candidate_actions[{index}].action_kind"),
                &value.action_kind,
            )?;
            Ok(EverQuestPredictiveCandidateAction {
                action_kind,
                alias: value
                    .alias
                    .map(|text| {
                        normalize_required_text(&format!("candidate_actions[{index}].alias"), &text)
                    })
                    .transpose()?,
                tool_name: value
                    .tool_name
                    .map(|text| {
                        normalize_required_text(
                            &format!("candidate_actions[{index}].tool_name"),
                            &text,
                        )
                    })
                    .transpose()?,
            })
        })
        .collect()
}

fn normalize_action_kind(field: &str, value: &str) -> Result<String, ErrorData> {
    let value = normalize_required_text(field, value)?;
    if !ACTION_KINDS.contains(&value.as_str()) {
        return Err(params_error(format!(
            "{field} must be one of {}",
            ACTION_KINDS.join(", ")
        )));
    }
    Ok(value)
}

fn normalize_source_refs(
    values: Vec<EverQuestPredictiveSourceRef>,
) -> Result<Vec<EverQuestPredictiveSourceRef>, ErrorData> {
    if values.len() > MAX_SOURCE_REFS {
        return Err(params_error(format!(
            "source_refs must contain <= {MAX_SOURCE_REFS} values"
        )));
    }
    values
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            Ok(EverQuestPredictiveSourceRef {
                kind: normalize_required_text(&format!("source_refs[{index}].kind"), &value.kind)?,
                row_key: value
                    .row_key
                    .map(|text| {
                        normalize_required_text(&format!("source_refs[{index}].row_key"), &text)
                    })
                    .transpose()?,
                path: value
                    .path
                    .map(|text| {
                        normalize_required_text(&format!("source_refs[{index}].path"), &text)
                    })
                    .transpose()?,
                start_offset: value.start_offset,
                next_offset: value.next_offset,
                content_sha256: value
                    .content_sha256
                    .map(|text| {
                        normalize_required_text(
                            &format!("source_refs[{index}].content_sha256"),
                            &text,
                        )
                    })
                    .transpose()?,
                note: value
                    .note
                    .map(|text| {
                        normalize_required_text(&format!("source_refs[{index}].note"), &text)
                    })
                    .transpose()?,
            })
        })
        .collect()
}

fn normalize_text_vec(
    field: &str,
    values: Vec<String>,
    max_values: usize,
) -> Result<Vec<String>, ErrorData> {
    if values.len() > max_values {
        return Err(params_error(format!(
            "{field} must contain <= {max_values} values"
        )));
    }
    values
        .into_iter()
        .enumerate()
        .map(|(index, value)| normalize_required_text(&format!("{field}[{index}]"), &value))
        .collect()
}

fn validate_profile_id(value: &str) -> Result<String, ErrorData> {
    let value = value.trim();
    if value != EVERQUEST_PROFILE_ID {
        return Err(params_error(format!(
            "profile_id must be {EVERQUEST_PROFILE_ID:?}; got {value:?}"
        )));
    }
    Ok(value.to_owned())
}

fn validate_id(field: &str, value: &str) -> Result<String, ErrorData> {
    let value = normalize_required_text(field, value)?;
    if value.len() > MAX_ID_BYTES {
        return Err(params_error(format!(
            "{field} must be <= {MAX_ID_BYTES} bytes"
        )));
    }
    if !value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
    {
        return Err(params_error(format!(
            "{field} must contain only ASCII letters, digits, '.', '_', or '-'"
        )));
    }
    Ok(value)
}

fn normalize_required_text(field: &str, value: &str) -> Result<String, ErrorData> {
    let value = value.trim();
    if value.is_empty() {
        return Err(params_error(format!("{field} must not be empty")));
    }
    if value.len() > MAX_TEXT_BYTES {
        return Err(params_error(format!(
            "{field} must be <= {MAX_TEXT_BYTES} bytes"
        )));
    }
    if value.chars().any(char::is_control) {
        return Err(params_error(format!(
            "{field} must not contain control characters"
        )));
    }
    Ok(value.to_owned())
}

fn validate_probability(field: &str, value: f32) -> Result<f32, ErrorData> {
    if !(0.0..=1.0).contains(&value) || !value.is_finite() {
        return Err(params_error(format!("{field} must be between 0.0 and 1.0")));
    }
    Ok(value)
}

fn dedupe_strings(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn enum_string<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".to_owned())
}

fn model_hash(row: &EverQuestPredictiveModelRow) -> String {
    #[derive(Serialize)]
    struct HashPayload<'a> {
        schema_version: u32,
        row_kind: &'a str,
        profile_id: &'a str,
        model_id: &'a str,
        algorithm: &'a str,
        status: &'a str,
        training: &'a EverQuestPredictiveTrainingSummary,
        source_trajectory_keys: &'a [String],
        source_transition_keys: &'a [String],
        entries: &'a [EverQuestPredictiveModelEntry],
        action_fallback_entries: &'a [EverQuestPredictiveModelEntry],
        global_fallback: &'a Option<EverQuestPredictiveModelEntry>,
        limitations: &'a [String],
    }
    let payload = HashPayload {
        schema_version: row.schema_version,
        row_kind: &row.row_kind,
        profile_id: &row.profile_id,
        model_id: &row.model_id,
        algorithm: &row.algorithm,
        status: &row.status,
        training: &row.training,
        source_trajectory_keys: &row.source_trajectory_keys,
        source_transition_keys: &row.source_transition_keys,
        entries: &row.entries,
        action_fallback_entries: &row.action_fallback_entries,
        global_fallback: &row.global_fallback,
        limitations: &row.limitations,
    };
    sha256_hex(&serde_json::to_vec(&payload).unwrap_or_default())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex_encode(&digest)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn evidence_boundary() -> EverQuestPredictiveEvidenceBoundary {
    EverQuestPredictiveEvidenceBoundary {
        supports_planning_quality: true,
        manual_fsv_required_for_runtime: true,
        is_fsv: false,
        redacted: true,
        no_game_input_executed: true,
        note: "Predictive model rows support attended planning only; gameplay claims still require manual physical EQ UI/log/storage FSV.".to_owned(),
    }
}

fn model_row_key(profile_id: &str, model_id: &str) -> String {
    format!("{MODEL_ROW_PREFIX}/{profile_id}/{model_id}")
}

fn prediction_row_key(profile_id: &str, prediction_id: &str) -> String {
    format!("{PREDICTION_ROW_PREFIX}/{profile_id}/{prediction_id}")
}

fn trajectory_prefix(profile_id: &str) -> String {
    format!("{TRAJECTORY_ROW_PREFIX}/{profile_id}/")
}

fn ratio(numerator: u32, denominator: u32) -> f32 {
    if denominator == 0 {
        0.0
    } else {
        let numerator = u16::try_from(numerator).unwrap_or(u16::MAX);
        let denominator = u16::try_from(denominator).unwrap_or(u16::MAX);
        f32::from(numerator) / f32::from(denominator)
    }
}

fn len_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn len_to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn params_error(message: impl Into<String>) -> ErrorData {
    mcp_error(error_codes::TOOL_PARAMS_INVALID, message)
}

fn default_profile_id() -> String {
    EVERQUEST_PROFILE_ID.to_owned()
}

const fn default_max_trajectories() -> u32 {
    DEFAULT_MAX_TRAJECTORIES
}

const fn default_min_transition_support() -> u32 {
    DEFAULT_MIN_TRANSITION_SUPPORT
}

const fn default_min_confidence() -> f32 {
    DEFAULT_MIN_CONFIDENCE
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(zone: &str) -> EverQuestPredictiveTarget {
        EverQuestPredictiveTarget {
            outcome_kind: "same_zone".to_owned(),
            next_zone_short_name: zone.to_owned(),
            next_coord_bucket: "x0_y0_z0".to_owned(),
            log_event_kind: "loc".to_owned(),
            con_delta: "no_change".to_owned(),
            surprise: false,
        }
    }

    #[test]
    fn majority_bucket_confidence_reflects_conflict() {
        let mut bucket = BucketStats::default();
        add_example(
            &mut bucket,
            &TrainingExample {
                state_signature: "s".to_owned(),
                action_kind: "loc_probe".to_owned(),
                target: target("neriaka"),
                transition_id: "t1".to_owned(),
                domain_transition_key: "k1".to_owned(),
            },
        );
        add_example(
            &mut bucket,
            &TrainingExample {
                state_signature: "s".to_owned(),
                action_kind: "loc_probe".to_owned(),
                target: target("neriaka"),
                transition_id: "t2".to_owned(),
                domain_transition_key: "k2".to_owned(),
            },
        );
        add_example(
            &mut bucket,
            &TrainingExample {
                state_signature: "s".to_owned(),
                action_kind: "loc_probe".to_owned(),
                target: target("nektulos"),
                transition_id: "t3".to_owned(),
                domain_transition_key: "k3".to_owned(),
            },
        );
        let entry = best_entry(
            "state_action",
            "s".to_owned(),
            "loc_probe".to_owned(),
            bucket,
        )
        .unwrap();
        assert_eq!(entry.sample_count, 3);
        assert_eq!(entry.winning_count, 2);
        assert!((entry.confidence - 0.666_666_7).abs() < 0.000_1);
    }

    #[test]
    fn stale_model_hash_abstains() {
        let model = EverQuestPredictiveModelRow {
            schema_version: SCHEMA_VERSION,
            row_kind: "everquest_predictive_model".to_owned(),
            profile_id: EVERQUEST_PROFILE_ID.to_owned(),
            model_id: "m".to_owned(),
            row_key: model_row_key(EVERQUEST_PROFILE_ID, "m"),
            trained_at: Utc::now(),
            algorithm: "action_conditioned_markov_baseline_v1".to_owned(),
            status: "trained".to_owned(),
            model_hash: "abc".to_owned(),
            training: EverQuestPredictiveTrainingSummary {
                trajectory_count: 1,
                transition_count: 1,
                accepted_transition_count: 1,
                rejected_transition_count: 0,
                scan_truncated: false,
                min_transition_support: 1,
                min_confidence: 0.60,
                competence_floor: 0.60,
                stretch_target: 0.80,
                conflicting_bucket_count: 0,
            },
            source_trajectory_keys: Vec::new(),
            source_transition_keys: Vec::new(),
            entries: Vec::new(),
            action_fallback_entries: Vec::new(),
            global_fallback: None,
            source_refs: Vec::new(),
            limitations: Vec::new(),
            evidence_boundary: evidence_boundary(),
        };
        let row = build_prediction_row(
            NormalizedPredictParams {
                prediction_id: "p".to_owned(),
                profile_id: EVERQUEST_PROFILE_ID.to_owned(),
                model_id: "m".to_owned(),
                state_row_key: "state".to_owned(),
                candidate_actions: Vec::new(),
                expected_model_hash: Some("different".to_owned()),
                min_transition_support: 1,
                min_confidence: 0.60,
                source_refs: Vec::new(),
                limitations: Vec::new(),
            },
            model,
            "s".to_owned(),
        );
        assert_eq!(row.decision, "abstain_stale_model_hash");
        assert!(row.abstain);
    }

    #[test]
    fn exact_state_action_beats_global_fallback_tie() {
        let model = EverQuestPredictiveModelRow {
            schema_version: SCHEMA_VERSION,
            row_kind: "everquest_predictive_model".to_owned(),
            profile_id: EVERQUEST_PROFILE_ID.to_owned(),
            model_id: "m".to_owned(),
            row_key: model_row_key(EVERQUEST_PROFILE_ID, "m"),
            trained_at: Utc::now(),
            algorithm: "action_conditioned_markov_baseline_v1".to_owned(),
            status: "trained".to_owned(),
            model_hash: "abc".to_owned(),
            training: EverQuestPredictiveTrainingSummary {
                trajectory_count: 1,
                transition_count: 1,
                accepted_transition_count: 1,
                rejected_transition_count: 0,
                scan_truncated: false,
                min_transition_support: 1,
                min_confidence: 0.60,
                competence_floor: 0.60,
                stretch_target: 0.80,
                conflicting_bucket_count: 0,
            },
            source_trajectory_keys: Vec::new(),
            source_transition_keys: Vec::new(),
            entries: vec![EverQuestPredictiveModelEntry {
                scope: "state_action".to_owned(),
                state_signature: "s".to_owned(),
                action_kind: "loc_probe".to_owned(),
                sample_count: 1,
                winning_count: 1,
                confidence: 1.0,
                target: target("neriaka"),
                source_transition_ids: vec!["t1".to_owned()],
            }],
            action_fallback_entries: Vec::new(),
            global_fallback: Some(EverQuestPredictiveModelEntry {
                scope: "global_fallback".to_owned(),
                state_signature: "*".to_owned(),
                action_kind: "*".to_owned(),
                sample_count: 1,
                winning_count: 1,
                confidence: 1.0,
                target: target("neriaka"),
                source_transition_ids: vec!["t1".to_owned()],
            }),
            source_refs: Vec::new(),
            limitations: Vec::new(),
            evidence_boundary: evidence_boundary(),
        };
        let row = build_prediction_row(
            NormalizedPredictParams {
                prediction_id: "p".to_owned(),
                profile_id: EVERQUEST_PROFILE_ID.to_owned(),
                model_id: "m".to_owned(),
                state_row_key: "state".to_owned(),
                candidate_actions: vec![
                    EverQuestPredictiveCandidateAction {
                        action_kind: "bounded_move".to_owned(),
                        alias: None,
                        tool_name: None,
                    },
                    EverQuestPredictiveCandidateAction {
                        action_kind: "loc_probe".to_owned(),
                        alias: None,
                        tool_name: None,
                    },
                ],
                expected_model_hash: None,
                min_transition_support: 1,
                min_confidence: 0.60,
                source_refs: Vec::new(),
                limitations: Vec::new(),
            },
            model,
            "s".to_owned(),
        );
        let selected = row.selected.expect("selected candidate");
        assert_eq!(selected.action_kind, "loc_probe");
        assert_eq!(selected.source_scope, "state_action");
    }

    #[test]
    fn no_data_status_is_explicit() {
        let row = build_model_row(
            NormalizedFitParams {
                model_id: "m".to_owned(),
                profile_id: EVERQUEST_PROFILE_ID.to_owned(),
                trajectory_row_keys: Vec::new(),
                max_trajectories: 1,
                min_transition_support: 1,
                min_confidence: 0.60,
                source_refs: Vec::new(),
                limitations: Vec::new(),
            },
            &[],
            &[],
            false,
        );
        assert_eq!(row.status, "no_verified_trajectories");
        assert!(row.entries.is_empty());
        assert!(
            row.limitations
                .contains(&"no_verified_trajectories_available".to_owned())
        );
        assert!(!row.model_hash.is_empty());
    }

    #[test]
    fn sparse_data_status_is_explicit() {
        let examples = vec![TrainingExample {
            state_signature: "s".to_owned(),
            action_kind: "loc_probe".to_owned(),
            target: target("neriaka"),
            transition_id: "t1".to_owned(),
            domain_transition_key: "k1".to_owned(),
        }];
        let row = build_model_row(
            NormalizedFitParams {
                model_id: "m".to_owned(),
                profile_id: EVERQUEST_PROFILE_ID.to_owned(),
                trajectory_row_keys: Vec::new(),
                max_trajectories: 1,
                min_transition_support: 2,
                min_confidence: 0.60,
                source_refs: Vec::new(),
                limitations: Vec::new(),
            },
            &[],
            &examples,
            false,
        );
        assert_eq!(row.status, "insufficient_transition_support");
        assert_eq!(row.entries[0].sample_count, 1);
    }

    #[test]
    fn source_refs_accept_absolute_physical_paths() {
        let refs = normalize_source_refs(vec![EverQuestPredictiveSourceRef {
            kind: "physical_eq_log_bytes".to_owned(),
            row_key: None,
            path: Some(
                "C:\\Users\\hotra\\AppData\\Local\\synapse\\everquest\\fsv\\log.txt".to_owned(),
            ),
            start_offset: Some(0),
            next_offset: Some(10),
            content_sha256: None,
            note: Some("physical source of truth path".to_owned()),
        }])
        .unwrap();
        assert_eq!(
            refs[0].path.as_deref(),
            Some("C:\\Users\\hotra\\AppData\\Local\\synapse\\everquest\\fsv\\log.txt")
        );
    }
}
