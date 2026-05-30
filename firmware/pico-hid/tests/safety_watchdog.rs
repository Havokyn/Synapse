#![cfg(all(not(feature = "loopback"), not(feature = "force-first-nak")))]

use pico_hid::dispatch::{DispatchState, IdentifyInfo, dispatch_frame};
use pico_hid::protocol::{Frame, HostCommand};
use pico_hid::reports::GamepadReport;
use pico_hid::safety::{
    DEFAULT_WATCHDOG_TIMEOUT_MS, WATCHDOG_DISABLED_TIMEOUT_MS, Watchdog, WatchdogPoll,
};

#[test]
fn watchdog_releases_all_after_default_timeout_once() {
    let identify = IdentifyInfo::new(*b"TESTHASH", 0x2E8A, 0x1F50);
    let mut state = DispatchState::new();
    let mut watchdog = Watchdog::new();

    dispatch_frame(
        &mut state,
        frame(1, HostCommand::MouseButton, &[1, 1]),
        identify,
    );
    dispatch_frame(
        &mut state,
        frame(2, HostCommand::KeyDown, &[0x04]),
        identify,
    );
    watchdog.record_valid_command(0, state.watchdog_timeout_ms);

    assert_eq!(
        watchdog.poll(DEFAULT_WATCHDOG_TIMEOUT_MS - 1, &mut state),
        WatchdogPoll::Noop
    );
    assert_eq!(state.mouse.buttons, 1);
    assert_eq!(state.keyboard.keycodes[0], 0x04);
    assert_eq!(state.telemetry.watchdog_fires, 0);

    assert_eq!(
        watchdog.poll(DEFAULT_WATCHDOG_TIMEOUT_MS, &mut state),
        WatchdogPoll::Fired
    );
    assert_eq!(state.mouse.to_bytes(), [0; 4]);
    assert_eq!(state.keyboard.to_bytes(), [0; 8]);
    assert_eq!(state.gamepad, GamepadReport::neutral());
    assert_eq!(state.telemetry.watchdog_fires, 1);

    assert_eq!(
        watchdog.poll(DEFAULT_WATCHDOG_TIMEOUT_MS + 200, &mut state),
        WatchdogPoll::Noop
    );
    assert_eq!(state.telemetry.watchdog_fires, 1);
}

#[test]
fn watchdog_disabled_timeout_never_releases_inputs() {
    let identify = IdentifyInfo::new(*b"TESTHASH", 0x2E8A, 0x1F50);
    let mut state = DispatchState::new();
    let mut watchdog = Watchdog::new();

    let disabled = WATCHDOG_DISABLED_TIMEOUT_MS.to_le_bytes();
    dispatch_frame(
        &mut state,
        frame(3, HostCommand::WatchdogKick, &disabled),
        identify,
    );
    watchdog.record_valid_command(10, state.watchdog_timeout_ms);
    dispatch_frame(
        &mut state,
        frame(4, HostCommand::KeyDown, &[0x05]),
        identify,
    );

    assert_eq!(watchdog.poll(60_000, &mut state), WatchdogPoll::Disabled);
    assert_eq!(state.keyboard.keycodes[0], 0x05);
    assert_eq!(state.telemetry.watchdog_fires, 0);
}

#[test]
fn fresh_command_after_watchdog_fire_resumes_normal_operation() {
    let identify = IdentifyInfo::new(*b"TESTHASH", 0x2E8A, 0x1F50);
    let mut state = DispatchState::new();
    let mut watchdog = Watchdog::new();

    dispatch_frame(
        &mut state,
        frame(5, HostCommand::KeyDown, &[0x04]),
        identify,
    );
    watchdog.record_valid_command(0, state.watchdog_timeout_ms);
    assert_eq!(
        watchdog.poll(DEFAULT_WATCHDOG_TIMEOUT_MS, &mut state),
        WatchdogPoll::Fired
    );
    assert_eq!(state.keyboard.to_bytes(), [0; 8]);

    dispatch_frame(
        &mut state,
        frame(6, HostCommand::KeyDown, &[0x06]),
        identify,
    );
    watchdog.record_valid_command(1200, state.watchdog_timeout_ms);
    assert!(!watchdog.fired());
    assert_eq!(state.keyboard.keycodes[0], 0x06);
    assert_eq!(watchdog.poll(2199, &mut state), WatchdogPoll::Noop);
    assert_eq!(state.keyboard.keycodes[0], 0x06);

    assert_eq!(watchdog.poll(2200, &mut state), WatchdogPoll::Fired);
    assert_eq!(state.keyboard.to_bytes(), [0; 8]);
    assert_eq!(state.telemetry.watchdog_fires, 2);
}

#[test]
fn watchdog_deadline_uses_wrapping_elapsed_time() {
    let mut state = DispatchState::new();
    let mut watchdog = Watchdog::new();
    let last_valid = u32::MAX - 500;

    state.mouse.buttons = 1;
    watchdog.record_valid_command(last_valid, DEFAULT_WATCHDOG_TIMEOUT_MS);

    assert_eq!(watchdog.poll(498, &mut state), WatchdogPoll::Noop);
    assert_eq!(state.mouse.buttons, 1);

    assert_eq!(watchdog.poll(499, &mut state), WatchdogPoll::Fired);
    assert_eq!(state.mouse.to_bytes(), [0; 4]);
    assert_eq!(state.telemetry.watchdog_fires, 1);
}

fn frame<'a>(seq: u32, command: HostCommand, payload: &'a [u8]) -> Frame<'a> {
    Frame {
        seq,
        command: command as u8,
        payload,
    }
}
