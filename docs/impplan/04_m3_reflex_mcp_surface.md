# 04 — M3: Reflex + MCP Surface (2-3 weeks)

PRD: `15_roadmap_and_milestones.md` §5. Reflexes: `04_reflex_runtime.md`. Storage: `07_storage_and_profiles.md`. Audio: `02_perception.md` §6. MCP HTTP+SSE: `01_architecture.md` §2 + `05_mcp_tool_surface.md` §5-7.

## Goal

Event bus + 5 reflex kinds. RocksDB CFs w/ TTL + GC + disk-pressure responder. Profile TOML loader + hot-reload. Streamable HTTP transport + SSE push. Audio loopback + Whisper-tiny STT.

## Demo gate

Notepad open → agent: `reflex_register(on_event, when={kind:"element-appeared",match:"Save As dialog"}, then=act_type+act_press(enter))` → `act_press(["ctrl","s"])` → reflex auto-fires → file saved. Zero agent intervention between Ctrl+S and saved file.

---

## Inputs

- M2 demo gate passed
- `rocksdb = "0.22"` builds clean on Win11 (`bzip2`/`zlib` C deps OK)
- `wasapi = "0.16"` available; system has default render device
- Test runner has > 4 GB free disk for storage tests; 1 GB tmpfs volume available for disk-pressure tests

---

## Deliverables

### Crates

