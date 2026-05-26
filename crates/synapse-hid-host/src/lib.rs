#![allow(unsafe_code)]

pub mod discover;
pub mod error;
pub mod handshake;
pub mod pipeline;
pub mod protocol;
pub mod reconnect;
pub mod transport;

pub use discover::{candidate_port_names, connect_auto, is_synapse_pico_port};
pub use error::{HidError, HidResult};
pub use handshake::{
    FirmwareIdentity, HandshakeError, IDENTIFY_RESP_LEN, IDENTIFY_TIMEOUT_MS,
    expected_version_triplet, parse_and_validate_identify_response, parse_identify_response,
    perform_identify_handshake, validate_expected_major,
};
pub use pipeline::{
    ACK_RETRY_BACKOFF_MS, ACK_TIMEOUT_MS, FIRST_PIPELINE_SEQUENCE, HidPipeline, HostCommandRequest,
    MAX_ACK_RETRIES, MAX_OUTSTANDING_FRAMES, NAK_REASON_BUFFER_FULL, NAK_REASON_CRC_INVALID,
    NAK_REASON_LEN_INVALID, NAK_REASON_PAYLOAD_INVALID, NAK_REASON_UNKNOWN_COMMAND,
    NAK_REASON_WATCHDOG_EXPIRED, PipelineConfig, PipelineResponse,
};
pub use protocol::{
    DEVICE_COMMAND_ACK, DEVICE_COMMAND_IDENTIFY_RESP, DEVICE_COMMAND_NAK, DEVICE_COMMAND_PONG,
    DEVICE_COMMAND_TELEMETRY_RESP, HOST_COMMAND_GET_TELEMETRY, HOST_COMMAND_IDENTIFY,
    HOST_COMMAND_KEY_DOWN, HOST_COMMAND_KEY_MODS, HOST_COMMAND_KEY_UP, HOST_COMMAND_MOUSE_BUTTON,
    HOST_COMMAND_MOUSE_MOVE_REL, HOST_COMMAND_MOUSE_WHEEL, HOST_COMMAND_PAD_REPORT,
    HOST_COMMAND_PING, HOST_COMMAND_RELEASE_ALL, HOST_COMMAND_RESET_TO_BOOTLOADER,
    HOST_COMMAND_WATCHDOG_KICK, HOST_MAGIC, MAX_FRAME_LEN, MAX_PAYLOAD_LEN, ParseError,
    crc16_ccitt_false, encode_device_frame, encode_host_frame, encode_identify_frame,
    parse_device_frame, parse_device_frame_prefix,
};
pub use reconnect::{
    HidReconnectGateway, HidReconnectTarget, RECONNECT_INTERVAL_MS, ReconnectConnector,
    ReconnectGateway, ReconnectLink, ReconnectSnapshot, ReconnectStateKind,
};
pub use transport::{DEFAULT_BAUD_RATE, DEFAULT_READ_TIMEOUT_MS, HidGateway};
