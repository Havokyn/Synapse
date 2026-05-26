# 07 — Reflex Runtime (`synapse-reflex`)

Source files covered:
- `crates/synapse-reflex/src/lib.rs`
- `crates/synapse-reflex/src/audit.rs`
- `crates/synapse-reflex/src/bus.rs`
- `crates/synapse-reflex/src/conflict.rs`
- `crates/synapse-reflex/src/error.rs`
- `crates/synapse-reflex/src/scheduler.rs`
- `crates/synapse-reflex/src/scheduler_tick.rs`
- `crates/synapse-reflex/src/scheduler_combo.rs`
- `crates/synapse-reflex/src/scheduler_stats.rs`
- `crates/synapse-reflex/src/scheduler_windows.rs`
- `crates/synapse-reflex/src/kinds/{mod, aim_track, combo, hold_button, hold_lifetime, hold_move, on_event}.rs`

## 1. Crate role

`synapse-reflex` runs the **sub-frame reactive controllers** that turn streamed events and operator-registered intents into emitted `synapse_action::ActionHandle` calls without round-tripping through the MCP client. It also owns the in-process event bus consumed by the HTTP SSE bridge.

## 2. Public surface (`lib.rs` re-exports)

| Symbol | Source |
|---|---|
| `ReflexRuntime`, `ReflexCancelOutcome` | `lib.rs` |
| `write_audit` | `audit.rs` |
| `EventBus`, `EventBusError`, `EventBusResult`, `PublishReport`, `SUBSCRIBER_QUEUE_CAPACITY`, `SubscriberHandle`, `EVENTS_DROPPED_METRIC`, `DEFAULT_MAX_SUBSCRIPTIONS`, `DEFAULT_MAX_SUBSCRIPTIONS_NONZERO` | `bus.rs` |
| `REFLEX_STARVED_KIND`, `STARVATION_AFTER` | `conflict.rs` |
| `ReflexError`, `ReflexResult` | `error.rs` |
| `AimTrackContext`, `AimTrackController`, `AimTrackOutput`, `AimTrackParams`, `AimTrackTarget`, `DEFAULT_EMA_ALPHA`, `DEFAULT_MAX_SPEED_PX_PER_TICK`, `REFLEX_TRACK_LOST_KIND`, `ResolvedElementBox`, `TRACK_LOST_AFTER` | `kinds/aim_track.rs` |
| `ComboContext`, `ComboController`, `ComboOutput`, `ComboParams`, `ComboPhase`, `REFLEX_COMBO_COMPLETED_KIND` | `kinds/combo.rs` |
| `HoldButtonController`, `HoldButtonOutput`, `HoldButtonParams`, `HoldButtonPhase` | `kinds/hold_button.rs` |
| `HoldLifetimeContext`, `HoldReleaseReason`, `REFLEX_LIFETIME_EXPIRED_KIND` | `kinds/hold_lifetime.rs` |
| `HoldMoveController`, `HoldMoveOutput`, `HoldMoveParams`, `HoldMovePhase` | `kinds/hold_move.rs` |
| `MAX_ON_EVENT_FIRINGS_PER_TICK`, `REFLEX_FIRED_KIND`, `REFLEX_RECURSION_LIMIT_KIND` | `kinds/on_event.rs` |
| `DEFAULT_REFLEX_PRIORITY`, `MAX_REFLEX_PRIORITY`, `MAX_SCHEDULED_REFLEXES`, `REFLEX_TICK_LATE_KIND`, `ReflexScheduler`, `ScheduledReflex`, `SchedulerConfig`, `SchedulerHandle`, `SchedulerTrigger`, `TickSample`, `p99_jitter_us` | `scheduler.rs` |
| Event-kind name constants `REFLEX_CANCELLED_KIND = "reflex_cancelled"`, `REFLEX_DISABLED_KIND = "reflex_disabled_by_operator"`, `REFLEX_REGISTERED_KIND = "reflex_registered"` | `lib.rs` |

## 3. `ReflexRuntime`

