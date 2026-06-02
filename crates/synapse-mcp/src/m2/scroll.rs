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
use synapse_core::{Action, Backend, Point};

use crate::m1::mcp_error;

#[cfg(windows)]
use std::ffi::c_void;
#[cfg(windows)]
use windows::{
    Win32::{
        Foundation::{HWND, LPARAM, POINT as WinPoint, RECT, WPARAM},
        UI::WindowsAndMessaging::{
            EnumChildWindows, GA_ROOT, GetAncestor, GetClassNameW, GetWindowRect, IsWindow,
            IsWindowVisible, PostMessageW, WM_MOUSEHWHEEL, WM_MOUSEWHEEL, WindowFromPoint,
        },
    },
    core::BOOL,
};

const SMOOTH_SCROLL_INTERVAL_MS: u32 = 30;
const MAX_SMOOTH_SCROLL_STEPS: u32 = 120;
const WHEEL_DELTA: i32 = 120;
#[cfg(windows)]
const MAX_TARGETED_WHEEL_MESSAGES: usize = 1024;

#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActScrollParams {
    #[serde(default)]
    #[schemars(default)]
    pub dy: i32,
    #[serde(default)]
    #[schemars(default)]
    pub dx: i32,
    pub at: Option<ActScrollPoint>,
    #[serde(default)]
    #[schemars(default)]
    pub smooth: bool,
}

#[derive(Copy, Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActScrollPoint {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActScrollResponse {
    pub ok: bool,
    pub dy: i32,
    pub dx: i32,
    pub smooth: bool,
    pub scrolled: bool,
    pub wheel_event_count: u32,
    pub smooth_interval_ms: u32,
    pub scheduled_smooth_total_ms: u32,
    pub backend_used: String,
    pub elapsed_ms: u32,
}

pub async fn act_scroll_with_handle(
    handle: ActionHandle,
    recording: Option<Arc<RecordingBackend>>,
    params: ActScrollParams,
) -> Result<ActScrollResponse, ErrorData> {
    validate_scroll_params(&params)?;
    let started = Instant::now();
    if params.dy == 0 && params.dx == 0 {
        if let Some(recording) = recording {
            execute_recording_noop(&recording, &params);
        }
        return Ok(response(&params, false, 0, "none", started));
    }

    let actions = scroll_actions(&params)?;
    let mut wheel_event_count = actions.len();
    let mut backend_used = "software";

    if let Some(recording) = recording {
        execute_recording(&recording, &actions, &params).await?;
    } else if let Some(point) = params.at.map(Into::into) {
        let dispatch = execute_targeted_scroll_actions(&params, point).await?;
        wheel_event_count = dispatch.wheel_event_count;
        backend_used = dispatch.backend_used;
    } else {
        execute_scroll_actions(&handle, actions, params.smooth).await?;
    }

    Ok(response(
        &params,
        true,
        wheel_event_count,
        backend_used,
        started,
    ))
}

impl From<ActScrollPoint> for Point {
    fn from(value: ActScrollPoint) -> Self {
        Self {
            x: value.x,
            y: value.y,
        }
    }
}

fn validate_scroll_params(params: &ActScrollParams) -> Result<(), ErrorData> {
    if params.smooth {
        let step_count = smooth_step_count(params.dy, params.dx);
        if step_count > MAX_SMOOTH_SCROLL_STEPS {
            return Err(mcp_error(
                synapse_core::error_codes::TOOL_PARAMS_INVALID,
                format!(
                    "act_scroll smooth=true step count {step_count} exceeds max {MAX_SMOOTH_SCROLL_STEPS}"
                ),
            ));
        }
    }
    Ok(())
}

fn scroll_actions(params: &ActScrollParams) -> Result<Vec<Action>, ErrorData> {
    if !params.smooth {
        return Ok(vec![scroll_action(
            params.dy,
            params.dx,
            params.at.map(Into::into),
        )]);
    }
    let step_count = smooth_step_count(params.dy, params.dx);
    let capacity = usize::try_from(step_count).map_err(|_err| {
        mcp_error(
            synapse_core::error_codes::TOOL_PARAMS_INVALID,
            "act_scroll smooth=true step count cannot fit in memory",
        )
    })?;
    let mut actions = Vec::with_capacity(capacity);
    let mut vertical_ticks_remaining = params.dy;
    let mut horizontal_ticks_remaining = params.dx;
    for step_index in 0..step_count {
        let vertical_tick = take_tick(&mut vertical_ticks_remaining);
        let horizontal_tick = take_tick(&mut horizontal_ticks_remaining);
        actions.push(scroll_action(
            vertical_tick,
            horizontal_tick,
            if step_index == 0 {
                params.at.map(Into::into)
            } else {
                None
            },
        ));
    }
    Ok(actions)
}

