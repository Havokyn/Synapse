# 12 — Milestones, Roadmap, and Open Decisions

Source files covered:
- `CHANGELOG.md`
- `README.md`
- `AGENTS.md`
- `docs/impplan/README.md`
- `docs/impplan/00_methodology.md`
- `docs/impplan/01_m0_bootstrap.md`
- `docs/impplan/02_m1_perception_mvp.md`
- `docs/impplan/03_m2_action_mvp.md`
- `docs/impplan/04_m3_reflex_mcp_surface.md`
- `docs/impplan/05_m4_hardware_hid_first_game.md`
- `docs/impplan/06_m5_production_polish.md`
- `docs/impplan/07_cross_cutting.md`
- `docs/computergames/15_roadmap_and_milestones.md`
- `docs/computergames/16_open_questions.md`
- `docs/adr/0001..0004*.md`

## 1. Authority order

Per `docs/impplan/README.md` §"State-tracking", the authority order is:

1. **Git tags + `CHANGELOG.md`** — what shipped.
2. **`main` branch** — what is in code now (impplan is wrong if it disagrees; patch the impplan in the same PR).
3. **GitHub Issues** — every PR-sized task, `[DECISION]`, `[DISCOVERY]`, bug, risk, context (labels: `phase:m{N}`, `area:*`).

## 2. Milestone state (as of 2026-05-25)

| # | Milestone | Tag | Date | Source |
|---|---|---|---|---|
| M0 | Workspace + rmcp stdio + `health` tool | `v0.1.0-m0` | 2026-05-23 | `CHANGELOG.md::v0.1.0-m0` |
| M1 | Perception MVP — capture + UIA + `observe()` + 5 tools | `v0.1.0-m1` | 2026-05-23 | `docs/impplan/README.md` |
| M2 | Action MVP — `synapse-action` + 9 tools + `release_all` | `v0.1.0-m2` | 2026-05-24 | `CHANGELOG.md::v0.1.0-m2` |
| **M3** | **Reflex + RocksDB + profiles + HTTP/SSE + audio + 11 tools** | — | **ACTIVE** | `docs/impplan/04_m3_reflex_mcp_surface.md` |
| M4 | RP2040 firmware + first game profile (Minecraft) | — | blocked by M3 | `docs/impplan/05_m4_hardware_hid_first_game.md` |
| M5 | Production polish — installer, overlay, ≥10 profiles, soak | — | blocked by M4 | `docs/impplan/06_m5_production_polish.md` |

M3 is in flight on `main` and has already landed:

- `synapse-storage` (RocksDB open + 11 CFs + GC + disk pressure + JSON codecs)
- `synapse-reflex` (event bus + scheduler + 5 reflex kinds + audit persistence)
- `synapse-profiles` (TOML loader + live reload via `notify`)
- `synapse-audio` (WASAPI loopback + Whisper-tiny STT scaffold)
- HTTP transport in `synapse-mcp/src/http/*` with Bearer auth, Origin/Host allow-list, MCP-Session-Id enforcement, SSE bridge
- 11 new MCP tools (see §3)

Outstanding M3 work (per `docs/impplan/04_m3_reflex_mcp_surface.md`):

- "subscribe `buffer_size` parameter is currently hard-pinned to 4096" — schema accepts any value but the live code rejects anything else (see `crates/synapse-mcp/src/m3/subscribe.rs::subscribe_to_events`). PRD calls for this to be honored per-subscriber.
- Persistent writers for `CF_EVENTS`, `CF_OBSERVATIONS`, `CF_SESSIONS`, `CF_TELEMETRY`, `CF_ACTION_LOG`, `CF_PROCESS_HISTORY`, `CF_KV` — only `CF_REFLEX_AUDIT` has a live writer in this build.
- Audio detectors are wired in `synapse-audio` but `M3State::ensure_audio_runtime` calls `AudioConfig { detectors_enabled: false }`. The detector→event_bus integration is reserved.
- HUD extraction pipeline (profile-driven OCR/template-match) is parsed but not run against live frames; `Observation.hud` stays empty unless populated by synthetic fixtures.
- VLM `describe` tool and Florence-2 integration → M5 (`docs/impplan/README.md`).
- `act_combo` standalone tool — combos work via `reflex_register(kind: combo, ...)`; standalone tool deferred to M4 per impplan §1.6.
- `act_run_shell`, `act_launch` — gated tools deferred to M4.

