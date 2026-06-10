# Synapse Pico HID Firmware

`pico-hid/` is a standalone Cargo workspace for the Raspberry Pi Pico
(RP2040). It is intentionally excluded from the root Synapse workspace because
it targets `thumbv6m-none-eabi` and uses embedded `no_std` dependencies.

## One-Time Toolchain Setup

```powershell
rustup target add thumbv6m-none-eabi
cargo install elf2uf2-rs
```

## Build

```powershell
cd firmware\pico-hid
cargo build --release
```

The project-local `.cargo/config.toml` sets the build target to
`thumbv6m-none-eabi`, so `--target` is not required.

## Build the Release UF2

```powershell
.\scripts\release\firmware\build_pico_hid.ps1
```

Run the release script from the repository root. It builds the firmware,
converts the release ELF with `elf2uf2-rs`, and writes the versioned artifact to
`scripts\release\firmware\pico-hid-<version>.uf2`. The script prints the UF2
path, byte length, and SHA-256 hash for manual source-of-truth readback.

Loopback/debug firmware uses the same helper with a distinct artifact name:

```powershell
.\scripts\release\firmware\build_pico_hid.ps1 -Features loopback
```

That writes `scripts\release\firmware\pico-hid-loopback-<version>.uf2` so a
physical loopback-PONG board can be flashed without overwriting the default HID
firmware release artifact.

Handshake mismatch acceptance firmware also uses the same helper:

```powershell
.\scripts\release\firmware\build_pico_hid.ps1 -Features fake-fw-major-mismatch
```

That writes
`scripts\release\firmware\pico-hid-fake-fw-major-mismatch-<version>.uf2`.
The image changes only the `IDENTIFY_RESP.fw_major` byte so the host can prove
the `HID_FIRMWARE_VERSION_MISMATCH` path against a real Pico.

ACK/NAK retry acceptance firmware forces one retry through the real CDC
dispatcher:

```powershell
.\scripts\release\firmware\build_pico_hid.ps1 -Features force-first-nak
```

That writes `scripts\release\firmware\pico-hid-force-first-nak-<version>.uf2`.
The image leaves `IDENTIFY` untouched, returns one `NAK_BUFFER_FULL` for the
first normal ACK-style command, then accepts the retry and all later commands.

For a local conversion while already inside `firmware\pico-hid`, the lower
level command remains:

```powershell
elf2uf2-rs target\thumbv6m-none-eabi\release\pico-hid ..\..\scripts\release\firmware\pico-hid-<version>.uf2
```

## Current Runtime Surface

The firmware now exposes the Synapse Pico HID composite USB device:

- USB VID/PID: `0x2E8A` / `0x1F50`
- Manufacturer/product: `Synapse` / `Synapse Pico HID`
- CDC ACM framed command channel
- HID boot mouse, boot keyboard, and controller interfaces

The CDC dispatcher handles identity, mouse, keyboard, controller, release-all,
watchdog, and telemetry commands. Runtime state drives HID reports, telemetry
counters, firmware-side action-command timing, watchdog release-all behavior,
and LED status. `GET_TELEMETRY` returns the base counters plus
`timed_commands`, `previous_command_delta_us`, `last_command_delta_us`, and
`last_timed_command_uptime_us` so hardware timing checks can inspect the device
clock instead of host send timestamps. Building with the `loopback` feature
enables a debug firmware path that responds with PONG frames instead of driving
HID reports. Building with `force-first-nak` preserves the normal HID runtime
but forces one retry for ACK/NAK pipeline acceptance.

## Flash

1. Hold the Pico `BOOTSEL` button while plugging it into USB.
2. Wait for the `RPI-RP2` mass-storage volume to appear.
3. Copy `scripts\release\firmware\pico-hid-<version>.uf2` to that volume.
4. The Pico reboots automatically.

After flashing, the configured Windows host should expose the Synapse USB
device by VID/PID and a CDC serial port. Manual verification must read that
physical source of truth directly, for example through Windows PnP/USB state
and `Win32_SerialPort`, before exercising commands.

## LED Status

The onboard GP25 LED is driven by runtime state:

- Idle: slow heartbeat, 0.5 Hz, when no command has arrived for at least 5 s.
- Active: steady on for 5 s after a valid command.
- Watchdog: fast blink, 5 Hz, for 2 s after a watchdog release-all event.
- Error: SOS pattern when CRC errors exceed 10 in the last second.

Priority is error, watchdog, active, then idle. Physical LED acceptance still
requires a real Pico connected to this host and flashed with the current UF2.
