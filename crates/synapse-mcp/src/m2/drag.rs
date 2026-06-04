use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use rmcp::ErrorData;
use rmcp::schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use synapse_action::{
    ActionBackend, ActionError, ActionHandle, EmitState, RecordedInput, RecordingBackend,
};
#[cfg(windows)]
use synapse_core::Rect;
use synapse_core::{
    Action, AimCurve, AimNaturalParams, Backend, ElementId, Key, KeyCode, MouseButton, MouseTarget,
    Point, error_codes,
};

use crate::m1::mcp_error;

const DEFAULT_DRAG_DURATION_MS: u32 = 200;
const MODIFIER_RELEASE_SETTLE_MS: u64 = 200;

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActDragParams {
    pub from: ActDragTarget,
    pub to: ActDragTarget,
    #[serde(default = "default_drag_button")]
    #[schemars(default = "default_drag_button")]
    pub button: DragButton,
    #[serde(default = "default_drag_velocity_profile")]
    #[schemars(default = "default_drag_velocity_profile")]
    pub velocity_profile: DragVelocityProfile,
    #[serde(default)]
    #[schemars(skip)]
    pub curve: Option<LegacyDragCurve>,
    #[serde(default = "default_drag_duration_ms")]
    #[schemars(default = "default_drag_duration_ms")]
    pub duration_ms: u32,
    #[serde(default = "default_drag_backend")]
    #[schemars(default = "default_drag_backend")]
    pub backend: DragBackend,
    #[serde(default)]
    #[schemars(default)]
    pub modifiers: Vec<DragModifier>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(untagged)]
#[schemars(untagged)]
pub enum ActDragTarget {
    Point(ActDragPointTarget),
    Element(ActDragElementTarget),
}

