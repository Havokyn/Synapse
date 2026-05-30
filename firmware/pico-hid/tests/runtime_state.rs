#![cfg(all(not(feature = "loopback"), not(feature = "force-first-nak")))]

use pico_hid::dispatch::IdentifyInfo;
use pico_hid::led::{LedMode, led_output};
use pico_hid::protocol::{DeviceCommand, Frame, HostCommand, NakReason};
use pico_hid::reports::{GAMEPAD_REPORT_LEN, GamepadReport};
use pico_hid::runtime::RuntimeState;
use pico_hid::safety::WatchdogPoll;

#[test]
fn runtime_dispatch_reports_feed_hid_readback() {
    let identify = IdentifyInfo::new(*b"TESTHASH", 0x2E8A, 0x1F50);
    let mut runtime = RuntimeState::new();

    let mouse = runtime.dispatch_frame_at(
        10,
        frame(1, HostCommand::MouseMoveRel, &[5, 0, 0xFD, 0xFF]),
        identify,
    );
    assert_eq!(mouse.command, DeviceCommand::Ack);
    assert_eq!(runtime.mouse_report_for_hid().to_bytes(), [0, 5, 0xFD, 0]);
    assert_eq!(runtime.mouse_report_for_hid().to_bytes(), [0, 0, 0, 0]);

    let key = runtime.dispatch_frame_at(20, frame(2, HostCommand::KeyDown, &[0x04]), identify);
    assert_eq!(key.command, DeviceCommand::Ack);
    assert_eq!(
        runtime.keyboard_report_for_hid().to_bytes(),
        [0, 0, 0x04, 0, 0, 0, 0, 0]
    );

    let mut pad = [0u8; GAMEPAD_REPORT_LEN];
    pad[0] = 0x03;
    pad[2] = 0x7F;
    let pad_outcome =
        runtime.dispatch_frame_at(30, frame(3, HostCommand::PadReport, &pad), identify);
    assert_eq!(pad_outcome.command, DeviceCommand::Ack);
    assert_eq!(runtime.gamepad_report_for_hid().buttons, 0x0003);
    assert_eq!(runtime.gamepad_report_for_hid().left_trigger, 0x7F);
}

#[test]
fn runtime_watchdog_releases_reports_and_marks_led_input() {
    let identify = IdentifyInfo::new(*b"TESTHASH", 0x2E8A, 0x1F50);
    let mut runtime = RuntimeState::new();

    runtime.dispatch_frame_at(100, frame(1, HostCommand::KeyDown, &[0x04]), identify);
    assert_eq!(runtime.poll_watchdog(1099), WatchdogPoll::Noop);
    assert_eq!(runtime.keyboard_report_for_hid().keycodes[0], 0x04);

    assert_eq!(runtime.poll_watchdog(1100), WatchdogPoll::Fired);
    assert_eq!(runtime.keyboard_report_for_hid().to_bytes(), [0; 8]);
    assert_eq!(runtime.dispatch_state().telemetry.watchdog_fires, 1);
    assert_eq!(
        led_output(runtime.led_inputs(1100)).mode,
        LedMode::WatchdogFastBlink
    );

    runtime.dispatch_frame_at(1200, frame(2, HostCommand::KeyDown, &[0x05]), identify);
    assert_eq!(runtime.keyboard_report_for_hid().keycodes[0], 0x05);
    assert_eq!(
        led_output(runtime.led_inputs(1200)).mode,
        LedMode::WatchdogFastBlink
    );
    assert_eq!(
        led_output(runtime.led_inputs(3201)).mode,
        LedMode::ActiveSteady
    );
}

#[test]
fn runtime_crc_window_drives_error_led_priority() {
    let mut runtime = RuntimeState::new();

    for offset in 0..=10 {
        runtime.record_parser_nak(1000 + offset, NakReason::CrcInvalid);
    }

    let output = led_output(runtime.led_inputs(1100));
    assert_eq!(output.mode, LedMode::ErrorSos);
    assert_eq!(runtime.dispatch_state().telemetry.crc_errors, 11);
    assert_eq!(runtime.dispatch_state().telemetry.link_errors, 11);

    let cooled = led_output(runtime.led_inputs(2200));
    assert_eq!(cooled.mode, LedMode::IdleSlowBlink);
}

#[test]
fn runtime_invalid_payload_does_not_refresh_watchdog_or_reports() {
    let identify = IdentifyInfo::new(*b"TESTHASH", 0x2E8A, 0x1F50);
    let mut runtime = RuntimeState::new();
    let mut bad_pad = [0u8; GAMEPAD_REPORT_LEN];
    bad_pad[GAMEPAD_REPORT_LEN - 1] = 1;

    let outcome =
        runtime.dispatch_frame_at(50, frame(1, HostCommand::PadReport, &bad_pad), identify);
    assert_eq!(outcome.command, DeviceCommand::Nak);
    assert_eq!(runtime.gamepad_report_for_hid(), GamepadReport::neutral());
    assert_eq!(runtime.poll_watchdog(1000), WatchdogPoll::Fired);
}

fn frame<'a>(seq: u32, command: HostCommand, payload: &'a [u8]) -> Frame<'a> {
    Frame {
        seq,
        command: command as u8,
        payload,
    }
}
