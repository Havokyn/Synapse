# 03 — M2: Action MVP (2 weeks) — DONE (archival + post-tag carry-over)

**Status:** Closed 2026-05-24 by release tag `v0.1.0-m2` (commit `51836fe`).
GitHub context issue: #136. All M2 sub-issues (#137-#248) closed; shipped
surface verified by 13 M2 FSV tests under `crates/synapse-mcp/tests/m2_*.rs`
and 16 unit/proptest/bench files under `crates/synapse-action/tests/**`.

**Carry-over for M3** (landed bugs / known limitations recorded after the
tag) is the table at the bottom of `docs/impplan/README.md` (#244, #239,
#234, #233, #231, #243, #242, #241, plus the M2 LoC overrun in `emitter.rs`,
`vigem.rs`, `invoke.rs`, `software.rs`, `m2/click.rs`, `m2/press.rs`). M3
either consumes them as-is or fixes them in a one-shot refactor PR before
adding reflex work on top.

> The rest of this file is preserved for onboarding so a fresh agent can see
> how M2 was structured (the M2 file is the template for M3+ phase docs:
> verbatim crate inventory, default-resolution table, FSV contract, manual
> happy-path + edge-case plan, Occam's-razor recap, trigger→outcome map).
> Every claim about the codebase was verified against `main` at tag time. If
> a discrepancy now exists, the **codebase wins** — file a follow-up issue
> with the `phase:m2` + `area:docs` labels and patch this file in the same
> PR.

PRD authority: `docs/computergames/03_action.md` (Action subsystem),
`docs/computergames/05_mcp_tool_surface.md` §3.11-3.19 + §3.26 (MCP tool
schemas), `docs/computergames/06_data_schemas.md` §4 (Action enum + sub-types),
`docs/computergames/10_performance_budget.md` §2 + §12 (latency budgets),
`docs/computergames/15_roadmap_and_milestones.md` §4 (M2 roadmap entry).
Doctrine: `docs/impplan/00_methodology.md` (apply every rule), `07_cross_cutting.md`
§12 (Natural-only motion invariant from OQ-004 DECIDED 2026-05-22).

---

## 0. Mission in one sentence

Fill out the empty `synapse-action` crate so that `synapse-mcp` exposes nine
new MCP tools that, when called from Claude Desktop or any MCP stdio client,
type into real Notepad, save the file with `Ctrl+S`, click real UI elements
via `IUIAutomationInvokePattern::Invoke`, drive a virtual Xbox 360 controller
through ViGEm, and release every held input on shutdown, SIGINT, or panic —
all with `Natural`-by-default motion and keystroke pacing, with no fallbacks,
no mocks gating completion, and full source-of-truth verification on every
test.

---

## 1. Where you are starting from (verified against `main` 2026-05-23)

The repository at `/home/cabdru/synapse` (Linux/WSL dev box; production
target is Win11) has these shipped assets you must consume verbatim. Do not
re-implement, do not rename, do not wrap unless instructed below.

### 1.1 Crate layout

```
crates/
├── synapse-mcp/             stdio MCP daemon; 6 M1 tools live; add 9 M2 tools here
├── synapse-core/            shared types + error codes; extend with Action enum + sub-types
├── synapse-action/          EMPTY STUB — this is the M2 build target
├── synapse-test-utils/      StdioMcpClient; extend with RecordingBackend + Notepad helpers
├── synapse-a11y/            UIA + WinEvent + CDP (re-resolve elements, InvokePattern)
├── synapse-capture/         windows-capture 2.0 + DXGI fallback (screen↔window coords)
├── synapse-perception/      observation assembler (read-only consumer at M2)
├── synapse-models/          ORT session factory (untouched at M2)
├── synapse-telemetry/       tracing JSON + console (init_tracing is called from main only)
├── synapse-storage/         empty stub (M3)
├── synapse-profiles/        empty stub (M3)
├── synapse-reflex/          empty stub (M3)
├── synapse-audio/           empty stub (M3)
├── synapse-hid-host/        empty stub (M4)
└── synapse-overlay/         binary skeleton (M5)
```

Workspace root: `Cargo.toml` at the repo root. `synapse-action`'s
`Cargo.toml` already lists the M2-needed workspace deps (`serde`, `thiserror`,
`tokio`, `tracing`, `synapse-core`). You will add: `enigo`, `vigem-client`,
`crossbeam` (channels), `tokio` features for `mpsc`, `arc-swap` (for panic
hook static), `windows` (Win32 `SendInput` direct path), `mockall` (dev-dep
for backend trait substitution in unit tests). All version pins live in
`Cargo.toml` `[workspace.dependencies]` (already pinned):
`enigo = "0.6.1"`, `vigem-client = "0.1.4"`, `crossbeam = "0.8.4"`,
`arc-swap = "1.9.1"`, `mockall = "0.14.0"`, `windows = "0.62.2"` with the
features listed at workspace root.

### 1.2 Already-declared M2 assets in `synapse-core`

**Do not redefine these.** They are exported from
`crates/synapse-core/src/error_codes.rs` and imported via
`synapse_core::error_codes::*`:

```
ACTION_QUEUE_FULL              ACTION_HID_PORT_DISCONNECTED      STUCK_KEY_AUTO_RELEASED
ACTION_RATE_LIMITED            ACTION_VIGEM_NOT_INSTALLED        SAFETY_RELEASE_ALL_FIRED
ACTION_BACKEND_UNAVAILABLE     ACTION_VIGEM_PLUGIN_FAILED        SAFETY_OPERATOR_HOTKEY_FIRED
ACTION_TARGET_INVALID          ACTION_ELEMENT_NOT_RESOLVED
ACTION_HOLD_EXCEEDED_MAX       ACTION_FOREGROUND_LOST
                               ACTION_UNSUPPORTED_KEY
                               ACTION_DRAG_DISTANCE_EXCEEDS_LIMIT
```

`crates/synapse-core/tests/error_codes_literal.rs` already asserts that every
declared code's `pub const` literal equals its identifier. The M2 work-items
below throw each of these codes at least once; the doc-cross-ref check in
`scripts/check_docs.ps1` is the catalog-completeness gate (no new code can be
thrown without a matching `pub const`).

`Backend`, `PerceptionMode`, `Point`, `Rect`, `Size`, `ElementId` already
exist in `synapse-core::types`. **You extend `synapse-core`** by adding the
new types listed in §3.2 below to the same module — do not create a parallel
module.

### 1.3 Already-wired entry points you will reuse

| Asset | Path | Use |
|---|---|---|
| `SynapseService::new()` + `tool_router` macro | `crates/synapse-mcp/src/server.rs:22` | M2 adds 9 `#[tool(...)]` methods next to `health`, `observe`, `find`, `read_text`, `set_capture_target`, `set_perception_mode` |
| Tool error helper `mcp_error(code, msg) -> ErrorData` | `crates/synapse-mcp/src/m1.rs:369` | M2 calls this with `error_codes::ACTION_*` for every structured error response |
| `init_process_dpi_awareness` | called once from `synapse-mcp::main` (`src/main.rs:52`) | M2 must NOT call this again; rely on the existing PROCESS_PER_MONITOR_AWARE_V2 state |
| `synapse_a11y::re_resolve(&ElementId)` | `crates/synapse-a11y/src/lib.rs:329` | M2 calls before every element-targeted click to refresh the UIA pointer |
| `synapse_a11y::foreground_context(hwnd)` | `crates/synapse-a11y/src/lib.rs:287` | M2 uses to assert foreground match before invoking gated actions |
| `synapse_capture::screen_to_window(point, hwnd)` / `window_to_screen` | `crates/synapse-capture/src/lib.rs:449`+`460` | M2 transforms element bbox center → screen coords for coordinate-fallback clicks |
| `synapse_test_utils::stdio_mcp_client::StdioMcpClient::launch_and_init_with_env(log_dir, &[(K,V)...])` | `crates/synapse-test-utils/src/stdio_mcp_client.rs:36` | every M2 E2E test spawns the daemon through this; never invoke `synapse-mcp` via `Command::new` directly |
| `ElementId::parts() -> ElementIdParts { hwnd, runtime_id_hex }` | `crates/synapse-core/src/types.rs:103` | M2 calls to obtain HWND for foreground assertions and InvokePattern dispatch |

### 1.4 OS reality

- Production target: the operator's configured Windows 11 host (DX11-capable GPU;
  ViGEmBus driver installed and verified working on that host).