#[derive(Copy, Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActDragPointTarget {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActDragElementTarget {
    pub element_id: ElementId,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DragButton {
    Left,
    Right,
    Middle,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DragVelocityProfile {
    Natural,
    Instant,
    Linear,
    EaseInOut,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegacyDragCurve {
    Natural,
    Instant,
    Linear,
    EaseInOut,
    Bezier,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DragBackend {
    Software,
    Hardware,
    Auto,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DragModifier {
    Ctrl,
    Shift,
    Alt,
    Super,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActDragResponse {
    pub ok: bool,
    pub button_used: DragButton,
    pub velocity_profile_used: DragVelocityProfile,
    pub deprecated_curve_alias_used: bool,
    pub deprecation: Option<String>,
    pub duration_ms: u32,
    pub distance_px: f64,
    pub modifiers_used: Vec<DragModifier>,
    pub backend_used: String,
    pub elapsed_ms: u32,
}

pub async fn act_drag_with_handle(
    handle: ActionHandle,
    recording: Option<Arc<RecordingBackend>>,
    params: ActDragParams,
) -> Result<ActDragResponse, ErrorData> {
    let started = Instant::now();
    let from = target_point(&params.from, "from")?;
    let to = target_point(&params.to, "to")?;
    let backend = params.backend.to_backend();
    let (velocity_profile, deprecated_curve_alias_used) = resolve_velocity_profile(&params)?;
    let action = Action::MouseDrag {
        from,
        to,
        button: params.button.to_mouse_button(),
        curve: velocity_profile.to_aim_curve(),
        duration_ms: params.duration_ms,
        backend,
    };

    let modifier_keys: Vec<_> = params
        .modifiers
        .iter()
        .map(|modifier| modifier.to_key())
        .collect();

    if let Some(recording) = recording {
        execute_recording(&recording, &modifier_keys, &action, backend)?;
    } else {
        execute_with_modifiers(&handle, &modifier_keys, action, backend).await?;
    }

    Ok(ActDragResponse {
        ok: true,
        button_used: params.button,
        velocity_profile_used: velocity_profile,
        deprecated_curve_alias_used,
        deprecation: deprecated_curve_alias_used.then(|| {
            "act_drag.curve is a deprecated compatibility alias; use velocity_profile. Spatial paths belong in act_stroke.path.".to_owned()
        }),
        duration_ms: params.duration_ms,
        distance_px: from.distance_to(to),
        modifiers_used: params.modifiers,
        backend_used: backend_used_name(backend).to_owned(),
        elapsed_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
    })
}

impl DragButton {
    const fn to_mouse_button(self) -> MouseButton {
        match self {
            Self::Left => MouseButton::Left,
            Self::Right => MouseButton::Right,
            Self::Middle => MouseButton::Middle,
        }
    }
}

impl DragVelocityProfile {
    const fn to_aim_curve(self) -> AimCurve {
        match self {
            Self::Natural => AimCurve::Natural {
                params: AimNaturalParams::FAST,
            },
            Self::Instant => AimCurve::Instant,
            Self::Linear => AimCurve::Linear,
            Self::EaseInOut => AimCurve::EaseInOut,
        }
    }
}

impl LegacyDragCurve {
    const fn to_velocity_profile(self) -> Option<DragVelocityProfile> {
        match self {
            Self::Natural => Some(DragVelocityProfile::Natural),
            Self::Instant => Some(DragVelocityProfile::Instant),
            Self::Linear => Some(DragVelocityProfile::Linear),
            Self::EaseInOut => Some(DragVelocityProfile::EaseInOut),
            Self::Bezier => None,
        }
    }
}

fn resolve_velocity_profile(
    params: &ActDragParams,
) -> Result<(DragVelocityProfile, bool), ErrorData> {
    let Some(curve) = params.curve else {
        return Ok((params.velocity_profile, false));
    };
    let Some(profile) = curve.to_velocity_profile() else {
        return Err(mcp_error(
            error_codes::TOOL_PARAMS_INVALID,
            "act_drag curve=bezier is deprecated and is not a valid velocity_profile; use act_stroke with path.kind=cubic_bezier for spatial Bezier paths, or velocity_profile=ease_in_out for point-to-point timing",
        ));
    };
    Ok((profile, true))
}

impl DragBackend {
    const fn to_backend(self) -> Backend {
        match self {
            Self::Software => Backend::Software,
            Self::Hardware => Backend::Hardware,
            Self::Auto => Backend::Auto,
        }
    }
}

impl DragModifier {
    fn to_key(self) -> Key {
        let value = match self {
            Self::Ctrl => "ctrl",
            Self::Shift => "shift",
            Self::Alt => "alt",
            Self::Super => "super",
        };
        Key {
            code: KeyCode::Named {
                value: value.to_owned(),
            },
            use_scancode: false,
        }
    }
}

fn target_point(target: &ActDragTarget, role: &'static str) -> Result<Point, ErrorData> {
    match target {
        ActDragTarget::Point(point) => Ok(Point {
            x: point.x,
            y: point.y,
        }),
        ActDragTarget::Element(element) => element_center(&element.element_id, role),
    }
}

#[cfg(windows)]
fn element_center(element_id: &ElementId, role: &'static str) -> Result<Point, ErrorData> {
    let rect = synapse_a11y::element_bounding_rect(element_id).map_err(|err| {
        action_error_to_mcp(&ActionError::ElementNotResolved {
            detail: format!("act_drag {role} element {element_id} could not be resolved: {err}"),
        })
    })?;
    center_from_rect_edges(RectEdges::from(rect)).map_err(|error| action_error_to_mcp(&error))
}

#[cfg(not(windows))]
fn element_center(element_id: &ElementId, role: &'static str) -> Result<Point, ErrorData> {
    Err(action_error_to_mcp(&ActionError::BackendUnavailable {
        detail: format!(
            "act_drag {role} element target {element_id} requires Windows UI Automation bbox resolution"
        ),
    }))
}

#[cfg(windows)]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
struct RectEdges {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[cfg(windows)]
impl From<Rect> for RectEdges {
    fn from(value: Rect) -> Self {
        Self {
            left: value.x,
            top: value.y,
            right: value.x.saturating_add(value.w),
            bottom: value.y.saturating_add(value.h),
        }
    }
}

#[cfg(windows)]
fn center_from_rect_edges(rect: RectEdges) -> Result<Point, ActionError> {
    if rect.right <= rect.left || rect.bottom <= rect.top {
        return Err(ActionError::TargetInvalid {
            detail: format!("act_drag element bbox is empty or inverted: {rect:?}"),
        });
    }

    let width = i64::from(rect.right) - i64::from(rect.left);
    let height = i64::from(rect.bottom) - i64::from(rect.top);
    let x = i64::from(rect.left) + width / 2;
    let y = i64::from(rect.top) + height / 2;

    Ok(Point {
        x: i32::try_from(x).map_err(|err| ActionError::TargetInvalid {
            detail: format!("act_drag element bbox center x overflowed i32: {err}"),
        })?,
        y: i32::try_from(y).map_err(|err| ActionError::TargetInvalid {
            detail: format!("act_drag element bbox center y overflowed i32: {err}"),
        })?,
    })
}

async fn execute_with_modifiers(
    handle: &ActionHandle,
    modifier_keys: &[Key],
    drag_action: Action,
    backend: Backend,
) -> Result<(), ErrorData> {
    let mut pressed = Vec::with_capacity(modifier_keys.len());
    for key in modifier_keys {
        if let Err(error) = handle
            .execute(Action::KeyDown {
                key: key.clone(),
                backend,
            })
            .await
        {
            let _ = release_pressed_modifiers(handle, &pressed, backend).await;
            return Err(action_error_to_mcp(&error));
        }
        pressed.push(key.clone());
    }

    let drag_result = handle.execute(drag_action).await;
    if drag_result.is_ok() && !pressed.is_empty() {
        tokio::time::sleep(Duration::from_millis(MODIFIER_RELEASE_SETTLE_MS)).await;
    }
    let release_result = release_pressed_modifiers(handle, &pressed, backend).await;

    if let Err(error) = drag_result {
        return Err(action_error_to_mcp(&error));
    }
    if let Err(error) = release_result {
        return Err(action_error_to_mcp(&error));
    }
    Ok(())
}

async fn release_pressed_modifiers(
    handle: &ActionHandle,
    pressed: &[Key],
    backend: Backend,
) -> Result<(), ActionError> {
    let mut release_error = None;
    for key in pressed.iter().rev() {
        if let Err(error) = handle
            .execute(Action::KeyUp {
                key: key.clone(),
                backend,
            })
            .await
            && release_error.is_none()
        {
            release_error = Some(error);
        }
    }
    release_error.map_or(Ok(()), Err)
}

fn execute_recording(
    recording: &RecordingBackend,
    modifier_keys: &[Key],
    drag_action: &Action,
    backend: Backend,
) -> Result<(), ErrorData> {
    let before_events = recording.events();
    let before_event_count = before_events.len();
    let mut emit_state = EmitState::new();
    for key in modifier_keys {
        recording
            .execute(
                &Action::KeyDown {
                    key: key.clone(),
                    backend,
                },
                &mut emit_state,
            )
            .map_err(|error| action_error_to_mcp(&error))?;
    }
    recording
        .execute(drag_action, &mut emit_state)
        .map_err(|error| action_error_to_mcp(&error))?;
    for key in modifier_keys.iter().rev() {
        recording
            .execute(
                &Action::KeyUp {
                    key: key.clone(),
                    backend,
                },
                &mut emit_state,
            )
            .map_err(|error| action_error_to_mcp(&error))?;
    }
    let after_events = recording.events();
    let new_events = &after_events[before_event_count..];
    let event_sequence = event_sequence(new_events);
    tracing::info!(
        code = "M2_ACT_DRAG_RECORDING_READBACK",
        kind = "act_drag",
        before_event_count,
        after_event_count = after_events.len(),
        new_event_count = new_events.len(),
        event_sequence,
        ?new_events,
        "readback=recording_backend tool=act_drag after_events_readback"
    );
    Ok(())
}

fn event_sequence(events: &[RecordedInput]) -> String {
    events.iter().map(event_label).collect::<Vec<_>>().join(">")
}

fn event_label(event: &RecordedInput) -> String {
    match event {
        RecordedInput::KeyDown { key } => format!("key_down:{}", key_label(key)),
        RecordedInput::KeyUp { key } => format!("key_up:{}", key_label(key)),
        RecordedInput::MouseButtonDown { button } => format!("down:{}", button_label(*button)),
        RecordedInput::MouseMove {
            to,
            curve,
            duration_ms,
        } => format!(
            "mouse_move:{}:{}:{duration_ms}",
            mouse_target_label(to),
            curve_label(curve)
        ),
        RecordedInput::MouseButtonUp { button } => format!("up:{}", button_label(*button)),
        other => format!("{other:?}"),
    }
}

fn key_label(key: &Key) -> String {
    match &key.code {
        KeyCode::Named { value } => value.clone(),
        KeyCode::Symbol { value } => value.to_string(),
        KeyCode::HidCode { value } => format!("hid:{value}"),
    }
}

fn mouse_target_label(target: &MouseTarget) -> String {
    match target {
        MouseTarget::Screen { point } => format!("screen({},{})", point.x, point.y),
        MouseTarget::Element { element_id } => format!("element({element_id})"),
    }
}

fn curve_label(curve: &AimCurve) -> &'static str {
    match curve {
        AimCurve::Natural {
            params: AimNaturalParams::FAST,
        } => "natural_fast",
        AimCurve::Natural { .. } => "natural",
        AimCurve::Instant => "instant",
        AimCurve::Linear => "linear",
        AimCurve::EaseInOut => "ease_in_out",
        AimCurve::Bezier { .. } => "bezier",
    }
}

const fn button_label(button: MouseButton) -> &'static str {
    match button {
        MouseButton::Left => "left",
        MouseButton::Right => "right",
        MouseButton::Middle => "middle",
        MouseButton::X1 => "x1",
        MouseButton::X2 => "x2",
    }
}

fn action_error_to_mcp(error: &ActionError) -> ErrorData {
    mcp_error(error.code(), error.to_string())
}

const fn default_drag_button() -> DragButton {
    DragButton::Left
}

const fn default_drag_velocity_profile() -> DragVelocityProfile {
    DragVelocityProfile::Natural
}

const fn default_drag_duration_ms() -> u32 {
    DEFAULT_DRAG_DURATION_MS
}

const fn default_drag_backend() -> DragBackend {
    DragBackend::Auto
}

const fn backend_used_name(backend: Backend) -> &'static str {
    match backend {
        Backend::Auto | Backend::Software => "software",
        Backend::Hardware => "hardware",
        Backend::Vigem => "vigem",
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use synapse_action::ActionEmitter;

    use super::{
        ActDragParams, ActDragPointTarget, ActDragTarget, DragBackend, DragButton, DragModifier,
        DragVelocityProfile, LegacyDragCurve, act_drag_with_handle, event_sequence,
    };

    #[tokio::test]
    async fn recording_backend_readback_exposes_drag_velocity_profiles() {
        for (velocity_profile, expected_label) in [
            (DragVelocityProfile::Natural, "natural_fast"),
            (DragVelocityProfile::Instant, "instant"),
            (DragVelocityProfile::Linear, "linear"),
            (DragVelocityProfile::EaseInOut, "ease_in_out"),
        ] {
            let (handle, _snapshot_handle, _emitter) = ActionEmitter::channel();
            let recording = Arc::new(synapse_action::RecordingBackend::new());
            let params = ActDragParams {
                from: ActDragTarget::Point(ActDragPointTarget { x: 10, y: 20 }),
                to: ActDragTarget::Point(ActDragPointTarget { x: 110, y: 140 }),
                button: DragButton::Left,
                velocity_profile,
                curve: None,
                duration_ms: 120,
                backend: DragBackend::Software,
                modifiers: Vec::new(),
            };
            let before = recording.events();
            println!(
                "readback=act_drag_recording edge=velocity_profile before=velocity_profile:{velocity_profile:?} events={before:?}"
            );

            let response = act_drag_with_handle(handle, Some(Arc::clone(&recording)), params)
                .await
                .unwrap_or_else(|error| panic!("act_drag recording should succeed: {error}"));
            let after = recording.events();
            let sequence = event_sequence(&after);
            println!(
                "readback=act_drag_recording edge=velocity_profile after=velocity_profile:{velocity_profile:?} sequence={sequence} distance={} result_value=ok",
                response.distance_px
            );

            assert!(response.ok);
            assert_eq!(response.velocity_profile_used, velocity_profile);
            assert!(!response.deprecated_curve_alias_used);
            assert_eq!(
                sequence,
                format!("down:left>mouse_move:screen(110,140):{expected_label}:120>up:left")
            );
        }
    }

    #[tokio::test]
    async fn recording_backend_readback_holds_drag_modifiers_around_drag() {
        let (handle, _snapshot_handle, _emitter) = ActionEmitter::channel();
        let recording = Arc::new(synapse_action::RecordingBackend::new());
        let params = ActDragParams {
            from: ActDragTarget::Point(ActDragPointTarget { x: 10, y: 20 }),
            to: ActDragTarget::Point(ActDragPointTarget { x: 70, y: 80 }),
            button: DragButton::Left,
            velocity_profile: DragVelocityProfile::Linear,
            curve: None,
            duration_ms: 80,
            backend: DragBackend::Software,
            modifiers: vec![DragModifier::Shift],
        };
        let before = recording.events();
        println!("readback=act_drag_recording edge=modifier before={before:?}");

        let response = act_drag_with_handle(handle, Some(Arc::clone(&recording)), params)
            .await
            .unwrap_or_else(|error| {
                panic!("act_drag recording with modifier should succeed: {error}")
            });
        let after = recording.events();
        let sequence = event_sequence(&after);
        println!(
            "readback=act_drag_recording edge=modifier after_sequence={sequence} modifiers={:?}",
            response.modifiers_used
        );

        assert!(response.ok);
        assert_eq!(response.modifiers_used, [DragModifier::Shift]);
        assert_eq!(
            sequence,
            "key_down:shift>down:left>mouse_move:screen(70,80):linear:80>up:left>key_up:shift"
        );
    }

    #[tokio::test]
    async fn deprecated_curve_alias_maps_non_bezier_values_to_velocity_profile() {
        let (handle, _snapshot_handle, _emitter) = ActionEmitter::channel();
        let recording = Arc::new(synapse_action::RecordingBackend::new());
        let params = ActDragParams {
            from: ActDragTarget::Point(ActDragPointTarget { x: 10, y: 20 }),
            to: ActDragTarget::Point(ActDragPointTarget { x: 70, y: 80 }),
            button: DragButton::Left,
            velocity_profile: DragVelocityProfile::Natural,
            curve: Some(LegacyDragCurve::Linear),
            duration_ms: 80,
            backend: DragBackend::Software,
            modifiers: Vec::new(),
        };
        let response = act_drag_with_handle(handle, Some(Arc::clone(&recording)), params)
            .await
            .unwrap_or_else(|error| panic!("legacy act_drag curve alias should parse: {error}"));
        let sequence = event_sequence(&recording.events());
        println!(
            "readback=act_drag_recording edge=legacy_curve_alias after_sequence={sequence} deprecated={} deprecation={:?}",
            response.deprecated_curve_alias_used, response.deprecation
        );

        assert!(response.ok);
        assert_eq!(response.velocity_profile_used, DragVelocityProfile::Linear);
        assert!(response.deprecated_curve_alias_used);
        assert!(
            response
                .deprecation
                .as_deref()
                .is_some_and(|message| message.contains("velocity_profile"))
        );
        assert_eq!(
            sequence,
            "down:left>mouse_move:screen(70,80):linear:80>up:left"
        );
    }

    #[tokio::test]
    async fn deprecated_curve_bezier_returns_clear_error() {
        let (handle, _snapshot_handle, _emitter) = ActionEmitter::channel();
        let recording = Arc::new(synapse_action::RecordingBackend::new());
        let params = ActDragParams {
            from: ActDragTarget::Point(ActDragPointTarget { x: 10, y: 20 }),
            to: ActDragTarget::Point(ActDragPointTarget { x: 70, y: 80 }),
            button: DragButton::Left,
            velocity_profile: DragVelocityProfile::Natural,
            curve: Some(LegacyDragCurve::Bezier),
            duration_ms: 80,
            backend: DragBackend::Software,
            modifiers: Vec::new(),
        };
        let error = act_drag_with_handle(handle, Some(Arc::clone(&recording)), params)
            .await
            .expect_err("legacy act_drag curve=bezier should fail closed");
        println!("readback=act_drag_recording edge=legacy_curve_bezier after_error={error:?}");

        assert!(error.message.to_string().contains("curve=bezier"));
        assert!(
            error
                .message
                .to_string()
                .contains("act_stroke with path.kind=cubic_bezier")
        );
        assert!(recording.events().is_empty());
    }
}
