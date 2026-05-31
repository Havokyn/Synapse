# CURRENT STATE - Synapse

## 2026-05-31
- Required doctrine loaded from `docs/AICodingAgentSuperPrompt.md` and `AGENTS.md`.
- GitHub issue queue read: open issues are #590, #589, #588, and #585.
- #351 closed decision read: manual FSV only; no FSV scripts/tests/harnesses/CI as acceptance; agent commits pushed must include `[skip ci]`.
- Worktree is dirty before #589 edits:
  - Modified: `crates/synapse-action/src/backend/software/mouse.rs`, `crates/synapse-mcp/src/m1.rs`, `crates/synapse-mcp/src/m4.rs`, `crates/synapse-profiles/profiles/photos.toml`, `docs/AICodingAgentSuperPrompt.md`.
  - Untracked: `STATE/`.
  - `Cargo.toml` and `firmware/pico-hid/**` were reported dirty on the first read but a later direct read showed them present and not modified.
- #589 has a progress comment saying `firmware/pico-hid` was deleted and the robust plan is to remove the dead HID implementation/operator surface while keeping hardware enum tags routed to a clear unavailable backend error.
- #589 resume comment posted: current SoT still contains `crates/synapse-hid-host`, `firmware/pico-hid`, `--hardware-hid`/`SYNAPSE_HARDWARE_HID`, and health HID status surfaces.

## Open Queue Snapshot
- #588: context/decision, software-only input strategy; physical HID path abandoned.
- #589: remove dead hardware-HID path. Claimed/resumed in issue comment on 2026-05-31.
- #590: add software-backend input fidelity benchmarks for SendInput and ViGEm timing.
- #585: hardening, move UIA calls to a dedicated MTA worker thread; prior comment says this is a larger refactor, not a correctness fix.
