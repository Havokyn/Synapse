#[cfg(not(feature = "loopback"))]
use pico_hid::dispatch::{
    DispatchOutcome, DispatchState, IdentifyInfo, MAX_RESPONSE_PAYLOAD_LEN, dispatch_frame,
};
#[cfg(not(feature = "loopback"))]
use pico_hid::protocol::DeviceCommand;
use pico_hid::protocol::{
    DropReason, EncodeError, HOST_MAGIC, HostCommand, MAX_FRAME_LEN, MAX_PAYLOAD_LEN, NakReason,
    ParseResult, crc16_ccitt_false, encode_host_frame, next_sequence, parse_host_frame,
};
#[cfg(not(feature = "loopback"))]
use pico_hid::reports::{GAMEPAD_REPORT_LEN, GamepadReport};

#[test]
fn crc16_ccitt_false_known_check_value() {
    assert_eq!(crc16_ccitt_false(b"123456789"), 0x29B1);
}

#[test]
fn malformed_frames_return_drop_or_nak_state() {
    let mut frame = [0u8; MAX_FRAME_LEN];
    let payload = [0xAA, 0x55, 0x10];
    let len = encode_host_frame(7, HostCommand::Ping, &payload, &mut frame).unwrap();

    let before_bad_magic = frame[..len].to_vec();
    frame[0] = 0x00;
    assert_eq!(
        parse_host_frame(&frame[..len]),
        ParseResult::Drop {
            reason: DropReason::BadMagic,
            consumed: 1
        }
    );
    frame[..len].copy_from_slice(&before_bad_magic);

    frame[len - 1] ^= 0x01;
    assert_eq!(
        parse_host_frame(&frame[..len]),
        ParseResult::Nak {
            nak: pico_hid::protocol::Nak {
                seq: 7,
                command: HostCommand::Ping as u8,
                reason: NakReason::CrcInvalid,
            },
            consumed: len,
        }
    );

    let overflow = [HOST_MAGIC, 0xFF, 0xFF];
    assert_eq!(
        parse_host_frame(&overflow),
        ParseResult::Drop {
            reason: DropReason::LenOverflow,
            consumed: 1
        }
    );
}

#[test]
fn sequence_wraparound_uses_wrapping_add() {
    assert_eq!(next_sequence(u32::MAX), 0);
    assert_eq!(next_sequence(0), 1);
}

#[test]
fn encode_rejects_payload_or_output_overflow() {
    let payload = [0u8; MAX_PAYLOAD_LEN + 1];
    let mut frame = [0u8; MAX_FRAME_LEN];
    assert_eq!(
        encode_host_frame(1, HostCommand::Ping, &payload, &mut frame),
        Err(EncodeError::PayloadTooLarge)
    );

    let payload = [0u8; 4];
    let mut too_small = [0u8; 4];
    assert_eq!(
        encode_host_frame(1, HostCommand::Ping, &payload, &mut too_small),
        Err(EncodeError::OutputTooSmall { needed: 14 })
    );
}

#[test]
fn protocol_roundtrips_1000_deterministic_frames() {
    let mut frame = [0u8; MAX_FRAME_LEN];
    let mut payload = [0u8; 128];
    let mut seed = 0x0C0D_E373_u32;

    for seq in 0..1000u32 {
        seed = lcg(seed);
        let payload_len = (seed as usize) % payload.len();
        for byte in &mut payload[..payload_len] {
            seed = lcg(seed);
            *byte = (seed >> 24) as u8;
        }

        let len = encode_host_frame(
            seq,
            HostCommand::PadReport,
            &payload[..payload_len],
            &mut frame,
        )
        .unwrap();

        match parse_host_frame(&frame[..len]) {
            ParseResult::Frame {
                frame: parsed,
                consumed,
            } => {
                assert_eq!(consumed, len);
                assert_eq!(parsed.seq, seq);
                assert_eq!(parsed.command, HostCommand::PadReport as u8);
                assert_eq!(parsed.payload, &payload[..payload_len]);
            }
            other => panic!("expected frame for seq {seq}, got {other:?}"),
        }
    }
}