- Dev box: WSL2 on Win11. `windows`/`enigo`/`vigem-client` only compile on
  Windows. The Linux build must compile via the `#[cfg(not(windows))]`
  branches of the action backends; those branches return a structured
  `ACTION_BACKEND_UNAVAILABLE` error rather than `unimplemented!()`. M2 shipping
  is not gated on GitHub Actions or CI; manual FSV on the configured host is the
  release evidence. A test that depends on `RecordingBackend` can still run on
  both Windows and Linux as local supporting evidence.
- ViGEmBus is **required** for any test using `vigem-client`. Install via
  `winget install Nefarius.ViGEmBus` on Win11 once, then complete the
  Nefarius installer GUI clickthrough. Current Win11 FSV showed the 1.22.0
  installer fails under `/SILENT`, `/VERYSILENT`, and extraction flags before
  writing a driver install or log, so M2 does not depend on unattended
  ViGEmBus bootstrap. For M2, the gate is that ViGEmBus is present and
  `act_pad` works on the operator's configured host. If absent at runtime,
  `VigemBackend::ensure_ready()` still surfaces `ACTION_VIGEM_NOT_INSTALLED`,
  but proving the absent-driver path on another machine is not an M2 release
  gate.

### 1.5 Things that are NOT done yet (you will not touch any of these at M2)

- Reflex runtime → M3 (`crates/synapse-reflex/src/lib.rs` is empty)
- `act_combo` MCP tool → M3 ships the standalone tool that compiles to a
  `combo` reflex; M2 does NOT ship `act_combo`. The `ComboStep` /
  `ComboInput` enum variants in `synapse-core` exist so M3 can wire them.
- RocksDB action log persistence → M3
- Hardware HID backend (`Backend::Hardware` routing) → M4 (M2 emits
  `ACTION_BACKEND_UNAVAILABLE` for `backend: "hardware"` requests)
- `act_run_shell`, `act_launch` → M4 (gated via `--allow-shell` /
  `--allow-launch`)
- Tool-level permission gating beyond `Backend::Hardware` rejection → M4
- HTTP transport → M3 (`--mode http` still returns `NOT_YET_IMPLEMENTED`
  exit 2 at the end of M2; do not remove this guard)

---

## 2. Demo gate (must pass to close M2)

Real Win11 box, ViGEmBus installed, Notepad open with cursor in the editor.
Claude Desktop is configured with `synapse-mcp` as MCP stdio server. The
operator opens a fresh chat and asks Claude:

> "Open Notepad, type 'Hello world.\\nThis is Synapse.', save the file as
> `m2-demo.txt` on the desktop."

Claude makes ≤ 8 tool calls. The sequence the agent picks does not matter
provided each call returns a non-error structured payload. A typical
sequence:

```
observe()                                       # M1 — locate Notepad editor
act_click({element_id: <editor>})               # M2
act_type({text: "Hello world.\nThis is Synapse."})
act_press({keys: ["ctrl","s"]})
observe()                                       # locate Save dialog
act_type({text: "m2-demo.txt"})
act_press({keys: ["enter"]})
observe()                                       # confirm dialog closed, title updated
```

