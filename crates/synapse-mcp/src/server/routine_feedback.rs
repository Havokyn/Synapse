//! `routine_feedback` MCP tool (#856) — its own router, merged in `server.rs`.
//!
//! Thin wrapper: permission gate, structured log line, then delegate to
//! [`crate::m3::routines::record_routine_feedback`], which owns the
//! `CF_ROUTINE_STATE` read-modify-write (flushed + read back), the escalating
//! decline cooldown, and the Wilson-bound acceptance confidence.

use rmcp::{RoleServer, service::RequestContext};

use super::{ErrorData, Json, Parameters, SynapseService, tool, tool_router};

use crate::m3::routines::{
    RoutineFeedbackParams, RoutineFeedbackResponse, record_routine_feedback,
    required_permissions_feedback,
};

#[tool_router(router = routine_feedback_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(
        description = "Record how a surfaced routine suggestion resolved (#856) and fold it into the routine's effective confidence + escalating decline cooldown, persisted in CF_ROUTINE_STATE. outcome: accepted (success — resets the decline streak and clears the cooldown), declined / ignored_timeout (escalate the cooldown geometrically: base 1h ×6 per consecutive non-accept, capped 14d), or abandoned (INTENT_ABANDONED — recorded for provenance only, never suppresses). Returns the updated counters, the cooldown deadline + whether the routine is currently suppressed, the Wilson lower bound of the accept rate, and the mined confidence folded with it (effective_confidence) — the single-user 'gets better the more you use it' signal the suggestion engine (#858) gates on. Pass now_ts_ns to evaluate as of a past instant (replay/test). Synchronous flushed write with read-back; routine_inspect surfaces the full outcome history."
    )]
    pub async fn routine_feedback(
        &self,
        params: Parameters<RoutineFeedbackParams>,
        request_context: RequestContext<RoleServer>,
    ) -> Result<Json<RoutineFeedbackResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "routine_feedback",
            routine_id = %params.0.routine_id,
            outcome = ?params.0.outcome,
            "tool.invocation kind=routine_feedback"
        );
        self.require_m3_permissions(
            "routine_feedback",
            &required_permissions_feedback(&params.0),
        )?;
        let by_session = super::context::mcp_session_id_from_request_context(&request_context)?
            .unwrap_or_else(|| "stdio".to_owned());
        let db = self.m3_storage()?;
        record_routine_feedback(&db, &params.0, &by_session).map(Json)
    }
}