```rust
pub struct ReflexRuntime {
    db: Arc<Db>,
    action_handle: ActionHandle,
    event_bus: EventBus,
    scheduler_config: SchedulerConfig,
    reflexes: Vec<ScheduledReflex>,
    disabled_reflex_ids: HashSet<ReflexId>,
    scheduler: Option<SchedulerHandle>,
}
```

Construction:

| Constructor | Default `SchedulerConfig` |
|---|---|
| `spawn(db, action_handle, event_bus)` | `SchedulerConfig::default()` |
| `spawn_with_config(db, action_handle, event_bus, scheduler_config)` | caller supplied |

`SchedulerConfig::default()` (`scheduler.rs:51`): `target_interval = 1 ms`, `fallback_interval = 2 ms`, `late_after = 2 ms`, `sample_limit = 4096`, `max_ticks = None`, `force_degraded = false`. `validate()` rejects zero intervals or zero sample_limit with `ReflexError::ParamsInvalid`.

### 3.1 `register(&ScheduledReflex)`

(`lib.rs:146`) Algorithm:

1. Check `reflex.priority <= MAX_REFLEX_PRIORITY (= 1000)`; else `ReflexError::PriorityInvalid`.
2. Clone the existing reflex list, append the new one, call `scheduler::validate_reflexes(&next)` — enforces `MAX_SCHEDULED_REFLEXES = 32`, unique reflex ids, and any cross-reflex constraints.
3. Spawn a fresh `ReflexScheduler` (`ReflexScheduler::spawn_with_audit_db`) with the new list and the same `scheduler_config`, sharing the `Arc<Db>` for audit persistence.
4. Replay disabled-state on the new scheduler so previously operator-disabled reflexes stay `Disabled` after the swap.
5. Replace the current scheduler. Stop the old one. (Hot-swap pattern: there is no "add reflex" channel into a live scheduler.)
6. Look up the new reflex's `ReflexStatus` from the new scheduler. Persist a `StoredReflexAudit` row with `details.kind = "reflex_registered"` (helper `write_registration_audit`), then `db.flush()`.
7. Return the `ReflexStatus`.

### 3.2 `cancel(reflex_id)`

(`lib.rs:198`) Algorithm:

1. Look up current status. Missing → `ReflexCancelOutcome::NotFound`.
2. Terminal states: `Expired` → `AlreadyExpired`, `Cancelled` → returns `Cancelled` (idempotent).
3. Tell the scheduler to cancel; failure to find at scheduler layer → `NotFound`.
4. Remove from `disabled_reflex_ids` (cancellation supersedes operator disable).
5. Look up the now-cancelled status, persist a `"reflex_cancelled"` audit row, flush, return `Cancelled { status }`.

### 3.3 `disable_all_by_operator()`

(`lib.rs:245`) Called only by `synapse-mcp/src/safety.rs::handle_operator_hotkey`. Algorithm:

1. If no scheduler is alive (no reflex registered yet), return empty `Vec`.
2. `scheduler.disable_all_reflexes()` flips every reflex to `Disabled` and returns the affected statuses.
3. Track ids in `disabled_reflex_ids` so subsequent re-registration (which spawns a fresh scheduler) preserves disable.
4. Persist one `StoredReflexAudit` per disabled status with `details.kind = "reflex_disabled_by_operator"`, `error_code = REFLEX_DISABLED_BY_OPERATOR`, `details.reason = "operator_hotkey"`. Flush.

### 3.4 `statuses()` / `list(include_expired)` / `history(reflex_id, limit)`

`statuses()` returns `scheduler.statuses()` (or empty if no scheduler).

`list(include_expired)`:
- Always include non-terminal scheduler statuses.
- If `include_expired = true`, also call `terminal_statuses_from_audit()`, which scans `CF_REFLEX_AUDIT`, groups rows by `reflex_id`, sorts each group by `(ts_ns, audit_id)`, and reconstructs a `ReflexStatus` from registration + fire-count + terminal audit (`reflex_cancelled` or expired). Final-state rows from the same reflex id in both the live scheduler and the audit log are deduplicated (the live row wins).
- Returns `ReflexResult<Vec<ReflexStatus>>`.

