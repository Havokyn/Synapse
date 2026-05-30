use crate::protocol::{DeviceCommand, Frame};
#[cfg(not(feature = "loopback"))]
use crate::protocol::{HostCommand, NakReason};
#[cfg(not(feature = "loopback"))]
use crate::reports::GAMEPAD_REPORT_LEN;
use crate::reports::{BootKeyboardReport, BootMouseReport, GamepadReport};
use crate::safety::DEFAULT_WATCHDOG_TIMEOUT_MS;

#[path = "../../../crates/synapse-core/src/firmware_version.rs"]
mod firmware_version;

#[cfg(feature = "loopback")]
pub const MAX_RESPONSE_PAYLOAD_LEN: usize = crate::protocol::MAX_PAYLOAD_LEN;
#[cfg(not(feature = "loopback"))]
pub const MAX_RESPONSE_PAYLOAD_LEN: usize = 32;
pub use firmware_version::{
    SYNAPSE_PICO_HID_BUILD_HASH_LEN as BUILD_HASH_LEN,
    SYNAPSE_PICO_HID_FW_MAJOR as FW_VERSION_MAJOR, SYNAPSE_PICO_HID_FW_MINOR as FW_VERSION_MINOR,
    SYNAPSE_PICO_HID_FW_PATCH as FW_VERSION_PATCH,
};
#[cfg(feature = "fake-fw-major-mismatch")]
pub const IDENTIFY_FW_MAJOR: u8 = FW_VERSION_MAJOR.wrapping_add(1);
#[cfg(not(feature = "fake-fw-major-mismatch"))]
pub const IDENTIFY_FW_MAJOR: u8 = FW_VERSION_MAJOR;
pub const CAPABILITY_MOUSE: u32 = 1 << 0;
pub const CAPABILITY_KEYBOARD: u32 = 1 << 1;
pub const CAPABILITY_GAMEPAD: u32 = 1 << 2;
pub const CAPABILITY_TELEMETRY: u32 = 1 << 3;
pub const CAPABILITY_WATCHDOG: u32 = 1 << 4;
pub const CAPABILITY_BOOTLOADER: u32 = 1 << 5;
pub const DEFAULT_CAPABILITIES: u32 = CAPABILITY_MOUSE
    | CAPABILITY_KEYBOARD
    | CAPABILITY_GAMEPAD
    | CAPABILITY_TELEMETRY
    | CAPABILITY_WATCHDOG
    | CAPABILITY_BOOTLOADER;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IdentifyInfo {
    pub fw_major: u8,
    pub fw_minor: u8,
    pub fw_patch: u8,
    pub build_hash: [u8; BUILD_HASH_LEN],
    pub vid: u16,
    pub pid: u16,
    pub capabilities: u32,
}

impl IdentifyInfo {
    pub const fn new(build_hash: [u8; BUILD_HASH_LEN], vid: u16, pid: u16) -> Self {
        Self {
            fw_major: IDENTIFY_FW_MAJOR,
            fw_minor: FW_VERSION_MINOR,
            fw_patch: FW_VERSION_PATCH,
            build_hash,
            vid,
            pid,
            capabilities: DEFAULT_CAPABILITIES,
        }
    }

