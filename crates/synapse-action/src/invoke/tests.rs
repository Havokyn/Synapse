use synapse_core::{
    AimCurve, AimNaturalParams, ElementId, MouseButton, MouseTarget, Point, error_codes,
};

use super::{
    CoordinateFallbackPlan, ElementClickOutcome,
    dispatch::{
        FALLBACK_MOVE_DURATION_MS, InvokeAttemptError, complete_click_attempt,
        emit_coordinate_fallback_click,
    },
    resolver::{
        RectEdges, center_from_rect_edges, element_not_resolved, invoke_pattern_failed,
        invoke_pattern_unavailable,
    },
};
#[cfg(not(windows))]
use super::{click_element_or_fallback, invoke_element};
use crate::ActionError;
use crate::{EmitState, RecordedInput, RecordingBackend};

#[test]
fn re_resolve_failures_map_to_element_not_resolved() {
    let before = "synthetic stale element";
    let after = element_not_resolved(before);
    assert_eq!(after.code(), error_codes::ACTION_ELEMENT_NOT_RESOLVED);
    assert_eq!(after.detail(), before);
    println!(
        "source_of_truth=invoke_error_mapping edge=re_resolve_failure before={before:?} after_code={} after_detail={:?}",
        after.code(),
        after.detail()
    );
}

#[test]
fn missing_invoke_pattern_maps_to_target_invalid_for_coordinate_fallback() {
    let element_id = synthetic_element_id();
    let before = "pattern not available";
    let after = invoke_pattern_unavailable(&element_id, before);
    assert_eq!(after.code(), error_codes::ACTION_TARGET_INVALID);
    assert!(after.detail().contains(element_id.as_str()));
    assert!(after.detail().contains("InvokePattern"));
    println!(
        "source_of_truth=invoke_error_mapping edge=missing_invoke_pattern before={before:?} after_code={} after_detail={:?}",
        after.code(),
        after.detail()
    );
}

#[test]
fn invoke_failures_map_to_target_invalid_without_cursor_fallback_in_bridge() {
    let element_id = synthetic_element_id();
    let before = "blocked by modal";
    let after = invoke_pattern_failed(&element_id, before);
    assert_eq!(after.code(), error_codes::ACTION_TARGET_INVALID);
    assert!(after.detail().contains(element_id.as_str()));
    assert!(after.detail().contains("InvokePattern.invoke failed"));
    println!(
        "source_of_truth=invoke_error_mapping edge=invoke_failure before={before:?} after_code={} after_detail={:?}",
        after.code(),
        after.detail()
    );
}

#[cfg(not(windows))]
#[test]
fn non_windows_stub_fails_closed() {
    let element_id = synthetic_element_id();
    let before = format!("os={} element_id={element_id}", std::env::consts::OS);
    let after = invoke_element(&element_id);
    let Err(ActionError::BackendUnavailable { detail }) = after else {
        panic!("expected non-Windows invoke_element to fail closed");
    };
    assert_eq!(
        ActionError::BackendUnavailable {
            detail: detail.clone()
        }
        .code(),
        error_codes::ACTION_BACKEND_UNAVAILABLE
    );
    assert!(detail.contains("requires Windows"));
    println!(
        "source_of_truth=invoke_non_windows_stub edge=non_windows before={before:?} after_code={} after_detail={detail:?}",
        error_codes::ACTION_BACKEND_UNAVAILABLE
    );
}

#[cfg(not(windows))]
#[test]
fn non_windows_click_fallback_fails_closed() {
    let element_id = synthetic_element_id();
    let backend = RecordingBackend::default();
    let mut state = EmitState::default();
    let before = format!(
        "os={} element_id={element_id} events={:?}",
        std::env::consts::OS,
        backend.events()
    );
    let after = click_element_or_fallback(&element_id, &backend, &mut state, MouseButton::Left);
    let Err(ActionError::BackendUnavailable { detail }) = after else {
        panic!("expected non-Windows click_element_or_fallback to fail closed");
    };
    assert_eq!(
        ActionError::BackendUnavailable {
            detail: detail.clone()
        }
        .code(),
        error_codes::ACTION_BACKEND_UNAVAILABLE
    );
    assert!(backend.events().is_empty());
    println!(
        "source_of_truth=invoke_coordinate_fallback edge=non_windows before={before:?} after_code={} after_detail={detail:?} after_events={:?}",
        error_codes::ACTION_BACKEND_UNAVAILABLE,
        backend.events()
    );
}

#[test]
fn coordinate_fallback_emits_move_down_up_at_bbox_center() {
    let backend = RecordingBackend::default();
    let mut state = EmitState::default();
    let plan = CoordinateFallbackPlan {
        screen_point: Point { x: 60, y: 120 },
        window_point: Point { x: 10, y: 20 },
    };
    let before = backend.events();

    if let Err(error) =
        emit_coordinate_fallback_click(&backend, &mut state, MouseButton::Left, plan)
    {
        panic!("recording backend should accept coordinate fallback: {error}");
    }

    let after = backend.events();
    let expected = vec![
        RecordedInput::MouseMove {
            to: MouseTarget::Screen {
                point: plan.screen_point,
            },
            curve: AimCurve::Natural {
                params: AimNaturalParams::FAST,
            },
            duration_ms: FALLBACK_MOVE_DURATION_MS,
        },
        RecordedInput::MouseButtonDown {
            button: MouseButton::Left,
        },
        RecordedInput::MouseButtonUp {
            button: MouseButton::Left,
        },
    ];
    assert_eq!(after, expected);
    println!(
        "source_of_truth=recording_backend edge=coordinate_fallback_sequence before={before:?} after={after:?}"
    );
}