## 3. Tools delivered vs planned

PRD `docs/computergames/05_mcp_tool_surface.md` defines a 30-tool hard cap. As of M3:

| # | Tool | Milestone | Status | Note |
|---|---|---|---|---|
| 1 | `observe` | M1 | live | |
| 2 | `find` | M1 | live | |
| 3 | `describe` | M5 (VLM) | not live | reserved |
| 4 | `read_text` | M1 | live | |
| 5 | `read_hud` | M3 | **not live** | HUD pipeline not yet wired |
| 6 | `audio_tail` | M3 | live | |
| 7 | `audio_transcribe` | M3 | live (en only) | |
| 8 | `subscribe` (+`subscribe_cancel`) | M3 | live | `buffer_size` pinned at 4096 |
| 9 | `set_capture_target` | M1 | live | |
| 10 | `set_perception_mode` | M1 | live | |
| 11 | `act_click` | M2 | live | modifiers not yet wired |
| 12 | `act_type` | M2 | live | |
| 13 | `act_press` | M2 | live | |
| 14 | `act_aim` | M2 | live | Element / Track targets return `ACTION_BACKEND_UNAVAILABLE` |
| 15 | `act_drag` | M2 | live | |
| 16 | `act_scroll` | M2 | live | |
| 17 | `act_pad` | M2 | live | |
| 18 | `act_combo` | M4 | not live | replicated via `reflex_register` |
| 19 | `act_clipboard` | M2 | live | |
| 20 | `act_run_shell` | M4 (gated) | not live | |
| 21 | `act_launch` | M4 (gated) | not live | |
| 22 | `reflex_register` | M3 | live | |
| 23 | `reflex_cancel` | M3 | live | |
| 24 | `reflex_list` | M3 | live | |
| 25 | `reflex_history` | M3 | live | |
| 26 | `release_all` | M2 | live | |
| 27 | `profile_list` | M3 | live | |
| 28 | `profile_activate` | M3 | live | use_scope=unknown requires `--allow-unknown-profile` |
| 29 | `health` | M0 | live | |
| 30 | `replay_record` | M3 | live | JSONL only |

Live count in `crates/synapse-mcp/src/server.rs`: **22** (M1: 6, M2: 9, M3: 7 unique tool methods, with `subscribe`+`subscribe_cancel` counted as 2 in the M3 stub array of 11; the M3 `m3_tool_stubs()` length-asserts to 11).

## 4. Architecture Decision Records (ADRs)

| File | Title | Decision summary |
|---|---|---|
| `docs/adr/0001-current-rust-and-dependencies.md` | Current Rust + dependencies | Pin to the current installed stable toolchain (`rust-version = "1.95"`); no MSRV downgrade; JSON-only persisted codecs in `synapse-storage` (per RUSTSEC-2025-0141) |
| `docs/adr/0002-rocksdb-primary-storage.md` | RocksDB as primary storage | Chose RocksDB over LMDB/sled for the 11-CF schema; rationale around column-family compaction filters and prefix bloom |
| `docs/adr/0003-reflex-recursion-guard.md` | Reflex recursion guard | OnEvent fires are capped at `MAX_ON_EVENT_FIRINGS_PER_TICK = 4` per tick; overflow emits `REFLEX_RECURSION_LIMIT` audit + bus event exactly once per tick |
| `docs/adr/0004-reflex-priority.md` | Reflex priority semantics | Lower number = higher priority; ties broken by registration order; `MAX_REFLEX_PRIORITY = 1000`, `DEFAULT_REFLEX_PRIORITY = 100` |

## 5. Operator-level invariants (from `docs/impplan/00_methodology.md`)

These are doctrine — **NEVER violate**:

