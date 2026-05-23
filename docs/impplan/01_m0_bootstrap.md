# 01 — M0: Bootstrap (1 week) — DONE

**Status:** Closed 2026-05-23 by release tag `v0.1.0-m0` (commit `f04b429`).
**Evidence:** `crates/synapse-mcp/tests/m0_demo_gate.rs` exercises the full
`tools/list` → `tools/call health {}` round-trip end-to-end. CI green on Ubuntu
+ Windows (`.github/workflows/ci.yml`). M0 work-items and acceptance gates
below are preserved for archival and onboarding.

PRD: `15_roadmap_and_milestones.md` §2.

## Goal

Empty repo → `synapse-mcp` binary serving MCP stdio, one tool (`health`) returning hardcoded JSON. CI green.

## Demo gate

Claude Desktop configured with `synapse-mcp` as MCP server → user asks Claude to call `health` → response `{"ok": true, "version": "0.1.0", ...}`.

---

## Inputs

- Fresh repo (or clean branch)
- Installed stable Rust toolchain (M0 currently verified with rustc/cargo 1.95.0; see `docs/adr/0001-current-rust-and-dependencies.md`)
- Windows 11 dev box (primary) or Linux (CI-only checks; OS-bound code stubbed)
- Claude Desktop (or any MCP-stdio client) for demo

---

## Deliverables

### Files

```
Cargo.toml                                 (workspace manifest, 14_build_and_packaging §1-2)
deny.toml                                  (cargo-deny config, 14 §14)
.gitignore
LICENSE-MIT, LICENSE-APACHE
README.md                                  ("Hello Synapse" only at M0)
.github/workflows/ci.yml                   (fmt, clippy, test, deny, audit)
scripts/new-crate.ps1                      (crate template)
scripts/check_docs.ps1                     (cross-doc CI)
```

### Crates (skeleton)

| Crate | M0 contents |
|---|---|
| `synapse-core` | `Backend`, `PerceptionMode`, `Point`, `Rect`, `Size`, `SessionId`, `SCHEMA_VERSION`, `error_codes` module with stubs for the catalog from `06 §8` |
| `synapse-mcp` | `main.rs` (≤ 300 LoC), CLI via `clap`, `--mode stdio\|http`, `rmcp` server with `health` tool |
| `synapse-telemetry` | `tracing-subscriber` JSON file + console layer, log dir `%LOCALAPPDATA%\synapse\logs\` |
| `synapse-test-utils` | Custom MCP client over stdio for E2E (used at M0 demo + later) |
| `synapse-storage` | stub `Db` trait, no impl |
| `synapse-perception`, `synapse-action`, `synapse-reflex`, `synapse-capture`, `synapse-a11y`, `synapse-audio`, `synapse-profiles`, `synapse-hid-host`, `synapse-models`, `synapse-overlay` | stub crates with `lib.rs` empty + `Cargo.toml` template |
| `firmware/pico-hid/` | Not created at M0 (added at M4) |

### Tool

| Tool | Schema | Behavior |
|---|---|---|
| `health` | `05_mcp_tool_surface.md` §3.29 (simplified) | Returns hardcoded `{ok:true, version, build, uptime_s, subsystems:{}}`; no real subsystem queries yet |

---

## Work-items (PR-sized, ordered)

| # | Title | Acceptance |
|---|---|---|
| 1 | `chore: workspace scaffold` | `Cargo.toml` + 15 crate stubs + `cargo build --workspace` green |
| 2 | `chore: deny + clippy + fmt` | CI matrix passes on a no-op PR using the installed stable toolchain |
| 3 | `feat(core): geometry + ids + Backend + PerceptionMode + SCHEMA_VERSION` | `synapse_core::types` snapshot test (`insta`) baseline |
| 4 | `feat(core): error_codes module stub` | every code from `06 §8` declared as `pub const NAME: &str = "NAME";`; test asserts `NAME == "NAME"` |
| 5 | `feat(telemetry): tracing JSON + console + rolling appender` | running binary produces `%LOCALAPPDATA%\synapse\logs\synapse.log` JSONL |
| 6 | `feat(mcp): clap CLI + rmcp stdio bootstrap` | binary launches, accepts JSON-RPC `initialize`, replies with capabilities |
| 7 | `feat(mcp): health tool registration` | `tools/list` shows `health`; `tools/call health {}` returns the schema |
| 8 | `feat(test-utils): stdio MCP client harness` | integration test spawns `synapse-mcp`, calls `health`, asserts shape |
| 9 | `chore(ci): doc cross-ref check via scripts/check_docs.ps1` | broken markdown link fails CI |
| 10 | `docs(readme): Hello Synapse quick-start` | reader follows instructions, sees `health` reply |

---

## Acceptance gates (block M1)

```
✓ `cargo build --release --workspace` on Win11 + Linux
✓ `cargo clippy --workspace --all-targets -- -D warnings`
✓ `cargo test --workspace`
✓ `cargo deny check`
✓ `cargo audit`
✓ scripts/check_docs.ps1 green
✓ Claude Desktop or `synapse-test-utils` integration test calls health(), receives valid response
✓ Process exits 0 on SIGINT; logs flushed
✓ Binary size release-stripped ≤ 5 MB at M0 (will grow through M5)
```

---

## Risks (`15 §9`)

| Risk | Mitigation |
|---|---|
| `rmcp` API churn | Pin `rmcp = "1.7"` exact; do not bump without manual test |
| Workspace deps version conflicts | All deps in `[workspace.dependencies]`; per-crate uses `dep.workspace = true`; current compatible versions are resolved against the installed stable toolchain |
| Win11-only paths in stub crates | All OS calls behind `#[cfg(windows)]`; Linux build sets stub functions to `unimplemented!()` (never called on Linux CI which only runs Linux-portable tests) |

---

## Out of scope at M0 (deferred ≥ M1)

- Perception of any kind — `health` is the only tool
- Action emission — no `SendInput`, no `enigo`
- Storage — `Db` trait stub only; no RocksDB at M0
- Profiles — bundled dir empty
- Overlay
- ViGEm, hardware HID
- Models (no ONNX)

---

## Definition of Done

M0 is closed when: (a) demo passes via Claude Desktop, (b) all acceptance gates green, (c) `git tag v0.1.0-m0` cuts a build artifact for archival.

Open next: `02_m1_perception_mvp.md`.