**Source-of-truth verification (manual, post-demo):** the operator opens
`%USERPROFILE%\Desktop\m2-demo.txt` in any editor and confirms the file
contents byte-for-byte equal `"Hello world.\nThis is Synapse."` (LF or CRLF
per Notepad's "Save As" line-ending option). The daemon's tracing log at
`%LOCALAPPDATA%\synapse\logs\synapse.log.<date>` must contain at least one
`tool.invocation kind=act_type` line and at least one
`tool.invocation kind=act_press` line.

**Failure modes that block the gate:** stuck modifier (`Ctrl` still held
after the chord), file written to the wrong directory, file with extra
keystrokes (auto-complete artifacts), Notepad lost focus mid-`act_type`,
panic hook did not fire `ReleaseAll`, daemon exited 0 with no log lines.

---

## 3. Deliverables (verbatim)

### 3.1 New file tree under `crates/synapse-action`

```
crates/synapse-action/
├── Cargo.toml                  (edit; add deps below)
├── src/
│   ├── lib.rs                  (≤ 200 LoC: re-exports + ActionEmitter::spawn)
│   ├── error.rs                (≤ 250 LoC: ActionError enum + .code())
│   ├── handle.rs               (≤ 120 LoC: ActionHandle + ReleaseAll static)
│   ├── emitter.rs              (≤ 500 LoC: ActionEmitter actor loop, held-state)
│   ├── safety.rs               (≤ 200 LoC: panic hook, SIGINT, shutdown wiring)
│   ├── rate_limit.rs           (≤ 150 LoC: per-backend token bucket)
│   ├── curve.rs                (≤ 500 LoC: AimCurve sampling for all 5 variants)
│   ├── dynamics.rs             (≤ 300 LoC: KeystrokeDynamics sampler)
│   ├── backend/
│   │   ├── mod.rs              (≤ 80 LoC: ActionBackend trait + selection)
│   │   ├── software.rs         (≤ 500 LoC: enigo + windows::SendInput batch)
│   │   ├── vigem.rs            (≤ 400 LoC: vigem-client X360 + DS4)
│   │   ├── recording.rs        (≤ 250 LoC: RecordingBackend for tests)
│   │   └── unavailable.rs      (≤ 120 LoC: stub returning ACTION_BACKEND_UNAVAILABLE for hardware @ M2)
│   └── invoke.rs               (≤ 200 LoC: IUIAutomationInvokePattern bridge to synapse-a11y)
├── benches/
│   ├── action_software_press.rs
│   ├── action_curve_step_calc_natural.rs
│   └── action_recording_round_trip.rs
└── tests/
    ├── action_no_stuck_keys_proptest.rs
    ├── action_release_all_fsv.rs
    ├── action_curve_sampling.rs
    ├── action_dynamics_round_trip.rs
    ├── action_rate_limit.rs
    └── action_held_key_auto_release.rs
```

The 500 LoC cap is hard. If a file would exceed it, split by concern (e.g.,
`emitter.rs` may grow `emit_keyboard.rs` + `emit_mouse.rs` + `emit_pad.rs`).
The 30 LoC function cap and cyclomatic ≤ 10 still apply.

`SYNAPSE_MCP_RECORDING_BACKEND=1` is the M2 test switch for MCP daemon runs.
When set through `StdioMcpClient::launch_and_init_with_env`, every action tool
that emits through a backend uses `RecordingBackend` instead of touching real
keyboard, mouse, or gamepad state; tests read the resulting event sequence from
the `SYNAPSE_LOG_DIR` JSONL side-channel. When unset, `M2State::from_env()`
keeps the normal emitter path and real backend resolution.

### 3.2 New `synapse-core` types (added to `crates/synapse-core/src/types.rs`; re-exported from `lib.rs`)

Every type below derives `Clone, Debug, Eq` or `PartialEq` as appropriate,
plus `serde::{Serialize, Deserialize}` with `#[serde(deny_unknown_fields)]`
on all structs and `#[serde(rename_all = ...)]` matching the variant naming
documented in `06_data_schemas.md` §4 / §4.1. Every enum that appears in an
MCP tool input schema also derives `schemars::JsonSchema`.

```
Action                          (enum tagged "kind", snake_case)
AimCurve                        (enum tagged "kind", snake_case; Natural carries AimNaturalParams)
AimNaturalParams                (struct; has impl AimNaturalParams::FAST associated const)
AimStyle                        (enum snake_case: Snap, Flick, Natural, Track)
KeystrokeDynamics               (enum tagged "kind", snake_case; Natural carries 3 fields)
KeystrokeNaturalParams          (NEW struct; has impl KeystrokeNaturalParams::FAST)
Key                             (struct { code, use_scancode })
KeyCode                         (enum tagged "kind", snake_case)
MouseButton                     (enum lowercase)
ButtonAction                    (enum lowercase: press, down, up)
MouseTarget                     (enum tagged "kind", snake_case: screen, element)
AimTarget                       (enum tagged "kind", snake_case: screen, element, track)
PadId                           (type alias u8 — keep at u8 to match PRD §4.1)
PadButton                       (enum lowercase)
Stick                           (enum lowercase)
Trigger                         (enum lowercase)
GamepadReport                   (struct)
ComboStep                       (struct)
ComboInput                      (enum — full set per PRD; M2 only consumes via type plumbing)
```

`AimNaturalParams::FAST` (associated `const`):

```rust
pub const FAST: Self = Self {
    control_point_jitter: 0.08,
    tremor_stddev_px: 0.2,
    overshoot_prob: 0.25,
    overshoot_factor_range: (1.02, 1.06),
    micro_correct_steps: 1,
    timing_stddev_ms: 1.5,
    seed: None,
};
```

`KeystrokeNaturalParams::FAST`:

```rust
pub const FAST: Self = Self {
    mean_iki_ms: 32.0,
    stddev_ms: 10.0,
    bigram_bias: true,
};
```

Both `FAST` presets are also the resolved defaults of every MCP tool that
takes a `curve` or `dynamics` field (see §5 schema defaults).

### 3.3 New tools in `synapse-mcp` (`crates/synapse-mcp/src/m2.rs` + `m2/`)

Mirror the M1 layout (`src/m1.rs` + `src/m1/`). Module skeleton:

```
crates/synapse-mcp/src/
├── m1.rs                       (existing)
├── m1/                         (existing submodules)
├── m2.rs                       (NEW: tool param/response types + shared helpers)
└── m2/                         (NEW)
    ├── click.rs                (act_click logic)
    ├── type_text.rs            (act_type logic)
    ├── press.rs                (act_press logic)
    ├── aim.rs                  (act_aim logic)
    ├── drag.rs                 (act_drag logic)
    ├── scroll.rs               (act_scroll logic)
    ├── pad.rs                  (act_pad logic)
    ├── clipboard.rs            (act_clipboard logic)
    └── release_all.rs          (release_all logic)
```

Each submodule exposes one `fn <verb>_in_state(state: &mut M2State, params: <Params>) -> Result<<Resp>, ErrorData>`. `server.rs` exports nine new `#[tool(...)]` methods that call those functions through a `SharedM2State = Arc<Mutex<M2State>>` field added to `SynapseService` alongside the existing `m1_state`.

The nine tools are exactly:

```
act_click          act_type           act_press
act_aim            act_drag           act_scroll
act_pad            act_clipboard      release_all
```

Schemas: §5 below. Acceptance bullet: §7 work-item 12.

### 3.4 Channel + lifetime invariants (hard contract)

- Action mpsc bounded capacity **256**. Saturation ⇒ tool returns
  `ACTION_QUEUE_FULL` immediately (no block).
- Per-backend rate cap (token bucket, per `03_action.md §15`):
  software 5000 events/s, ViGEm 1000 reports/s. Excess ⇒ tool returns
  `ACTION_RATE_LIMITED`. The token-bucket refill MUST use
  `tokio::time::Instant` (monotonic), never `chrono::Utc::now()`.
- `held_key_max_duration_ms = 30_000`. Every `KeyDown` enqueues a
  cancellable timer; on fire the emitter sends `KeyUp` for that key and
  emits a `STUCK_KEY_AUTO_RELEASED` event through the tracing log (real
  event bus comes online in M3 — M2 logs only).
- Panic hook stored in `static RELEASE_ALL_HANDLE: OnceLock<ActionHandle>`
  (use `std::sync::OnceLock`, not `OnceCell` from `once_cell` —
  `OnceLock` is in stable std as of Rust 1.70). On panic the hook calls
  `handle.fire_release_all_blocking_with_timeout(Duration::from_millis(10))`
  before re-panicking. Timeout exceeded ⇒ log
  `SAFETY_RELEASE_ALL_FIRED code=timeout` and continue (best-effort, never
  swallow the panic).
- SIGINT (and SIGTERM via `tokio::signal::ctrl_c`) wired in
  `synapse-mcp::main` already cancels the rmcp service; M2 wires the
  cancellation token through to `ActionEmitter::run` which calls
  `release_all()` before returning.

### 3.5 Tracing instrumentation

Every public `fn` in `synapse-action` carries `#[tracing::instrument(skip_all, fields(...))]` with the fields the M1 tests already inspect (search the repo for `code = "MCP_TOOL_INVOCATION"` and follow the convention). Every error-code emit also logs a `tracing::warn!(code = "<CODE>", "...")` line. CI greps the test stdout for these codes.

---

## 4. Action enum shape (canonical)

Match `06_data_schemas.md` §4 byte-for-byte (the PRD wins on any conflict).
Repeated here so this doc is self-contained:

```rust
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    KeyPress  { key: Key, hold_ms: u32, backend: Backend },
    KeyDown   { key: Key, backend: Backend },
    KeyUp     { key: Key, backend: Backend },
    KeyChord  { keys: Vec<Key>, hold_ms: u32, backend: Backend },
    TypeText  { text: String, dynamics: KeystrokeDynamics, backend: Backend },

    MouseMove { to: MouseTarget, curve: AimCurve, duration_ms: u32, backend: Backend },
    MouseMoveRelative { dx: f32, dy: f32, backend: Backend },
    MouseButton { button: MouseButton, action: ButtonAction, hold_ms: u32, backend: Backend },
    MouseDrag { from: Point, to: Point, button: MouseButton, curve: AimCurve, duration_ms: u32, backend: Backend },
    MouseScroll { dy: i32, dx: i32, at: Option<Point>, backend: Backend },

    PadButton  { pad: PadId, button: PadButton, action: ButtonAction, hold_ms: u32 },
    PadStick   { pad: PadId, stick: Stick, x: f32, y: f32 },
    PadTrigger { pad: PadId, trigger: Trigger, value: f32 },
    PadReport  { pad: PadId, report: GamepadReport },

    AimAt    { target: AimTarget, style: AimStyle, deadline_ms: u32, backend: Backend },
    Combo    { steps: Vec<ComboStep>, backend: Backend },   // M3-fires; M2 only carries the type

    ReleaseAll,
}
```

`hold_ms`, `duration_ms`, `deadline_ms`, `hold_ms`: integer milliseconds — the
PRD uses `Duration` in prose but the JSON tool schema uses integer
milliseconds (see `05 §3.13 act_press.hold_ms`). The Rust struct stores
`u32` ms; converters to `std::time::Duration` live inside `synapse-action`
internal helpers only.

---

## 5. MCP tool schemas (default-resolution table — every test asserts these)

These defaults come from `07_cross_cutting.md §12` (the Natural-only invariant)
and `05_mcp_tool_surface.md §3.11-3.19, §3.26`. Wherever the PRD says
"default: `EaseInOut`" or "default: `burst`" the OQ-004-DECIDED defaults below
take precedence — the PRD's `act_type` block still names `burst` as the
default and **that is stale**; the M2 schema MUST resolve `dynamics` to
`"natural"`, and the canonical PRD doc-fix lands in this PR (one-line patch
to `05 §3.12`).

| Tool | Field | Default | Source |
|---|---|---|---|
| `act_click` | `curve` | `"natural"` | OQ-004 |
| `act_click` | `duration_ms` | `50` | `07 §12` Snap-50ms |
| `act_click` | `button` | `"left"` | PRD `05 §3.11` |
| `act_click` | `clicks` | `1` | PRD |
| `act_click` | `use_invoke_pattern` | `true` | PRD |
| `act_click` | `backend` | `"auto"` | PRD |
| `act_type` | `dynamics` | `"natural"` | OQ-004 (PRD doc-fix patches `05 §3.12` from `"burst"` to `"natural"`) |
| `act_type` | `backend` | `"auto"` | PRD |
| `act_type` | `press_enter_after` | `false` | PRD |
| `act_type` | `use_scancodes` | `false` | PRD |
| `act_press` | `hold_ms` | `33` | PRD `05 §3.13` |
| `act_press` | `backend` | `"auto"` | PRD |
| `act_aim` | `style` | `"snap"` | PRD `05 §3.14`; `Snap` compiles to `Natural` curve, 50 ms |
| `act_aim` | `deadline_ms` | `80` | PRD |
| `act_drag` | `curve` | `"natural"` | OQ-004 |
| `act_drag` | `duration_ms` | `200` | PRD `05 §3.15` |
| `act_drag` | `button` | `"left"` | PRD |
| `act_scroll` | `dy`/`dx` | `0` | PRD `05 §3.16` |
| `act_scroll` | `smooth` | `false` | PRD |
| `act_pad` | `pad_id` | `0` | PRD `05 §3.17` |
| `act_pad` | `backend` | `"vigem"` | PRD |
| `act_clipboard` | `verb` | required | PRD `05 §3.19` |
| `act_clipboard` | `format` | `"unicode"` | PRD |
| `release_all` | (none) | `additionalProperties:false` empty object | PRD `05 §3.26` |

**Style-to-curve compilation table** (every `AimStyle` resolves to a
`Natural` curve except `Track`, which registers a reflex in M3):

| Style | Curve | Total ms (default) |
|---|---|---|
| `snap` | `Natural::FAST` | 50 |
| `flick` | `Natural::FAST` (over-shoot bias up) | 35 |
| `natural` (explicit slower) | `Natural` w/ slower preset | 100-200 |
| `track` | reflex registration (M3); M2 returns `ACTION_BACKEND_UNAVAILABLE` |

All schemas serialize with `additionalProperties: false` and the `insta`
snapshot at `tests/snapshots/m2_tools_list.snap` enforces every field above.

---

## 6. Error codes (must throw + test — verified `pub const` declarations)

Every code below already has a `pub const NAME: &str = "NAME"` in
`crates/synapse-core/src/error_codes.rs` (lines 24-38, 93-97). The
M2 work-items throw each at least once and a test asserts the structured
error response carries `data.code == "<NAME>"` (the M1 helper
`mcp_error(code, msg)` in `src/m1.rs:369` produces the
`{"code":-32099,"message":..,"data":{"code":"<NAME>"}}` shape — re-use it
verbatim).

```
ACTION_QUEUE_FULL                    ACTION_HID_PORT_DISCONNECTED
ACTION_RATE_LIMITED                  ACTION_VIGEM_NOT_INSTALLED
ACTION_BACKEND_UNAVAILABLE           ACTION_VIGEM_PLUGIN_FAILED
ACTION_TARGET_INVALID                ACTION_ELEMENT_NOT_RESOLVED
ACTION_HOLD_EXCEEDED_MAX             ACTION_FOREGROUND_LOST
ACTION_DRAG_DISTANCE_EXCEEDS_LIMIT   ACTION_UNSUPPORTED_KEY
STUCK_KEY_AUTO_RELEASED              SAFETY_RELEASE_ALL_FIRED
                                     SAFETY_OPERATOR_HOTKEY_FIRED  (M3 wires hotkey; M2 only emits via panic path)
```

Mapping (which throw site corresponds to which code) — every work-item lists
the code(s) it must produce.

---

## 7. Work-items (PR-sized, ordered; each is one merge)

| # | Title | Throws codes | Acceptance (FSV-mandatory) |
|---|---|---|---|
| 1 | `feat(core): Action enum + every sub-type from 06 §4 (no Duration; ms u32)` | — | `serde_json::from_value(serde_json::to_value(action)?)? == action` for every variant via `proptest` (1000 cases each); `insta::assert_json_snapshot!("action_variants")` of every variant once; **FSV**: print each variant's JSON before/after round-trip, assert byte-equal |
| 2 | `feat(action): error.rs + ActionError + .code() table` | every M2 code | `crates/synapse-action/tests/error_codes_match.rs` asserts each enum variant's `.code()` equals the matching `error_codes::*` constant (read via `synapse_core::error_codes::ACTION_QUEUE_FULL` etc.) |
| 3 | `feat(action): ActionEmitter mpsc actor + held-state tracking (BitSet for keys/buttons, HashMap<PadId, GamepadReport> for pads)` | `ACTION_QUEUE_FULL` | proptest: 1000 randomized `Action` streams ending in `Action::ReleaseAll` produce empty `held_keys`/`held_buttons` BitSets (use a `tokio::sync::oneshot` to read the final BitSet snapshot after `ReleaseAll`); **FSV**: each proptest run prints the final BitSet contents |
| 4 | `feat(action): SoftwareBackend via enigo + direct windows::SendInput batched on Win` | `ACTION_BACKEND_UNAVAILABLE` (Linux), `ACTION_UNSUPPORTED_KEY` (unknown KeyCode) | bench `action_software_press` p99 ≤ 1 ms on Win (criterion file at `benches/action_software_press.rs`; local `critcmp` baseline export outside git); **FSV**: on Win11 test box, install a low-level `WH_KEYBOARD_LL` hook in the test harness, fire `act_press({keys:["a"]})`, assert the hook observed exactly `[KeyDown(a), KeyUp(a)]` within 33 ms (default hold) |
| 5 | `feat(action): RecordingBackend (synapse-test-utils dev-dep) capturing the exact INPUT stream` | — | unit test: every `Action` variant routed to `RecordingBackend::execute` produces a deterministic `Vec<RecordedInput>`; `insta` snapshot of the recorded sequence for each of the 11 variants; **FSV**: assert the `Vec<RecordedInput>` length and per-step fields against a hard-coded expected sequence, not a fuzzy match |
| 6 | `feat(action): curve.rs — Instant/Linear/EaseInOut/Bezier/Natural sampler with seeded RNG` | — | proptest: `samples[0] == start` and `samples[n-1] == end` for every curve over 1000 random `(start, end, duration_ms)` triples; bench `aim_curve_step_calc_natural` ≤ 1 µs/step (criterion); **FSV**: pin seed `42`, generate `Natural::FAST` samples for `start=(0,0) end=(100,100) duration_ms=50`, assert byte-equal hash to baseline committed in `tests/snapshots/curve_natural_seed_42.snap` |
| 7 | `feat(action): dynamics.rs — Burst/Linear/Natural sampler` | — | proptest: characters typed via `RecordingBackend` round-trip equal to input across 10k random strings up to 200 chars; modifier ordering invariant (`Shift` down before key down, up after key up); **FSV**: pinned-seed `Natural::FAST` sample of `"Hello world."` matches the baseline IKI sequence in `tests/snapshots/dynamics_natural_hello_world.snap` |
| 8 | `feat(action): InvokePattern bridge (invoke.rs) → synapse_a11y::re_resolve + uiautomation::Pattern::Invoke` | `ACTION_ELEMENT_NOT_RESOLVED`, `ACTION_TARGET_INVALID` | Win-only E2E: launch real Notepad via `tests/fixtures/launch_notepad.rs`, call `act_click({element_id: <File menu>})` with `use_invoke_pattern: true`; **FSV** read back via `synapse_a11y::snapshot(focused_window, 2)` and assert the `File` menu's `is_expanded` UIA property flipped to `true` within 25 ms; assert no cursor motion (the M1 capture path's cursor coords stayed put). 3 edge cases: (a) `element_id` for a destroyed window → `ACTION_ELEMENT_NOT_RESOLVED`; (b) `element_id` for an element without InvokePattern → falls through to coordinate click on element bbox center; (c) `element_id` malformed string → `ACTION_TARGET_INVALID` |
| 9 | `feat(action): VigemBackend via vigem-client (X360 + DS4); lazy plug-in + wait_for_ready` | `ACTION_VIGEM_NOT_INSTALLED`, `ACTION_VIGEM_PLUGIN_FAILED` | Win-only E2E with ViGEmBus: enumerate `vigem-client`'s `Client::new`, send `Action::PadReport { pad: 0, report: GamepadReport { buttons: vec![PadButton::A], ..neutral } }`, **FSV** open the test harness's `XInputGetState(0, ..)` and assert `wButtons & XINPUT_GAMEPAD_A == XINPUT_GAMEPAD_A` within 50 ms; release, assert `wButtons == 0`. ViGEm absent ⇒ test calls `VigemBackend::ensure_ready()` and asserts `ACTION_VIGEM_NOT_INSTALLED` |
| 10 | `feat(action): rate_limit.rs token bucket per backend; overshoot ⇒ ACTION_RATE_LIMITED + re-queue with 1 ms backoff` | `ACTION_RATE_LIMITED` | unit test (mocked clock via `tokio::time::pause`): submit 5100 software events in 1 simulated second; assert exactly 5000 succeed, ≥ 100 surface `ACTION_RATE_LIMITED`; **FSV**: print before/after bucket state; assert refill matches `5000/s` token rate |
| 11 | `feat(action): held-key auto-release timer ⇒ STUCK_KEY_AUTO_RELEASED event` | `STUCK_KEY_AUTO_RELEASED`, `ACTION_HOLD_EXCEEDED_MAX` | unit test with `tokio::time::pause + advance(31s)`: send `KeyDown a` then advance time; **FSV** read `RecordingBackend::events()` and assert it contains `KeyUp(a)` and the tracing log captured by `tracing-subscriber::fmt::TestWriter` contains a line `code=STUCK_KEY_AUTO_RELEASED key=a held_ms=30000`. Caller-requested `hold_ms > 30_000` is rejected at the API boundary with `ACTION_HOLD_EXCEEDED_MAX` (no enqueue) |
| 12 | `feat(action): ReleaseAll wiring — shutdown + SIGINT + panic hook` | `SAFETY_RELEASE_ALL_FIRED` | Win-only E2E: spawn `synapse-mcp` via `StdioMcpClient`, call `act_press({keys:["a"], hold_ms: 5000})`, immediately send SIGINT; **FSV** install a low-level keyboard hook in the test harness BEFORE spawning the daemon, assert the hook observes `KeyUp(a)` within 10 ms of the SIGINT being delivered; assert the daemon process exits with code `0` and the last log line is `code=SAFETY_RELEASE_ALL_FIRED reason=sigint`. 3 edge cases: (a) graceful Ctrl+C → ReleaseAll fires; (b) `std::panic!("force")` from a tool handler → panic hook fires ReleaseAll before re-panic; (c) parent stdin closes (rmcp `connection closed`) → ReleaseAll fires |
| 13 | `feat(action): MouseDrag (down + curve + up), MouseScroll (`MOUSEEVENTF_WHEEL`/`HWHEEL`), double/triple-click via `GetDoubleClickTime`` | `ACTION_DRAG_DISTANCE_EXCEEDS_LIMIT` (drag distance > 4096 px) | Win-only E2E: launch Notepad, type `"abcdef"`, call `act_click({clicks: 2, target: <editor>})` → **FSV** read `synapse_a11y::focused_element().value()` (or fall back to clipboard via `act_clipboard(verb:"read")` after `Ctrl+C`) and assert the selection equals the word containing the click point. Scroll: open a long text file, `act_scroll({dy: -10})`, **FSV** observe the editor's `Scroll` pattern current position changed via UIA |
| 14 | `feat(mcp): act_click / act_type / act_press / act_aim / act_drag / act_scroll / act_pad / act_clipboard / release_all` with `Natural`-by-default schemas | every M2 code via tool routes | `tests/m2_tools_fsv.rs` enumerates `tools/list`, asserts the names are exactly `["act_aim","act_click","act_clipboard","act_drag","act_pad","act_press","act_scroll","act_type","find","health","observe","read_text","release_all","set_capture_target","set_perception_mode"]` (15 names sorted), asserts every schema sets `additionalProperties:false`, and asserts every default in §5's table by reading the `default` JSON-Schema field directly. `insta` snapshot of every tool schema. **FSV** for each tool: spawn daemon, call tool with `RecordingBackend` env var, assert the resulting `Vec<RecordedInput>` matches a hard-coded expectation for that tool |
| 15 | `bench: action_software_press, action_curve_step_calc_natural, action_recording_round_trip` | — | `criterion` runs all three; local `critcmp` exports are stored outside git; delta gate per `07 §1` |
| 16 | `test(e2e): notepad_type_save` (the demo gate scenario, automated) | — | spawn Notepad via `tests/fixtures/launch_notepad.rs`, run the full sequence in §2 against the real OS; **FSV** read the resulting file from disk and `assert_eq!(fs::read_to_string(path)?, "Hello world.\nThis is Synapse.")`; **3 edge cases**: (a) save to non-existent directory → `act_type` succeeds but Notepad's save dialog reports the error; the test asserts the dialog text via UIA — the M2 test passes when the agent's last call returned a non-error response AND the source-of-truth file was either written or absent (no half-saved file); (b) Notepad loses focus mid-type → next `act_type` returns `ACTION_FOREGROUND_LOST`; (c) malformed `element_id` for the editor → `ACTION_TARGET_INVALID` |
| 17 | `docs(prd): one-line patch to 05_mcp_tool_surface.md §3.12 act_type.dynamics default from "burst" to "natural"` | — | grep gate in `scripts/check_docs.ps1` (add): `act_type` schema in PRD has `"default": "natural"`; cross-PR ADR not needed because OQ-004 already records the decision |

