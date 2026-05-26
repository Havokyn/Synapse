use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use embassy_rp::peripherals::USB;
use embassy_rp::usb::{Driver, InterruptHandler};
use embassy_time::Timer;
use embassy_usb::class::cdc_acm::{CdcAcmClass, State};
use embassy_usb::class::hid::{
    Config as HidConfig, HidBootProtocol, HidProtocolMode, HidSubclass, HidWriter, ReportId,
    RequestHandler, State as HidState,
};
use embassy_usb::control::OutResponse;
use embassy_usb::driver::EndpointError;
use embassy_usb::{Builder, Config, UsbDevice};
use static_cell::StaticCell;

use crate::usb;
use pico_hid::hid_descriptors;
#[cfg(feature = "loopback")]
use pico_hid::protocol::{
    DeviceCommand, MAX_FRAME_LEN, ParseResult, encode_device_frame, encode_nak,
    parse_host_frame_any_command,
};
use pico_hid::reports::{self, BootKeyboardReport, BootMouseReport, GamepadReport};
#[cfg(not(feature = "loopback"))]
use pico_hid::{
    dispatch::{DispatchState, IdentifyInfo, dispatch_frame},
    protocol::{
        MAX_FRAME_LEN, NakReason, ParseResult, encode_device_frame, encode_nak, parse_host_frame,
    },
};

const CDC_MAX_PACKET_SIZE: u16 = 64;
const HID_POLL_MS: u8 = 1;
const CONFIG_DESCRIPTOR_LEN: usize = 512;
const BOS_DESCRIPTOR_LEN: usize = 256;
const CONTROL_BUF_LEN: usize = 64;

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => InterruptHandler<USB>;
});

type UsbDriver = Driver<'static, USB>;
type PicoUsbDevice = UsbDevice<'static, UsbDriver>;
type PicoCdcAcmClass = CdcAcmClass<'static, UsbDriver>;
type MouseWriter = HidWriter<'static, UsbDriver, { reports::BOOT_MOUSE_REPORT_LEN }>;
type KeyboardWriter = HidWriter<'static, UsbDriver, { reports::BOOT_KEYBOARD_REPORT_LEN }>;
type GamepadWriter = HidWriter<'static, UsbDriver, { reports::GAMEPAD_REPORT_LEN }>;

struct HidWriters {
    mouse: MouseWriter,
    keyboard: KeyboardWriter,
    gamepad: GamepadWriter,
}

pub fn spawn_usb(usb_peripheral: embassy_rp::Peri<'static, USB>, spawner: &Spawner) -> bool {
    let (usb_device, serial_class, hid_writers) = build_usb_serial(usb_peripheral);

    let device_spawned = match usb_task(usb_device) {
        Ok(token) => {
            spawner.spawn(token);
            true
        }
        Err(_) => false,
    };
    let serial_spawned = match cdc_serial_task(serial_class) {
        Ok(token) => {
            spawner.spawn(token);
            true
        }
        Err(_) => false,
    };
    let mouse_spawned = match mouse_hid_task(hid_writers.mouse) {
        Ok(token) => {
            spawner.spawn(token);
            true
        }
        Err(_) => false,
    };
    let keyboard_spawned = match keyboard_hid_task(hid_writers.keyboard) {
        Ok(token) => {
            spawner.spawn(token);
            true
        }
        Err(_) => false,
    };
    let gamepad_spawned = match gamepad_hid_task(hid_writers.gamepad) {
        Ok(token) => {
            spawner.spawn(token);
            true
        }
        Err(_) => false,
    };

    device_spawned && serial_spawned && mouse_spawned && keyboard_spawned && gamepad_spawned
}