| Crate | M3 contents |
|---|---|
| `synapse-reflex` | Event bus (in-process broadcast via `arc-swap::ArcSwap<Vec<Subscriber>>` + per-subscriber `crossbeam` bounded ch); reflex scheduler on dedicated `THREAD_PRIORITY_TIME_CRITICAL` thread; 1 ms tick via `CreateWaitableTimerEx` high-resolution; `aim_track`, `hold_move`, `hold_button`, `combo`, `on_event` controllers; conflict resolution (priority + newer-wins); `ReflexLifetime` (`OneShot`/`UntilCancelled`/`UntilEvent`/`Duration`); audit log to `CF_REFLEX_AUDIT`; reflex cap 32/session; recursion guard ≤ 4 firings/tick (`OQ-022`); panic hotkey `Ctrl+Alt+Shift+P` clears all reflexes + `ReleaseAll` ≤ 50 ms |
| `synapse-storage` | RocksDB open w/ CFs from `07 §4` (`CF_EVENTS`, `CF_OBSERVATIONS`, `CF_PROFILES`, `CF_MODEL_CACHE`, `CF_SESSIONS`, `CF_REFLEX_AUDIT`, `CF_OCR_CACHE`, `CF_TELEMETRY`, `CF_ACTION_LOG`, `CF_PROCESS_HISTORY`, `CF_KV`); per-CF compaction filter w/ TTL from runtime config; write batcher flush 100 ms / 64 KB / explicit; GC task @ 5 min checking soft caps; disk-pressure responder 4 levels (`07 §6.3`); `--feature sled-backend` opt-in fallback |
| `synapse-profiles` | TOML parser → `Profile` struct (`06 §6`); `notify = "6.1"` watcher on profile dir(s); precedence: `--profile-dir` > `%APPDATA%\synapse\profiles\` > bundled `profiles/`; match by `exe` + `title_regex` + `steam_appid`; bundled profiles: `notepad`, `vscode`, `chrome`, `terminal` |
| `synapse-audio` | WASAPI loopback ring 5 s; STT via Whisper-tiny-int8 ONNX (~40 MB; lazy load); naive direction estimate (L/R energy ratio + GCC-PHAT lag); audio event detectors: `loud_transient`, `speech_started`/`ended`, `music_started`/`ended`; Silero VAD ONNX ~2 MB |
| `synapse-core` (extensions) | `Profile`, `ProfileMatch`, `ProfileCapture`, `ProfileDetection`, `ProfileOcr`, `HudFieldSpec`, `HudExtractor`, `HudParser`, `HudRegion`, `WindowEdge`, `ProfileBackends`, `EventExtension`, `ReflexRegistration`, `ReflexKind`, `ReflexLifetime`, `ReflexState`, `ReflexStatus`, `StoredEvent`, `StoredObservation`, `StoredReflexAudit`, `StoredSession`, `OcrResult`, `OcrWord` |
| `synapse-mcp` (add tools) | `subscribe`, `reflex_register`, `reflex_cancel`, `reflex_list`, `reflex_history`, `profile_list`, `profile_activate`, `replay_record`, `audio_tail`, `audio_transcribe` per `05 §3.5-3.8, §3.22-3.28, §3.30`; Streamable HTTP transport via `axum = "0.7"` + `Mcp-Session-Id` header; SSE push (per-event, no batching at v1 per `OQ-029`); bearer-token auth + Host/Origin check (`11 §3.2`) |

### Bundled profiles (`profiles/`)

All profiles set `mouse_curve_default = "natural"` and `keyboard_dynamics_default = "natural"` per OQ-004 DECIDED. No bundled profile selects `Instant` or `Burst` as a default.

| Profile | Mode | Highlights |
|---|---|---|
| `notepad.toml` | `a11y_only` | smoke-test app, no HUD, no detection |
| `vscode.toml` | `a11y_only` | example in `07 §8.2`; keymap aliases for save/quick-open/command-palette |
| `chrome.toml` | `hybrid` | CDP attach when debug port present |
| `terminal.toml` | `a11y_only` | Windows Terminal + PowerShell |

### Error codes (must throw + test)

```
REFLEX_CAP_REACHED
REFLEX_KIND_INVALID
REFLEX_PARAMS_INVALID
REFLEX_TARGET_INVALID
REFLEX_FILTER_INVALID
REFLEX_PRIORITY_INVALID
REFLEX_TICK_LATE
REFLEX_TRACK_LOST
REFLEX_STARVED
REFLEX_DISABLED_BY_OPERATOR
REFLEX_LIFETIME_EXPIRED
PROFILE_NOT_FOUND
PROFILE_PARSE_ERROR
PROFILE_VERSION_INCOMPATIBLE
PROFILE_KEYMAP_INVALID
PROFILE_HUD_REGION_INVALID
SESSION_NOT_FOUND
SESSION_EXPIRED
SUBSCRIPTION_NOT_FOUND
SUBSCRIPTION_CAP_REACHED
TOOL_NOT_FOUND
TOOL_PARAMS_INVALID
TOOL_INTERNAL_ERROR
STORAGE_OPEN_FAILED
STORAGE_WRITE_FAILED
STORAGE_READ_FAILED
STORAGE_CORRUPTED
STORAGE_SCHEMA_MISMATCH
HUD_NO_ACTIVE_PROFILE
HUD_FIELD_NOT_DEFINED
HUD_EXTRACTION_FAILED
AUDIO_DEVICE_LOST
AUDIO_LOOPBACK_INIT_FAILED
AUDIO_STT_MODEL_NOT_LOADED
```

---

## Work-items (PR-sized, ordered)

### Block A — storage (work-items 1-5)

| # | Title | Acceptance |
|---|---|---|
| 1 | `feat(storage): open Db w/ all 11 CFs + tuning per 07 §12` | `Db::open(tempdir)` succeeds; CF names match `synapse-core::cf` consts; test asserts all CFs created |
| 2 | `feat(storage): per-CF compaction filter w/ TTL from runtime config` | proptest: insert records w/ timestamps spanning > TTL; compact; old rows gone |
| 3 | `feat(storage): write batch task (mpsc + 100 ms / 64 KB / explicit flush)` | bench: 10k events writes ≤ 200 ms wall via batch; per-write ≤ 100 µs avg |
| 4 | `feat(storage): GC task @ 5 min w/ soft-cap DeleteRange + compact` | scenario test: fill CF to 2× soft cap; GC tick; live size drops below soft; `cache_evictions_total{cf,reason}` increments |
| 5 | `feat(storage): disk-pressure responder 4 levels (07 §6.3)` | 1 GB tmpfs scenario; fill DB; observe transitions through L1 → L2 → L3 → L4; events `STORAGE_DISK_PRESSURE_LEVEL_N` emitted |

### Block B — event bus + reflex runtime (work-items 6-12)

| # | Title | Acceptance |
|---|---|---|
| 6 | `feat(reflex): EventBus broadcast w/ filtered subscribers + drop-oldest backpressure` | per-subscriber 4096 buffer; slow consumer drops events; `events_dropped_for_subscriber{id}` metric |
| 7 | `feat(reflex): scheduler thread + 1ms tick via CreateWaitableTimerEx + MMCSS` | bench `reflex_tick_jitter_idle` p99 ≤ 200 µs; under-load ≤ 500 µs |
| 8 | `feat(reflex): aim_track controller (delta + gain + deadzone + max_speed)` | E2E: register vs static detected entity (mock), 60 ticks, cursor settles at target ± deadzone |
| 9 | `feat(reflex): hold_move + hold_button (KeyDown on register, KeyUp on lifetime end)` | E2E: hold W for 2 s via UntilEvent fake; lifetime fires; KeyUp emitted |
| 10 | `feat(reflex): combo (timed step sequence, scheduler ticks fire steps when at_ms due)` | bench: 3-step combo step intervals within 500 µs of scheduled (10 §11) |
| 11 | `feat(reflex): on_event w/ EventFilter eval + debounce + recursion guard (OQ-022)` | proptest filter eval: `Not(Not(x))==x` for total filters; per-tick max 4 firings ⇒ `REFLEX_RECURSION_LIMIT` event |
| 12 | `feat(reflex): conflict resolution (priority + newer-wins + starvation logging)` | two contending aim_tracks: higher priority wins; loser logs `reflex_starved` after 2 s |

### Block C — profiles (work-items 13-15)

| # | Title | Acceptance |
|---|---|---|
| 13 | `feat(profiles): TOML loader → Profile struct + version compat check` | every bundled profile parses; PROFILE_VERSION_INCOMPATIBLE on major mismatch |
| 14 | `feat(profiles): notify watcher hot-reload + match resolver` | edit `profiles/vscode.toml` → in-memory profile replaced on next event tick |
| 15 | `feat(profiles): bundled notepad / vscode / chrome / terminal w/ Natural defaults` | E2E: launch each, `profile_list` shows active match correct; profile validation asserts `mouse_curve_default == "natural"` + `keyboard_dynamics_default == "natural"` on every bundled profile |

### Block D — audio (work-items 16-18)

| # | Title | Acceptance |
|---|---|---|
| 16 | `feat(audio): WASAPI loopback ring 5 s + audio event detectors` | playback test asset; `loud_transient`, `speech_started/ended` events emitted; RMS metric flows |
| 17 | `feat(audio): Whisper-tiny-int8 STT (lazy load)` | 5 s known clip; `audio_transcribe()` p99 ≤ 200 ms (10 §12); `AUDIO_STT_MODEL_NOT_LOADED` until present |
| 18 | `feat(audio): direction estimate (L/R energy + GCC-PHAT)` | stereo test clips at ±60° azimuth; estimate within ±15° |

> Dev-loop note: the dev box exposes a Windows-side PulseAudio daemon on `tcp:127.0.0.1:4713` reachable from WSL (mirrored networking). Useful for replaying fixtures or recording `output.monitor` from the Linux shell while iterating on 16-18. Production path stays WASAPI direct — PulseAudio is **not** in the shipped surface. Snapshot + commands: #85.

### Block E — MCP HTTP + SSE + new tools (work-items 19-23)

| # | Title | Acceptance |
|---|---|---|
| 19 | `feat(mcp): axum HTTP transport + Mcp-Session-Id + bearer auth + Origin/Host check` | curl test: no token ⇒ 401; bad Origin ⇒ 403; session round-trip works |
| 20 | `feat(mcp): SSE push notifications for subscriptions w/ Last-Event-ID resume` | reconnect test: drop SSE mid-stream, reconnect w/ `Last-Event-ID: <seq>`, server replays buffered events from there (buffer 4096) |
| 21 | `feat(mcp): subscribe + subscribe_cancel + event filter conversion` | EventFilter from `06 §3.2` (Kind/Source/And/Or/Not/Data with DataPredicate) round-trips; snapshot_first works |
| 22 | `feat(mcp): reflex_register + reflex_cancel + reflex_list + reflex_history` | E2E: register on_event for `value-changed`, fire, observe `reflex-fired` event in audit + list |
| 23 | `feat(mcp): profile_list + profile_activate + replay_record + audio_tail + audio_transcribe` | tools/list returns 10 new tools; schemas match `05 §3.x` (insta snapshot) |

### Block F — safety + demo (work-items 24-25)

| # | Title | Acceptance |
|---|---|---|
| 24 | `feat(safety): panic hotkey RegisterHotKey Ctrl+Alt+Shift+P → ReleaseAll + reflex_disable_all` | timer test: register 1 reflex, press hotkey, all reflexes terminate + ReleaseAll fires within 50 ms |
| 25 | `test(e2e): notepad save-dialog reflex demo (M3 demo gate scenario)` | full path passes via stdio + via HTTP w/ token |

---

## Acceptance gates (block M4)

```
✓ M3 demo passes (Notepad save-dialog reflex)
✓ Bench reflex_tick_jitter_idle p99 ≤ 200 µs (10 §2, 13 §7)
✓ Bench event_to_subscriber p99 ≤ 50 ms (10 §2, 13 §7)
✓ Bench observe_warm_hybrid p99 still ≤ 30 ms (no regression from M1 baseline)
✓ Disk pressure scenario passes through all 4 levels deterministically
✓ Profile hot-reload picks up edits in ≤ 1 tick
✓ HTTP transport: bearer auth + Host/Origin check + SSE resume work end-to-end
✓ All M3 error codes throwable + tested
✓ All 10 new MCP tools schema-snapshotted
✓ No mocks gate completion — real RocksDB on real disk, real WASAPI on real device, real Notepad in E2E
✓ Soak (1h) clean: no memory growth > 50 MB, no deadlocks
```

---

## Risks (`15 §9` + extras)

| Risk | Mitigation |
|---|---|
| Time-critical thread jitter on Windows | `CreateWaitableTimerEx` w/ `CREATE_WAITABLE_TIMER_HIGH_RESOLUTION` + MMCSS Pro Audio characteristic; fallback to `tokio::time` 2 ms tick if MMCSS unavailable (degraded) |
| Hot-reload of profile vs. active reflexes | Reflex params snapshot at registration; profile alias resolution happens at register-time; subsequent profile changes don't retroactively break running reflexes; if missing alias surfaces on fire ⇒ `REFLEX_PARAMS_INVALID` |
| Streamable HTTP/SSE reconnect semantics | `Last-Event-ID` header on reconnect; buffer 4096/sub; deeper outage ⇒ subscription marked `lossy=true` in next push |
| RocksDB Windows hiccups (`OQ-001`) | `--feature sled-backend` escape valve; if > 2 RocksDB crashes during M3, flip default per `OQ-001` |
| Whisper-tiny accuracy weaker than expected (`OQ-014`) | Operator opt-in upgrade to `whisper-base` via `models import`; bundled-default decision deferred to M5 |
| Multi-monitor profile match (`OQ-012`) | one capture target at a time; agent picks via `set_capture_target` |
| `RuntimeId` instability under heavy mutation (`OQ-023`) | tested in M2; if observed, M3 wraps with our own ID; deferred unless reproducible |

---

## Out of scope at M3 (deferred ≥ M4)

- Hardware HID backend
- `act_combo` MCP tool — internally combos work via `reflex_register(combo, ...)`; the standalone `act_combo` tool ships in M4 (uses the same scheduler)
- `act_run_shell`, `act_launch` (M4, gated)
- Game profiles (Minecraft etc. land in M4)
- VLM `describe` (M5)
- Debug overlay (M5)

---

## Definition of Done

M3 closed when demo passes + acceptance gates green + `git tag v0.1.0-m3`. Open next: `05_m4_hardware_hid_first_game.md`.
