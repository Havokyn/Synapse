use chrono::{DateTime, Utc};
use rmcp::{ErrorData, schemars::JsonSchema};
use serde::{Deserialize, Serialize};
use synapse_core::{EventFilter, error_codes};

use crate::{http::sse::SseState, m1::mcp_error};

use super::M3ToolStub;

const DEFAULT_BUFFER_SIZE: u32 = 4096;

const fn default_kinds() -> Vec<String> {
    Vec::new()
}

const fn default_snapshot_first() -> bool {
    false
}

const fn default_buffer_size() -> u32 {
    DEFAULT_BUFFER_SIZE
}

const fn default_filter() -> Option<EventFilter> {
    None
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SubscribeParams {
    #[serde(default = "default_kinds")]
    #[schemars(default = "default_kinds")]
    pub kinds: Vec<String>,
    #[serde(default = "default_filter")]
    #[schemars(default = "default_filter")]
    pub filter: Option<EventFilter>,
    #[serde(default = "default_snapshot_first")]
    #[schemars(default = "default_snapshot_first")]
    pub snapshot_first: bool,
    #[serde(default = "default_buffer_size")]
    #[schemars(default = "default_buffer_size")]
    pub buffer_size: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SubscribeResponse {
    pub subscription_id: String,
    pub started_at: DateTime<Utc>,
}

#[must_use]
pub const fn subscribe() -> M3ToolStub {
    M3ToolStub::new("subscribe")
}

#[must_use]
pub const fn subscribe_cancel() -> M3ToolStub {
    M3ToolStub::new("subscribe_cancel")
}

pub fn subscribe_to_events(
    sse_state: &SseState,
    params: &SubscribeParams,
) -> Result<SubscribeResponse, ErrorData> {
    if params.buffer_size != DEFAULT_BUFFER_SIZE {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            format!(
                "subscribe buffer_size must be {DEFAULT_BUFFER_SIZE}; got {}",
                params.buffer_size
            ),
        ));
    }
    let filter = params.filter.clone().unwrap_or(EventFilter::All);
    let subscription_id = sse_state
        .subscribe(filter, params.kinds.clone(), params.snapshot_first)
        .map_err(|error| mcp_error(error.code(), error.message()))?;
    Ok(SubscribeResponse {
        subscription_id,
        started_at: Utc::now(),
    })
}