#[test]
fn missing_invoke_pattern_branch_emits_coordinate_fallback() {
    let backend = RecordingBackend::default();
    let mut state = EmitState::default();
    let plan = CoordinateFallbackPlan {
        screen_point: Point { x: 42, y: 84 },
        window_point: Point { x: 2, y: 4 },
    };
    let before = backend.events();

    let after = complete_click_attempt(
        Err(InvokeAttemptError::MissingPattern),
        || Ok(plan),
        &backend,
        &mut state,
        MouseButton::Right,
    );

    assert_eq!(after, Ok(ElementClickOutcome::CoordinateFallback(plan)));
    let events = backend.events();
    assert_eq!(
        events,
        vec![
            RecordedInput::MouseMove {
                to: MouseTarget::Screen {
                    point: plan.screen_point,
                },
                curve: AimCurve::Natural {
                    params: AimNaturalParams::FAST,
                },
                duration_ms: FALLBACK_MOVE_DURATION_MS,
            },
            RecordedInput::MouseButtonDown {
                button: MouseButton::Right,
            },
            RecordedInput::MouseButtonUp {
                button: MouseButton::Right,
            },
        ]
    );
    println!(
        "source_of_truth=invoke_coordinate_fallback edge=missing_invoke_pattern before={before:?} after_outcome={after:?} after_events={events:?}"
    );
}

#[test]
fn successful_invoke_branch_does_not_emit_coordinate_fallback() {
    let backend = RecordingBackend::default();
    let mut state = EmitState::default();
    let before = backend.events();

    let after = complete_click_attempt(
        Ok(()),
        || {
            Ok(CoordinateFallbackPlan {
                screen_point: Point { x: 1, y: 1 },
                window_point: Point { x: 1, y: 1 },
            })
        },
        &backend,
        &mut state,
        MouseButton::Left,
    );

    assert_eq!(after, Ok(ElementClickOutcome::Invoked));
    assert!(backend.events().is_empty());
    println!(
        "source_of_truth=invoke_coordinate_fallback edge=invoke_success before={before:?} after_outcome={after:?} after_events={:?}",
        backend.events()
    );
}

#[test]
fn failed_invoke_branch_does_not_emit_coordinate_fallback() {
    let backend = RecordingBackend::default();
    let mut state = EmitState::default();
    let expected_error = ActionError::TargetInvalid {
        detail: "synthetic invoke failure".to_owned(),
    };
    let before = backend.events();

    let after = complete_click_attempt(
        Err(InvokeAttemptError::InvokeFailed(expected_error.clone())),
        || {
            Ok(CoordinateFallbackPlan {
                screen_point: Point { x: 99, y: 99 },
                window_point: Point { x: 9, y: 9 },
            })
        },
        &backend,
        &mut state,
        MouseButton::Left,
    );

    assert_eq!(after, Err(expected_error));
    assert!(backend.events().is_empty());
    println!(
        "source_of_truth=invoke_coordinate_fallback edge=invoke_failure before={before:?} after_outcome={after:?} after_events={:?}",
        backend.events()
    );
}

#[test]
fn bbox_center_rounds_inside_odd_sized_rectangle() {
    let rect = RectEdges {
        left: 10,
        top: 20,
        right: 111,
        bottom: 221,
    };
    let before = format!("{rect:?}");
    let after = match center_from_rect_edges(rect) {
        Ok(point) => point,
        Err(error) => panic!("odd-sized rect should have a center: {error}"),
    };
    let expected_exact_center = (60.5_f64, 120.5_f64);
    let dx = f64::from(after.x) - expected_exact_center.0;
    let dy = f64::from(after.y) - expected_exact_center.1;
    assert!(after.x >= rect.left && after.x < rect.right);
    assert!(after.y >= rect.top && after.y < rect.bottom);
    assert!(dx.hypot(dy) <= 1.0);
    println!(
        "source_of_truth=bbox_center edge=odd_sized before={before:?} after={after:?} expected_exact_center={expected_exact_center:?}"
    );
}

#[test]
fn bbox_center_rejects_empty_or_inverted_rectangles() {
    for rect in [
        RectEdges {
            left: 5,
            top: 5,
            right: 5,
            bottom: 10,
        },
        RectEdges {
            left: 10,
            top: 10,
            right: 9,
            bottom: 12,
        },
        RectEdges {
            left: 10,
            top: 10,
            right: 12,
            bottom: 10,
        },
    ] {
        let before = format!("{rect:?}");
        let after = center_from_rect_edges(rect);
        let Err(error) = after else {
            panic!("expected invalid rect to fail: {rect:?}");
        };
        assert_eq!(error.code(), error_codes::ACTION_TARGET_INVALID);
        println!(
            "source_of_truth=bbox_center edge=invalid_rect before={before:?} after_code={} after_detail={:?}",
            error.code(),
            error.detail()
        );
    }
}

#[test]
fn bbox_center_handles_large_screen_coordinates_without_overflow() {
    let rect = RectEdges {
        left: i32::MAX - 100,
        top: i32::MAX - 200,
        right: i32::MAX,
        bottom: i32::MAX - 20,
    };
    let before = format!("{rect:?}");
    let after = match center_from_rect_edges(rect) {
        Ok(point) => point,
        Err(error) => panic!("large rect should stay in i32 bounds: {error}"),
    };
    assert_eq!(
        after,
        Point {
            x: i32::MAX - 50,
            y: i32::MAX - 110,
        }
    );
    println!(
        "source_of_truth=bbox_center edge=large_coordinates before={before:?} after={after:?}"
    );
}

fn synthetic_element_id() -> ElementId {
    match ElementId::parse("0x1234:0000002a00000001") {
        Ok(element_id) => element_id,
        Err(error) => panic!("synthetic element id must parse: {error}"),
    }
}
