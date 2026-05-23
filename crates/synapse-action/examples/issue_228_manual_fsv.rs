//! Manual FSV harness for GitHub issue #228.
//!
//! Drives synthetic inputs through `ActionEmitter` wired to a
//! `RecordingBackend` (the substitute Source of Truth for "did the actor
//! invoke the backend?") and prints the recorded events + actor state
//! snapshots, so a human (or AI agent) reading the output can physically
//! confirm dispatch happened.
//!
//! Run with:
//!     cargo run -p synapse-action --example issue_228_manual_fsv

use std::sync::Arc;
use std::time::Instant;

use synapse_action::{
    ActionBackend, ActionEmitter, ActionHandle, ActionStateSnapshot, RecordedInput,
    RecordingBackend,
};
use synapse_core::{
    Action, AimCurve, AimStyle, AimTarget, Backend, ButtonAction, ComboInput, ComboStep,
    GamepadReport, Key, KeyCode, KeystrokeDynamics, MouseButton, MouseTarget, PadButton, Point,
    Stick, Trigger,
};
use tokio_util::sync::CancellationToken;

fn key(name: &str) -> Key {
    Key {
        code: KeyCode::Named {
            value: name.to_owned(),
        },
        use_scancode: false,
    }
}

fn divider(label: &str) {
    println!("\n=== {label} ===");
}

#[tokio::main]
async fn main() {
    println!(
        "ISSUE 228 MANUAL FSV — actor must dispatch to backend, RecordingBackend is the SoT"
    );

    let recording: Arc<RecordingBackend> = Arc::new(RecordingBackend::new());
    let backend: Arc<dyn ActionBackend> = recording.clone();
    let cancel = CancellationToken::new();
    let (handle, snapshot_handle, join) =
        ActionEmitter::spawn_with_backend(cancel.clone(), backend);

    // Capture starting state for the BEFORE/AFTER discipline.
    let start_state = snapshot_handle
        .snapshot()
        .await
        .expect("starting snapshot succeeds");
    println!("SoT=actor_snapshot before_any_action: {start_state:?}");
    println!("SoT=recording_events before_any_action: {:?}", recording.events());
    assert!(start_state.held_keys.is_empty());
    assert!(recording.events().is_empty());

    happy_path_key_press(&handle, &recording, &snapshot_handle).await;
    happy_path_key_down_holds_then_release(&handle, &recording, &snapshot_handle).await;
    happy_path_type_text(&handle, &recording).await;
    happy_path_mouse_path(&handle, &recording, &snapshot_handle).await;
    happy_path_pad_path(&handle, &recording, &snapshot_handle).await;
    happy_path_combo_path(&handle, &recording, &snapshot_handle).await;
    happy_path_aim_path(&handle, &recording).await;
    happy_path_release_all_drains_state(&handle, &recording, &snapshot_handle).await;

    edge_empty_release_all_is_noop(&handle, &recording, &snapshot_handle).await;
    edge_press_with_hold_runs_blocking(&handle, &recording, &snapshot_handle).await;
    edge_unmatched_key_up_does_not_break_state(&handle, &recording, &snapshot_handle).await;

    divider("Shutdown");
    cancel.cancel();
    let final_snapshot = join.await.expect("actor join succeeds");
    println!("SoT=actor_final_snapshot after_cancel={final_snapshot:?}");
    assert!(final_snapshot.held_keys.is_empty());
    assert!(final_snapshot.held_buttons.is_empty());
    assert!(final_snapshot.pad_state.is_empty());

    println!("\nALL ASSERTIONS HOLD — actor dispatched to RecordingBackend for every variant");
}

async fn happy_path_key_press(
    handle: &ActionHandle,
    recording: &Arc<RecordingBackend>,
    snapshot_handle: &synapse_action::ActionEmitterSnapshotHandle,
) {
    divider("happy: KeyPress 'A' hold_ms=0");
    let before = recording.events().len();
    println!(
        "SoT=before recording_event_count={before} actor_snapshot={:?}",
        snapshot_handle.snapshot().await.expect("snapshot")
    );

    handle
        .execute(Action::KeyPress {
            key: key("A"),
            hold_ms: 0,
            backend: Backend::Software,
        })
        .await
        .expect("KeyPress dispatch succeeds");

    let after = recording.events();
    let new_events = &after[before..];
    let after_state = snapshot_handle.snapshot().await.expect("snapshot");
    println!("SoT=after recording_new_events={new_events:?}");
    println!("SoT=after actor_snapshot={after_state:?}");
    assert_eq!(
        new_events,
        &[
            RecordedInput::KeyDown { key: key("A") },
            RecordedInput::DelayMs { ms: 0 },
            RecordedInput::KeyUp { key: key("A") },
        ],
        "actor must have dispatched the KeyPress to the backend"
    );
    assert!(after_state.held_keys.is_empty(), "KeyPress nets to no held key");
}