Total: 17 PRs. Order matters: 1 → 2 → 3 (lays the channel + actor) → 4-9
(backends + curves + invoke + vigem) → 10-13 (rate limit + safety + drag) →
14 (MCP wiring) → 15 (benches) → 16 (E2E) → 17 (PRD doc-fix).

---

## 8. Full-State Verification — the M2 contract (mandatory for every test)

Every test under `crates/synapse-action/tests/**` and
`crates/synapse-mcp/tests/m2_*.rs` follows this template exactly. A test that
does not follow it fails review.

### 8.1 Identify the source of truth

For every action the test takes, name the external system that holds the
post-state and read it back. The full M2 source-of-truth table:

| Action under test | Source of truth | How to read it |
|---|---|---|
| `act_type` text into Notepad | UIA `ValuePattern.value` of the editor element | `synapse_a11y::focused_element()?.get_pattern::<ValuePattern>()?.get_value()?` |
| `act_press(["ctrl","s"])` in Notepad | UIA window title changes; if `Save As` dialog appears, dialog's `Window` element from a fresh `synapse_a11y::focused_window()` snapshot | full UIA snapshot, depth-2 |
| File saved via Notepad | `std::fs::read_to_string(path)` | direct disk read |
| `act_click(element_id=<File menu>)` Invoke | UIA `ExpandCollapsePattern.expand_state` on the menu element | UIA snapshot |
| `act_click(x,y)` coordinate click | `RecordingBackend::events()` → exactly one `MouseButton{down}` + `MouseButton{up}` at the resolved screen point | the dev-dep recording backend |
| `act_pad(buttons=[A])` ViGEm | `XInputGetState(0)` returns `wButtons & XINPUT_GAMEPAD_A != 0` | `windows` crate `XInput_1_4` import in the test harness |
| `act_clipboard(verb:"write", text:"foo")` | `GetClipboardData(CF_UNICODETEXT)` | the test harness opens the clipboard via the `clipboard-win = "5.0"` test-only dep |
| `act_clipboard(verb:"read")` | structured response contains the text the harness pre-loaded into the clipboard | direct value compare |
| `release_all` | `RecordingBackend::held_keys()` empty; `RecordingBackend::held_buttons()` empty; ViGEm pad reports neutral; daemon log contains `code=SAFETY_RELEASE_ALL_FIRED` | three reads, all required |
| Stuck-key auto-release | tracing-subscriber `TestWriter` log captures `code=STUCK_KEY_AUTO_RELEASED` | the test harness installs a `tracing_subscriber::fmt::TestWriter` before spawning the in-process emitter |
| Rate-limit overshoot | structured error response carries `data.code == "ACTION_RATE_LIMITED"` AND token-bucket counter went to zero AND refilled at the expected rate | counter is `pub(crate)` for tests behind `#[cfg(test)]` |
| `act_aim(style:"track")` at M2 | structured error `ACTION_BACKEND_UNAVAILABLE` (no reflex runtime yet) | the structured error response |

