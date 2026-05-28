#![no_main]

use libfuzzer_sys::fuzz_target;
use pico_hid::protocol::{
    CRC_SIZE as FW_CRC_SIZE, DEVICE_MAGIC as FW_DEVICE_MAGIC, DeviceCommand, DropReason,
    EncodeError as FirmwareEncodeError, HostCommand, LEN_FIELD_SIZE as FW_LEN_FIELD_SIZE,
    MAX_FRAME_LEN as FW_MAX_FRAME_LEN, MAX_PAYLOAD_LEN as FW_MAX_PAYLOAD_LEN,
    MIN_LEN_FIELD as FW_MIN_LEN_FIELD, NakReason, ParseResult,
    crc16_ccitt_false as firmware_crc16_ccitt_false, encode_ack as encode_firmware_ack,
    encode_device_frame as encode_firmware_device, encode_host_frame as encode_firmware_host,
    encode_nak as encode_firmware_nak, next_sequence, parse_device_frame as parse_firmware_device,
    parse_host_frame as parse_firmware_host, parse_host_frame_any_command,
};
use synapse_hid_host::protocol::{
    CRC_SIZE, DEVICE_MAGIC, EncodeError, HOST_COMMAND_IDENTIFY, HOST_COMMAND_PING, HOST_MAGIC,
    LEN_FIELD_SIZE, MAX_FRAME_LEN, MAX_PAYLOAD_LEN, MIN_LEN_FIELD, ParseError, crc16_ccitt_false,
    encode_device_frame, encode_host_frame, encode_identify_frame, parse_device_frame,
    parse_device_frame_prefix,
};

fuzz_target!(|data: &[u8]| {
    check_host_device_parser(data);
    check_firmware_parser(data, FW_DEVICE_MAGIC, parse_firmware_device);
    check_firmware_parser(data, pico_hid::protocol::HOST_MAGIC, parse_firmware_host);
    check_host_round_trips(data);
    check_firmware_round_trips(data);
    check_protocol_helpers(data);
});

fn check_host_device_parser(input: &[u8]) {
    match parse_device_frame_prefix(input) {
        Ok((frame, consumed)) => {
            assert!(consumed <= input.len());
            assert_eq!(input[0], DEVICE_MAGIC);

            let len = read_len(input);
            assert!(len >= usize::from(MIN_LEN_FIELD));
            assert_eq!(consumed, 1 + LEN_FIELD_SIZE + len);
            assert_eq!(frame.payload.len(), len - usize::from(MIN_LEN_FIELD));

            let crc_start = consumed - CRC_SIZE;
            assert_eq!(frame.command, input[7]);
            assert_eq!(
                frame.seq,
                u32::from_le_bytes([input[3], input[4], input[5], input[6]])
            );
            assert_eq!(
                u16::from_le_bytes([input[crc_start], input[crc_start + 1]]),
                crc16_ccitt_false(&input[1..crc_start])
            );
        }
        Err(ParseError::NeedMore { needed }) => {
            assert!(needed > input.len());
        }
        Err(ParseError::BadMagic { actual }) => {
            assert!(!input.is_empty());
            assert_eq!(actual, input[0]);
            assert_ne!(actual, DEVICE_MAGIC);
        }
        Err(ParseError::LenTooShort { len }) => {
            assert!(input.len() >= 3);
            assert_eq!(len, read_len(input));
            assert!(len < usize::from(MIN_LEN_FIELD));
        }
        Err(ParseError::LenOverflow { payload_len }) => {
            assert!(input.len() >= 3);
            assert_eq!(payload_len, read_len(input) - usize::from(MIN_LEN_FIELD));
            assert!(payload_len > MAX_PAYLOAD_LEN);
        }
        Err(ParseError::CrcInvalid { expected, actual }) => {
            assert!(input.len() >= 3);
            let len = read_len(input);
            let frame_len = 1 + LEN_FIELD_SIZE + len;
            assert!(input.len() >= frame_len);
            let crc_start = frame_len - CRC_SIZE;
            assert_eq!(
                expected,
                u16::from_le_bytes([input[crc_start], input[crc_start + 1]])
            );
            assert_eq!(actual, crc16_ccitt_false(&input[1..crc_start]));
            assert_ne!(expected, actual);
        }
    }
}