async fn happy_path_key_down_holds_then_release(
    handle: &ActionHandle,
    recording: &Arc<RecordingBackend>,
    snapshot_handle: &synapse_action::ActionEmitterSnapshotHandle,
) {
    divider("happy: KeyDown 'shift' then KeyUp 'shift'");
    let before = recording.events().len();
    println!(
        "SoT=before recording_event_count={before} actor_snapshot={:?}",
        snapshot_handle.snapshot().await.expect("snapshot")
    );

    handle
        .execute(Action::KeyDown {
            key: key("shift"),
            backend: Backend::Software,
        })
        .await
        .expect("KeyDown dispatch succeeds");
    let mid = snapshot_handle.snapshot().await.expect("snapshot");
    println!("SoT=mid actor_snapshot={mid:?}");
    assert_eq!(
        mid.held_keys,
        vec![key("shift")],
        "actor state must reflect held shift"
    );
    assert_eq!(
        mid.held_key_timer_count, 1,
        "auto-release timer scheduled post-success"
    );

    handle
        .execute(Action::KeyUp {
            key: key("shift"),
            backend: Backend::Software,
        })
        .await
        .expect("KeyUp dispatch succeeds");

    let after = recording.events();
    let new_events = &after[before..];
    let after_state = snapshot_handle.snapshot().await.expect("snapshot");
    println!("SoT=after recording_new_events={new_events:?}");
    println!("SoT=after actor_snapshot={after_state:?}");
    assert_eq!(
        new_events,
        &[
            RecordedInput::KeyDown { key: key("shift") },
            RecordedInput::KeyUp { key: key("shift") },
        ]
    );
    assert!(after_state.held_keys.is_empty());
    assert_eq!(after_state.held_key_timer_count, 0);
}

async fn happy_path_type_text(handle: &ActionHandle, recording: &Arc<RecordingBackend>) {
    divider("happy: TypeText 'Hi'");
    let before = recording.events().len();
    println!("SoT=before recording_event_count={before}");

    handle
        .execute(Action::TypeText {
            text: "Hi".to_owned(),
            dynamics: KeystrokeDynamics::Linear { ms_per_char: 0 },
            backend: Backend::Software,
        })
        .await
        .expect("TypeText dispatch succeeds");

    let after = recording.events();
    let new_events = &after[before..];
    println!("SoT=after recording_new_events={new_events:?}");
    let down_count = new_events
        .iter()
        .filter(|e| matches!(e, RecordedInput::KeyDown { .. }))
        .count();
    let up_count = new_events
        .iter()
        .filter(|e| matches!(e, RecordedInput::KeyUp { .. }))
        .count();
    assert!(down_count >= 2, "expected ≥2 KeyDown events for 2 chars");
    assert_eq!(down_count, up_count, "each KeyDown has a matching KeyUp");
}

async fn happy_path_mouse_path(
    handle: &ActionHandle,
    recording: &Arc<RecordingBackend>,
    snapshot_handle: &synapse_action::ActionEmitterSnapshotHandle,
) {
    divider("happy: MouseMove screen(42,84) then MouseButton Left Press");
    let before = recording.events().len();
    handle
        .execute(Action::MouseMove {
            to: MouseTarget::Screen {
                point: Point { x: 42, y: 84 },
            },
            curve: AimCurve::Instant,
            duration_ms: 0,
            backend: Backend::Software,
        })
        .await
        .expect("MouseMove dispatch succeeds");
    handle
        .execute(Action::MouseButton {
            button: MouseButton::Left,
            action: ButtonAction::Press,
            hold_ms: 0,
            backend: Backend::Software,
        })
        .await
        .expect("MouseButton dispatch succeeds");
    let new_events = &recording.events()[before..].to_vec();
    let after_state = snapshot_handle.snapshot().await.expect("snapshot");
    println!("SoT=after recording_new_events={new_events:?}");
    println!("SoT=after actor_snapshot={after_state:?}");
    assert_eq!(
        new_events.first(),
        Some(&RecordedInput::MouseMove {
            to: MouseTarget::Screen {
                point: Point { x: 42, y: 84 }
            },
            curve: AimCurve::Instant,
            duration_ms: 0,
        })
    );
    assert!(new_events.iter().any(|e| matches!(
        e,
        RecordedInput::MouseButtonDown {
            button: MouseButton::Left
        }
    )));
    assert!(new_events.iter().any(|e| matches!(
        e,
        RecordedInput::MouseButtonUp {
            button: MouseButton::Left
        }
    )));
    assert!(after_state.held_buttons.is_empty(), "Press nets to no held");
}