const fn scroll_action(dy: i32, dx: i32, at: Option<Point>) -> Action {
    Action::MouseScroll {
        dy,
        dx,
        at,
        backend: Backend::Auto,
    }
}

fn smooth_step_count(dy: i32, dx: i32) -> u32 {
    dy.unsigned_abs().max(dx.unsigned_abs())
}

fn take_tick(value: &mut i32) -> i32 {
    match (*value).cmp(&0) {
        std::cmp::Ordering::Less => {
            *value += 1;
            -1
        }
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => {
            *value -= 1;
            1
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
struct ScrollDispatchResult {
    backend_used: &'static str,
    wheel_event_count: usize,
}

async fn execute_targeted_scroll_actions(
    params: &ActScrollParams,
    point: Point,
) -> Result<ScrollDispatchResult, ErrorData> {
    execute_targeted_scroll_actions_platform(params, point).await
}

#[cfg(windows)]
async fn execute_targeted_scroll_actions_platform(
    params: &ActScrollParams,
    point: Point,
) -> Result<ScrollDispatchResult, ErrorData> {
    let readback =
        windows_hwnd_message_scroll_readback(point).map_err(|error| action_error_to_mcp(&error))?;
    let mut wheel_event_count = 0_usize;
    for delta in wheel_delta_chunks(params.dy).map_err(|error| action_error_to_mcp(&error))? {
        post_wheel_message(readback.hwnd, WM_MOUSEWHEEL, delta, point)
            .map_err(|error| action_error_to_mcp(&error))?;
        wheel_event_count = wheel_event_count.saturating_add(1);
        tracing::info!(
            code = "M2_ACT_SCROLL_HWND_MESSAGE",
            kind = "act_scroll",
            target_hwnd = readback.hwnd,
            target_class = %readback.class_name,
            screen_x = point.x,
            screen_y = point.y,
            delta = i32::from(delta),
            axis = "vertical",
            "readback=window_message tool=act_scroll targeted_scroll_after"
        );
    }
    for delta in wheel_delta_chunks(params.dx).map_err(|error| action_error_to_mcp(&error))? {
        post_wheel_message(readback.hwnd, WM_MOUSEHWHEEL, delta, point)
            .map_err(|error| action_error_to_mcp(&error))?;
        wheel_event_count = wheel_event_count.saturating_add(1);
        tracing::info!(
            code = "M2_ACT_SCROLL_HWND_MESSAGE",
            kind = "act_scroll",
            target_hwnd = readback.hwnd,
            target_class = %readback.class_name,
            screen_x = point.x,
            screen_y = point.y,
            delta = i32::from(delta),
            axis = "horizontal",
            "readback=window_message tool=act_scroll targeted_scroll_after"
        );
    }
    Ok(ScrollDispatchResult {
        backend_used: "software_window_message",
        wheel_event_count,
    })
}

#[cfg(not(windows))]
async fn execute_targeted_scroll_actions_platform(
    _params: &ActScrollParams,
    point: Point,
) -> Result<ScrollDispatchResult, ErrorData> {
    Err(action_error_to_mcp(&ActionError::BackendUnavailable {
        detail: format!("act_scroll at={point:?} targeted window-message path requires Windows"),
    }))
}

async fn execute_scroll_actions(
    handle: &ActionHandle,
    actions: Vec<Action>,
    smooth: bool,
) -> Result<(), ErrorData> {
    let last_index = actions.len().saturating_sub(1);
    for (index, action) in actions.into_iter().enumerate() {
        handle
            .execute(action)
            .await
            .map_err(|error| action_error_to_mcp(&error))?;
        if smooth && index < last_index {
            tokio::time::sleep(Duration::from_millis(u64::from(SMOOTH_SCROLL_INTERVAL_MS))).await;
        }
    }
    Ok(())
}

fn response(
    params: &ActScrollParams,
    scrolled: bool,
    wheel_event_count: usize,
    backend_used: &'static str,
    started: Instant,
) -> ActScrollResponse {
    let wheel_event_count = u32::try_from(wheel_event_count).unwrap_or(u32::MAX);
    ActScrollResponse {
        ok: true,
        dy: params.dy,
        dx: params.dx,
        smooth: params.smooth,
        scrolled,
        wheel_event_count,
        smooth_interval_ms: if params.smooth {
            SMOOTH_SCROLL_INTERVAL_MS
        } else {
            0
        },
        scheduled_smooth_total_ms: scheduled_smooth_total_ms(params.smooth, wheel_event_count),
        backend_used: backend_used.to_owned(),
        elapsed_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
    }
}

const fn scheduled_smooth_total_ms(smooth: bool, wheel_event_count: u32) -> u32 {
    if !smooth || wheel_event_count == 0 {
        return 0;
    }
    wheel_event_count
        .saturating_sub(1)
        .saturating_mul(SMOOTH_SCROLL_INTERVAL_MS)
}

async fn execute_recording(
    recording: &RecordingBackend,
    actions: &[Action],
    params: &ActScrollParams,
) -> Result<(), ErrorData> {
    let before_events = recording.events();
    let before_event_count = before_events.len();
    let mut emit_state = EmitState::new();
    let last_index = actions.len().saturating_sub(1);
    for (index, action) in actions.iter().enumerate() {
        recording
            .execute(action, &mut emit_state)
            .map_err(|error| action_error_to_mcp(&error))?;
        if params.smooth && index < last_index {
            tokio::time::sleep(Duration::from_millis(u64::from(SMOOTH_SCROLL_INTERVAL_MS))).await;
        }
    }
    let after_events = recording.events();
    let new_events = &after_events[before_event_count..];
    log_recording_readback(before_event_count, &after_events, new_events, params);
    Ok(())
}

fn execute_recording_noop(recording: &RecordingBackend, params: &ActScrollParams) {
    let before_events = recording.events();
    let before_event_count = before_events.len();
    let after_events = recording.events();
    let new_events = &after_events[before_event_count..];
    log_recording_readback(before_event_count, &after_events, new_events, params);
}

fn log_recording_readback(
    before_event_count: usize,
    after_events: &[RecordedInput],
    new_events: &[RecordedInput],
    params: &ActScrollParams,
) {
    let event_sequence = event_sequence(new_events);
    let smooth_step_count = if params.smooth {
        smooth_step_count(params.dy, params.dx)
    } else {
        0
    };
    tracing::info!(
        code = "M2_ACT_SCROLL_RECORDING_READBACK",
        kind = "act_scroll",
        before_event_count,
        after_event_count = after_events.len(),
        new_event_count = new_events.len(),
        dy = params.dy,
        dx = params.dx,
        smooth = params.smooth,
        smooth_step_count,
        smooth_interval_ms = if params.smooth {
            SMOOTH_SCROLL_INTERVAL_MS
        } else {
            0
        },
        scheduled_smooth_total_ms = scheduled_smooth_total_ms(params.smooth, smooth_step_count),
        event_sequence,
        ?new_events,
        "readback=recording_backend tool=act_scroll after_events_readback"
    );
}

fn event_sequence(events: &[RecordedInput]) -> String {
    events.iter().map(event_label).collect::<Vec<_>>().join(">")
}

fn event_label(event: &RecordedInput) -> String {
    match event {
        RecordedInput::MouseScroll { dy, dx, at } => {
            format!("mouse_scroll:dy={dy}:dx={dx}:at={}", at_label(*at))
        }
        other => format!("{other:?}"),
    }
}

fn at_label(at: Option<Point>) -> String {
    at.map_or_else(
        || "none".to_owned(),
        |point| format!("screen({},{})", point.x, point.y),
    )
}

fn action_error_to_mcp(error: &ActionError) -> ErrorData {
    mcp_error(error.code(), error.to_string())
}

#[cfg(windows)]
#[derive(Clone, Debug)]
struct HwndMessageScrollReadback {
    hwnd: i64,
    class_name: String,
}

#[cfg(windows)]
#[derive(Clone, Debug)]
struct WindowCandidate {
    hwnd: HWND,
    rect: RECT,
    class_name: String,
}

#[cfg(windows)]
struct ChildEnumContext {
    point: Point,
    candidates: Vec<WindowCandidate>,
}

#[cfg(windows)]
fn windows_hwnd_message_scroll_readback(
    point: Point,
) -> Result<HwndMessageScrollReadback, ActionError> {
    let seed = unsafe {
        WindowFromPoint(WinPoint {
            x: point.x,
            y: point.y,
        })
    };
    if seed.0.is_null() {
        return Err(ActionError::TargetInvalid {
            detail: format!("act_scroll at point {point:?} is not over a live window"),
        });
    }
    let root = unsafe { GetAncestor(seed, GA_ROOT) };
    let root = if root.0.is_null() { seed } else { root };
    if !unsafe { IsWindow(Some(root)) }.as_bool() {
        return Err(ActionError::TargetInvalid {
            detail: format!(
                "act_scroll root hwnd 0x{:x} for point {point:?} is not a live window",
                hwnd_to_i64(root)
            ),
        });
    }

    let target = hit_test_hwnd_for_screen_point(seed, root, point)?;
    let _ = screen_lparam(point)?;
    Ok(HwndMessageScrollReadback {
        hwnd: hwnd_to_i64(target.hwnd),
        class_name: target.class_name,
    })
}

#[cfg(windows)]
fn hit_test_hwnd_for_screen_point(
    seed: HWND,
    root: HWND,
    point: Point,
) -> Result<WindowCandidate, ActionError> {
    let root_rect = window_rect(root)?;
    if !rect_contains_point(&root_rect, point) {
        return Err(ActionError::TargetInvalid {
            detail: format!(
                "act_scroll point {point:?} is outside root hwnd 0x{:x} rect {:?}",
                hwnd_to_i64(root),
                rect_tuple(&root_rect)
            ),
        });
    }

    if let Ok(seed_rect) = window_rect(seed)
        && unsafe { IsWindowVisible(seed) }.as_bool()
        && rect_contains_point(&seed_rect, point)
        && rect_area(&seed_rect) > 0
    {
        return Ok(WindowCandidate {
            hwnd: seed,
            rect: seed_rect,
            class_name: window_class_name(seed),
        });
    }

    best_child_hwnd_for_screen_point(root, root_rect, point)
}

#[cfg(windows)]
fn best_child_hwnd_for_screen_point(
    root: HWND,
    root_rect: RECT,
    point: Point,
) -> Result<WindowCandidate, ActionError> {
    let mut context = ChildEnumContext {
        point,
        candidates: Vec::new(),
    };
    let context_ptr = (&raw mut context).cast::<c_void>();
    let _ = unsafe {
        EnumChildWindows(
            Some(root),
            Some(enum_child_containing_point),
            LPARAM(context_ptr as isize),
        )
    };

    Ok(context
        .candidates
        .into_iter()
        .min_by_key(|candidate| rect_area(&candidate.rect))
        .unwrap_or_else(|| WindowCandidate {
            hwnd: root,
            rect: root_rect,
            class_name: window_class_name(root),
        }))
}

#[cfg(windows)]
unsafe extern "system" fn enum_child_containing_point(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let context = unsafe { &mut *(lparam.0 as *mut ChildEnumContext) };
    if unsafe { IsWindowVisible(hwnd) }.as_bool()
        && let Ok(rect) = window_rect(hwnd)
        && rect_contains_point(&rect, context.point)
        && rect_area(&rect) > 0
    {
        context.candidates.push(WindowCandidate {
            hwnd,
            rect,
            class_name: window_class_name(hwnd),
        });
    }
    BOOL(1)
}

#[cfg(windows)]
fn post_wheel_message(
    hwnd: i64,
    message: u32,
    delta: i16,
    screen_point: Point,
) -> Result<(), ActionError> {
    let hwnd = hwnd_from_i64(hwnd)?;
    let wparam = wheel_wparam(delta)?;
    let lparam = screen_lparam(screen_point)?;
    unsafe { PostMessageW(Some(hwnd), message, wparam, lparam) }.map_err(|error| {
        ActionError::BackendUnavailable {
            detail: format!(
                "PostMessageW act_scroll wheel message 0x{message:x} failed for hwnd 0x{:x} screen_point={screen_point:?} delta={delta}: {error}",
                hwnd_to_i64(hwnd)
            ),
        }
    })
}

#[cfg(windows)]
fn wheel_delta_chunks(ticks: i32) -> Result<Vec<i16>, ActionError> {
    if ticks == 0 {
        return Ok(Vec::new());
    }
    let max_ticks_per_message = i32::from(i16::MAX) / WHEEL_DELTA;
    let mut remaining = ticks;
    let mut chunks = Vec::new();
    while remaining != 0 {
        if chunks.len() >= MAX_TARGETED_WHEEL_MESSAGES {
            return Err(ActionError::TargetInvalid {
                detail: format!(
                    "act_scroll targeted wheel message count exceeds {MAX_TARGETED_WHEEL_MESSAGES} for ticks={ticks}"
                ),
            });
        }
        let step_ticks = remaining.clamp(-max_ticks_per_message, max_ticks_per_message);
        let delta = step_ticks.saturating_mul(WHEEL_DELTA);
        chunks.push(
            i16::try_from(delta).map_err(|error| ActionError::TargetInvalid {
                detail: format!(
                    "act_scroll wheel delta {delta} cannot fit WM_MOUSE*WHEEL i16: {error}"
                ),
            })?,
        );
        remaining = remaining.saturating_sub(step_ticks);
    }
    Ok(chunks)
}

#[cfg(windows)]
fn wheel_wparam(delta: i16) -> Result<WPARAM, ActionError> {
    let high_word = u32::from(u16::from_ne_bytes(delta.to_ne_bytes())) << 16;
    Ok(WPARAM(usize::try_from(high_word).map_err(|error| {
        ActionError::TargetInvalid {
            detail: format!("act_scroll wheel wParam overflowed usize: {error}"),
        }
    })?))
}

#[cfg(windows)]
fn screen_lparam(point: Point) -> Result<LPARAM, ActionError> {
    let x = i16::try_from(point.x).map_err(|error| ActionError::TargetInvalid {
        detail: format!(
            "act_scroll screen x {} cannot fit a WM_MOUSE*WHEEL lParam i16: {error}",
            point.x
        ),
    })?;
    let y = i16::try_from(point.y).map_err(|error| ActionError::TargetInvalid {
        detail: format!(
            "act_scroll screen y {} cannot fit a WM_MOUSE*WHEEL lParam i16: {error}",
            point.y
        ),
    })?;
    let packed = (u32::from(u16::from_ne_bytes(y.to_ne_bytes())) << 16)
        | u32::from(u16::from_ne_bytes(x.to_ne_bytes()));
    Ok(LPARAM(isize::try_from(packed).unwrap_or(isize::MAX)))
}

#[cfg(windows)]
fn window_rect(hwnd: HWND) -> Result<RECT, ActionError> {
    let mut rect = RECT::default();
    unsafe { GetWindowRect(hwnd, &raw mut rect) }.map_err(|error| {
        ActionError::ElementNotResolved {
            detail: format!(
                "GetWindowRect failed for act_scroll hwnd 0x{:x}: {error}",
                hwnd_to_i64(hwnd)
            ),
        }
    })?;
    Ok(rect)
}

#[cfg(windows)]
fn rect_contains_point(rect: &RECT, point: Point) -> bool {
    point.x >= rect.left && point.x < rect.right && point.y >= rect.top && point.y < rect.bottom
}

#[cfg(windows)]
fn rect_area(rect: &RECT) -> i64 {
    let width = i64::from(rect.right.saturating_sub(rect.left).max(0));
    let height = i64::from(rect.bottom.saturating_sub(rect.top).max(0));
    width.saturating_mul(height)
}

#[cfg(windows)]
fn rect_tuple(rect: &RECT) -> (i32, i32, i32, i32) {
    (rect.left, rect.top, rect.right, rect.bottom)
}

#[cfg(windows)]
fn window_class_name(hwnd: HWND) -> String {
    let mut buffer = vec![0_u16; 256];
    let len = unsafe { GetClassNameW(hwnd, &mut buffer) };
    String::from_utf16_lossy(&buffer[..usize::try_from(len).unwrap_or(0)])
}

#[cfg(windows)]
fn hwnd_from_i64(hwnd: i64) -> Result<HWND, ActionError> {
    if hwnd == 0 {
        return Err(ActionError::TargetInvalid {
            detail: "act_scroll target hwnd is null".to_owned(),
        });
    }
    Ok(HWND(hwnd as isize as *mut c_void))
}

#[cfg(windows)]
fn hwnd_to_i64(hwnd: HWND) -> i64 {
    hwnd.0 as isize as i64
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::*;

    #[cfg(windows)]
    #[test]
    fn targeted_wheel_chunks_fit_signed_message_delta() {
        let before_ticks = 2400;
        let after = wheel_delta_chunks(before_ticks)
            .unwrap_or_else(|error| panic!("targeted scroll chunking should fit: {error}"));

        println!(
            "readback=act_scroll_targeted_chunks before_ticks={before_ticks} after_chunks={after:?}"
        );
        assert_eq!(
            after.iter().map(|delta| i32::from(*delta)).sum::<i32>(),
            before_ticks * 120
        );
        assert!(after.iter().all(|delta| delta.unsigned_abs() <= 32_760));
    }

    #[cfg(windows)]
    #[test]
    fn targeted_wheel_chunks_preserve_negative_direction() {
        let before_ticks = -20;
        let after = wheel_delta_chunks(before_ticks)
            .unwrap_or_else(|error| panic!("negative targeted scroll should fit: {error}"));

        println!(
            "readback=act_scroll_targeted_chunks edge=negative before_ticks={before_ticks} after_chunks={after:?}"
        );
        assert_eq!(after, [-2400]);
    }
}