### 8.2 The required print pattern

Every test emits at least four log lines per scenario. Pattern (from
existing M1 tests in `tests/m1_tools_fsv.rs`):

```
println!("source_of_truth=<name> edge=<edge> before=<state>");
let resp = client.tools_call("act_type", json!({...})).await?;
println!("source_of_truth=<name> edge=<edge> after_response={}", resp["structuredContent"]);
let truth_after = read_truth(...)?;       // <- THE SEPARATE READ
println!("source_of_truth=<name> edge=<edge> after_truth={truth_after}");
assert_eq!(truth_after, expected);
```

The `after_truth` line is the line a reviewer greps for. Tests without it
fail review.

### 8.3 Boundary & edge-case audit

For every work-item that includes a primary path, exercise **at least three
edge cases** with the same before/after read-back pattern:

1. **Empty input**: `act_type({text: ""})` → returns OK with zero
   `RecordedInput` events; the source-of-truth UIA value did not change.
2. **Maximum boundary**: `act_press({keys: [...], hold_ms: 30000})` →
   succeeds; `hold_ms: 30001` returns `ACTION_HOLD_EXCEEDED_MAX`.
3. **Structurally invalid**: `act_click({target: {x: "not-int", y: 0}})` →
   returns `TOOL_PARAMS_INVALID` (the existing M1 framework already maps
   schemars-rejected params to this code at
   `crates/synapse-mcp/src/server.rs:163`).

