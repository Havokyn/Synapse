//! Suggestion-engine MCP tools (#858) — own router, merged in `server.rs`.
//!
//! `suggestion_tick` runs one decision pass (expire/abandon + gated creation);
//! `suggestion_list` reads the persisted suggestion rows. Thin wrappers around
//! [`crate::m3::suggestions`], which owns the CF_KV truth and the anti-Clippy
//! gates.

use super::{ErrorData, Json, Parameters, SynapseService, tool, tool_router};

use crate::m3::suggestions::{
    SuggestionListParams, SuggestionListResponse, SuggestionTickParams, SuggestionTickResponse,
    list_suggestions, required_permissions_list, required_permissions_tick, suggestion_tick,
};

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
}