fn build_usb_serial(
    usb_peripheral: embassy_rp::Peri<'static, USB>,
) -> (PicoUsbDevice, PicoCdcAcmClass, HidWriters) {
    let driver = Driver::new(usb_peripheral, Irqs);
    let identity = usb::identity();

    let mut config = Config::new(identity.vid, identity.pid);
    config.manufacturer = Some(identity.manufacturer);
    config.product = Some(identity.product);
    config.serial_number = Some(identity.serial_prefix);
    config.max_power = 100;
    config.max_packet_size_0 = 64;

    static CONFIG_DESCRIPTOR: StaticCell<[u8; CONFIG_DESCRIPTOR_LEN]> = StaticCell::new();
    static BOS_DESCRIPTOR: StaticCell<[u8; BOS_DESCRIPTOR_LEN]> = StaticCell::new();
    static CONTROL_BUF: StaticCell<[u8; CONTROL_BUF_LEN]> = StaticCell::new();
    static CDC_STATE: StaticCell<State<'static>> = StaticCell::new();
    static MOUSE_STATE: StaticCell<HidState<'static>> = StaticCell::new();
    static KEYBOARD_STATE: StaticCell<HidState<'static>> = StaticCell::new();
    static GAMEPAD_STATE: StaticCell<HidState<'static>> = StaticCell::new();
    static MOUSE_REQUESTS: StaticCell<HidRequestState> = StaticCell::new();
    static KEYBOARD_REQUESTS: StaticCell<HidRequestState> = StaticCell::new();
    static GAMEPAD_REQUESTS: StaticCell<HidRequestState> = StaticCell::new();

    let mut builder = Builder::new(
        driver,
        config,
        CONFIG_DESCRIPTOR.init([0; CONFIG_DESCRIPTOR_LEN]),
        BOS_DESCRIPTOR.init([0; BOS_DESCRIPTOR_LEN]),
        &mut [],
        CONTROL_BUF.init([0; CONTROL_BUF_LEN]),
    );

    let mouse = HidWriter::new(
        &mut builder,
        MOUSE_STATE.init(HidState::new()),
        hid_config(
            hid_descriptors::BOOT_MOUSE,
            HidSubclass::Boot,
            HidBootProtocol::Mouse,
            MOUSE_REQUESTS.init(HidRequestState::new(true)),
            reports::BOOT_MOUSE_REPORT_LEN as u16,
        ),
    );
    let keyboard = HidWriter::new(
        &mut builder,
        KEYBOARD_STATE.init(HidState::new()),
        hid_config(
            hid_descriptors::BOOT_KEYBOARD,
            HidSubclass::Boot,
            HidBootProtocol::Keyboard,
            KEYBOARD_REQUESTS.init(HidRequestState::new(true)),
            reports::BOOT_KEYBOARD_REPORT_LEN as u16,
        ),
    );
    let gamepad = HidWriter::new(
        &mut builder,
        GAMEPAD_STATE.init(HidState::new()),
        hid_config(
            hid_descriptors::GAMEPAD,
            HidSubclass::No,
            HidBootProtocol::None,
            GAMEPAD_REQUESTS.init(HidRequestState::new(false)),
            reports::GAMEPAD_REPORT_LEN as u16,
        ),
    );

    let state = CDC_STATE.init(State::new());
    let serial_class = CdcAcmClass::new(&mut builder, state, CDC_MAX_PACKET_SIZE);
    let usb_device = builder.build();

    (
        usb_device,
        serial_class,
        HidWriters {
            mouse,
            keyboard,
            gamepad,
        },
    )
}

fn hid_config(
    report_descriptor: &'static [u8],
    hid_subclass: HidSubclass,
    hid_boot_protocol: HidBootProtocol,
    request_handler: &'static mut HidRequestState,
    max_packet_size: u16,
) -> HidConfig<'static> {
    HidConfig {
        report_descriptor,
        request_handler: Some(request_handler),
        poll_ms: HID_POLL_MS,
        max_packet_size,
        hid_subclass,
        hid_boot_protocol,
    }
}

#[embassy_executor::task]
async fn usb_task(mut usb: PicoUsbDevice) -> ! {
    usb.run().await
}

#[embassy_executor::task]
async fn cdc_serial_task(mut serial_class: PicoCdcAcmClass) -> ! {
    loop {
        serial_class.wait_connection().await;
        let _disconnected = serial_until_disconnect(&mut serial_class).await;
    }
}

#[embassy_executor::task]
async fn mouse_hid_task(mut writer: MouseWriter) -> ! {
    let neutral = BootMouseReport::neutral().to_bytes();

    loop {
        writer.ready().await;
        let _ = writer.write(&neutral).await;
        Timer::after_secs(1).await;
    }
}

#[embassy_executor::task]
async fn keyboard_hid_task(mut writer: KeyboardWriter) -> ! {
    let neutral = BootKeyboardReport::neutral().to_bytes();

    loop {
        writer.ready().await;
        let _ = writer.write(&neutral).await;
        Timer::after_secs(1).await;
    }
}

#[embassy_executor::task]
async fn gamepad_hid_task(mut writer: GamepadWriter) -> ! {
    let neutral = GamepadReport::neutral().to_bytes();

    loop {
        writer.ready().await;
        let _ = writer.write(&neutral).await;
        Timer::after_secs(1).await;
    }
}

