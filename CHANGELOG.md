# Changelog

## Unreleased

- Added the repository agent doctrine: manual FSV must be performed by the
  agent with direct source-of-truth readback; automated tests, scripts,
  benchmarks, GitHub Actions, and CI are supporting evidence only.

## v0.1.0-m3 - 2026-05-25

M3 adds the reflex, storage, profile, audio, replay, and HTTP/SSE runtime
surface for Synapse, shipping from manual configured-host FSV rather than CI.

- Added the M3 MCP tools: `subscribe`, `subscribe_cancel`,
  `reflex_register`, `reflex_cancel`, `reflex_list`, `reflex_history`,
  `profile_get`, `profile_set_active`, `replay_record`, `audio_tail`, and
  `audio_transcribe`.
- Filled the M3 crates `synapse-reflex`, `synapse-storage`,
  `synapse-profiles`, and `synapse-audio` with the runtime implementations
  used by the daemon.
- Added streamable HTTP transport and manual SSE event routes with bearer-token
  auth plus Origin and Host enforcement.
- Added the M3 error families `REFLEX_RECURSION_LIMIT`, `HTTP_*`,
  `STORAGE_DISK_PRESSURE_LEVEL_1` through `STORAGE_DISK_PRESSURE_LEVEL_4`, and
  `REPLAY_*`.
- Carried forward M2 fixes for #244, #234, #233, #231, and #239, including the
  LoC splits needed to keep M2 action modules maintainable.
- Recorded the M3 operating doctrine: manual FSV must use the real Synapse
  runtime surface where available, then separately inspect the physical SoT.

## v0.1.0-m2 - 2026-05-24

M2 adds the action MVP for the configured Windows host with manual FSV as the
release gate.

- Added the nine M2 MCP action tools: `act_click`, `act_type`, `act_press`,
  `act_aim`, `act_drag`, `act_scroll`, `act_pad`, `act_clipboard`, and
  `release_all`.
- Wired real Windows input paths for keyboard, mouse, UIA InvokePattern, and
  ViGEm-backed virtual Xbox 360 controller reports.
- Added ReleaseAll safety coverage for explicit cleanup, shutdown, SIGINT,
  stdio disconnect, and panic paths.
- Verified the configured host's ViGEmBus installation through driver/device
  readback, live `act_pad`, XInput state, `release_all`, and daemon logs.
- Clarified that M2 ships from manual configured-host FSV, not GitHub
  Actions/CI or missing-dependency portability tests.

## v0.1.0-m0 - 2026-05-23

M0 bootstraps Synapse as a Rust MCP server with a single `health` tool over stdio.

- Added the Rust workspace, crate skeletons, dual license files, cargo-deny configuration, CI workflow, and helper scripts.
- Implemented `synapse-core` shared types, schema version, and M0 error-code constants.
- Implemented `synapse-telemetry` JSON file logging, console logging, and log-dir validation.
- Implemented `synapse-mcp` stdio startup, CLI flags, graceful shutdown logging, and the `health` MCP tool.
- Implemented `synapse-test-utils` raw stdio JSON-RPC client and M0 end-to-end health demo tests.
- Added root README quick start, current Rust/dependency ADR, documentation link checking, and WSL-global Codex/Claude Code MCP configuration guidance.
