use std::{sync::Arc, time::Instant};

use rmcp::ErrorData;
use synapse_action::{ActionError, ActionHandle, RecordingBackend, cached_double_click_timing};
use synapse_core::{Action, Backend, ButtonAction, MouseTarget, Point, error_codes};

use crate::m1::mcp_error;

mod element;
mod record;
mod schema;
#[cfg(test)]
mod tests;

use schema::ActClickTarget;
pub use schema::{ActClickParams, ActClickResponse};

const MAX_CLICK_HOLD_MS: u32 = 30_000;

pub async fn act_click_with_handle(
    handle: ActionHandle,
    recording: Option<Arc<RecordingBackend>>,
    params: ActClickParams,
) -> Result<ActClickResponse, ErrorData> {
    validate_click_params(&params)?;
    let started = Instant::now();
    let double_click_timing = cached_double_click_timing();
    // #686: a web element id (cdcd sentinel) routes through CDP instead of UIA.
    #[cfg(windows)]
    if let ActClickTarget::Element(element) = &params.target {
        if let Some(backend) = synapse_a11y::cdp_backend_from_element_id(&element.element_id) {
            return execute_cdp_click(&params, element, backend, double_click_timing, started)
                .await;
        }
    }
    if let ActClickTarget::Element(element) = &params.target {
        return element::execute_element_click(
            handle,
            &params,
            element,
            recording.as_deref(),
            double_click_timing,
            started,
        )
        .await;
    }

    let target = point_mouse_target(&params.target)?;
    let mut actions = Vec::with_capacity(usize::from(params.clicks) + 1);
    actions.push(Action::MouseMove {
        to: target,
        curve: params.curve.to_aim_curve(),
        duration_ms: params.duration_ms,
        backend: params.backend,
    });
    for _ in 0..params.clicks {
        actions.push(Action::MouseButton {
            button: params.button,
            action: ButtonAction::Press,
            hold_ms: params.hold_ms,
            backend: params.backend,
        });
    }

    if let Some(recording) = recording {
        record::execute_recording(&recording, &actions, params.clicks, double_click_timing).await?;
    } else {
        record::execute_actor_actions(handle, actions, double_click_timing).await?;
    }

    Ok(ActClickResponse {
        ok: true,
        used_invoke_pattern: false,
        backend_used: backend_used_name(params.backend).to_owned(),
        press_hold_ms: params.hold_ms,
        double_click_window_ms: double_click_timing.window_ms,
        inter_click_delay_ms: double_click_timing.inter_click_delay_ms,
        elapsed_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
    })
}

/// Routes a click on a CDP web element id through CDP (#686): resolve the
/// browser's debug endpoint from the element's window, scroll the node into
/// view, and dispatch the click in viewport coordinates. Fail-loud if the
/// endpoint is gone or the node cannot be resolved.
#[cfg(windows)]
async fn execute_cdp_click(
    params: &ActClickParams,
    element: &schema::ActClickElementTarget,
    backend_node_id: i64,
    double_click_timing: synapse_action::DoubleClickTiming,
    started: Instant,
) -> Result<ActClickResponse, ErrorData> {
    use synapse_core::MouseButton;

    let hwnd = element
        .element_id
        .parts()
        .map_err(|err| {
            mcp_error(
                error_codes::ACTION_ELEMENT_NOT_RESOLVED,
                format!("web element id is malformed: {err}"),
            )
        })?
        .hwnd;
    let endpoint = synapse_a11y::endpoint_for_window(hwnd).ok_or_else(|| {
        mcp_error(
            error_codes::A11Y_CDP_UNREACHABLE,
            format!(
                "no reachable CDP endpoint for web element {} (browser closed or debug port gone)",
                element.element_id
            ),
        )
    })?;
    // Foreground window title disambiguates which tab owns the per-document node.
    let title_hint = synapse_a11y::foreground_context(hwnd)
        .map(|context| context.window_title)
        .unwrap_or_default();
    let button = match params.button {
        MouseButton::Left => synapse_a11y::CdpMouseButton::Left,
        MouseButton::Right => synapse_a11y::CdpMouseButton::Right,
        MouseButton::Middle => synapse_a11y::CdpMouseButton::Middle,
        other => {
            return Err(mcp_error(
                error_codes::TOOL_PARAMS_INVALID,
                format!("act_click button {other:?} is not supported for web (CDP) elements"),
            ));
        }
    };

    synapse_a11y::cdp_click_node(
        &endpoint,
        &title_hint,
        backend_node_id,
        button,
        i64::from(params.clicks),
    )
    .await
    .map_err(|err| action_error_to_mcp(&a11y_to_action_error(&err)))?;

    Ok(ActClickResponse {
        ok: true,
        used_invoke_pattern: false,
        backend_used: "cdp".to_owned(),
        press_hold_ms: params.hold_ms,
        double_click_window_ms: double_click_timing.window_ms,
        inter_click_delay_ms: double_click_timing.inter_click_delay_ms,
        elapsed_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
    })
}

/// Maps an a11y CDP error to an action error so it surfaces with the same shape
/// as other action failures.
#[cfg(windows)]
fn a11y_to_action_error(err: &synapse_a11y::A11yError) -> ActionError {
    ActionError::TargetInvalid {
        detail: format!("{} ({})", err, err.code()),
    }
}

fn validate_click_params(params: &ActClickParams) -> Result<(), ErrorData> {
    if !(1..=3).contains(&params.clicks) {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            format!("act_click clicks must be in 1..=3, got {}", params.clicks),
        ));
    }
    if params.hold_ms == 0 {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            "act_click hold_ms must be at least 1",
        ));
    }
    if params.hold_ms > MAX_CLICK_HOLD_MS {
        return Err(action_error_to_mcp(&ActionError::HoldExceededMax {
            detail: format!(
                "act_click hold_ms {} exceeds max {MAX_CLICK_HOLD_MS}",
                params.hold_ms
            ),
        }));
    }
    if !params.modifiers.is_empty() {
        return Err(action_error_to_mcp(&ActionError::BackendUnavailable {
            detail: "act_click modifiers are not wired in the M2 click schema slice".to_owned(),
        }));
    }
    Ok(())
}

fn point_mouse_target(target: &ActClickTarget) -> Result<MouseTarget, ErrorData> {
    match target {
        ActClickTarget::Point(point) => Ok(MouseTarget::Screen {
            point: Point {
                x: point.x,
                y: point.y,
            },
        }),
        ActClickTarget::Element(element) => Err(action_error_to_mcp(&ActionError::TargetInvalid {
            detail: format!(
                "act_click element target {} reached the point-target path unexpectedly",
                element.element_id
            ),
        })),
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
