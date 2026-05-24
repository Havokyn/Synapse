#[cfg(any(test, windows))]
use synapse_core::{Action, AimCurve, AimNaturalParams, Backend, ButtonAction, MouseTarget};
use synapse_core::{ElementId, MouseButton};

#[cfg(windows)]
use synapse_a11y::{UIElement, uiautomation::patterns::UIInvokePattern};

use crate::{ActionBackend, ActionError, ActionResult, EmitState};

use super::{CoordinateFallbackPlan, ElementClickOutcome};
#[cfg(windows)]
use crate::invoke::resolver::{invoke_pattern_failed, invoke_pattern_unavailable};

#[cfg(any(test, windows))]
pub(super) const FALLBACK_MOVE_DURATION_MS: u32 = 50;

#[cfg(windows)]
pub(super) fn invoke_resolved_element(
    element_id: &ElementId,
    element: &UIElement,
) -> ActionResult<()> {
    match try_invoke_resolved_element(element_id, element) {
        Ok(()) => Ok(()),
        Err(InvokeAttemptError::MissingPattern) => Err(invoke_pattern_unavailable(
            element_id,
            "pattern not available",
        )),
        Err(InvokeAttemptError::InvokeFailed(error)) => Err(error),
    }
}

#[cfg(windows)]
pub(super) fn try_invoke_resolved_element(
    element_id: &ElementId,
    element: &UIElement,
) -> Result<(), InvokeAttemptError> {
    let pattern: UIInvokePattern = element
        .get_pattern()
        .map_err(|_err| InvokeAttemptError::MissingPattern)?;

    pattern
        .invoke()
        .map_err(|err| InvokeAttemptError::InvokeFailed(invoke_pattern_failed(element_id, err)))
}

#[cfg(any(test, windows))]
pub(super) fn complete_click_attempt<B, F>(
    invoke_attempt: Result<(), InvokeAttemptError>,
    fallback_plan: F,
    backend: &B,
    state: &mut EmitState,
    button: MouseButton,
) -> ActionResult<ElementClickOutcome>
where
    B: ActionBackend,
    F: FnOnce() -> ActionResult<CoordinateFallbackPlan>,
{
    match invoke_attempt {
        Ok(()) => Ok(ElementClickOutcome::Invoked),
        Err(InvokeAttemptError::MissingPattern) => {
            let plan = fallback_plan()?;
            emit_coordinate_fallback_click(backend, state, button, plan)?;
            Ok(ElementClickOutcome::CoordinateFallback(plan))
        }
        Err(InvokeAttemptError::InvokeFailed(error)) => Err(error),
    }
}

#[cfg(any(test, windows))]
pub(super) enum InvokeAttemptError {
    MissingPattern,
    InvokeFailed(ActionError),
}

#[cfg(any(test, windows))]
pub(super) fn emit_coordinate_fallback_click<B>(
    backend: &B,
    state: &mut EmitState,
    button: MouseButton,
    plan: CoordinateFallbackPlan,
) -> ActionResult<()>
where
    B: ActionBackend,
{
    backend.execute(
        &Action::MouseMove {
            to: MouseTarget::Screen {
                point: plan.screen_point,
            },
            curve: AimCurve::Natural {
                params: AimNaturalParams::FAST,
            },
            duration_ms: FALLBACK_MOVE_DURATION_MS,
            backend: Backend::Software,
        },
        state,
    )?;
    backend.execute(
        &Action::MouseButton {
            button,
            action: ButtonAction::Down,
            hold_ms: 0,
            backend: Backend::Software,
        },
        state,
    )?;
    backend.execute(
        &Action::MouseButton {
            button,
            action: ButtonAction::Up,
            hold_ms: 0,
            backend: Backend::Software,
        },
        state,
    )
}