`history(reflex_id, limit)` (`lib.rs:311`):
- `limit == 0` returns empty.
- `db.flush()` first (so any uncommitted audit batches are durable before scan).
- If `reflex_id` is `Some`, `db.scan_cf_prefix(CF_REFLEX_AUDIT, b"<reflex_id>:")`; else `db.scan_cf(CF_REFLEX_AUDIT)`.
- Decode each value as `StoredReflexAudit`. Sort by `(ts_ns desc, audit_id desc, reflex_id desc)`. Truncate to `limit`.

### 3.5 Health-feeder accessors

| Method | Source | Returned |
|---|---|---|
| `storage_path()` | `lib.rs:361` | `&Path` for `Db.path` |
| `schema_version()` | `lib.rs:368` | `u32` (= `synapse_core::SCHEMA_VERSION`) |
| `storage_pressure_level()` | `lib.rs:375` | `DiskPressureLevel` |
| `storage_cf_sizes()` | `lib.rs:385` | `BTreeMap<String, u64>` from `Db::cf_sizes` |
| `active_count()` | `lib.rs:392` | count of statuses with state `Active` |
| `last_tick_jitter_us()` | `lib.rs:402` | `Option<u64>` from latest `TickSample` |
| `degraded_latency()` | `lib.rs:411` | true if the last sample was `degraded || late` |
| `recursion_clamps_total()` | `lib.rs:424` | counts audit rows whose `error_code == REFLEX_RECURSION_LIMIT` |
| `action_handle()` | `lib.rs:447` | `&ActionHandle` |
| `event_bus()` | `lib.rs:455` | `&EventBus` |

## 4. Audit persistence

`audit.rs::write_audit(db, &StoredReflexAudit)` JSON-encodes the audit and writes one row into `CF_REFLEX_AUDIT` keyed by `"{reflex_id}:{audit_id}"`. The audit_id is a fresh uuid v7, so per-reflex prefix iteration returns rows in registration order.

Audit kinds emitted by the runtime/scheduler (the discriminant is `details.kind` inside the audit payload):

| Kind constant | Emitter | Pairs with status | Error code |
|---|---|---|---|
| `REFLEX_REGISTERED_KIND = "reflex_registered"` | `ReflexRuntime::register` | `Active` | — |
| `REFLEX_CANCELLED_KIND = "reflex_cancelled"` | `ReflexRuntime::cancel` | `Cancelled` | — |
| `REFLEX_DISABLED_KIND = "reflex_disabled_by_operator"` | `ReflexRuntime::disable_all_by_operator` | `Disabled` | `REFLEX_DISABLED_BY_OPERATOR` |
| `REFLEX_FIRED_KIND = "reflex_fired"` | `kinds::on_event::publish_fired` | `Active` | — |
| `REFLEX_RECURSION_LIMIT_KIND = "reflex_recursion_limit"` | `OnEventTickGuard::report_limit_once` | `Active` | `REFLEX_RECURSION_LIMIT` |
| `REFLEX_TICK_LATE_KIND = "reflex_tick_late"` | scheduler when `Δ > late_after` | `Active` | — |
| `REFLEX_STARVED_KIND = "reflex_starved"` | conflict resolver after `STARVATION_AFTER` ticks | `Starved` | `REFLEX_STARVED` |
| `REFLEX_LIFETIME_EXPIRED_KIND = "reflex_lifetime_expired"` | hold-lifetime check | `Expired` | `REFLEX_LIFETIME_EXPIRED` |
| `REFLEX_TRACK_LOST_KIND` | aim-track after `TRACK_LOST_AFTER` of no resolution | `Active` (still) but emits event | `REFLEX_TRACK_LOST` |
| `REFLEX_COMBO_COMPLETED_KIND` | combo controller after last step | `Active` (or `Expired` for OneShot lifetime) | — |

## 5. Event bus (`bus.rs`)

```rust
pub struct EventBus { inner: Arc<EventBusInner> }
struct EventBusInner {
    subscribers: ArcSwap<Vec<Arc<Subscriber>>>,
    updates: Mutex<()>,
    max_subscriptions: NonZeroUsize,
}
struct Subscriber {
    id: SubscriptionId,
    filter: EventFilter,
    kinds: BTreeSet<String>,
    sender: Sender<Event>,
    receiver: Receiver<Event>,
    lossy: Arc<AtomicBool>,
    dropped_since_read: Arc<AtomicU64>,
}
```

