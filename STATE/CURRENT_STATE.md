# CURRENT STATE - Synapse

## 2026-05-31
- Required doctrine re-read after compaction from:
  - `C:\code\Synapse\docs\AICodingAgentSuperPrompt.md`
  - `C:\Users\hotra\Downloads\AICodingAgentSuperPrompt.md`
  - `AGENTS.md`
  - #351 manual-FSV/no-CI decision
- GitHub issue queue read after closing #589: open issues are #590, #588, and #585.
- `main` and `origin/main` are both at `d9edad8 docs(state): update session state after docs alignment commit [skip ci]`.
- Worktree is clean after pushing `828eec2`.
- #589 implementation commits already on `main`:
  - `e0e9993 refactor: retire physical hardware-HID path for software-only input (#588)`
  - `a44d845 docs: align documentation with software-only input refactor [skip ci]`
  - `d9edad8 docs(state): update session state after docs alignment commit [skip ci]`
  - `828eec2 test(profiles): update signed manifest digest after HID removal [skip ci]`
- #589 removed the dead RP2040/Pico and serial hardware-HID path while retaining hardware enum/profile tokens as fail-closed compatibility tags through `HardwareUnavailableBackend`.
- Uncommitted fix: signed package manifest expected digest updated to `sha256:4013ce772c32c5ba641d78848ba1b04add3224fe7fab12822572e6343b5b38c7` after hardware metadata removal changed the deterministic signature payload.

## #589 Manual FSV Evidence Captured
- Repo-built daemon launched from `C:\code\Synapse\target\release\synapse-mcp.exe`.
- Runtime SoT:
  - PID `56908`
  - command line: `"C:\code\Synapse\target\release\synapse-mcp.exe" --mode http --bind 127.0.0.1:7791`
  - socket: `127.0.0.1:7791` listening, owning process `56908`
  - isolated DB: `C:\code\Synapse\.runs\589\http-fsv\db`
  - bearer token source in startup log: `env`
- Authenticated health SoT:
  - direct `/health` and MCP `health` returned `ok=true`
  - subsystems are `action,audio,http,profiles,reflex,storage`
  - `hid_host` subsystem is absent
  - action detail has no `hardware_hid`, `--hardware-hid`, or `SYNAPSE_HARDWARE_HID`
- Strict client-parity SoT:
  - official MCP Inspector CLI was used against `http://127.0.0.1:7791/mcp`
  - `tools/list` exited 0 with 80 tools, including `health`, `storage_inspect`, and `act_press`
  - no tool names matched `hid|hardware`
  - tools-list output did not mention `InputHardwareHid`, `INPUT_HARDWARE_HID`, `--hardware-hid`, `SYNAPSE_HARDWARE_HID`, or `hid_host`
- Storage/action SoT using real MCP `tools/call`:
  - Before happy path, `storage_inspect` read `CF_ACTION_LOG=0`.
  - Happy path trigger: `act_press` with `keys=["shift"]`, `hold_ms=1`, `backend=software`; result `ok=true`, `backend_used=software`, `keys_pressed=1`.
  - After happy path, `storage_inspect` read `CF_ACTION_LOG=2` with `act_press` rows `started` and `ok`.
  - Edge 1 trigger: `act_press` with `keys=["shift"]`, `hold_ms=1`, `backend=hardware`; Inspector exited 1 with `ACTION_BACKEND_UNAVAILABLE` and detail `hardware backend removed; use backend=software or backend=vigem action_kind=key_down`.
  - After edge 1, `storage_inspect` read `CF_ACTION_LOG=4` with new rows `started` and `error` / `ACTION_BACKEND_UNAVAILABLE`.
  - Edge 2 trigger: `act_press` with `keys=[]`, `hold_ms=1`, `backend=hardware`; Inspector exited 1 with `TOOL_PARAMS_INVALID` / `act_press keys must contain at least one key`.
  - After edge 2, `storage_inspect` read `CF_ACTION_LOG=6` with new rows `started` and `error` / `TOOL_PARAMS_INVALID`.
  - Edge 3 trigger: `act_press` with `keys=["definitely-not-a-key"]`, `hold_ms=1`, `backend=hardware`; Inspector exited 1 with `ACTION_UNSUPPORTED_KEY`.
  - After edge 3, `storage_inspect` read `CF_ACTION_LOG=8` with new rows `started` and `error` / `ACTION_UNSUPPORTED_KEY`.
- CLI/operator-surface SoT:
  - `target\release\synapse-mcp.exe --help` contains no `--hardware-hid`, no `SYNAPSE_HARDWARE_HID`, and no `--reset-hardware-consent`.
  - `target\release\synapse-mcp.exe --mode http --bind 127.0.0.1:7792 --hardware-hid auto` exited 2 with `unexpected argument '--hardware-hid'`; before/after socket reads on port 7792 were empty.
  - `SYNAPSE_MCP_ALLOWED_PERMISSIONS=INPUT_HARDWARE_HID target\release\synapse-mcp.exe --mode http --bind 127.0.0.1:7793` exited 1 with `unknown M3 permission "INPUT_HARDWARE_HID"`; before/after socket reads on port 7793 were empty.
- Cleanup SoT:
  - repo-built FSV daemon PID `56908` was stopped after evidence capture
  - follow-up process/socket read showed PID `56908` absent and no listener on port 7791
  - pre-existing installed `%USERPROFILE%\.cargo\bin\synapse-mcp.exe` stdio daemons were left untouched
- File/dependency SoT:
  - `Test-Path` false for `crates\synapse-hid-host`, `firmware\pico-hid`, `crates\synapse-action\src\backend\hardware`, and `crates\synapse-mcp\src\safety\hardware_consent.rs`.
  - live-reference scan returned no matches for removed hardware-HID flags/crates/deps/permissions after excluding historical changelog text, retired-plan stubs, and absence assertion tests.
  - `Cargo.toml` / `Cargo.lock` scan returned no matches for `synapse-hid-host`, `serialport`, `crc16`, or `pico-hid`.

## Supporting Checks
- `cargo fmt`
- `cargo check -p synapse-mcp` (passes; warning only: pre-existing `element_screen_point` dead-code warning in `synapse-action`)
- `pwsh scripts/check_docs.ps1`
- `cargo test -p synapse-mcp --test cli_modes help_lists_m4_policy_flags_and_omits_removed_hardware_hid`
- `cargo test -p synapse-mcp register_permissions_do_not_add_removed_hardware_backend_gate`
- `cargo test -p synapse-action --test hardware_unavailable`
- `cargo test -p synapse-core --test error_codes_literal`
- `cargo test -p synapse-profiles --test parse_bundled`
- `cargo test -p synapse-profiles --test package_manifest` after digest fix: 12 passed
- `git diff --check` passes with line-ending warnings only.

## Open Queue Snapshot
- #589: RESOLVED comment posted and issue closed.
- #590: active next issue; add software-backend input fidelity benchmarks for SendInput and ViGEm timing.
- #585: hardening, move UIA calls to a dedicated MTA worker thread.
- #588: context issue, close after #589 and #590 are resolved.