The same three categories apply to every M2 tool. The default-resolution
table in §5 also gets exercised: a call with no `curve`/`dynamics` field
present resolves to `Natural::FAST` — the test reads back the recorded
backend events and asserts the per-step IKI or sub-pixel tremor matches the
`Natural::FAST` pinned-seed baseline.

The panic edge uses the debug-only `SYNAPSE_MCP_FORCE_PANIC_DURING_ACT=1`
test gate. In debug builds, `synapse-mcp` panics during `act_press` after the
tool invocation is accepted; in release builds the env var is ignored. The FSV
must read the daemon JSONL log after the trigger and prove
`SAFETY_RELEASE_ALL_FIRED reason=panic` was emitted by the installed panic
hook.

### 8.4 Evidence of success

Every test ends with a log line that prints the literal post-state of the
source of truth:

```
println!("source_of_truth=<name> edge=happy_path final_value={truth_after:?}");
```

A `cargo test --workspace -- --nocapture` run on the green build prints
≥ 1 such line per test. CI greps for the `final_value=` pattern in the
windows-runner stdout; tests that produce zero of these lines fail review
even if `assert_*` passed.

### 8.5 Trigger-and-outcome reasoning

For every test, document the trigger → outcome chain in a doc-comment on
the test fn:

```rust
/// Trigger: caller invokes `act_type({text:"hi"})` against Notepad.
/// X (process): SoftwareBackend issues SendInput with two KEYEVENTF_UNICODE
///   pairs (down+up for 'h', down+up for 'i') interleaved per Natural::FAST
///   IKI sampler.
/// Y (outcome, observable): Notepad's editor UIA `ValuePattern.value`
///   transitions from "" to "hi"; the daemon's tracing log contains
///   `code=MCP_TOOL_INVOCATION kind=act_type` once.
/// Source of truth: UIA ValuePattern.value of the focused Edit element.
```

The trigger event (the tool call), the process (X), and the observable
outcome (Y) are explicit. A reviewer can audit X by reading the
`RecordingBackend` events; Y by reading UIA value.

---

## 9. Manual happy-path + edge-case test plan (run on real Win11 box before tagging `v0.1.0-m2`)

Operator runs these by hand on the configured Win11 host with ViGEmBus installed.
The results table lives in the GitHub issue comments for the M2 tag gate.

### Happy paths

| # | Steps | Source of truth | Expected outcome |
|---|---|---|---|
| H1 | Open Notepad. `act_click(element_id=<editor>)`. | UIA `focused_element().role` after | `"Edit"` |
| H2 | After H1: `act_type({text: "Hello world.\nThis is Synapse."})`. | UIA `focused_element().value` | exactly `"Hello world.\nThis is Synapse."` |
| H3 | After H2: `act_press({keys:["ctrl","s"]})`. | UIA `focused_window().title` | `"Save As"` dialog title |
| H4 | After H3: `act_type({text: "m2-demo.txt"})` then `act_press({keys:["enter"]})`. | `fs::read_to_string(%USERPROFILE%\Desktop\m2-demo.txt)` | exactly `"Hello world.\nThis is Synapse."` (CRLF on Win) |
| H5 | Open Calculator (`calc.exe`). `act_click(element_id=<7>)` then `act_click(element_id=<+>)` then `act_click(element_id=<3>)` then `act_click(element_id=<=>)`. | UIA `focused_window()` text of the result display | `"10"` |
| H6 | With ViGEmBus + a controller-test app showing pad state: `act_pad({pad_id:0, report:{buttons:["a"]}, hold_ms:500})`. | the controller-test app's A-button indicator | lights for ~500 ms |
| H7 | `act_clipboard({verb:"write", text:"synapse-m2"})` then `act_clipboard({verb:"read"})`. | response `text` field + clipboard contents via `Get-Clipboard` PowerShell | both equal `"synapse-m2"` |
| H8 | `act_aim({target:{x:200,y:200}, style:"snap"})`. | cursor position via `GetCursorPos` | within ±1 px of `(200, 200)` after ≤ 60 ms |
| H9 | `act_drag({from:{x:100,y:100}, to:{x:300,y:300}, button:"left"})`. | cursor position + screen capture of any drag-aware app (e.g., Paint with a stroke) | cursor ends at `(300, 300)`; stroke visible from start to end |
| H10 | `release_all()` after any of H1-H9. | `GetAsyncKeyState` for every modifier (Ctrl, Shift, Alt, Win, mouse buttons) | all return non-pressed bits |

### Edge cases

