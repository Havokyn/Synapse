//! `routine_label_export` MCP tool (#851) — its own router, merged in
//! `server.rs`.
//!
//! Kept self-contained (not folded into `m3_tools.rs`) so the tool surface
//! composes additively without contending on the shared M3 tool router. The
//! handler is a thin wrapper: permission gate, structured log line, then
//! delegate to [`crate::m3::routines::export_routine_label`], which owns the
//! storage reads and the compact-bundle construction.

use super::{ErrorData, Json, Parameters, SynapseService, tool, tool_router};

use crate::m3::routines::{
    RoutineLabelExportParams, RoutineLabelExportResponse, export_routine_label,
    required_permissions_label_export,
};

#[tool_router(router = routine_labeling_tool_router, vis = "pub(super)")]
impl SynapseService {
    #[tool(
        description = "Export a compact, prompt-ready naming bundle for ONE mined routine (#851), so an LLM can turn its machine identity (e.g. chrome:mail.google.com → excel:report.xlsx → teams) into a human name + description. Returns the ordered step template, schedule signature (label, day-of-week class, mean start time), support/confidence, the current operator label/lifecycle, and the most-recent sample occurrences (newest first, each with stable episode ids resolvable via episode_get for deeper evidence) — plus a ready-to-use `prompt` block and a `writeback_hint` showing the exact routine_update rename call to persist the chosen label. Read-only. Errors honestly (ROUTINE_NOT_MINED) when the id has no CF_ROUTINES template to name."
    )]
    pub async fn routine_label_export(
        &self,
        params: Parameters<RoutineLabelExportParams>,
    ) -> Result<Json<RoutineLabelExportResponse>, ErrorData> {
        tracing::info!(
            code = "MCP_TOOL_INVOCATION",
            kind = "routine_label_export",
            routine_id = %params.0.routine_id,
            max_samples = params.0.max_samples,
            "tool.invocation kind=routine_label_export"
        );
        self.require_m3_permissions(
            "routine_label_export",
            &required_permissions_label_export(&params.0),
        )?;
        let db = self.m3_storage()?;
        export_routine_label(&db, &params.0).map(Json)
    }
}
