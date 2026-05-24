# 02 — M1: Perception MVP (2-3 weeks) — DONE (archival)

**Status:** Closed 2026-05-23 by release tag `v0.1.0-m1` (commit `b8ad120`).
GitHub context issue: #86. All M1 sub-issues (#87-#135) are closed.

**Shipped MCP tools (6):** `health` (carried from M0), `observe`, `find`,
`read_text`, `set_capture_target`, `set_perception_mode`. Schemas frozen via
`crates/synapse-mcp/tests/snapshots/m1_tools_fsv__m1_tools_list.snap`.

Manual configured-host FSV is the shipping gate (operator decision 2026-05-24,
issues #246/#247). CI runs the portable Linux/Windows test subset as a
regression safety net only; M3+ must keep it green.

**Shipped M1 surface (consume from M2 — do not re-implement):**

| Asset | Path | Used by M2 |
|---|---|---|
| All 80+ error codes (M0-M5) as `pub const` | `crates/synapse-core/src/error_codes.rs` | M2 imports `error_codes::ACTION_*` + `SAFETY_*` directly |
| `Backend`, `PerceptionMode`, `Point`, `Rect`, `Size`, `ElementId` (`<hwnd_hex>:<runtime_id_hex>` composite), `SCHEMA_VERSION = 1` | `crates/synapse-core/src/types.rs` + `defaults.rs` | M2 extends `synapse-core` with `Action` enum + sub-types (`AimCurve`, `AimNaturalParams`, `AimStyle`, `KeystrokeDynamics`, `Key`, `KeyCode`, `MouseButton`, `MouseTarget`, `ButtonAction`, `PadId`, `PadButton`, `Stick`, `Trigger`, `GamepadReport`, `ComboStep`, `ComboInput`, `AimTarget`) per `06 §4` |
| `ElementId::parts() -> ElementIdParts { hwnd, runtime_id_hex }` | `crates/synapse-core/src/types.rs:103` | M2 uses for InvokePattern resolution and HWND-targeting |
| `synapse_a11y::re_resolve(&ElementId)` (Windows) | `crates/synapse-a11y/src/lib.rs:329` | M2 calls before every element-targeted click to refresh UIA pointer |
| `synapse_a11y::focused_window()`, `foreground_context(hwnd)`, `snapshot(root, depth)` | `crates/synapse-a11y/src/lib.rs` | M2 uses to record focused element state for FSV before/after |
| `uiautomation = "0.25.0"` re-exported as `synapse_a11y::UIElement` | `crates/synapse-a11y/src/lib.rs:15` | M2 invokes `IUIAutomationInvokePattern::Invoke` via the same crate's pattern API |
| `synapse_capture::screen_to_window(point, hwnd)` / `window_to_screen(...)` | `crates/synapse-capture/src/lib.rs:449` | M2 uses for element-center coordinate clicks when InvokePattern is unsupported |
| `synapse_test_utils::stdio_mcp_client::StdioMcpClient::launch_and_init_with_env(log_dir, envs)` | `crates/synapse-test-utils/src/stdio_mcp_client.rs:36` | M2 E2E tests reuse for every tool round-trip |
| `synapse_telemetry::init_tracing(TelemetryConfig)` + `TelemetryGuard` | `crates/synapse-telemetry/src/lib.rs:124` | M2 must NOT call init in lib code — `synapse-mcp::main` already wires it |
| Synthetic fixture flag `SYNAPSE_MCP_SYNTHETIC_FIXTURE=notepad` | `crates/synapse-mcp/src/m1/sources.rs:13` (`synthetic_notepad_input`) | M2 adds a `SYNAPSE_MCP_RECORDING_BACKEND=1` flag that swaps `SoftwareBackend` for `RecordingBackend` so E2E asserts the exact INPUT sequence emitted |
| Forced-error flags `SYNAPSE_MCP_FORCE_NO_PERCEPTION`, `SYNAPSE_MCP_FORCE_OBSERVE_INTERNAL` | `crates/synapse-mcp/src/m1.rs:39-49` | M2 adds parallel flags `SYNAPSE_MCP_FORCE_ACTION_*` to drive every error-code path |

The M1 codebase is the contract. **Do not redefine these types or rename these
exports.** Extend `synapse-core::types` and `synapse-action::*` only.

PRD: `15_roadmap_and_milestones.md` §3. Subsystem detail: `02_perception.md`. Schemas: `06_data_schemas.md` §2.

## Goal

`observe()` returns structured JSON for any focused Win32 / Chromium / WinRT app. UIA primary; capture path online but detection model loading is stub-or-real per profile.

## Demo gate

Notepad open with cursor in editor → agent calls `observe()` → reply has `foreground.process_name == "notepad.exe"`, `focused.role == "Edit"`, editor `bbox` populated. Round-trip ≤ 50 ms p99.

---

## Inputs

- M0 demo gate passed
- Win11 dev box w/ DX11-capable GPU (DirectML preferred; CPU ORT fallback acceptable for M1)
- DPI-aware Windows session (Synapse calls `SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)` at startup)

---

## Deliverables

### Crates (fill out)

| Crate | M1 contents |
|---|---|
| `synapse-capture` | Wraps `windows-capture = "2.0.0"`; `CapturedFrame { texture, w, h, format, captured_at, frame_seq, dirty_region }`; DXGI Output Duplication fallback; capture thread @ `THREAD_PRIORITY_TIME_CRITICAL`; bounded channel cap 2, drop-oldest |
| `synapse-a11y` | `uiautomation = "0.25.0"` tree walker; `IUIAutomationCacheRequest` batched property fetch; `SetWinEventHook` on COM apartment thread (events from `02 §3b`); `chromiumoxide = "0.9.1"` CDP client (attaches when foreground process is Chromium + debug port reachable); coordinate transforms `screen_to_window` / `window_to_screen` |
| `synapse-perception` | `Observation` assembler; fuses UIA + capture + (stub) detection + (stub) audio into one `Observation`; perception-mode auto-select per `02 §9` |
| `synapse-models` | Minimum `ort = "2.0.0-rc.12"` loader; YOLOv10n loadable when present in `%LOCALAPPDATA%\synapse\models\`; CUDA → DirectML → CPU EP order per `02 §4`; sha256 verification on load |
| `synapse-core` (extensions) | `Observation`, `ForegroundContext`, `FocusedElement`, `AccessibleNode`, `DetectedEntity`, `HudReadings`, `AudioContext`, `ObservationDiagnostics`, `EventSummary`, `Event`, `EventSource`, `EventFilter` + `DataPredicate` per `06 §1-3` |
| `synapse-mcp` (add tools) | `observe`, `find`, `read_text`, `set_capture_target`, `set_perception_mode` per `05 §3.1-3.4, §3.9-3.10` |

### Models (operator imports; not bundled)

| Model | Purpose | Source |
|---|---|---|
| `yolov10n_general.onnx` | Default detection | downloaded at first need (sha-verified); license-permissive build only — Ultralytics AGPL weights forbidden in bundle (`OQ-025`) |
| WinRT OCR | OS-provided via `Windows.Media.Ocr` | no download |

### Error codes (must throw + test)

```
OBSERVE_NO_PERCEPTION_AVAILABLE
OBSERVE_INTERNAL
CAPTURE_GRAPHICS_API_UNSUPPORTED
CAPTURE_TARGET_LOST
CAPTURE_NO_DIRTY_REGIONS
A11Y_NOT_AVAILABLE
A11Y_ELEMENT_STALE
A11Y_NO_FOREGROUND
A11Y_CDP_UNREACHABLE
DETECTION_MODEL_NOT_LOADED
DETECTION_MODEL_INFER_FAILED
DETECTION_NO_FRAME
OCR_NO_TEXT
OCR_BACKEND_UNAVAILABLE
MODEL_DOWNLOAD_FAILED
MODEL_HASH_MISMATCH
MODEL_LOAD_FAILED
MODEL_BACKEND_UNAVAILABLE
PERCEPTION_MODE_INVALID
CAPTURE_TARGET_INVALID
```

---

## Work-items (PR-sized, ordered)

| # | Title | Acceptance |
|---|---|---|
| 1 | `feat(core): Observation + Event + EventFilter types` | round-trip serde JSON (no binary codec at v1 — `bincode` excluded by RUSTSEC-2025-0141 per ADR-0001); `insta` snapshot of sample observation |
| 2 | `feat(capture): windows-capture wrap + CapturedFrame` | bench: 60 fps capture loop on primary monitor stays ≤ 2% CPU steady-state idle |
| 3 | `feat(capture): DXGI Output Duplication fallback` | env var `SYNAPSE_CAPTURE_FORCE_DXGI=1` selects fallback; same frame shape emitted |
| 4 | `feat(capture): coordinate transforms + DPI awareness` | `screen_to_window(window_to_screen(p, h), h) == p` proptest |
| 5 | `feat(a11y): UIA snapshot with cache request batching` | depth-2 60-element snapshot ≤ 10 ms p99 on focused Notepad |
| 6 | `feat(a11y): WinEvent hook on COM apartment thread` | event stream emits `foreground-changed`, `focus-changed`, `value-changed`, `name-changed`, `element-appeared`, `element-disappeared`, `selection-changed`, `menustart`/`end`, `alert` |
| 7 | `feat(a11y): event filter coalescing + value-change debounce (50ms/200ms per `02 §3 filter`)` | proptest: no two events with same `(window_id, element_id, kind)` within 50ms |
| 8 | `feat(a11y): chromiumoxide CDP attach for Chromium foreground` | attaches when debug port reachable; surfaces `A11Y_CDP_UNREACHABLE` otherwise; never silently falls through |
| 9 | `feat(models): ort loader w/ CUDA/DirectML/CPU EP selection + sha256` | model load surfaces `MODEL_BACKEND_UNAVAILABLE` if no EP works; `MODEL_HASH_MISMATCH` if sha differs |
| 10 | `feat(perception): WinRT OCR wrapper` | `read_text(region)` ≤ 8 ms p99 small region, ≤ 30 ms full screen |
| 11 | `feat(perception): Observation assembler + auto-mode selector` | `observe()` with default `include` returns ≤ 6 KB JSON on Notepad |
| 12 | `feat(mcp): observe + find + read_text + set_capture_target + set_perception_mode` | tools/list shows all 5; tool schemas match `05 §3.1-3.4, §3.9-3.10` |
| 13 | `bench: observe_warm_a11y_only ≤ 10 ms; observe_warm_hybrid ≤ 30 ms` | criterion baseline committed to `bench_results/<sha>/` |
| 14 | `test(e2e): notepad_observe` | spawn Notepad via tests/fixtures, agent observes editor, asserts role + bbox |

---

## Acceptance gates (block M2)

```
✓ observe() Notepad demo passes (≤ 50 ms round-trip including stdio JSON serialize)
✓ tools/list returns the 5 M1 tools with correct schema (insta snapshot)
✓ Bench observe_warm_hybrid p99 ≤ 30 ms (10 §2, 13 §7)
✓ Capture loop steady-state CPU ≤ 2% (10 §3)
✓ UIA snapshot depth-2/60-elem ≤ 10 ms p99 (10 §5)
✓ OCR WinRT small region ≤ 8 ms p99 (10 §2)
✓ All 5 tools serialize `additionalProperties: false` per `05 §1 rule 3`
✓ Error codes from this phase declared as pub const in synapse-core::error_codes
✓ No mocks gating completion — real UIA, real capture, real OCR on a real Notepad
✓ VRAM steady-state ≤ 500 MB with no model loaded; ≤ 1500 MB with YOLOv10n loaded
```

---

## Risks (`15 §9` + extras)

| Risk | Mitigation |
|---|---|
| UIA cross-process COM marshaling slow | `IUIAutomationCacheRequest` batched fetch from work-item 5; fallback to depth-1 if > 25 ms p99 |
| DirectX texture lifetime bugs | `Drop` impl audited; integration test verifies no leak after 10 min 60fps loop |
| `ort` + DirectML install paperwork on clean Win | document MSVC redist prereq in README; CI installs runtime in setup |
| Chromiumoxide debug-port discovery | requires browser launched with `--remote-debugging-port=<N>`; surface `CDP_UNREACHABLE` clearly (`OQ-010`) |
| `RuntimeId` instability across mutations (`OQ-023`) | composite `ElementId = "<hwnd>:<runtime_id_hex>"`; re-resolve on action call; M2 testing decides whether wrapper layer needed |
| Multi-monitor (`OQ-012`) | one monitor active target at a time; `set_capture_target(monitor_index=...)` from agent |

---

## Out of scope at M1 (deferred ≥ M2)

- Action emission (M2)
- Audio loopback + STT (M3)
- HUD profiles (M3 / M4)
- Reflexes (M3)
- Replay log persistence (M3)
- VLM `describe` (M5)
- Hardware HID perception of any kind (n/a)

---

## Definition of Done

Closed 2026-05-23 by `v0.1.0-m1` (commit `b8ad120`). M2 began against `main`
without waiting on a self-hosted runner; manual configured-host FSV is the
shipping evidence (see `00_methodology.md` §5 + issues #246/#247). Open
next: `03_m2_action_mvp.md` (also closed); active phase: `04_m3_reflex_mcp_surface.md`.
