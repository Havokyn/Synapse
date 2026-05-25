# 04 — Reflex Runtime

## 1. Why reflexes exist

Tool-use round-trip is 200–500 ms. A 60 fps game runs at 16.7 ms/frame; fighting-game windows are 1–3 frames. Round-tripping every action makes the agent unable to:

- Click fast-moving targets on-screen for a few frames
- React to "low HP" with a medkit in time
- Hit frame-perfect combos
- Track per-frame entity positions
- Auto-dismiss input-blocking popups

The reflex runtime is the sub-frame body. Agent installs *intents* ("track this enemy", "press medkit if HP < 20", "hold W until told to stop"); runtime executes them locally at frame rate.

Reflexes are NOT:

- Goal planning (model's job)
- Policies (model's job — declarative, not learned)
- Long-lived agents (each reflex is a small reactive primitive)
- Stateful skill libraries (runtime forgets every reflex on unregister)

---

## 2. Surface

Five controller families plus a generic event→action binding:

| Controller | What it does |
|---|---|
| `aim_track` | Continuously moves the mouse so a tracked entity stays under the crosshair; cancels on track loss or explicit stop |
| `hold_move` | Holds a set of keys until a condition is met (timeout, event, or stop) |
| `hold_button` | Holds a pad button or mouse button until a condition is met |
| `combo` | Fires a frame-accurate sequence of inputs at precise ms offsets |
| `on_event` | When an event matches the registered filter, emits an action and (optionally) unregisters itself |

All are first-class MCP tools (see `05_mcp_tool_surface.md`) and internally addressable for composition.

Each registered reflex has:

- A stable `reflex_id` (UUID, returned to agent)
- A `kind` (one of the families above)
- A `parameters` blob
- A `lifetime` (`OneShot`, `UntilCancelled`, `UntilEvent(filter)`, `Duration(d)`)
- A `priority` (used when two reflexes contend for the same device)

---

## 3. Execution model

```
       Event bus
  ┌────────────┐
  │ Perception │──┐                    ┌─ aim_track    ──► action emitter
  │   events   │  │                    │
  └────────────┘  │   ┌────────────┐   ├─ hold_move    ──► action emitter
                  ├──►│ Reflex     │──►│
  ┌────────────┐  │   │  scheduler │   ├─ hold_button  ──► action emitter
  │  Frame     │──┘   └────────────┘   │
  │  ticks     │                       ├─ combo        ──► action emitter
  └────────────┘                       │
                                       └─ on_event     ──► action emitter
```

Scheduler runs on a **dedicated OS thread** at `THREAD_PRIORITY_TIME_CRITICAL` with a 1 ms tick (driven by `multimedia timer` / `CreateWaitableTimer` with high-resolution period). No tokio runtime; tokio scheduler jitter would cost frames.

Each tick the scheduler:

1. Drains the event bus (perception, frame-ticks, completion notifications)
2. For every active reflex, in priority order:
   - Continuous controllers (`aim_track`, `hold_move`, `hold_button`): invoke `step()`, queue actions
   - `on_event` reflexes: check whether the most recent events match; if so, fire its action
   - `combo`: check whether next step's `at_ms` is due (or past); if so, fire it
3. Pushes queued actions through the bounded `try_send` channel into the action emitter
4. Sleeps until the next tick

Slept time is typically 0.5–0.9 ms per tick. Thread parks via `WaitForSingleObject` on the timer handle.

---

## 4. The event bus

Small in-process broadcast bus:

```rust
pub struct EventBus {
    senders: ArcSwap<Vec<Subscriber>>,
}
pub struct Subscriber {
    id: SubscriberId,
    filter: EventFilter,
    sender: crossbeam::channel::Sender<Event>,
}
```

- Publishers: perception (capture/a11y/audio), action emitter (completion notifications), reflex scheduler (own state changes for observability).
- Subscribers: reflex scheduler (filtered by reflex bindings), MCP push-event subscribers (per-agent filter), storage writer (writes every event to `CF_EVENTS`).
- Delivery discipline: per ADR-0007, the bus publishes one event at a time and
  never waits to form a batch. Subscribers/clients may batch downstream after
  receiving individual events.
- Backpressure: each subscriber has a bounded channel. Slow subscriber backs up
  → bus drops the oldest queued event for that subscriber, enqueues the newest
  event, marks the subscription lossy, and increments
  `events_dropped_for_subscriber{id}`. Never blocks the publisher.

Bus capacity per subscriber: 4096 events. At typical 100–1000 events/s perception rate, this is many seconds of buffer.

---

## 5. The five controller families

### 5.1 `aim_track`

```rust
pub struct AimTrackParams {
    pub target: AimTarget,                   // ScreenPoint | EntityTrack(track_id) | NamedElement
    pub axis: AimAxis,                       // XY | XOnly | YOnly
    pub gain: f32,                           // 0.0..2.0; 1.0 = exact tracking
    pub deadzone_px: f32,                    // ignore micro-movements
    pub max_speed_px_per_ms: f32,            // cap; protects against teleport-on-track-spike
    pub curve_per_step: AimCurve,            // usually Linear or short-Bezier per-step
    pub backend: Backend,
    pub lifetime: ReflexLifetime,
}
```

Each tick:

1. Resolve target position. If `EntityTrack(track_id)`, look up current detection track. If missing, increment `track_lost_ticks`; after 3 consecutive misses → tick fails this cycle (does not cancel; another detection may revive).
2. Compute screen delta from cursor to target.
3. Apply deadzone, gain, speed cap.
4. Emit `MouseMoveRelative { dx, dy }`.
5. If `track_lost_ticks > 60` (1 second), unregister with reason `track_lost`.

Cancellation: agent calls `reflex_cancel(reflex_id)`. Scheduler unregisters, emits `reflex_terminated` event.

### 5.2 `hold_move`

```rust
pub struct HoldMoveParams {
    pub keys: Vec<Key>,                       // e.g. [W, Shift]
    pub backend: Backend,
    pub lifetime: ReflexLifetime,
}
```

On registration: emits `KeyDown` for each key. On cancellation or lifetime expiry: emits `KeyUp` for each.

Cooperates with the action emitter's "held key" tracking — if some OTHER caller releases W, the reflex re-presses it on next tick (configurable; default `re_assert = false`).

### 5.3 `hold_button`

Identical structure for mouse buttons or pad buttons.

### 5.4 `combo`

```rust
pub struct ComboParams {
    pub steps: Vec<ComboStep>,                // time-stamped inputs in ms offsets from start
    pub backend: Backend,
}
pub struct ComboStep {
    pub at_ms: u32,                           // offset from combo start
    pub input: ComboInput,                    // KeyDown/Up/Press, MouseButton, PadButton, PadStick
}
```

Frame-accurate execution. On registration, scheduler records start `Instant`. Each tick fires every step whose `at_ms` has passed and not yet fired. After last step fires, reflex auto-terminates.

If scheduler tick falls behind (extreme load), late steps still fire but are tagged `late_by_ms` in the audit log. Agent can detect late combos.

### 5.5 `on_event`

```rust
pub struct OnEventParams {
    pub when: EventFilter,                    // e.g., kind="hud_value_changed" AND field="hp" AND new<20
    pub then: Action,                         // any Action enum variant
    pub debounce_ms: u32,                     // re-fire suppression
    pub lifetime: ReflexLifetime,             // OneShot | UntilCancelled
}
```

On each event matching `when`:

1. Check debounce — if `now < last_fire + debounce_ms`, skip.
2. Emit `then` action via action emitter.
3. Record `last_fire = now`.
4. If `lifetime == OneShot`, unregister.

`EventFilter` is a small declarative expression language (see `06_data_schemas.md` §EventFilter) — kind match + optional field comparisons + optional source filter.

---

## 6. Conflict resolution

Two reflexes contending for the same device (e.g., two `aim_track` reflexes moving the mouse) resolve by:

1. **Priority.** Lower numeric `u32` priority wins this tick. Default priority is `100`.
2. **Newer over older,** ties broken by registration order.

A reflex consistently losing for >2 seconds logs a `reflex_starved` event so the agent can fix the conflict.

Agent can register reflexes with `exclusive: true` — only one such reflex per device class active; previous auto-cancelled.

---

## 7. Lifetimes

```rust
pub enum ReflexLifetime {
    OneShot,                              // fires once, unregisters
    UntilCancelled,                       // active until agent cancels
    UntilEvent { filter: EventFilter },   // active until matching event
    Duration(Duration),                   // active for at most this long
}
```

`UntilEvent` is most useful for game agents: "track this enemy until they die" → `lifetime: UntilEvent { kind: "entity_disappeared", track_id: target_id }`.

Default lifetime: `UntilCancelled`. Agents should explicitly cancel reflexes they no longer want; runtime does not garbage-collect.

Per-session cap (`max_reflexes_per_session: 32`) prevents runaway registration. Hitting the cap returns `REFLEX_CAP_REACHED`.

---

## 8. Reflex audit log

Every registration, firing, cancellation, lifetime expiry, conflict, and starvation is written to `CF_REFLEX_AUDIT` and surfaced via `reflex_history` MCP tool. Keyed by `(reflex_id, fired_at_ns)` → JSON record.

Critical for debugging — when an agent says "my aim-track didn't work," we need to know whether it never registered, never matched a target, or matched but lost the priority contest.

---

## 9. Composition examples

Agent composes reflexes by registering several at once. Runtime treats them independently.

### Pattern A: Track-and-shoot

```
reflex_register(aim_track, target=entity_42, lifetime=UntilEvent(kind=entity_disappeared))
reflex_register(on_event,
    when=(kind=entity_visible AND track_id=42 AND inside_crosshair),
    then=mouse_button(left, hold=16ms),
    debounce_ms=200)
```

Tracking + auto-fire assembled. Runtime handles both at frame rate. Agent makes ONE round-trip.

### Pattern B: Survival hotkey

```
reflex_register(on_event,
    when=(kind=hud_value_changed AND field=hp AND new<20),
    then=key_press("4"),                # "4" binds to medkit slot in this game
    debounce_ms=2000,                   # don't burn medkits in spasms
    lifetime=UntilCancelled)
```

### Pattern C: Long press until something happens

```
reflex_register(hold_move,
    keys=[W],
    lifetime=UntilEvent(kind=arrived_at_destination))
```

Agent hands W key control to runtime. Agent doesn't poll. When perception fires `arrived_at_destination`, the reflex releases W.

---

## 10. Reflex stacking and the scheduler's two phases

A tick has two phases:

**Phase 1 — event drain.** Drain bus, fire on_event reflexes, update target positions for aim_track / combo state.

**Phase 2 — controller step.** For each active continuous reflex (aim_track / hold_move / hold_button / combo), call `.step()` and queue actions.

Guarantees that within one tick, event-driven actions fire BEFORE continuous-controller actions. If the agent has an `on_event` that should cancel an `aim_track`, the cancellation lands first.

---

## 11. Latency targets

| Stage | Target p99 |
|---|---|
| Tick interval | 1 ms (with 100 µs jitter) |
| Event → matching reflex check | ≤ 200 µs |
| Reflex matched → action queued | ≤ 100 µs |
| Action queued → action emitter dequeues | ≤ 500 µs |
| Action emitter → SendInput call | ≤ 1 ms (software) |
| Total event → input on the wire | ≤ 5 ms |

Frame-accurate combos need the tick scheduler to fire at the right ms; 100–300 µs jitter is acceptable. 1 ms tick with high-resolution timer gives this on Windows 11 if MMCSS is active. Never use `tokio::time::sleep` here (accuracy is poor below 5 ms on Windows).

---

## 12. Safe defaults and limits

| Limit | Default | Why |
|---|---|---|
| Max reflexes per session | 32 | Prevent runaway |
| Max reflex lifetime | 1 hour | Agents should cancel; this catches lost cancellations |
| `aim_track` max speed | 5000 px/ms (5 px/µs, generous) | Avoid teleport-on-spike |
| `hold_move` re-assert | false | Don't fight with the user |
| `combo` total length | 5 seconds | Anything longer should be a sequence of combos |
| Tick deadline miss | logged as `reflex_tick_late` | Detect host overload |
| Event bus subscriber buffer | 4096 events | Allow >4s buffer at 1 kHz |

---

## 13. Reflex disable switch

Operator-controlled kill switch:

- CLI: `--reflex-disabled` flag at startup
- Runtime: `reflex_disable_all()` MCP tool (emits `ReleaseAll` first)
- Hotkey: user-bindable global hotkey (`reflex_panic_hotkey` config) immediately disables ALL reflexes and emits `ReleaseAll`. Default: `Ctrl+Alt+Shift+P`.

Non-negotiable safety machinery. If anything goes wrong, operator hits the hotkey, all reflexes terminate, all held inputs release.

---

## 14. Error codes

```rust
pub const REFLEX_CAP_REACHED: &str = "REFLEX_CAP_REACHED";
pub const REFLEX_TARGET_INVALID: &str = "REFLEX_TARGET_INVALID";
pub const REFLEX_FILTER_INVALID: &str = "REFLEX_FILTER_INVALID";
pub const REFLEX_PRIORITY_INVALID: &str = "REFLEX_PRIORITY_INVALID";
pub const REFLEX_TICK_LATE: &str = "REFLEX_TICK_LATE";
pub const REFLEX_TRACK_LOST: &str = "REFLEX_TRACK_LOST";
pub const REFLEX_STARVED: &str = "REFLEX_STARVED";
pub const REFLEX_DISABLED_BY_OPERATOR: &str = "REFLEX_DISABLED_BY_OPERATOR";
```

---

## 15. Out of scope

- Reflex MCP tool schemas → `05_mcp_tool_surface.md`
- Event filter grammar → `06_data_schemas.md`
- Audit log persistence → `07_storage_and_profiles.md`
- Action back-end details → `03_action.md`