#[test]
#[cfg(not(feature = "loopback"))]
fn dispatcher_applies_mouse_keyboard_pad_and_release_all() {
    let identify = IdentifyInfo::new(*b"TESTHASH", 0x2E8A, 0x1F50);
    let mut state = DispatchState::new();

    let mouse_move = frame(1, HostCommand::MouseMoveRel, &[5, 0, 0xFD, 0xFF]);
    assert_eq!(dispatch_frame(&mut state, mouse_move, identify), ack(1));
    assert_eq!(state.mouse.to_bytes(), [0, 5, 0xFD, 0]);

    let mouse_button_down = frame(2, HostCommand::MouseButton, &[1, 1]);
    assert_eq!(
        dispatch_frame(&mut state, mouse_button_down, identify),
        ack(2)
    );
    assert_eq!(state.mouse.buttons, 1);

    let key_down = frame(3, HostCommand::KeyDown, &[0x04]);
    assert_eq!(dispatch_frame(&mut state, key_down, identify), ack(3));
    assert_eq!(state.keyboard.to_bytes(), [0, 0, 0x04, 0, 0, 0, 0, 0]);

    let key_mods = frame(4, HostCommand::KeyMods, &[0x03]);
    assert_eq!(dispatch_frame(&mut state, key_mods, identify), ack(4));
    assert_eq!(state.keyboard.modifiers, 0x03);

    let gamepad = GamepadReport {
        buttons: 0x0003,
        left_trigger: 7,
        right_trigger: 9,
        thumb_lx: -10,
        thumb_ly: 11,
        thumb_rx: -12,
        thumb_ry: 13,
        reserved: 0,
    };
    let gamepad_bytes = gamepad.to_bytes();
    let pad_report = frame(5, HostCommand::PadReport, &gamepad_bytes);
    assert_eq!(dispatch_frame(&mut state, pad_report, identify), ack(5));
    assert_eq!(state.gamepad, gamepad);

    let release_all = frame(6, HostCommand::ReleaseAll, &[]);
    assert_eq!(dispatch_frame(&mut state, release_all, identify), ack(6));
    assert_eq!(state.mouse.to_bytes(), [0; 4]);
    assert_eq!(state.keyboard.to_bytes(), [0; 8]);
    assert_eq!(state.gamepad, GamepadReport::neutral());
}

#[test]
#[cfg(not(feature = "loopback"))]
fn dispatcher_returns_query_responses_and_updates_telemetry() {
    let identify = IdentifyInfo::new(*b"TESTHASH", 0x2E8A, 0x1F50);
    let mut state = DispatchState::new();

    let ping = frame(10, HostCommand::Ping, &[1, 2, 3, 4]);
    let ping_outcome = dispatch_frame(&mut state, ping, identify);
    assert_eq!(ping_outcome.command, DeviceCommand::Pong);
    assert_eq!(
        &ping_outcome.payload[..ping_outcome.payload_len],
        &[1, 2, 3, 4]
    );

    let identify_frame = frame(11, HostCommand::Identify, &[]);
    let identify_outcome = dispatch_frame(&mut state, identify_frame, identify);
    assert_eq!(identify_outcome.command, DeviceCommand::IdentifyResp);
    assert_eq!(identify_outcome.payload_len, 20);
    assert_eq!(&identify_outcome.payload[4..12], b"TESTHASH");
    assert_eq!(
        u16::from_le_bytes([identify_outcome.payload[12], identify_outcome.payload[13]]),
        0x2E8A
    );
    assert_eq!(
        u16::from_le_bytes([identify_outcome.payload[14], identify_outcome.payload[15]]),
        0x1F50
    );

    let watchdog_timeout = 2500u32.to_le_bytes();
    let watchdog = frame(12, HostCommand::WatchdogKick, &watchdog_timeout);
    assert_eq!(dispatch_frame(&mut state, watchdog, identify), ack(12));
    assert_eq!(state.watchdog_timeout_ms, 2500);

    state.telemetry.uptime_ms = 1234;
    let telemetry = frame(13, HostCommand::GetTelemetry, &[]);
    let telemetry_outcome = dispatch_frame(&mut state, telemetry, identify);
    assert_eq!(telemetry_outcome.command, DeviceCommand::TelemetryResp);
    assert_eq!(telemetry_outcome.payload_len, 28);
    assert_eq!(
        u32::from_le_bytes([
            telemetry_outcome.payload[0],
            telemetry_outcome.payload[1],
            telemetry_outcome.payload[2],
            telemetry_outcome.payload[3],
        ]),
        1234
    );
}

