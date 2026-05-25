use std::{sync::Arc, time::Instant};

use rmcp::ErrorData;
use synapse_action::{ActionError, ActionHandle, RecordingBackend};
use synapse_core::{Action, Backend, error_codes};
use tokio_util::sync::CancellationToken;

use crate::m1::mcp_error;

const MAX_HOLD_MS: u32 = 30_000;

mod keys;
mod live;
mod record;
mod schema;
#[cfg(test)]
mod tests;

pub use schema::{ActPressParams, ActPressResponse};

pub async fn act_press_with_handle(
    handle: ActionHandle,
    recording: Option<Arc<RecordingBackend>>,
    connection_closed_cancel: Option<CancellationToken>,
    params: ActPressParams,
) -> Result<ActPressResponse, ErrorData> {
    let started = Instant::now();
    let keys = keys::normalized_keys(&params.keys)?;
    let key_count = u32::try_from(keys.len()).map_err(|_err| {
        mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            "act_press keys length exceeds u32::MAX",
        )
    })?;
    let backend = params.backend.to_backend();
    validate_hold_ms(params.hold_ms)?;
    let action = press_action(keys.clone(), params.hold_ms, backend);

    if let Some(recording) = recording {
        record::execute_recording(&recording, &action)?;
    } else {
        live::execute_live_press_sequence(
            handle,
            keys,
            params.hold_ms,
            backend,
            connection_closed_cancel,
        )
        .await?;
    }

    Ok(ActPressResponse {
        ok: true,
        keys_pressed: key_count,
        elapsed_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
        backend_used: backend_used_name(backend).to_owned(),
    })
}

pub fn action_from_press_params(params: &ActPressParams) -> Result<Action, ErrorData> {
    validate_hold_ms(params.hold_ms)?;
    let keys = keys::normalized_keys(&params.keys)?;
    Ok(press_action(
        keys,
        params.hold_ms,
        params.backend.to_backend(),
    ))
}

fn validate_hold_ms(hold_ms: u32) -> Result<(), ErrorData> {
    if hold_ms == 0 {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            "act_press hold_ms must be at least 1",
        ));
    }
    if hold_ms > MAX_HOLD_MS {
        return Err(action_error_to_mcp(&ActionError::HoldExceededMax {
            detail: format!("act_press hold_ms {hold_ms} exceeds max {MAX_HOLD_MS}"),
        }));
    }
    Ok(())
}

fn press_action(keys: Vec<synapse_core::Key>, hold_ms: u32, backend: Backend) -> Action {
    if let [key] = keys.as_slice() {
        return Action::KeyPress {
            key: key.clone(),
            hold_ms,
            backend,
        };
    }
    Action::KeyChord {
        keys,
        hold_ms,
        backend,
    }
}

fn action_error_to_mcp(error: &ActionError) -> ErrorData {
    mcp_error(error.code(), error.to_string())
}

const fn backend_used_name(backend: Backend) -> &'static str {
    match backend {
        Backend::Auto | Backend::Software => "software",
        Backend::Vigem => "vigem",
        Backend::Hardware => "hardware",
    }
}