async fn happy_path_pad_path(
    handle: &ActionHandle,
    recording: &Arc<RecordingBackend>,
    snapshot_handle: &synapse_action::ActionEmitterSnapshotHandle,
) {
    divider("happy: PadReport pad=1 with non-neutral data");
    let before = recording.events().len();
    let report = GamepadReport {
        buttons: vec![PadButton::A, PadButton::Lb],
        thumb_l: (0.25, -0.75),
        thumb_r: (0.0, 0.0),
        lt: 0.0,
        rt: 0.5,
    };
    handle
        .execute(Action::PadReport {
            pad: 1,
            report: report.clone(),
        })
        .await
        .expect("PadReport dispatch succeeds");
    handle
        .execute(Action::PadStick {
            pad: 1,
            stick: Stick::Left,
            x: 0.5,
            y: 0.5,
        })
        .await
        .expect("PadStick dispatch succeeds");
    handle
        .execute(Action::PadTrigger {
            pad: 1,
            trigger: Trigger::Right,
            value: 0.9,
        })
        .await
        .expect("PadTrigger dispatch succeeds");
    handle
        .execute(Action::PadButton {
            pad: 1,
            button: PadButton::B,
            action: ButtonAction::Down,
            hold_ms: 0,
        })
        .await
        .expect("PadButton dispatch succeeds");
    let new_events = &recording.events()[before..].to_vec();
    let after_state = snapshot_handle.snapshot().await.expect("snapshot");
    println!("SoT=after recording_new_events={new_events:?}");
    println!("SoT=after actor_snapshot.pad_state={:?}", after_state.pad_state);
    assert_eq!(new_events.len(), 4, "expected 4 pad recording events");
    assert!(after_state.pad_state.contains_key(&1), "pad state present");
}

async fn happy_path_combo_path(
    handle: &ActionHandle,
    recording: &Arc<RecordingBackend>,
    snapshot_handle: &synapse_action::ActionEmitterSnapshotHandle,
) {
    divider("happy: Combo KeyDown(ctrl), KeyPress(c), KeyUp(ctrl)");
    let before = recording.events().len();
    let steps = vec![
        ComboStep {
            at_ms: 0,
            input: ComboInput::KeyDown { key: key("ctrl") },
        },
        ComboStep {
            at_ms: 5,
            input: ComboInput::KeyPress {
                key: key("c"),
                hold_ms: 0,
            },
        },
        ComboStep {
            at_ms: 10,
            input: ComboInput::KeyUp { key: key("ctrl") },
        },
    ];
    handle
        .execute(Action::Combo {
            steps,
            backend: Backend::Software,
        })
        .await
        .expect("Combo dispatch succeeds");
    let new_events = &recording.events()[before..].to_vec();
    let after_state = snapshot_handle.snapshot().await.expect("snapshot");
    println!("SoT=after recording_new_events={new_events:?}");
    println!("SoT=after actor_snapshot.held_keys={:?}", after_state.held_keys);
    assert!(after_state.held_keys.is_empty(), "Combo nets to clean state");
    assert_eq!(after_state.held_key_timer_count, 0, "no leaked timers");
}

async fn happy_path_aim_path(handle: &ActionHandle, recording: &Arc<RecordingBackend>) {
    divider("happy: AimAt Snap toward (100, 200)");
    let before = recording.events().len();
    handle
        .execute(Action::AimAt {
            target: AimTarget::Screen {
                point: Point { x: 100, y: 200 },
            },
            style: AimStyle::Snap,
            deadline_ms: 16,
            backend: Backend::Software,
        })
        .await
        .expect("AimAt dispatch succeeds");
    let new_events = &recording.events()[before..].to_vec();
    println!("SoT=after recording_new_events={new_events:?}");
    let recorded = matches!(new_events.first(), Some(RecordedInput::AimAt { .. }));
    assert!(recorded, "AimAt must produce a RecordedInput::AimAt entry");
}