fn check_firmware_parser<'a>(
    input: &'a [u8],
    expected_magic: u8,
    parser: fn(&'a [u8]) -> ParseResult<'a>,
) {
    match parser(input) {
        ParseResult::Frame { frame, consumed } => {
            assert!(consumed <= input.len());
            assert_eq!(input[0], expected_magic);

            let len = read_len(input);
            assert!(len >= usize::from(FW_MIN_LEN_FIELD));
            assert_eq!(consumed, 1 + FW_LEN_FIELD_SIZE + len);
            assert_eq!(frame.payload.len(), len - usize::from(FW_MIN_LEN_FIELD));

            let crc_start = consumed - FW_CRC_SIZE;
            assert_eq!(frame.command, input[7]);
            assert_eq!(
                frame.seq,
                u32::from_le_bytes([input[3], input[4], input[5], input[6]])
            );
            assert_eq!(
                u16::from_le_bytes([input[crc_start], input[crc_start + 1]]),
                firmware_crc16_ccitt_false(&input[1..crc_start])
            );
        }
        ParseResult::NeedMore { needed } => {
            assert!(needed > input.len());
        }
        ParseResult::Drop { reason, consumed } => {
            assert_eq!(consumed, 1);
            match reason {
                DropReason::BadMagic => {
                    assert!(!input.is_empty());
                    assert_ne!(input[0], expected_magic);
                }
                DropReason::LenTooShort => {
                    assert!(input.len() >= 3);
                    assert_eq!(input[0], expected_magic);
                    assert!(read_len(input) < usize::from(FW_MIN_LEN_FIELD));
                }
                DropReason::LenOverflow => {
                    assert!(input.len() >= 3);
                    assert_eq!(input[0], expected_magic);
                    assert!(read_len(input) - usize::from(FW_MIN_LEN_FIELD) > FW_MAX_PAYLOAD_LEN);
                }
            }
        }
        ParseResult::Nak { nak, consumed } => {
            assert!(input.len() >= consumed);
            assert!(consumed >= 1 + FW_LEN_FIELD_SIZE + usize::from(FW_MIN_LEN_FIELD));
            let crc_start = consumed - FW_CRC_SIZE;
            let expected = u16::from_le_bytes([input[crc_start], input[crc_start + 1]]);
            let actual = firmware_crc16_ccitt_false(&input[1..crc_start]);
            if nak.reason == pico_hid::protocol::NakReason::CrcInvalid {
                assert_ne!(expected, actual);
            } else {
                assert_eq!(expected, actual);
            }
        }
    }
}

fn read_len(input: &[u8]) -> usize {
    usize::from(u16::from_le_bytes([input[1], input[2]]))
}

fn check_host_round_trips(input: &[u8]) {
    let seq = input_seq(input);
    let command = input.first().copied().unwrap_or(HOST_COMMAND_PING);
    let payload = clipped_payload(input, MAX_PAYLOAD_LEN);
    let mut frame = [0u8; MAX_FRAME_LEN];

    let host_len = encode_host_frame(seq, command, payload, &mut frame)
        .expect("clipped host payload fits max frame");
    assert_eq!(frame[0], HOST_MAGIC);
    assert_eq!(
        host_len,
        1 + LEN_FIELD_SIZE + usize::from(MIN_LEN_FIELD) + payload.len()
    );

    let device_len = encode_device_frame(seq, command, payload, &mut frame)
        .expect("clipped device payload fits max frame");
    let parsed = parse_device_frame(&frame[..device_len]).expect("encoded device frame parses");
    assert_eq!(parsed.seq, seq);
    assert_eq!(parsed.command, command);
    assert_eq!(parsed.payload, payload);

    let identify_len =
        encode_identify_frame(seq, &mut frame).expect("empty identify frame fits max frame");
    assert_eq!(frame[0], HOST_MAGIC);
    assert_eq!(frame[7], HOST_COMMAND_IDENTIFY);
    assert_eq!(
        identify_len,
        1 + LEN_FIELD_SIZE + usize::from(MIN_LEN_FIELD)
    );

    let mut too_small = [0u8; 1];
    assert_eq!(
        encode_host_frame(seq, command, payload, &mut too_small),
        Err(EncodeError::OutputTooSmall {
            needed: 1 + LEN_FIELD_SIZE + usize::from(MIN_LEN_FIELD) + payload.len()
        })
    );

    if input.len() > MAX_PAYLOAD_LEN {
        assert_eq!(
            encode_device_frame(seq, command, input, &mut frame),
            Err(EncodeError::PayloadTooLarge)
        );
    }
}