#[test]
#[cfg(not(feature = "loopback"))]
fn telemetry_response_contains_all_counter_fields() {
    let identify = IdentifyInfo::new(*b"TESTHASH", 0x2E8A, 0x1F50);
    let mut state = DispatchState::new();
    state.telemetry.uptime_ms = 1;
    state.telemetry.frames_received = 10;
    state.telemetry.frames_dropped = 2;
    state.telemetry.link_errors = 3;
    state.telemetry.commands_executed = 4;
    state.telemetry.watchdog_fires = 5;
    state.telemetry.crc_errors = 6;

    let telemetry = dispatch_frame(
        &mut state,
        frame(14, HostCommand::GetTelemetry, &[]),
        identify,
    );
    assert_eq!(telemetry.command, DeviceCommand::TelemetryResp);
    assert_eq!(telemetry.payload_len, 28);
    assert_eq!(payload_u32(&telemetry, 0), 1);
    assert_eq!(payload_u32(&telemetry, 4), 11);
    assert_eq!(payload_u32(&telemetry, 8), 2);
    assert_eq!(payload_u32(&telemetry, 12), 3);
    assert_eq!(payload_u32(&telemetry, 16), 4);
    assert_eq!(payload_u32(&telemetry, 20), 5);
    assert_eq!(payload_u32(&telemetry, 24), 6);
    assert_eq!(state.telemetry.commands_executed, 5);
}

#[test]
#[cfg(not(feature = "loopback"))]
fn release_all_does_not_reset_telemetry_counters() {
    let identify = IdentifyInfo::new(*b"TESTHASH", 0x2E8A, 0x1F50);
    let mut state = DispatchState::new();
    state.telemetry.frames_dropped = 9;
    state.telemetry.link_errors = 8;
    state.telemetry.commands_executed = 7;
    state.telemetry.watchdog_fires = 6;
    state.telemetry.crc_errors = 5;

    assert_eq!(
        dispatch_frame(
            &mut state,
            frame(15, HostCommand::ReleaseAll, &[]),
            identify
        ),
        ack(15)
    );

    assert_eq!(state.telemetry.frames_dropped, 9);
    assert_eq!(state.telemetry.link_errors, 8);
    assert_eq!(state.telemetry.commands_executed, 8);
    assert_eq!(state.telemetry.watchdog_fires, 6);
    assert_eq!(state.telemetry.crc_errors, 5);
}

#[test]
#[cfg(not(feature = "loopback"))]
fn dispatcher_rejects_invalid_payload_boundaries() {
    let identify = IdentifyInfo::new(*b"TESTHASH", 0x2E8A, 0x1F50);
    let mut state = DispatchState::new();

    let mut invalid_mouse_payload = [0u8; 4];
    invalid_mouse_payload[..2].copy_from_slice(&200i16.to_le_bytes());
    let invalid_mouse = frame(20, HostCommand::MouseMoveRel, &invalid_mouse_payload);
    assert_eq!(
        dispatch_frame(&mut state, invalid_mouse, identify),
        nak(20, NakReason::PayloadInvalid)
    );

    for seq in 21..=26 {
        let key = [seq as u8];
        assert_eq!(
            dispatch_frame(&mut state, frame(seq, HostCommand::KeyDown, &key), identify),
            ack(seq)
        );
    }
    assert_eq!(
        dispatch_frame(
            &mut state,
            frame(27, HostCommand::KeyDown, &[0x30]),
            identify
        ),
        nak(27, NakReason::BufferFull)
    );

    let mut bad_pad = [0u8; GAMEPAD_REPORT_LEN];
    bad_pad[GAMEPAD_REPORT_LEN - 1] = 1;
    assert_eq!(
        dispatch_frame(
            &mut state,
            frame(28, HostCommand::PadReport, &bad_pad),
            identify
        ),
        nak(28, NakReason::PayloadInvalid)
    );
}

fn lcg(seed: u32) -> u32 {
    seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223)
}

#[cfg(not(feature = "loopback"))]
fn frame<'a>(seq: u32, command: HostCommand, payload: &'a [u8]) -> pico_hid::protocol::Frame<'a> {
    pico_hid::protocol::Frame {
        seq,
        command: command as u8,
        payload,
    }
}

#[cfg(not(feature = "loopback"))]
fn ack(seq: u32) -> DispatchOutcome {
    let mut payload = [0u8; MAX_RESPONSE_PAYLOAD_LEN];
    payload[..4].copy_from_slice(&seq.to_le_bytes());
    DispatchOutcome {
        command: DeviceCommand::Ack,
        payload,
        payload_len: 4,
    }
}

#[cfg(not(feature = "loopback"))]
fn nak(seq: u32, reason: NakReason) -> DispatchOutcome {
    let mut payload = [0u8; MAX_RESPONSE_PAYLOAD_LEN];
    payload[..4].copy_from_slice(&seq.to_le_bytes());
    payload[4] = reason as u8;
    DispatchOutcome {
        command: DeviceCommand::Nak,
        payload,
        payload_len: 5,
    }
}

#[cfg(not(feature = "loopback"))]
fn payload_u32(outcome: &DispatchOutcome, start: usize) -> u32 {
    u32::from_le_bytes([
        outcome.payload[start],
        outcome.payload[start + 1],
        outcome.payload[start + 2],
        outcome.payload[start + 3],
    ])
}