    #[cfg(not(feature = "loopback"))]
    fn write_payload(self, out: &mut [u8; MAX_RESPONSE_PAYLOAD_LEN]) -> usize {
        out[0] = self.fw_major;
        out[1] = self.fw_minor;
        out[2] = self.fw_patch;
        out[3] = 0;
        out[4..12].copy_from_slice(&self.build_hash);
        out[12..14].copy_from_slice(&self.vid.to_le_bytes());
        out[14..16].copy_from_slice(&self.pid.to_le_bytes());
        out[16..20].copy_from_slice(&self.capabilities.to_le_bytes());
        20
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DispatchState {
    pub mouse: BootMouseReport,
    pub keyboard: BootKeyboardReport,
    pub gamepad: GamepadReport,
    pub telemetry: Telemetry,
    pub watchdog_timeout_ms: u32,
    #[cfg(feature = "force-first-nak")]
    pub force_first_nak_sent: bool,
}

impl DispatchState {
    pub const fn new() -> Self {
        Self {
            mouse: BootMouseReport::neutral(),
            keyboard: BootKeyboardReport::neutral(),
            gamepad: GamepadReport::neutral(),
            telemetry: Telemetry::new(),
            watchdog_timeout_ms: DEFAULT_WATCHDOG_TIMEOUT_MS,
            #[cfg(feature = "force-first-nak")]
            force_first_nak_sent: false,
        }
    }

    pub(crate) fn release_all(&mut self) {
        self.mouse = BootMouseReport::neutral();
        self.keyboard = BootKeyboardReport::neutral();
        self.gamepad = GamepadReport::neutral();
    }
}

impl Default for DispatchState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Telemetry {
    pub uptime_ms: u32,
    pub frames_received: u32,
    pub frames_dropped: u32,
    pub link_errors: u32,
    pub commands_executed: u32,
    pub watchdog_fires: u32,
    pub crc_errors: u32,
}

impl Telemetry {
    pub const fn new() -> Self {
        Self {
            uptime_ms: 0,
            frames_received: 0,
            frames_dropped: 0,
            link_errors: 0,
            commands_executed: 0,
            watchdog_fires: 0,
            crc_errors: 0,
        }
    }

    #[cfg(not(feature = "loopback"))]
    fn write_payload(self, out: &mut [u8; MAX_RESPONSE_PAYLOAD_LEN]) -> usize {
        out[0..4].copy_from_slice(&self.uptime_ms.to_le_bytes());
        out[4..8].copy_from_slice(&self.frames_received.to_le_bytes());
        out[8..12].copy_from_slice(&self.frames_dropped.to_le_bytes());
        out[12..16].copy_from_slice(&self.link_errors.to_le_bytes());
        out[16..20].copy_from_slice(&self.commands_executed.to_le_bytes());
        out[20..24].copy_from_slice(&self.watchdog_fires.to_le_bytes());
        out[24..28].copy_from_slice(&self.crc_errors.to_le_bytes());
        28
    }

    pub fn record_frame_received(&mut self) {
        self.frames_received = self.frames_received.wrapping_add(1);
    }

    pub fn record_frame_dropped(&mut self) {
        self.frames_dropped = self.frames_dropped.wrapping_add(1);
    }

    pub fn record_link_error(&mut self) {
        self.link_errors = self.link_errors.wrapping_add(1);
    }

    pub fn record_crc_error(&mut self) {
        self.crc_errors = self.crc_errors.wrapping_add(1);
        self.record_link_error();
    }

    pub fn record_command_executed(&mut self) {
        self.commands_executed = self.commands_executed.wrapping_add(1);
    }

    pub fn record_watchdog_fire(&mut self) {
        self.watchdog_fires = self.watchdog_fires.wrapping_add(1);
    }
}

impl Default for Telemetry {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DispatchOutcome {
    pub command: DeviceCommand,
    pub payload: [u8; MAX_RESPONSE_PAYLOAD_LEN],
    pub payload_len: usize,
}

impl DispatchOutcome {
    #[cfg(not(feature = "loopback"))]
    fn ack(seq: u32) -> Self {
        let mut payload = [0u8; MAX_RESPONSE_PAYLOAD_LEN];
        payload[..4].copy_from_slice(&seq.to_le_bytes());
        Self {
            command: DeviceCommand::Ack,
            payload,
            payload_len: 4,
        }
    }

    #[cfg(not(feature = "loopback"))]
    fn nak(seq: u32, reason: NakReason) -> Self {
        let mut payload = [0u8; MAX_RESPONSE_PAYLOAD_LEN];
        payload[..4].copy_from_slice(&seq.to_le_bytes());
        payload[4] = reason as u8;
        Self {
            command: DeviceCommand::Nak,
            payload,
            payload_len: 5,
        }
    }

    fn response(command: DeviceCommand, data: &[u8]) -> Self {
        let mut payload = [0u8; MAX_RESPONSE_PAYLOAD_LEN];
        payload[..data.len()].copy_from_slice(data);
        Self {
            command,
            payload,
            payload_len: data.len(),
        }
    }
}

#[cfg(feature = "loopback")]
pub fn dispatch_frame(
    state: &mut DispatchState,
    frame: Frame<'_>,
    _identify: IdentifyInfo,
) -> DispatchOutcome {
    state.telemetry.record_frame_received();
    state.telemetry.record_command_executed();
    DispatchOutcome::response(DeviceCommand::Pong, frame.payload)
}

#[cfg(not(feature = "loopback"))]
pub fn dispatch_frame(
    state: &mut DispatchState,
    frame: Frame<'_>,
    identify: IdentifyInfo,
) -> DispatchOutcome {
    let command = match HostCommand::from_u8(frame.command) {
        Some(command) => command,
        None => return DispatchOutcome::nak(frame.seq, NakReason::UnknownCommand),
    };

    state.telemetry.record_frame_received();

    let mut outcome = match command {
        HostCommand::Ping => dispatch_ping(frame),
        HostCommand::Identify => dispatch_identify(frame, identify),
        HostCommand::MouseMoveRel => dispatch_mouse_move_rel(state, frame),
        HostCommand::MouseButton => dispatch_mouse_button(state, frame),
        HostCommand::MouseWheel => dispatch_mouse_wheel(state, frame),
        HostCommand::KeyDown => dispatch_key_down(state, frame),
        HostCommand::KeyUp => dispatch_key_up(state, frame),
        HostCommand::KeyMods => dispatch_key_mods(state, frame),
        HostCommand::PadReport => dispatch_pad_report(state, frame),
        HostCommand::ReleaseAll => dispatch_release_all(state, frame),
        HostCommand::WatchdogKick => dispatch_watchdog_kick(state, frame),
        HostCommand::GetTelemetry => dispatch_get_telemetry(state, frame),
        HostCommand::ResetToBootloader => dispatch_empty_ack(frame),
    };
    if should_force_first_nak(state, command, outcome.command) {
        outcome = DispatchOutcome::nak(frame.seq, NakReason::BufferFull);
    }

    if outcome.command == DeviceCommand::Nak {
        state.telemetry.record_link_error();
    } else {
        state.telemetry.record_command_executed();
    }

    outcome
}

#[cfg(all(not(feature = "loopback"), feature = "force-first-nak"))]
fn should_force_first_nak(
    state: &mut DispatchState,
    command: HostCommand,
    outcome_command: DeviceCommand,
) -> bool {
    if state.force_first_nak_sent {
        return false;
    }
    if outcome_command != DeviceCommand::Ack {
        return false;
    }

    if matches!(
        command,
        HostCommand::MouseMoveRel
            | HostCommand::MouseButton
            | HostCommand::MouseWheel
            | HostCommand::KeyDown
            | HostCommand::KeyUp
            | HostCommand::KeyMods
            | HostCommand::PadReport
            | HostCommand::ReleaseAll
            | HostCommand::WatchdogKick
    ) {
        state.force_first_nak_sent = true;
        return true;
    }

    false
}

#[cfg(all(not(feature = "loopback"), not(feature = "force-first-nak")))]
const fn should_force_first_nak(
    _state: &mut DispatchState,
    _command: HostCommand,
    _outcome_command: DeviceCommand,
) -> bool {
    false
}

#[cfg(not(feature = "loopback"))]
fn dispatch_ping(frame: Frame<'_>) -> DispatchOutcome {
    if frame.payload.len() != 4 {
        return DispatchOutcome::nak(frame.seq, NakReason::PayloadInvalid);
    }

    DispatchOutcome::response(DeviceCommand::Pong, frame.payload)
}

#[cfg(not(feature = "loopback"))]
fn dispatch_identify(frame: Frame<'_>, identify: IdentifyInfo) -> DispatchOutcome {
    if !frame.payload.is_empty() {
        return DispatchOutcome::nak(frame.seq, NakReason::PayloadInvalid);
    }

    let mut payload = [0u8; MAX_RESPONSE_PAYLOAD_LEN];
    let payload_len = identify.write_payload(&mut payload);
    DispatchOutcome {
        command: DeviceCommand::IdentifyResp,
        payload,
        payload_len,
    }
}

#[cfg(not(feature = "loopback"))]
fn dispatch_mouse_move_rel(state: &mut DispatchState, frame: Frame<'_>) -> DispatchOutcome {
    if frame.payload.len() != 4 {
        return DispatchOutcome::nak(frame.seq, NakReason::PayloadInvalid);
    }

    let dx = i16::from_le_bytes([frame.payload[0], frame.payload[1]]);
    let dy = i16::from_le_bytes([frame.payload[2], frame.payload[3]]);
    if !(-127..=127).contains(&dx) || !(-127..=127).contains(&dy) {
        return DispatchOutcome::nak(frame.seq, NakReason::PayloadInvalid);
    }

    state.mouse.x = dx as i8;
    state.mouse.y = dy as i8;
    DispatchOutcome::ack(frame.seq)
}

#[cfg(not(feature = "loopback"))]
fn dispatch_mouse_button(state: &mut DispatchState, frame: Frame<'_>) -> DispatchOutcome {
    if frame.payload.len() != 2 {
        return DispatchOutcome::nak(frame.seq, NakReason::PayloadInvalid);
    }

    let button = frame.payload[0];
    let down = frame.payload[1];
    if !(1..=3).contains(&button) || down > 1 {
        return DispatchOutcome::nak(frame.seq, NakReason::PayloadInvalid);
    }

    let mask = 1 << (button - 1);
    if down == 1 {
        state.mouse.buttons |= mask;
    } else {
        state.mouse.buttons &= !mask;
    }

    DispatchOutcome::ack(frame.seq)
}

#[cfg(not(feature = "loopback"))]
fn dispatch_mouse_wheel(state: &mut DispatchState, frame: Frame<'_>) -> DispatchOutcome {
    if frame.payload.len() != 2 {
        return DispatchOutcome::nak(frame.seq, NakReason::PayloadInvalid);
    }

    let dy = frame.payload[0] as i8;
    let dx = frame.payload[1] as i8;
    if dx != 0 {
        return DispatchOutcome::nak(frame.seq, NakReason::PayloadInvalid);
    }

    state.mouse.wheel = dy;
    DispatchOutcome::ack(frame.seq)
}

#[cfg(not(feature = "loopback"))]
fn dispatch_key_down(state: &mut DispatchState, frame: Frame<'_>) -> DispatchOutcome {
    if frame.payload.len() != 1 || frame.payload[0] == 0 {
        return DispatchOutcome::nak(frame.seq, NakReason::PayloadInvalid);
    }

    let key = frame.payload[0];
    if state.keyboard.keycodes.contains(&key) {
        return DispatchOutcome::ack(frame.seq);
    }

    for slot in &mut state.keyboard.keycodes {
        if *slot == 0 {
            *slot = key;
            return DispatchOutcome::ack(frame.seq);
        }
    }

    DispatchOutcome::nak(frame.seq, NakReason::BufferFull)
}

#[cfg(not(feature = "loopback"))]
fn dispatch_key_up(state: &mut DispatchState, frame: Frame<'_>) -> DispatchOutcome {
    if frame.payload.len() != 1 || frame.payload[0] == 0 {
        return DispatchOutcome::nak(frame.seq, NakReason::PayloadInvalid);
    }

    let key = frame.payload[0];
    for slot in &mut state.keyboard.keycodes {
        if *slot == key {
            *slot = 0;
        }
    }

    DispatchOutcome::ack(frame.seq)
}

#[cfg(not(feature = "loopback"))]
fn dispatch_key_mods(state: &mut DispatchState, frame: Frame<'_>) -> DispatchOutcome {
    if frame.payload.len() != 1 {
        return DispatchOutcome::nak(frame.seq, NakReason::PayloadInvalid);
    }

    state.keyboard.modifiers = frame.payload[0];
    DispatchOutcome::ack(frame.seq)
}

#[cfg(not(feature = "loopback"))]
fn dispatch_pad_report(state: &mut DispatchState, frame: Frame<'_>) -> DispatchOutcome {
    if frame.payload.len() != GAMEPAD_REPORT_LEN {
        return DispatchOutcome::nak(frame.seq, NakReason::PayloadInvalid);
    }

    let mut bytes = [0u8; GAMEPAD_REPORT_LEN];
    bytes.copy_from_slice(frame.payload);
    let report = GamepadReport::from_bytes(bytes);
    if report.reserved != 0 {
        return DispatchOutcome::nak(frame.seq, NakReason::PayloadInvalid);
    }

    state.gamepad = report;
    DispatchOutcome::ack(frame.seq)
}

#[cfg(not(feature = "loopback"))]
fn dispatch_release_all(state: &mut DispatchState, frame: Frame<'_>) -> DispatchOutcome {
    if !frame.payload.is_empty() {
        return DispatchOutcome::nak(frame.seq, NakReason::PayloadInvalid);
    }

    state.release_all();
    DispatchOutcome::ack(frame.seq)
}

#[cfg(not(feature = "loopback"))]
fn dispatch_watchdog_kick(state: &mut DispatchState, frame: Frame<'_>) -> DispatchOutcome {
    if frame.payload.len() != 4 {
        return DispatchOutcome::nak(frame.seq, NakReason::PayloadInvalid);
    }

    state.watchdog_timeout_ms = u32::from_le_bytes([
        frame.payload[0],
        frame.payload[1],
        frame.payload[2],
        frame.payload[3],
    ]);
    DispatchOutcome::ack(frame.seq)
}

#[cfg(not(feature = "loopback"))]
fn dispatch_get_telemetry(state: &DispatchState, frame: Frame<'_>) -> DispatchOutcome {
    if !frame.payload.is_empty() {
        return DispatchOutcome::nak(frame.seq, NakReason::PayloadInvalid);
    }

    let mut payload = [0u8; MAX_RESPONSE_PAYLOAD_LEN];
    let payload_len = state.telemetry.write_payload(&mut payload);
    DispatchOutcome {
        command: DeviceCommand::TelemetryResp,
        payload,
        payload_len,
    }
}

#[cfg(not(feature = "loopback"))]
fn dispatch_empty_ack(frame: Frame<'_>) -> DispatchOutcome {
    if !frame.payload.is_empty() {
        return DispatchOutcome::nak(frame.seq, NakReason::PayloadInvalid);
    }

    DispatchOutcome::ack(frame.seq)
}