fn check_firmware_round_trips(input: &[u8]) {
    let seq = input_seq(input);
    let payload = clipped_payload(input, FW_MAX_PAYLOAD_LEN);
    let mut frame = [0u8; FW_MAX_FRAME_LEN];

    let host_len = encode_firmware_host(seq, HostCommand::Ping, payload, &mut frame)
        .expect("clipped firmware host payload fits max frame");
    match parse_firmware_host(&frame[..host_len]) {
        ParseResult::Frame { frame, consumed } => {
            assert_eq!(consumed, host_len);
            assert_eq!(frame.seq, seq);
            assert_eq!(frame.command, HostCommand::Ping as u8);
            assert_eq!(frame.payload, payload);
        }
        other => panic!("encoded firmware host frame should parse, got {other:?}"),
    }

    let device_len = encode_firmware_device(seq, DeviceCommand::Pong, payload, &mut frame)
        .expect("clipped firmware device payload fits max frame");
    match parse_firmware_device(&frame[..device_len]) {
        ParseResult::Frame { frame, consumed } => {
            assert_eq!(consumed, device_len);
            assert_eq!(frame.seq, seq);
            assert_eq!(frame.command, DeviceCommand::Pong as u8);
            assert_eq!(frame.payload, payload);
        }
        other => panic!("encoded firmware device frame should parse, got {other:?}"),
    }

    let ack_len = encode_firmware_ack(seq, &mut frame).expect("ACK frame fits max frame");
    assert!(matches!(
        parse_firmware_device(&frame[..ack_len]),
        ParseResult::Frame { .. }
    ));

    let nak_len =
        encode_firmware_nak(seq, NakReason::PayloadInvalid, &mut frame).expect("NAK frame fits");
    assert!(matches!(
        parse_firmware_device(&frame[..nak_len]),
        ParseResult::Frame { .. }
    ));

    let mut too_small = [0u8; 1];
    assert_eq!(
        encode_firmware_device(seq, DeviceCommand::Pong, payload, &mut too_small),
        Err(FirmwareEncodeError::OutputTooSmall {
            needed: 1 + FW_LEN_FIELD_SIZE + usize::from(FW_MIN_LEN_FIELD) + payload.len()
        })
    );

    if input.len() > FW_MAX_PAYLOAD_LEN {
        assert_eq!(
            encode_firmware_host(seq, HostCommand::Ping, input, &mut frame),
            Err(FirmwareEncodeError::PayloadTooLarge)
        );
    }
}

fn check_protocol_helpers(input: &[u8]) {
    let byte = input.first().copied().unwrap_or_default();
    let _ = HostCommand::from_u8(byte);
    for command in [
        HostCommand::Ping,
        HostCommand::Identify,
        HostCommand::MouseMoveRel,
        HostCommand::MouseButton,
        HostCommand::MouseWheel,
        HostCommand::KeyDown,
        HostCommand::KeyUp,
        HostCommand::KeyMods,
        HostCommand::PadReport,
        HostCommand::ReleaseAll,
        HostCommand::WatchdogKick,
        HostCommand::GetTelemetry,
        HostCommand::ResetToBootloader,
    ] {
        assert_eq!(HostCommand::from_u8(command as u8), Some(command));
    }
    assert_eq!(HostCommand::from_u8(0x7F), None);

    let seq = input_seq(input);
    assert_eq!(next_sequence(seq), seq.wrapping_add(1));

    let mut frame = [0u8; FW_MAX_FRAME_LEN];
    let len = encode_firmware_device(seq, DeviceCommand::Pong, &[], &mut frame)
        .expect("empty device frame fits");
    assert!(matches!(
        parse_host_frame_any_command(&frame[..len]),
        ParseResult::Drop {
            reason: DropReason::BadMagic,
            consumed: 1
        }
    ));
}

fn clipped_payload(input: &[u8], max_len: usize) -> &[u8] {
    &input[..input.len().min(max_len)]
}

fn input_seq(input: &[u8]) -> u32 {
    let mut bytes = [0u8; 4];
    for (dst, src) in bytes.iter_mut().zip(input.iter().copied()) {
        *dst = src;
    }
    u32::from_le_bytes(bytes)
}