Subscribers use a **per-subscriber bounded crossbeam channel**. When a publisher's `try_send` fills the channel, it sets the `lossy` flag and increments `dropped_since_read`, then drops the event. Subscribers read non-blockingly via `SubscriberHandle::drain()` / `take_dropped_since_read()` / `take_lossy()` from the HTTP SSE state (`crates/synapse-mcp/src/http/sse.rs::sync_subscription`).

Constants:

| Constant | Value | Purpose |
|---|---|---|
| `SUBSCRIBER_QUEUE_CAPACITY` | `4096` | Per-subscriber bounded channel size |
| `DEFAULT_MAX_SUBSCRIPTIONS` | `64` | Default cap on simultaneous subscribers |
| `EVENTS_DROPPED_METRIC` | `"events_dropped_for_subscriber"` | Prometheus counter name |

`EventBus::subscribe(filter, kinds, snapshot_first) -> EventBusResult<SubscriberHandle>`:
1. Validate the filter via `EventFilter::validate()` → `EventBusError::FilterInvalid` on failure.
2. Check `subscribers.load().len() < max_subscriptions` → `SubscriptionCapReached { limit }`.
3. Build a bounded channel, register the new subscriber.

`EventBus::publish(Event) -> PublishReport`:
- Snapshot the `ArcSwap` once, iterate, evaluate `EventFilter::matches(event)`, then check the per-subscriber `kinds` allow-list.
- Per match: increment `matched`. `try_send` → if Ok, `queued += 1`; if `TrySendError::Full`, increment `dropped_since_read` + `lossy.store(true)` + `dropped += 1`; if `Disconnected`, schedule unsubscribe.

`EventBus::unsubscribe(id)` re-snapshots and replaces with the filtered vec under `updates` guard.

## 6. Reflex scheduler

`scheduler.rs` orchestrates ticks across all registered reflexes on a dedicated thread (Windows: `scheduler_windows.rs` raises priority to `TIME_CRITICAL`; portable path: `scheduler_tick.rs`).

### 6.1 `SchedulerConfig`

| Field | Default | Constraint |
|---|---|---|
| `target_interval` | `1 ms` | non-zero |
| `fallback_interval` | `2 ms` | non-zero |
| `late_after` | `target_interval * 2` | — |
| `sample_limit` | `4096` | non-zero |
| `max_ticks` | `None` | tests can bound the loop |
| `force_degraded` | `false` | when true, scheduler reports `degraded=true` on every sample regardless |

### 6.2 Per-tick algorithm (`scheduler_tick::tick`)

For each scheduled tick:

1. Capture `now = Instant::now()`. Record `jitter_us = (actual_interval - target_interval).max(0).as_micros() as u64` (capped at `u64::MAX`).
2. Flag `late = (actual_interval > late_after)`. Flag `degraded = late || config.force_degraded`.
3. Build an `OnEventTickGuard` to enforce `MAX_ON_EVENT_FIRINGS_PER_TICK = 4` across all `OnEvent` reflexes for this tick.
4. For each active reflex (in priority order, ascending):
   - Call its driver (`AimTrackController::step`, `HoldMoveController::step`, `HoldButtonController::step`, `ComboController::step`, or the `OnEvent` resolver). Each step returns an emission set (`Vec<Action>`), state updates, and lifecycle decisions.
   - Apply the conflict resolver (`conflict.rs`): if two reflexes both want to emit to the same exclusive resource and one has lower priority, mark the loser `Starved`. After `STARVATION_AFTER` ticks of consecutive loss, persist a `REFLEX_STARVED` audit row and flip state to `Starved`.
   - Lifetime check (`hold_lifetime.rs`): `Duration { ms }` and `UntilDeadline { ms }` use the registration time; `OneShot` expires after the first fire; `UntilEvent { filter }` watches the inbound stream; expiry persists `REFLEX_LIFETIME_EXPIRED` audit.
   - For successful fires, push the actions into `action_handle` (the shared M2 emitter producer).
