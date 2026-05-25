use std::time::Duration;

use synapse_core::{Action, ReflexId};

use super::RuntimeState;
use crate::{
    error::{ReflexError, ReflexResult},
    kinds::combo::{ComboContext, ComboController, ComboParams},
};

pub(super) fn step_active_combos(
    runtime: &mut RuntimeState,
    elapsed: Duration,
    dispatched_actions: &mut usize,
    dispatch_blocked: &mut bool,
) {
    for combo in &mut runtime.active_combos {
        match combo.step_dispatch(
            &ComboContext {
                tick_elapsed: elapsed,
            },
            &runtime.action_handle,
            &runtime.event_bus,
        ) {
            Ok(output) => {
                *dispatched_actions = dispatched_actions.saturating_add(output.action_count());
            }
            Err(error) => {
                *dispatch_blocked = true;
                tracing::warn!(
                    component = "reflex_scheduler",
                    error_code = error.code(),
                    detail = %error,
                    "combo action dispatch blocked"
                );
                break;
            }
        }
    }
    if !*dispatch_blocked {
        runtime.active_combos.retain(|combo| !combo.is_completed());
    }
}

pub(super) fn dispatch_reflex_action(
    runtime: &mut RuntimeState,
    reflex_id: &ReflexId,
    action: Action,
) -> ReflexResult<usize> {
    match action {
        Action::Combo { steps, backend } => {
            let mut combo =
                ComboController::new(reflex_id.clone(), ComboParams::new(steps, backend));
            let result = combo.start_dispatch(&runtime.action_handle, &runtime.event_bus);
            let completed = combo.is_completed();
            let actions = match &result {
                Ok(output) => output.action_count(),
                Err(_error) => 0,
            };
            if !completed {
                runtime.active_combos.push(combo);
            }
            result?;
            Ok(actions)
        }
        action => {
            runtime.action_handle.try_execute(action).map_err(|error| {
                ReflexError::ParamsInvalid {
                    detail: format!("scheduler action dispatch failed: {error}"),
                }
            })?;
            Ok(1)
        }
    }
}
