# 00 — Methodology

Discipline applied across M0-M5. PRD authority: `docs/computergames/README.md` §"Authoring rules" + `14_build_and_packaging.md` + `13_testing_strategy.md`. State-tracking authority: git tags + `CHANGELOG.md` (final) > codebase on `main` (operational) > GitHub Issues (https://github.com/ChrisRoyse/Synapse/issues — every PR-sized task, `[DECISION]`, `[DISCOVERY]`, `[BUG]`, `[RISK]`, `[CONTEXT]`). Use `gh issue list --state all --search 'phase:m3'` (etc.) to walk landed-decision history for context the commit message did not carry. **All 244 historical issues are closed as of 2026-05-24**; new work opens fresh issues with the same labels.

**Three load-bearing operator directives (NEVER violate):**

1. **No backwards compatibility (pre-v1).** Schema/API changes break callers. No fallbacks, no compatibility shims, no silent error swallowing. Fail fast with a structured `synapse_core::error_codes::*` code and a `tracing` line carrying that code so the failure is debuggable.
2. **No mocks gate completion.** Unit fakes are fine for isolation. An OS-bound work-item is **not done** until a real-OS integration test exercises it and a separate source-of-truth read confirms the side effect landed.
3. **Manual configured-host FSV is the shipping gate, not GitHub Actions** (issues #246/#247, operator decision 2026-05-24). CI is the regression safety net; do not block a tag on CI; do not attempt to wait on workflow runs to "ship" a phase.

---

## 1. Hard code rules (CI-enforced)

| Rule | Mechanism |
|---|---|
| `#![forbid(unsafe_code)]` workspace-wide | per-crate override only for `synapse-action` (Win32 SendInput batching), `synapse-capture` (DX FFI), `synapse-hid-host` (serial OS handle), `firmware/pico-hid` |
| File ≤ 500 LoC, function ≤ 30 LoC, cyclomatic ≤ 10 | clippy + custom lint check in CI. **M2 left 6 files over cap** (emitter.rs 1474, vigem.rs 1131, invoke.rs 653, software.rs 586, m2/click.rs 506, m2/press.rs 545) — M3 work-item A.0a/A.0b splits them before building reflex on top. Reviewers must enforce at ≤ 450 LoC during code review to leave 50 LoC of margin. |
| `unwrap()` / `expect()` forbidden outside `#[cfg(test)]` | `#[deny(clippy::unwrap_used, clippy::expect_used)]` |
| `anyhow` forbidden in library crates | manual review + workspace dep gating |
| No `println!` / `eprintln!` | clippy lint + grep gate |
| Public API constants are `pub const`, not magic strings | test asserts every CF name / error code matches its literal |
| Error variants ⇒ `SCREAMING_SNAKE_CASE` code via `thiserror` impl with `.code()` | snapshot test (`13_testing_strategy.md` §3) |
| Schema change pre-v1 ⇒ wipe DB, no migration shim | doc gate; CI runs sample wipe-and-rebuild |
| Files referenced in docs exist | doc-link CI check |

---

## 2. Crate layout invariants

`synapse-core` ← zero internal deps (type/error/const root). Verified each PR via `cargo tree -p synapse-core --depth 1` showing only crates.io deps.

Acyclic graph: `01_architecture.md` §5. CI fails if any new edge introduces a cycle.

Per-crate `Cargo.toml` scaffold from `14_build_and_packaging.md` §17. New crate via `scripts/new-crate.ps1 -Name synapse-<name>`.

---

## 3. Concurrency invariants

| Invariant | Where |
|---|---|
| Action emission serialized per device through one mpsc actor | `synapse-action` |
| Capture, reflex on dedicated OS threads at `THREAD_PRIORITY_TIME_CRITICAL` — never tokio pool | `synapse-capture`, `synapse-reflex` |
| UIA event handlers on COM apartment thread, marshal across channel | `synapse-a11y` |
| Audio loopback thread at MMCSS "Pro Audio" | `synapse-audio` |
| Perception result is single-producer multi-consumer via `tokio::sync::watch` | `synapse-perception` |
| Reflex + MCP tool handlers are the only writers to action channel | `synapse-action` |

Bounded channels everywhere. Drop policy documented per channel (`10_performance_budget.md` §10).

---

## 4. Error handling discipline

Three classes (`01_architecture.md` §9):

| Class | Where | Strategy |
|---|---|---|
| Recoverable (transient OS) | capture/a11y/audio/action | log warn → retry w/ backoff → structured error if persistent |
| User-facing | MCP tool handlers | JSON-RPC error: `code: -32099`, `data.code: SCREAMING_SNAKE_CASE` |
| Fatal | storage corruption, unsafe FFI | `panic!` → panic hook fires `ReleaseAll` → process exits |

`#[derive(thiserror::Error)]` per crate. `.code() -> &'static str` on every variant. Codes stable post-v1; pre-v1 free to rename.

Error code catalog: `06_data_schemas.md` §8. Exported as `pub const` in `synapse-core::error_codes`. Test asserts constants = literal strings.

---

## 5. Test discipline

Test pyramid: `13_testing_strategy.md` §2.

| Per PR | Frequency |
|---|---|
| `cargo fmt --check` | every PR |
| `cargo clippy --workspace --all-targets -- -D warnings` | every PR |
| `cargo test --workspace` | every PR |
| `cargo deny check` | every PR |
| `cargo audit` | every PR + daily cron |
| `insta review --check` | every PR |
| `cargo bench` perf-regression (tracked benches only) | weekly + PR delta ≤ 20% gate |
| E2E real Windows | nightly self-hosted |
| Hardware-in-loop (RP2040) | weekly self-hosted |
| Fuzz (`cargo-fuzz`) | nightly, 10 min/target |
| Soak (8h) | weekly |

**No mocks gate completion.** Integration tests use real `windows-capture` frames (or `MockCaptureSource` for unit), real RocksDB (`tempfile::TempDir`), real `SendInput` (or `RecordingBackend` for unit). Unit fakes never substitute for integration coverage of the OS-bound path.

**Full-state verification (FSV) — mandatory for every M2+ test:**

1. **Identify the source of truth** for the work the test is meant to perform (UIA `ValuePattern.value` for a typed string, file bytes on disk for a saved file, `RecordingBackend::events()` for the emitted `INPUT` sequence, `vigem-client` device state for a pad report, the daemon's tracing log line for an audit event, the `held_keys` BitSet for stuck-key state, RocksDB `Db::scan(CF_REFLEX_AUDIT, ..)` for a reflex fire, `GetClipboardData(CF_UNICODETEXT)` for clipboard write, `XInputGetState(0)` for ViGEm pad state, etc.).
2. **Print `before` state**, execute the action, **print `after` state**. Use the M1-established `println!("source_of_truth=<name> edge=<edge> before=<...> after=<...>")` convention.
3. **Read back from the source of truth** with a separate operation, distinct from the action under test. Never trust the return value of the operation under test as evidence that the side effect landed.
4. **Three edge cases minimum** per primary path:
   - **Empty / zero input** (`act_type({text:""})`, `audio_tail({seconds:0})`, empty event filter, empty profile dir)
   - **Boundary value** (`act_press({hold_ms:30000})` succeeds vs `30001` rejected; reflex cap 32 vs 33; 4096-event SSE ring boundary)
   - **Structurally invalid** (`schemars` reject → `TOOL_PARAMS_INVALID`; unknown enum variant; out-of-range float)
   Each edge case prints its own `before` / `after` and asserts on the source of truth.
5. **Evidence of success in the log.** The test must emit at least one `source_of_truth=<name> edge=<edge> after_truth=<state>` line and one `source_of_truth=<name> edge=<edge> final_value=<state>` line per scenario. A reviewer greps `--nocapture` stdout for these patterns to verify the side effect landed.
6. **Trigger → outcome reasoning** in a doc-comment on the test fn: identify the trigger event, the process X inside the daemon, the observable outcome Y, and the source of truth that proves Y. See M2 §8.5 for the canonical example.
7. **Synthetic inputs with known expected outputs.** Pick fixtures whose expected source-of-truth state is unambiguous (`"Hello world."` → exact byte sequence in `Notepad ValuePattern.value`; a 5 s WAV with known transcript → Whisper response within Levenshtein ≤ 10%; 33 reflex registrations → cap rejection on the 33rd). Tests assert byte-equal or count-equal, not "looks ok."
8. **Process-restart durability check** for any test touching persistent storage (RocksDB CFs from M3 onward): write data, drop the daemon, spawn a fresh `StdioMcpClient`, read back, assert data survived.

A test that asserts only on return values is **not done**; review fails the PR. A test that produces zero `final_value=` lines in stdout fails review even if `assert_*` passed.

---

## 6. Performance gates

Targets: `10_performance_budget.md` §2 (end-to-end), §3 (CPU), §4 (memory/VRAM), §12 (per-tool SLA).

Hot loops:

- Capture: zero allocs/frame, texture pool reused
- Reflex tick: zero allocs, pre-compiled `EventFilter`
- Action emit: ≤ 1 alloc (the `Vec<INPUT>` for `SendInput`)
- Detection: pre-allocated tensors

CI bench: `criterion` benches stored at `bench_results/<commit_sha>/`. PR delta > 20% on tracked benches blocks merge. Tracked list: `13_testing_strategy.md` §7.

Spike check (`10_performance_budget.md` §15): subsystem > 2× p99 for > 5s ⇒ `synapse-performance-degraded` event + `health.subsystems.X.status = "degraded_latency"`.

---

## 7. Security gates

Per `11_security_and_safety.md`:

- stdio mode trusts parent process; no extra auth
- HTTP mode: bearer token in `%APPDATA%\synapse\token.txt`, Origin/Host check, loopback-only default
- Redaction patterns (`synapse-core::redact`, 19 patterns at v1) apply to: `observe()` text, `read_text()`, `audio_transcribe()`, clipboard summaries, `CF_EVENTS` payloads, replay export, tracing logs, OTLP
- Forbidden capabilities (compile-time `#[cfg(feature)]` off): DLL injection, kernel drivers, raw process memory r/w, FS writes outside profile paths, non-loopback by default
- Panic hotkey `Ctrl+Alt+Shift+P` registered via `RegisterHotKey`; fires `ReleaseAll` + reflex disable in ≤ 50 ms
- `cargo deny`: allow only `MIT`, `Apache-2.0`, `BSD-2/3`, `MPL-2.0`, `ISC`, `Zlib`, `Unicode-3.0`, `BSL-1.0`, `CC0-1.0`. Block GPL/AGPL/SSPL.
- Supported-use gates: `08` §6 — explicit operator configuration for hardware HID and other sensitive capabilities

---

## 8. Observability gates

Per `12_observability.md`:

- Every subsystem instrumented with `tracing::instrument` or `span!`
- Metric set per subsystem (`12_observability.md` §4.1) registered through `synapse-telemetry::metrics`
- Labels bounded — no unbounded values (session IDs, image hashes) as label keys
- `tracing-appender::rolling` daily rotation, 7-day keep, gzip rotated, 500 MB dir cap
- `health` MCP tool returns subsystem statuses matching `06_data_schemas.md::SensorStatus`
- Crash dump on panic: `%LOCALAPPDATA%\synapse\crashes\YYYYMMDD-HHMMSS.dump` with backtrace + last 100 log lines + last 100 events

---

## 9. Storage discipline

Per `07_storage_and_profiles.md` §6 (data lifecycle):

- Every new CF declares TTL + soft cap + hard cap in `synapse-core::retention::DEFAULTS`. No "decide later."
- Binary codec for hot/binary CFs, JSON for human-readable/audit CFs. `bincode` is disallowed after RUSTSEC-2025-0141 (ADR-0001); pick a maintained codec before M3 storage. M0-M2 use JSON only.
- Per-frame writes forbidden — aggregate, batch every 100 ms or 64 KB
- Three cleanup layers: compaction filter, periodic GC (5 min), disk-pressure responder
- Test with 1 GB tmpfs DB volume in CI to verify pressure levels (`07_storage_and_profiles.md` §6.3)

---

## 10. ADR / Open Question workflow

When a `16_open_questions.md` decision is forced during a phase:

1. Check OQ list first
2. If matching OQ exists and answer is now clear ⇒ create `docs/adr/NNN-<title>.md` with decision rationale + diff to PRD
3. Update OQ entry to `## OQ-NNN — <summary> — DECIDED <YYYY-MM-DD>` pointing to the ADR
4. Patch any PRD doc whose claim becomes stale
5. PR title: `adr(NNN): <one-line>`

No silent decisions. A code change that resolves an open question without an ADR fails review.

---

## 11. Cross-doc consistency CI

CI checks:

- Every `pub const` in `synapse-core::error_codes` listed in `06_data_schemas.md` §8
- Every CF name listed in `07_storage_and_profiles.md` §4
- Every tool name listed in `05_mcp_tool_surface.md` §2 and registered in `synapse-mcp::tools`
- Every error code thrown in code has a `pub const` definition
- Markdown links in `docs/**/*.md` resolve

Run: `scripts/check_docs.ps1` (M0 deliverable).

---

## 12. Dev-environment notes (informational, not contract)

These are facts about the current dev box that help when iterating, not requirements on the shipped product. New contributors are not assumed to have any of this set up.

| Resource | Detail |
|---|---|
| WSL → Windows PulseAudio bridge | Windows host runs `pulseaudio.exe` v1.1 on TCP `127.0.0.1:4713` (mirrored WSL networking). Useful when building Block D of M3 (`04 §Block D`, work-items 16-18) for replaying audio fixtures from WSL or recording `output.monitor` without leaving the Linux shell. Does **not** replace WASAPI loopback — production capture stays direct on Windows. Full snapshot + setup commands in #85. |

---

## 13. Definition of release-ready

Per `15_roadmap_and_milestones.md` §10 — repeated here for forcing function:

1. M0-M5 demo gates all pass
2. Perf budgets met on reference machine (RTX 3060 + 8-core)
3. CI green 3 consecutive days on `main`
4. Soak 8 h clean
5. Manual test plan signed off (`13_testing_strategy.md` §15)
6. PRD docs internally consistent
7. `cargo deny check` clean
8. No `unsafe` outside `synapse-capture` / `synapse-hid-host` / `firmware/pico-hid`
9. No `unwrap()` outside test code
10. Crash dumps land on intentional panics