#[cfg(not(feature = "loopback"))]
async fn serial_until_disconnect(serial_class: &mut PicoCdcAcmClass) -> Result<(), Disconnected> {
    let mut packet = [0u8; CDC_MAX_PACKET_SIZE as usize];
    let mut rx = [0u8; MAX_FRAME_LEN];
    let mut rx_len = 0usize;
    let mut tx = [0u8; MAX_FRAME_LEN];
    let usb_identity = usb::identity();
    let identify = IdentifyInfo::new(*b"DEVBUILD", usb_identity.vid, usb_identity.pid);
    let mut state = DispatchState::new();

    loop {
        let count = serial_class.read_packet(&mut packet).await?;
        if rx_len + count > rx.len() {
            state.telemetry.record_frame_dropped();
            rx_len = 0;
            continue;
        }

        rx[rx_len..rx_len + count].copy_from_slice(&packet[..count]);
        rx_len += count;

        loop {
            let consumed = match parse_host_frame(&rx[..rx_len]) {
                ParseResult::Frame { frame, consumed } => {
                    let outcome = dispatch_frame(&mut state, frame, identify);
                    let tx_len = encode_device_frame(
                        frame.seq,
                        outcome.command,
                        &outcome.payload[..outcome.payload_len],
                        &mut tx,
                    )
                    .expect("dispatch responses always fit in a device frame");
                    write_serial_bytes(serial_class, &tx[..tx_len]).await?;
                    consumed
                }
                ParseResult::Nak { nak, consumed } => {
                    if matches!(nak.reason, NakReason::CrcInvalid) {
                        state.telemetry.record_crc_error();
                    } else {
                        state.telemetry.record_link_error();
                    }
                    let tx_len = encode_nak(nak.seq, nak.reason, &mut tx)
                        .expect("NAK payload always fits in a device frame");
                    write_serial_bytes(serial_class, &tx[..tx_len]).await?;
                    consumed
                }
                ParseResult::Drop { consumed, .. } => {
                    state.telemetry.record_frame_dropped();
                    consumed
                }
                ParseResult::NeedMore { .. } => break,
            };

            consume_rx(&mut rx, &mut rx_len, consumed);
        }
    }
}

#[cfg(feature = "loopback")]
async fn serial_until_disconnect(serial_class: &mut PicoCdcAcmClass) -> Result<(), Disconnected> {
    let mut packet = [0u8; CDC_MAX_PACKET_SIZE as usize];
    let mut rx = [0u8; MAX_FRAME_LEN];
    let mut rx_len = 0usize;
    let mut tx = [0u8; MAX_FRAME_LEN];

    loop {
        let count = serial_class.read_packet(&mut packet).await?;
        if rx_len + count > rx.len() {
            rx_len = 0;
            continue;
        }

        rx[rx_len..rx_len + count].copy_from_slice(&packet[..count]);
        rx_len += count;

        loop {
            let consumed = match parse_host_frame_any_command(&rx[..rx_len]) {
                ParseResult::Frame { frame, consumed } => {
                    let tx_len =
                        encode_device_frame(frame.seq, DeviceCommand::Pong, frame.payload, &mut tx)
                            .expect("PONG payload came from a parsed max-size host frame");
                    write_serial_bytes(serial_class, &tx[..tx_len]).await?;
                    consumed
                }
                ParseResult::Nak { nak, consumed } => {
                    let tx_len = encode_nak(nak.seq, nak.reason, &mut tx)
                        .expect("NAK payload always fits in a device frame");
                    write_serial_bytes(serial_class, &tx[..tx_len]).await?;
                    consumed
                }
                ParseResult::Drop { consumed, .. } => consumed,
                ParseResult::NeedMore { .. } => break,
            };

            consume_rx(&mut rx, &mut rx_len, consumed);
        }
    }
}

async fn write_serial_bytes(
    serial_class: &mut PicoCdcAcmClass,
    data: &[u8],
) -> Result<(), Disconnected> {
    let max_packet_size = serial_class.max_packet_size() as usize;
    let mut offset = 0usize;

    while offset < data.len() {
        let end = core::cmp::min(offset + max_packet_size, data.len());
        serial_class.write_packet(&data[offset..end]).await?;
        offset = end;
    }

    if data.len().is_multiple_of(max_packet_size) {
        serial_class.write_packet(&[]).await?;
    }

    Ok(())
}

fn consume_rx(rx: &mut [u8; MAX_FRAME_LEN], rx_len: &mut usize, consumed: usize) {
    if consumed >= *rx_len {
        *rx_len = 0;
        return;
    }

    rx.copy_within(consumed..*rx_len, 0);
    *rx_len -= consumed;
}

struct HidRequestState {
    supports_boot_protocol: bool,
    protocol: HidProtocolMode,
    idle_ms: u32,
}

impl HidRequestState {
    const fn new(supports_boot_protocol: bool) -> Self {
        Self {
            supports_boot_protocol,
            protocol: HidProtocolMode::Report,
            idle_ms: u32::MAX,
        }
    }
}

impl RequestHandler for HidRequestState {
    fn get_protocol(&self) -> HidProtocolMode {
        self.protocol
    }

    fn set_protocol(&mut self, protocol: HidProtocolMode) -> OutResponse {
        if matches!(protocol, HidProtocolMode::Boot) && !self.supports_boot_protocol {
            return OutResponse::Rejected;
        }

        self.protocol = protocol;
        OutResponse::Accepted
    }

    fn get_idle_ms(&mut self, id: Option<ReportId>) -> Option<u32> {
        if id.is_some() {
            None
        } else {
            Some(self.idle_ms)
        }
    }

    fn set_idle_ms(&mut self, id: Option<ReportId>, duration_ms: u32) {
        if id.is_none() {
            self.idle_ms = duration_ms;
        }
    }
}

struct Disconnected;

impl From<EndpointError> for Disconnected {
    fn from(value: EndpointError) -> Self {
        match value {
            EndpointError::BufferOverflow | EndpointError::Disabled => Self,
        }
    }
}