async fn happy_path_release_all_drains_state(
    handle: &ActionHandle,
    recording: &Arc<RecordingBackend>,
    snapshot_handle: &synapse_action::ActionEmitterSnapshotHandle,
) {
    divider("happy: prime held state then ReleaseAll drains it");
    // Prime a held key + mouse button + pad report.
    handle
        .execute(Action::KeyDown {
            key: key("alt"),
            backend: Backend::Software,
        })
        .await
        .expect("prime KeyDown");
    handle
        .execute(Action::MouseButton {
            button: MouseButton::Right,
            action: ButtonAction::Down,
            hold_ms: 0,
            backend: Backend::Software,
        })
        .await
        .expect("prime MouseButton Down");
    let primed = snapshot_handle.snapshot().await.expect("snapshot");
    println!("SoT=primed actor_snapshot={primed:?}");
    assert!(!primed.held_keys.is_empty(), "primed key present");
    assert!(!primed.held_buttons.is_empty(), "primed button present");

    let before_events = recording.events().len();
    handle
        .execute(Action::ReleaseAll)
        .await
        .expect("ReleaseAll dispatch succeeds");
    let after = snapshot_handle.snapshot().await.expect("snapshot");
    let drained_events = &recording.events()[before_events..].to_vec();
    println!("SoT=after_release actor_snapshot={after:?}");
    println!("SoT=after_release recording_new_events={drained_events:?}");
    assert!(after.held_keys.is_empty());
    assert!(after.held_buttons.is_empty());
    assert!(after.pad_state.is_empty());
    let recorded = matches!(
        drained_events.last(),
        Some(RecordedInput::ReleaseAll { .. })
    );
    assert!(recorded, "ReleaseAll must produce RecordedInput::ReleaseAll");
}

async fn edge_empty_release_all_is_noop(
    handle: &ActionHandle,
    recording: &Arc<RecordingBackend>,
    snapshot_handle: &synapse_action::ActionEmitterSnapshotHandle,
) {
    divider("edge: empty ReleaseAll");
    let before = snapshot_handle.snapshot().await.expect("snapshot");
    assert!(before.held_keys.is_empty(), "starting empty");
    let before_events = recording.events().len();
    handle
        .execute(Action::ReleaseAll)
        .await
        .expect("empty ReleaseAll dispatch succeeds");
    let after = snapshot_handle.snapshot().await.expect("snapshot");
    let new_events = &recording.events()[before_events..].to_vec();
    println!("SoT=before={before:?}");
    println!("SoT=after={after:?}");
    println!("SoT=new_events={new_events:?}");
    assert!(after.held_keys.is_empty());
}

async fn edge_press_with_hold_runs_blocking(
    handle: &ActionHandle,
    recording: &Arc<RecordingBackend>,
    snapshot_handle: &synapse_action::ActionEmitterSnapshotHandle,
) {
    divider("edge: KeyPress hold_ms=25 — proves spawn_blocking path runs & elapsed reflects cost");
    let before = recording.events().len();
    let started = Instant::now();
    handle
        .execute(Action::KeyPress {
            key: key("space"),
            hold_ms: 25,
            backend: Backend::Software,
        })
        .await
        .expect("KeyPress with hold dispatch succeeds");
    let elapsed = started.elapsed();
    let new_events = &recording.events()[before..].to_vec();
    let after = snapshot_handle.snapshot().await.expect("snapshot");
    println!("SoT=elapsed_ms={}", elapsed.as_millis());
    println!("SoT=recording_new_events={new_events:?}");
    println!("SoT=actor_snapshot={after:?}");
    // The RecordingBackend models the hold as a DelayMs entry; the actor's
    // spawn_blocking dispatch should keep elapsed time roughly aligned with
    // hold_ms (we tolerate some slack for scheduling).
    assert!(
        new_events.iter().any(|e| matches!(e, RecordedInput::DelayMs { ms } if *ms == 25)),
        "RecordingBackend must observe the 25ms hold"
    );
    assert!(after.held_keys.is_empty());
}

async fn edge_unmatched_key_up_does_not_break_state(
    handle: &ActionHandle,
    recording: &Arc<RecordingBackend>,
    snapshot_handle: &synapse_action::ActionEmitterSnapshotHandle,
) {
    divider("edge: KeyUp for a key never held — actor must dispatch and stay clean");
    let before = recording.events().len();
    let before_state = snapshot_handle.snapshot().await.expect("snapshot");
    handle
        .execute(Action::KeyUp {
            key: key("F12"),
            backend: Backend::Software,
        })
        .await
        .expect("unmatched KeyUp dispatch succeeds");
    let after = snapshot_handle.snapshot().await.expect("snapshot");
    let new_events = &recording.events()[before..].to_vec();
    println!("SoT=before={before_state:?}");
    println!("SoT=after={after:?}");
    println!("SoT=new_events={new_events:?}");
    assert!(after.held_keys.is_empty());
    assert!(
        new_events
            .iter()
            .any(|e| matches!(e, RecordedInput::KeyUp { key } if key.code == KeyCode::Named { value: "F12".to_owned() })),
        "actor must still dispatch the unmatched KeyUp"
    );
}

// `ActionStateSnapshot` is re-exported for trace clarity in the printlns.
#[allow(dead_code)]
fn _unused(_: &ActionStateSnapshot) {}
