use std::{sync::Arc, time::Instant};

use rmcp::ErrorData;
use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use synapse_action::{
    ActionBackend, ActionError, ActionHandle, EmitState, RecordedInput, RecordingBackend,
};
use synapse_core::{Action, Backend, ElementId, KeystrokeDynamics, KeystrokeNaturalParams};

use crate::m1::mcp_error;

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActTypeParams {
    pub text: String,
    #[serde(default)]
    pub into_element: Option<ElementId>,
    #[serde(default = "default_type_dynamics")]
    #[schemars(default = "default_type_dynamics")]
    pub dynamics: TypeDynamics,
    #[serde(default = "default_linear_ms_per_char")]
    #[schemars(default = "default_linear_ms_per_char")]
    pub linear_ms_per_char: u32,
    #[serde(default = "default_use_scancodes")]
    #[schemars(default = "default_use_scancodes")]
    pub use_scancodes: bool,
    #[serde(default = "default_press_enter_after")]
    #[schemars(default = "default_press_enter_after")]
    pub press_enter_after: bool,
    #[serde(default = "default_type_backend")]
    #[schemars(default = "default_type_backend")]
    pub backend: TypeBackend,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TypeDynamics {
    Burst,
    Linear,
    Natural,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TypeBackend {
    Software,
    Hardware,
    Auto,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActTypeResponse {
    pub ok: bool,
    pub chars_typed: u32,
    pub elapsed_ms: u32,
}

pub async fn act_type_with_handle(
    handle: ActionHandle,
    recording: Option<Arc<RecordingBackend>>,
    params: ActTypeParams,
) -> Result<ActTypeResponse, ErrorData> {
    let started = Instant::now();
    let action = action_from_type_params(&params)?;
    let chars_typed = match &action {
        Action::TypeText { text, .. } => char_count(text)?,
        _ => unreachable!("act_type builds only TypeText actions"),
    };

    if let Some(recording) = recording {
        execute_recording(&recording, &action)?;
    } else {
        handle
            .execute(action)
            .await
            .map_err(|error| action_error_to_mcp(&error))?;
    }

    Ok(ActTypeResponse {
        ok: true,
        chars_typed,
        elapsed_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
    })
}

pub fn action_from_type_params(params: &ActTypeParams) -> Result<Action, ErrorData> {
    validate_type_params(params)?;
    Ok(Action::TypeText {
        text: emitted_text(params),
        dynamics: params
            .dynamics
            .to_keystroke_dynamics(params.linear_ms_per_char),
        backend: params.backend.to_backend(),
    })
}

impl TypeDynamics {
    const fn to_keystroke_dynamics(self, linear_ms_per_char: u32) -> KeystrokeDynamics {
        match self {
            Self::Burst => KeystrokeDynamics::Burst,
            Self::Linear => KeystrokeDynamics::Linear {
                ms_per_char: linear_ms_per_char,
            },
            Self::Natural => KeystrokeDynamics::Natural {
                params: KeystrokeNaturalParams::FAST,
            },
        }
    }
}

impl TypeBackend {
    const fn to_backend(self) -> Backend {
        match self {
            Self::Software => Backend::Software,
            Self::Hardware => Backend::Hardware,
            Self::Auto => Backend::Auto,
        }
    }
}

fn validate_type_params(params: &ActTypeParams) -> Result<(), ErrorData> {
    if let Some(element_id) = &params.into_element {
        return Err(action_error_to_mcp(&ActionError::BackendUnavailable {
            detail: format!(
                "act_type into_element target {element_id} requires the dedicated focus and clear wiring issue"
            ),
        }));
    }
    if params.use_scancodes {
        return Err(action_error_to_mcp(&ActionError::BackendUnavailable {
            detail: "act_type use_scancodes=true is not wired for the M2 unicode typing path"
                .to_owned(),
        }));
    }
    Ok(())
}

fn emitted_text(params: &ActTypeParams) -> String {
    if params.press_enter_after {
        let mut text = params.text.clone();
        text.push('\n');
        text
    } else {
        params.text.clone()
    }
}

fn char_count(text: &str) -> Result<u32, ErrorData> {
    u32::try_from(text.chars().count()).map_err(|_err| {
        mcp_error(
            synapse_core::error_codes::TOOL_PARAMS_INVALID,
            "act_type text has more than u32::MAX chars",
        )
    })
}

fn execute_recording(recording: &RecordingBackend, action: &Action) -> Result<(), ErrorData> {
    let before_events = recording.events();
    let before_event_count = before_events.len();
    let mut emit_state = EmitState::new();
    recording
        .execute(action, &mut emit_state)
        .map_err(|error| action_error_to_mcp(&error))?;
    let after_events = recording.events();
    let new_events = &after_events[before_event_count..];
    let recorded_ikis = recorded_ikis(new_events);
    tracing::info!(
        code = "M2_ACT_TYPE_RECORDING_READBACK",
        kind = "act_type",
        before_event_count,
        after_event_count = after_events.len(),
        new_event_count = new_events.len(),
        ?recorded_ikis,
        ?new_events,
        "readback=recording_backend tool=act_type after_events_readback"
    );
    Ok(())
}

fn recorded_ikis(events: &[RecordedInput]) -> Vec<u32> {
    events
        .iter()
        .filter_map(|event| match event {
            RecordedInput::DelayMs { ms } => Some(*ms),
            _ => None,
        })
        .collect()
}

fn action_error_to_mcp(error: &ActionError) -> ErrorData {
    mcp_error(error.code(), error.to_string())
}

const fn default_type_dynamics() -> TypeDynamics {
    TypeDynamics::Natural
}

const fn default_linear_ms_per_char() -> u32 {
    30
}

const fn default_type_backend() -> TypeBackend {
    TypeBackend::Auto
}

const fn default_use_scancodes() -> bool {
    false
}

const fn default_press_enter_after() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use synapse_action::{ActionEmitter, RecordedInput, sample_typing_schedule};
    use synapse_core::KeystrokeNaturalParams;

    use super::{
        ActTypeParams, TypeBackend, TypeDynamics, act_type_with_handle, default_linear_ms_per_char,
        default_press_enter_after, default_type_backend, default_type_dynamics,
        default_use_scancodes, recorded_ikis,
    };

    #[tokio::test]
    async fn recording_backend_readback_uses_natural_fast_ikis() {
        let (handle, _snapshot_handle, _emitter) = ActionEmitter::channel();
        let recording = Arc::new(synapse_action::RecordingBackend::new());
        let text = "Hello world.";
        let params = ActTypeParams {
            text: text.to_owned(),
            into_element: None,
            dynamics: default_type_dynamics(),
            linear_ms_per_char: default_linear_ms_per_char(),
            use_scancodes: false,
            press_enter_after: false,
            backend: default_type_backend(),
        };
        let before = recording.events();
        println!("readback=act_type_recording edge=natural_fast before={before:?}");

        let response = act_type_with_handle(handle, Some(Arc::clone(&recording)), params)
            .await
            .unwrap_or_else(|error| panic!("act_type recording should succeed: {error}"));
        let after = recording.events();
        let actual_ikis = recorded_ikis(&after);
        let expected_ikis: Vec<u32> = sample_typing_schedule(
            text,
            &TypeDynamics::Natural.to_keystroke_dynamics(default_linear_ms_per_char()),
            None,
        )
        .into_iter()
        .filter_map(|event| (event.iki_ms_before > 0).then_some(event.iki_ms_before))
        .collect();
        println!(
            "readback=act_type_recording edge=natural_fast after={after:?} expected_ikis={expected_ikis:?} actual_ikis={actual_ikis:?} chars_typed={}",
            response.chars_typed
        );

        assert!(response.ok);
        assert_eq!(response.chars_typed, 12);
        assert_eq!(actual_ikis, expected_ikis);
        assert_eq!(
            TypeDynamics::Natural.to_keystroke_dynamics(default_linear_ms_per_char()),
            synapse_core::KeystrokeDynamics::Natural {
                params: KeystrokeNaturalParams::FAST
            }
        );
    }

    #[test]
    fn defaults_are_issue_required_values() {
        assert_eq!(default_type_dynamics(), TypeDynamics::Natural);
        assert_eq!(default_linear_ms_per_char(), 30);
        assert_eq!(default_type_backend(), TypeBackend::Auto);
        assert!(!default_use_scancodes());
        assert!(!default_press_enter_after());
    }

    #[test]
    fn recorded_ikis_only_reads_delay_events() {
        let before = vec![
            RecordedInput::DelayMs { ms: 17 },
            RecordedInput::DelayMs { ms: 0 },
        ];
        let after = recorded_ikis(&before);
        println!("readback=act_type_recording edge=iki_readback before={before:?} after={after:?}");
        assert_eq!(after, [17, 0]);
    }
}