1. **No backward compatibility (pre-v1).** Schema/API changes break callers; no fallbacks, no shims, no silent error swallowing. Anything that does not work must fail fast with a structured `synapse_core::error_codes::*` code and a tracing log line containing that code.
2. **No mocks gate completion.** OS-bound work-items are not done until a real-OS integration test exercises them against the real SoT (UIA `ValuePattern`, `XInputGetState`, RocksDB key, `GetClipboardData`, `GetCursorPos`, low-level keyboard hook, etc.).
3. **Full-State Verification (FSV) is mandatory and manual.** The agent reads the SoT before, executes the trigger, performs a separate read for "after", exercises ≥3 edge cases (empty/boundary/structurally-invalid), and records actual state. **Scripts, tests, benchmarks, harnesses, GitHub Actions, and CI are supporting evidence only.** They never count as FSV. Do not add `*_fsv` tests, FSV harnesses, or FSV scripts.
4. **Natural-only motion (OQ-004 DECIDED 2026-05-22).** `Natural` curves + `Natural` keystroke dynamics tuned `FAST` are the resolved default of every tool, profile, and reflex. `Instant`/`Burst` exist for explicit opt-in only.
5. **Manual FSV on the configured Windows host is the shipping gate, not CI** (operator decision 2026-05-24, issues #246/#247/#350/#351). Do not dispatch, wait on, or block a tag on GitHub Actions/CI. Do not add `*_fsv` tests.

`AGENTS.md` reinforces these and pins **`[skip ci]` on every agent commit**.

## 6. Per-PR contract (from `docs/impplan/README.md`)

Every PR must satisfy:

```
✓ Compiles release + dev
✓ Clippy zero warnings (workspace + all-targets)
✓ Tests pass (`cargo test --workspace`)
✓ Files ≤ 500 LoC; functions ≤ 30 LoC; cyclomatic ≤ 10
✓ Error variants carry SCREAMING_SNAKE_CASE .code()
✓ Public APIs / CF names are `pub const`
✓ Tracing spans on every non-trivial fn
✓ No mocks gate completion (real captures, real RocksDB, real SendInput, real ViGEm)
✓ Schema change ⇒ wipe-and-rebuild (pre-v1, no shim)
✓ Bench delta ≤ 20% on tracked metrics
✓ Docs cross-refs intact (`scripts/check_docs.ps1`)
✓ Manual issue evidence captures SoT before/readback-after state
```

The 500-LoC file cap is violated in three places per impplan and per current code:

- `crates/synapse-mcp/src/server.rs` (1250 LoC) — the tool router is exempt by design
- `crates/synapse-action/src/emitter.rs` (split across `emitter/*.rs` submodules; the umbrella file is under cap)
- `crates/synapse-a11y/src/lib.rs` (2087 LoC) — single-file lib; impplan calls it out as needing a split before M3 builds on top
- `crates/synapse-capture/src/lib.rs` (1798 LoC) — same
- `crates/synapse-core/src/types.rs` (1567 LoC) — type catalog, exempt

## 7. Performance budgets (binding — from PRD §11)

| Stage | Target p99 |
|---|---|
| Frame capture (zero-copy GPU surface) | ≤ 3 ms |
| Detection inference (small CNN on 5090-class GPU) | ≤ 8 ms |
| UIA tree snapshot for focused window | ≤ 10 ms |
| Full `observe()` response | ≤ 30 ms (`REFERENCE_OBSERVE_WARM_HYBRID_P99_MS`) |
| Event push from underlying frame/UIA event to subscriber | ≤ 50 ms (`REFERENCE_EVENT_TO_SUBSCRIBER_P99_MS`) |
| `act_aim` start-of-motion latency | ≤ 5 ms |
| `act_press` to electrical signal on USB | ≤ 2 ms (software) / ≤ 4 ms (hardware HID) |
| Reflex `on_event` action emission | ≤ 5 ms from event |
| Reflex scheduler tick jitter idle | ≤ 200 µs (`REFERENCE_REFLEX_TICK_JITTER_IDLE_P99_US`) |
| MCP idle-tick CPU usage | ≤ 1% on one core |
| Steady-state VRAM when models loaded | ≤ 2 GB |

These targets are verified via the criterion benches in `crates/*/benches/` and tracked in the bench-delta script (`scripts/check-bench-delta.ps1`, ≤20% regression gate).

## 8. Open questions (PRD `16_open_questions.md`) and their decisions

The PRD's "Open Questions" file enumerates roughly 30 numbered items (OQ-001 … OQ-029). The ones explicitly DECIDED that show up in code:

| OQ | Decision | Code/artifact |
|---|---|---|
| OQ-004 | Natural-only motion defaults (Natural curves + Natural keystroke dynamics tuned `FAST`) | `AimNaturalParams::FAST`, `KeystrokeNaturalParams::FAST` in `synapse-core/src/types.rs` |
| OQ-001/005/010/012/015/022/023/024/029 | Various M3 design closures (event filter depth, reflex recursion, audit retention, etc.) | See ADRs 0003/0004 and `synapse-reflex` source |
| operator decisions 2026-05-24 (issues #246/#247/#350/#351) | No GitHub Actions / CI as a shipping gate | `AGENTS.md` |

Open items remaining (PRD §16) cover: VLM `describe` model selection (M5), `act_combo` API ergonomics (M4), profile schema v2 plans, packaging/signing strategy, telemetry export endpoint.

## 9. Doctrine documents

| File | What it pins |
|---|---|
| `docs/computergames/README.md` | Project mission, repository layout, performance targets, authoring rules |
| `docs/computergames/00_vision_and_scope.md` | Non-goals, supported contexts |
| `docs/computergames/01_architecture.md` | Process boundaries, thread model, crate dep graph |
| `docs/computergames/02_perception.md` | Capture/A11y/OCR/Audio sensors and the perception mode auto-selector |
| `docs/computergames/03_action.md` | Action emitter design, backends, rate limits, curve/dynamics |
| `docs/computergames/04_reflex_runtime.md` | Reflex semantics, scheduler, conflict resolution |
| `docs/computergames/05_mcp_tool_surface.md` | The 30-tool registry (the contract) |
| `docs/computergames/06_data_schemas.md` | Wire schemas + error code catalog |
| `docs/computergames/07_storage_and_profiles.md` | RocksDB CFs, retention defaults, profile TOML |
| `docs/computergames/08_supported_use_policy.md` | Allowed/disallowed contexts, operator acknowledgments |
| `docs/computergames/09_hardware_hid_gateway.md` | M4 Pi Pico HID firmware + serial protocol + host driver |
| `docs/computergames/10_performance_budget.md` | Per-stage p99 targets + optimization rules |
| `docs/computergames/11_security_and_safety.md` | Threat model, permissions, redaction, kill switches |
| `docs/computergames/12_observability.md` | Logging, tracing, metrics, debug overlay, replay tool |
| `docs/computergames/13_testing_strategy.md` | Unit/integration/E2E, fixtures, manual FSV, perf regression |
| `docs/computergames/14_build_and_packaging.md` | Workspace, deps, profiles, installer, signing |
| `docs/computergames/15_roadmap_and_milestones.md` | M0-M5 phases, scope per milestone, demo criteria |
| `docs/computergames/16_open_questions.md` | Unresolved decisions, ADRs needed |
| `docs/computergames/17_research_appendix.md` | Web research, comparable projects, references |
| `docs/impplan/00_methodology.md` | Dev discipline, FSV protocol, work-item shape |
| `docs/impplan/0{1..6}_m{0..5}_*.md` | Per-milestone work-item ledger |
| `docs/impplan/07_cross_cutting.md` | Perf gates, security, observability, release |
| `docs/dev-host-hygiene.md` | Configured-host hygiene checklist |
| `docs/m1_error_throw_map.md` | M1 error-code throw-site map |
| `docs/AICodingAgentSuperPrompt.md` | Repository agent wake-up prompt |
| `docs/compressionprompt.md` | Doctrine for compressed implementation-plan authoring |

## 10. M3 demo gate (acceptance criteria)

From `docs/impplan/04_m3_reflex_mcp_surface.md::§2`:

1. Real Win11 box. Notepad open. Claude Desktop configured with `synapse-mcp` over stdio.
2. Agent registers an `on_event` reflex that fires when a `Save As` window appears.
3. Agent observes Notepad, types text, and triggers Save As (Ctrl+S).
4. Reflex fires and emits the configured actions (type filename, press Enter), persists a `reflex_fired` audit row to `CF_REFLEX_AUDIT`, and updates an SSE subscriber if attached.
5. Operator verifies via direct UIA/file-system readback that:
   - The file exists.
   - The audit row is present in `CF_REFLEX_AUDIT`.
   - The reflex priority and lifetime evolved correctly.
6. Operator hotkey `Ctrl+Alt+Shift+P` cleanly disables all reflexes and fires `release_all` within 50 ms.

## 11. What is NOT covered in this doc

- **Detailed per-issue history.** That lives in the GitHub issue tracker (https://github.com/ChrisRoyse/Synapse/issues). The impplan files reference issue numbers but do not duplicate full discussion threads.
- **Operator runbook / install steps.** Those are in `README.md` and `docs/dev-host-hygiene.md`.
- **Future v2 work (Linux / macOS / cross-platform).** Out of scope per PRD §"Out of scope".