5. Push a `TickSample { jitter_us, degraded, late, t: now }` into the bounded sample ring (`sample_limit`). If `late`, persist a `REFLEX_TICK_LATE_KIND` audit row tagged with the offending reflex (if any).
6. Update Prometheus metrics: `reflex_fires_total{kind, reflex_id}`, `reflex_tick_jitter_us` histogram, `reflex_recursion_clamps_total`, `reflex_starved_total`.

`TickSample` + `p99_jitter_us(samples: &[TickSample]) -> u64` is the basis for `synapse-mcp` reporting `degraded_latency` and the `REFERENCE_REFLEX_TICK_JITTER_IDLE_P99_US = 200` budget tested in `crates/synapse-reflex/benches/reflex_tick_jitter_idle.rs`.

### 6.3 Trigger types

`SchedulerTrigger` (declared inside `scheduler.rs`) drives when a reflex's controller is invoked:

| Trigger | Behavior |
|---|---|
| Every-tick (e.g., `AimTrack`, `HoldMove`, `HoldButton`) | Step runs every tick |
| Combo step deadline | `scheduler_combo.rs` fires next combo step at `at_ms` after start |
| Event-bus subscriber (`OnEvent`) | Drains the per-reflex `SubscriberHandle` and runs the `then` actions for each matching event up to the per-tick guard |

### 6.4 ScheduledReflex shape

```rust
pub struct ScheduledReflex {
    pub reflex_id: ReflexId,
    pub trigger: SchedulerTrigger,
    pub then: Vec<Action>,            // canonical action list compiled from ReflexKind
    pub driver: ScheduledReflexDriver,// holds the controller state
    pub priority: u32,
    pub lifetime: ReflexLifetime,
    pub exclusive: bool,
}
```

`ScheduledReflex::on_event(reflex_id, EventFilter, actions)` is a convenience constructor used in `lib.rs` tests.

## 7. Reflex kinds

### 7.1 `AimTrack` (`kinds/aim_track.rs`)

- Constants: `DEFAULT_EMA_ALPHA`, `DEFAULT_MAX_SPEED_PX_PER_TICK`, `TRACK_LOST_AFTER`, `REFLEX_TRACK_LOST_KIND`.
- Maintains a smoothed target position (EMA with `DEFAULT_EMA_ALPHA`) and per-tick step toward it bounded by `max_speed_px_per_tick`.
- Axis lock (`Xy` / `XOnly` / `YOnly`) clamps off-axis deltas to zero.
- `deadzone_px` near the target suppresses output entirely.
- If the target resolver (`AimTrackTarget::Element` looks up via UIA on the action thread) cannot find the element for `TRACK_LOST_AFTER` consecutive ticks, publishes a `REFLEX_TRACK_LOST_KIND` event and records `REFLEX_TRACK_LOST` audit but does not cancel the reflex (lifetime semantics still apply).

### 7.2 `HoldMove` (`kinds/hold_move.rs`)

- Drives a held set of keys through `Action::KeyDown` followed by `Action::KeyUp` at expiry.
- `re_assert: bool` re-issues a `KeyDown` per tick (used for game engines that drop holds during scene loads).
- Phase: `Pressing` / `Holding` / `Releasing`.

### 7.3 `HoldButton` (`kinds/hold_button.rs`)

- Same shape but for `MouseButton` or `PadButton` targets via `ReflexButtonTarget`.

### 7.4 `Combo` (`kinds/combo.rs`)

- Owns the `Vec<ComboStep>` with `at_ms` offsets.
- Phase progression: `NotStarted` → `Running` → `Completed`.
- At each tick the controller compares `now - start_at` against the next step's `at_ms` and emits any due steps in order.
- On the final emission, publishes a `REFLEX_COMBO_COMPLETED_KIND` event. `OneShot` lifetime then marks the reflex `Expired`.

### 7.5 `OnEvent` (`kinds/on_event.rs`)