| # | Steps | Source of truth | Expected outcome |
|---|---|---|---|
| E1 | `act_type({text:""})`. | UIA value before+after | unchanged; structured response `chars_typed: 0` |
| E2 | `act_press({keys:["ctrl","shift","alt","f12"], hold_ms:30000})`. | the daemon log + held_keys via in-process emitter snapshot | succeeds; at t=30s a `STUCK_KEY_AUTO_RELEASED` log line appears IF the caller never released; if the chord completes within the hold, no auto-release |
| E3 | `act_press({keys:["ctrl"], hold_ms:30001})`. | structured error response | `data.code == "ACTION_HOLD_EXCEEDED_MAX"` |
| E4 | Notepad open. Click the Notepad title bar to grab focus, then quickly Alt-Tab to another app, then call `act_type({text:"x"})` while the other app is foreground. | structured error response | `data.code == "ACTION_FOREGROUND_LOST"` (the daemon asserts the expected foreground via `synapse_a11y::foreground_context(hwnd_before) == foreground_context_now`); no keystroke recorded |
| E5 | `act_click({target:{x:0,y:0}, clicks:3})` over the empty desktop. | `RecordingBackend::events()` | exactly 3 `MouseButton{down}` + 3 `MouseButton{up}` events within `3 * GetDoubleClickTime` ms |
| E6 | Confirm ViGEmBus is installed and `act_pad({pad_id:0, report:{buttons:["a"]}, hold_ms:500})` reaches the virtual controller path. | Windows driver/device readback + XInput/controller state + daemon log | driver/device present before and after; `act_pad` succeeds with `backend_used:"vigem"`; no stuck pad state after `release_all()` |
| E7 | `act_pad({pad_id:0, report:{thumb_l:[1.5,0]}})` (out-of-range stick). | structured error | `data.code == "TOOL_PARAMS_INVALID"` (schemars rejects the >1.0 value) |
| E8 | `act_aim({target:{track_id:42}, style:"track"})`. | structured error | `data.code == "ACTION_BACKEND_UNAVAILABLE"` with detail mentioning "reflex runtime lands at M3" |
| E9 | Spam `act_press({keys:["a"]})` 6000× in a tight loop within ≤ 1 s. | response codes | at least one response carries `data.code == "ACTION_RATE_LIMITED"`; eventual completion (re-queue + backoff) |
| E10 | Send SIGINT (`Ctrl+C`) to the daemon while a long `act_type` is mid-flight. | low-level keyboard hook in the test harness | `KeyUp` for every previously-down key within 10 ms; exit code 0 |
| E11 | Kill the parent (rmcp `connection closed`) while a key is held. | external low-level keyboard hook | `KeyUp` observed within 10 ms; daemon exits 0 |
| E12 | Force `std::panic!` via `SYNAPSE_MCP_FORCE_PANIC_DURING_ACT=1` mid-`act_press`. | external low-level keyboard hook + last log line | `KeyUp` observed within 10 ms; last log line `code=SAFETY_RELEASE_ALL_FIRED reason=panic` |

For each row the operator pastes both the structured response and the
source-of-truth read-back into the PR description. **No row is "ok by
inspection." Every row prints the actual post-state.**

---

## 10. Synthetic-input fixtures (what to type and what should come out)

These power the unit tests; pick byte sequences whose expected outcomes are
unambiguous.

| Synthetic input | Tool call | Expected UIA value after | Expected `RecordingBackend` event count |
|---|---|---|---|
| `""` | `act_type` | unchanged | 0 |
| `"a"` | `act_type` | `"a"` (prepended to existing) | 2 (down + up) |
| `"AB"` | `act_type` | `"AB"` | 6 (shift down, A down, A up, shift up, B-down sequence depends on shift state — record exactly) |
| `"Hello"` | `act_type` | `"Hello"` | 10 (5 chars × 2 events; shift handling for `H` capital) |
| `"\n"` | `act_type` | one `\n` appended | 2 (Enter down + up) |
| `"€"` | `act_type` | `"€"` (via `KEYEVENTF_UNICODE`) | 2 |
| 256-byte ASCII | `act_type` | string appended | 512 |
| `keys=["a"], hold_ms=1` | `act_press` | one `"a"` typed | 2 |
| `keys=["ctrl","s"], hold_ms=33` | `act_press` | Save dialog appears | 4 (ctrl-down, s-down, s-up, ctrl-up) |
| `keys=["ctrl","shift","alt","super","f12"], hold_ms=50` | `act_press` | chord released cleanly | 10 |
| `clicks=2, target:{x:300,y:300}` | `act_click` | two click pairs within ≤ `GetDoubleClickTime` | 4 (down, up, down, up) |
| `dy:-3` | `act_scroll` | scroll up by 3 wheel ticks | 1 (`MOUSEEVENTF_WHEEL` event) |
| `report:{buttons:["a","b"],thumb_l:[0.0,0.0],thumb_r:[0.0,0.0],lt:0.0,rt:0.0}` | `act_pad` | pad state has A+B held | 1 ViGEm report |
| `release_all` | `release_all` | all BitSets empty | n+m (one Up per held key/button) |

The tests use these tables as the contract for `RecordingBackend` event
count and order. Mismatch ⇒ fail with the actual recorded sequence printed.

---

## 11. Acceptance gates (block M3)

```
✓ Notepad type-save demo (§2) passes via stdio MCP on a real Win11 box
✓ Manual H1-H10 happy paths all green; operator pastes source-of-truth read-back in PR
✓ Manual E1-E12 edge cases all match expected outcome
✓ bench action_software_press p99 ≤ 3 ms (10 §12)
✓ bench aim_curve_step_calc_natural ≤ 1 µs/step (07 §1)
✓ act_click(element_id) semantic invoke p99 ≤ 25 ms (10 §12)
✓ act_click(x,y) Natural::FAST 50ms p99 ≤ 60 ms (10 §12)
✓ act_type("Hello world.") Natural::FAST ≤ 400 ms total wall (12 chars × ~32 ms ± stddev)
✓ Default-resolution test: tools/list snapshot shows every default per §5 table
✓ ReleaseAll fires within 10 ms on Ctrl+C / SIGINT / panic (E10, E11, E12)
✓ proptest no_stuck_keys passes 1000 cases (work-item 3)
✓ No mocks gate completion — work-items 4, 8, 9, 12, 13, 16 use real SendInput / real UIA / real ViGEm / real disk
✓ ViGEm pad updates round-trip via XInputGetState (work-item 9 + H6)
✓ All 9 M2 tools schema-snapshotted (work-item 14)
✓ All M2 error codes throwable + tested (work-item 2 + per-tool tests)
✓ Local supporting checks run where useful; GitHub Actions/CI are not M2 shipping gates
✓ FSV evidence: every passing test stdout contains ≥ 1 `final_value=` line per scenario
✓ scripts/check_docs.ps1 green (after work-item 17 PRD doc-fix)
```

---

## 12. Risks (`15 §9` + extras)

