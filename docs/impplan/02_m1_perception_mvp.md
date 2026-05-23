# 02 — M1: Perception MVP (2-3 weeks)

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
| `synapse-capture` | Wraps `windows-capture = "2.0"`; `CapturedFrame { texture, w, h, format, captured_at, frame_seq, dirty_region }`; DXGI Output Duplication fallback; capture thread @ `THREAD_PRIORITY_TIME_CRITICAL`; bounded channel cap 2, drop-oldest |
| `synapse-a11y` | `uiautomation = "0.24"` tree walker; `IUIAutomationCacheRequest` batched property fetch; `SetWinEventHook` on COM apartment thread (events from `02 §3b`); `chromiumoxide = "0.7"` CDP client (attaches when foreground process is Chromium + debug port reachable); coordinate transforms `screen_to_window` / `window_to_screen` |
| `synapse-perception` | `Observation` assembler; fuses UIA + capture + (stub) detection + (stub) audio into one `Observation`; perception-mode auto-select per `02 §9` |
| `synapse-models` | Minimum `ort = "2.0"` loader; YOLOv10n loadable when present in `%LOCALAPPDATA%\synapse\models\`; CUDA → DirectML → CPU EP order per `02 §4`; sha256 verification on load |
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
| 1 | `feat(core): Observation + Event + EventFilter types` | round-trip serde JSON + maintained binary codec; `insta` snapshot of sample observation |
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

M1 closed when demo passes + all acceptance gates green + `git tag v0.1.0-m1` cuts archival build. Open next: `03_m2_action_mvp.md`.