- Subscribes to the event bus with the reflex's `EventFilter`.
- Drains matched events each tick (subject to `OnEventTickGuard` cap of `MAX_ON_EVENT_FIRINGS_PER_TICK = 4`).
- For each fired event:
  - Publishes a `REFLEX_FIRED_KIND` event on the bus (with `EventSource::Reflex`, `data = { reflex_id, tick_index, trigger_event_id, actions: [...] }`).
  - Pushes the `then` actions into the action emitter.
  - Persists a `reflex_fired` audit row with `steps` populated from the action list.
- Debounce: `debounce_ms` between fires per-reflex via `OnEventState::allows_fire`.
- Recursion guard: if a single tick exceeds `MAX_ON_EVENT_FIRINGS_PER_TICK`, the guard publishes a `REFLEX_RECURSION_LIMIT_KIND` event and persists a `REFLEX_RECURSION_LIMIT` audit row exactly once per tick. Further events for that reflex are dropped this tick.

## 8. Error mapping (`error.rs`)

`ReflexError::code()` returns:

| Variant | Code |
|---|---|
| `CapReached { .. }` | `REFLEX_CAP_REACHED` |
| `KindInvalid { .. }` | `REFLEX_KIND_INVALID` |
| `ParamsInvalid { .. }` | `REFLEX_PARAMS_INVALID` |
| `TargetInvalid { .. }` | `REFLEX_TARGET_INVALID` |
| `FilterInvalid { .. }` | `REFLEX_FILTER_INVALID` |
| `PriorityInvalid { .. }` | `REFLEX_PRIORITY_INVALID` |
| `DisabledByOperator { .. }` | `REFLEX_DISABLED_BY_OPERATOR` |
| `Storage(error)` (forwards `synapse_storage::StorageError`) | uses inner code |
| (additional internal variants — see source) | per-variant mapping |

## 9. Integration with the rest of the daemon

| Edge | Direction | Mechanism |
|---|---|---|
| `SynapseService` → `ReflexRuntime` | sync (mcp tool calls) | `reflex_runtime()` helper lazily opens RocksDB + spawns runtime; `ensure_a11y_event_bridge` plumbs UIA events into the bus |
| `ReflexRuntime` → `synapse-action::ActionHandle` | async producer (mpsc) | `ActionHandle::execute` / `try_execute` push `Action` messages with the standard token-bucket rate limit applied downstream |
| `ReflexRuntime` → `EventBus` | sync `publish` | fire/starved/recursion/tick-late events |
| `EventBus` → SSE | per-subscription bounded channel | `crates/synapse-mcp/src/http/sse.rs::sync_subscription` |
| `ReflexRuntime` → `synapse-storage::Db` | sync `put_batch` + `flush` | every register/cancel/disable/fire writes one `StoredReflexAudit` to `CF_REFLEX_AUDIT` |
| Operator hotkey → `ReflexRuntime::disable_all_by_operator` | callback on hook thread | `crates/synapse-mcp/src/safety.rs::handle_operator_hotkey` |

## 10. Observability constants and metrics

The reflex subsystem feeds these metrics (defined in `crates/synapse-telemetry/src/metrics.rs`):

| Metric | Kind | Labels | Description |
|---|---|---|---|
| `events_published_total` | counter | `source`, `kind` | every `EventBus::publish` |
| `events_dropped_for_subscriber` | counter | `subscription_id` | per-subscriber overflow |
| `reflex_fires_total` | counter | `kind`, `reflex_id` | scheduler fires |
| `reflex_tick_jitter_us` | histogram | — | scheduler tick jitter |
| `reflex_recursion_clamps_total` | counter | — | OnEventTickGuard clamps |
| `reflex_starved_total` | counter | `reflex_id` | starvation events |

## 11. What is NOT covered

- **No public scheduler tick API.** External callers cannot drive ticks manually outside test harnesses (`max_ticks` in `SchedulerConfig` is the only test affordance).
- **No reflex priority migration.** Cancel-then-re-register is the only way to change priority for a live reflex.
- **No cross-process bus.** `EventBus` is in-memory; the only cross-process delivery channel is HTTP SSE.
- **No transactional batching across reflexes.** Each fire writes its own audit row + flushes; concurrent registers serialize via the `Mutex<ReflexRuntime>` held by the M3 state.