| Risk | Mitigation |
|---|---|
| ViGEm install friction | M2 assumes the operator's configured host has ViGEmBus installed. `VigemBackend::ensure_ready` still surfaces `ACTION_VIGEM_NOT_INSTALLED` and **never** auto-installs if the driver is missing, but absent-driver portability testing is not an M2 release gate. **No silent fallback to software backend.** |
| `Natural::FAST` feel iteration | preset frozen at M2 per §3.2; tune from real telemetry at M5 without changing the default-class |
| `OnceCell` vs `OnceLock` | use `std::sync::OnceLock` (stable since 1.70); do NOT add `once_cell` dep |
| `enigo` raw scan codes | `Key::use_scancode = true` routes through direct `windows::SendInput` w/ `KEYEVENTF_SCANCODE`; profile flag deferred to M3 (M2 hardcodes per-call) |
| Unicode typing into games that ignore `KEYEVENTF_UNICODE` | profile-flag fallback deferred to M3; M2 always uses Unicode for `act_type` and documents it |
| `held_key_max_duration_ms` collisions with reflex `hold_move` (M3) | M3 raises cap; M2 enforces 30 s strictly |
| Element drift between `observe()` and `act_click(element_id)` | `synapse_a11y::re_resolve` called inside the M2 InvokePattern bridge; if `ACTION_ELEMENT_NOT_RESOLVED`, the agent re-`observe()` and retries — no implicit retry inside the emitter |
| Foreground changed between tool calls | `act_*` tools snapshot the expected foreground HWND from the call's `target` (when it's an `element_id`); mismatch ⇒ `ACTION_FOREGROUND_LOST` |
| WSL Linux build cannot exercise `SendInput` | non-Windows backends always return `ACTION_BACKEND_UNAVAILABLE`; Linux checks are supporting evidence only, not the M2 shipping gate |
| Real UIA test flakiness (timing-sensitive Notepad startup) | the test harness has a `wait_for_window_title_regex` helper using `synapse_a11y::foreground_context` polled at 20 ms intervals with a 5 s timeout; never `sleep(Duration::from_secs(N))` |

---

## 13. Out of scope at M2 (deferred ≥ M3)

- Reflexes / event-driven actions (synapse-reflex stays empty)
- `act_combo` MCP tool (M3; the `ComboStep` / `ComboInput` types ship in M2's
  `synapse-core` extension so M3 can plumb them without a schema change)
- `act_run_shell`, `act_launch` (M4, gated via `--allow-shell` / `--allow-launch`)
- Hardware HID backend (`Backend::Hardware` routes return
  `ACTION_BACKEND_UNAVAILABLE`; M4 ships the real path)
- RocksDB action log persistence — M2 emits `tracing` events only; no
  `CF_ACTION_LOG` writes
- Streamable HTTP transport (`--mode http` keeps the M1 `NOT_YET_IMPLEMENTED`
  exit-2 guard until M3)
- Operator hotkey `Ctrl+Alt+Shift+P` (M3 wires `RegisterHotKey`; M2 only
  fires `SAFETY_RELEASE_ALL_FIRED` via the panic hook path)
- Profile-driven backend selection (M3); M2 honors only the per-call
  `backend` field and the `Backend::Auto` resolution rule (auto = software
  for keyboard/mouse, vigem for pad)

---

## 14. Definition of Done

M2 closes when:

1. The demo gate (§2) passes on a real Win11 box, hand-driven through Claude
   Desktop, with the source-of-truth file written byte-correct.
2. Every acceptance gate (§11) is green on `main`.
3. The manual H1-H10 + E1-E12 table (§9) is filled in by the operator in the
   PR description, with literal source-of-truth read-back values pasted in.
4. `git tag v0.1.0-m2` cuts a build artifact for archival.

Open next: `04_m3_reflex_mcp_surface.md`.

---

## Appendix A — Occam's razor recap

The single simplest description of M2: **fill out the empty
`synapse-action` crate, add nine `act_*` / `release_all` tools to the MCP
surface, and wire `ReleaseAll` into SIGINT + panic + shutdown.** Every other
clause in this document is a consequence of that single statement plus the
PRD invariants (Natural-only motion, no mocks gating completion, no
backwards-compat). If a contributor finds themselves designing something
that doesn't trace back to this sentence, the design is wrong.

## Appendix B — Where to look when something breaks (root-cause-first)

| Symptom | First file to read |
|---|---|
| `tools/list` doesn't show new tool | `crates/synapse-mcp/src/server.rs` `#[tool_router]` block |
| Schema rejected on call | `crates/synapse-mcp/src/m2.rs` `Params` struct missing `#[serde(deny_unknown_fields)]` |
| Held key not released after panic | `crates/synapse-action/src/safety.rs` — confirm `OnceLock` set before any `Action` is enqueued |
| Action queue full immediately | bounded `mpsc(256)` — check work-item 3's actor; the M1 capture path also uses bounded channels (`crates/synapse-capture/src/lib.rs:17`) — same pattern |
| `enigo` compile failure on Linux | `synapse-action` `Cargo.toml` `[target.'cfg(windows)'.dependencies]` — `enigo` and `vigem-client` MUST be Win-only |
| FSV test passes locally but CI fails on Ubuntu | Linux backend stub returning the wrong error code — assert from `error_codes::ACTION_BACKEND_UNAVAILABLE` not `unimplemented!()` |
| `cargo deny check` fails on new dep | check `deny.toml` SPDX allowlist; `vigem-client` is MIT, `enigo` is MIT, `arc-swap` is MIT-OR-Apache-2.0 |
| ViGEm pad does not appear to `XInputGetState` | `wait_for_ready` not called or ViGEmBus not installed; **never** silently swallow — surface `ACTION_VIGEM_PLUGIN_FAILED` with detail |

## Appendix C — Trigger → outcome map (the audit framework)

Every M2 tool has a single trigger event (the JSON-RPC `tools/call`) and a
single observable outcome. The table below is the M2 audit framework — when
debugging, identify which row applies and read both ends of the chain:

| Trigger | Process X | Outcome Y (observable) | Source of truth |
|---|---|---|---|
| `tools/call act_type` | `M2State.handle.execute(Action::TypeText{...})` → `SoftwareBackend::execute` → batched `SendInput` `KEYEVENTF_UNICODE` | Notepad UIA value updated | UIA `ValuePattern.value` |
| `tools/call act_press` | `Action::KeyChord{...}` → modifier-aware down/up sequence | OS receives chord | low-level keyboard hook |
| `tools/call act_click(element_id)` | `re_resolve` → `Invoke()` UIA call | Target Invokable executes (menu opens, button presses) | UIA `ExpandCollapsePattern` / focused-element change |
| `tools/call act_click(x,y)` | `MouseMove(curve=Natural::FAST, 50ms)` → curve sample → `SendInput` deltas → `MouseButton(down)` + `MouseButton(up)` | Cursor at (x,y); click landed | `GetCursorPos` + `RecordingBackend` events |
| `tools/call act_pad` | `Action::PadReport{...}` → `vigem_client::Xbox360Wired::update(report)` | Game sees gamepad input | `XInputGetState(0)` |
| `tools/call act_clipboard write` | `OpenClipboard` → `SetClipboardData(CF_UNICODETEXT)` | Clipboard owns the text | `Get-Clipboard` |
| `tools/call act_aim style=snap` | `AimAt` → compile to `MouseMove(Natural::FAST, 50ms)` | Cursor at target | `GetCursorPos` |
| `tools/call act_drag` | `MouseButton(down)` → `MouseMove(curve)` → `MouseButton(up)` | drag-aware app sees the stroke | Paint / capture of pixels |
| `tools/call act_scroll` | `SendInput` `MOUSEEVENTF_WHEEL`/`HWHEEL` | scroll-aware app scrolled | UIA `ScrollPattern.vertical_view_size` change |
| `tools/call release_all` | drain `held_keys` BitSet → KeyUp each; drain `held_buttons` → MouseUp; ViGEm neutral report; log `SAFETY_RELEASE_ALL_FIRED` | nothing held | `GetAsyncKeyState` returns 0 for every key |
| SIGINT / panic | `safety.rs` panic hook → `handle.fire_release_all_blocking_with_timeout(10ms)` | same as `release_all` | same |
| Stuck-key timer | held-state timer fires at `held_key_max_duration_ms` | KeyUp emitted; event logged | `RecordingBackend::events()` + tracing log |

When a manual test fails, identify the row, read both columns, and the bug
is in the X column (the process). The test's source-of-truth read-back tells
you whether Y landed; the daemon's tracing log + RecordingBackend events
tell you whether X ran.
