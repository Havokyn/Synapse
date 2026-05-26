use synapse_hid_host::{
    DEVICE_COMMAND_PONG, HOST_COMMAND_PING, HOST_MAGIC, MAX_FRAME_LEN, MAX_PAYLOAD_LEN, ParseError,
    crc16_ccitt_false, encode_device_frame, encode_host_frame, parse_device_frame,
};

#[test]
fn crc16_ccitt_false_known_vectors_match_reference() {
    let vectors = [
        (&b""[..], 0xFFFF),
        (&b"123456789"[..], 0x29B1),
        (&b"ABC"[..], reference_crc16_ccitt_false(b"ABC")),
        (
            &[0x00, 0xFF, 0x10, 0x20, 0x5A, 0xA5][..],
            reference_crc16_ccitt_false(&[0x00, 0xFF, 0x10, 0x20, 0x5A, 0xA5]),
        ),
    ];

    for (payload, expected) in vectors {
        assert_eq!(crc16_ccitt_false(payload), expected);
        assert_eq!(
            crc16_ccitt_false(payload),
            reference_crc16_ccitt_false(payload)
        );
    }
}

#[test]
fn crc16_ccitt_false_matches_reference_for_1000_payloads() {
    let mut rng = DeterministicRng::new(0x5359_4E41_5053_4521);

    for case_index in 0..1000 {
        let len = match case_index {
            0 => 0,
            1 => MAX_PAYLOAD_LEN,
            _ => usize::from(rng.next_u16() % 1025),
        };
        let mut payload = vec![0u8; len];
        for byte in &mut payload {
            *byte = rng.next_u8();
        }

        assert_eq!(
            crc16_ccitt_false(&payload),
            reference_crc16_ccitt_false(&payload)
        );
    }
}

#[test]
fn encoded_host_frame_crc_matches_reference() {
    let payload = [0x10, 0x20, 0x30, 0x40, 0x50];
    let mut frame = [0u8; MAX_FRAME_LEN];
    let len = match encode_host_frame(42, HOST_COMMAND_PING, &payload, &mut frame) {
        Ok(len) => len,
        Err(error) => panic!("host frame should encode: {error:?}"),
    };

    assert_eq!(frame[0], HOST_MAGIC);
    let crc_start = len - 2;
    let stored_crc = u16::from_le_bytes([frame[crc_start], frame[crc_start + 1]]);
    assert_eq!(
        stored_crc,
        reference_crc16_ccitt_false(&frame[1..crc_start])
    );
}

#[test]
fn device_frame_crc_rejects_one_bit_corruption() {
    let payload = [0xAA, 0xBB, 0xCC];
    let mut frame = [0u8; MAX_FRAME_LEN];
    let len = match encode_device_frame(7, DEVICE_COMMAND_PONG, &payload, &mut frame) {
        Ok(len) => len,
        Err(error) => panic!("device frame should encode: {error:?}"),
    };

    frame[8] ^= 0x01;

    match parse_device_frame(&frame[..len]) {
        Err(ParseError::CrcInvalid { .. }) => {}
        other => panic!("corrupted frame should fail CRC validation, got {other:?}"),
    }
}

fn reference_crc16_ccitt_false(bytes: &[u8]) -> u16 {
    let mut crc = 0xFFFFu16;
    for byte in bytes {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            if crc & 0x8000 != 0 {
                crc = (crc << 1) ^ 0x1021;
            } else {
                crc <<= 1;
            }
        }
    }
    crc
}

struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
    const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    const fn next_u8(&mut self) -> u8 {
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.state.to_le_bytes()[4]
    }

    const fn next_u16(&mut self) -> u16 {
        let lo = self.next_u8();
        let hi = self.next_u8();
        u16::from_le_bytes([lo, hi])
    }
}
