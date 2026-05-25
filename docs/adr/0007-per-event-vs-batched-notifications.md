# ADR-0007: Per-Event Notifications

## Context

OQ-029 asked whether Synapse should deliver one notification per event or batch
events at the bus/SSE layer. M3 has a 50 ms p99 event-to-subscriber budget and
reflex paths that depend on low-jitter event delivery.

## Decision

Synapse delivers notifications per event. The `EventBus` publishes each
`Event` independently and never waits to form a batch. Each subscriber owns a
bounded queue; slow subscribers use drop-oldest backpressure with explicit
`lossy` state and `events_dropped_for_subscriber` metrics.

HTTP SSE also emits one `synapse/event` frame per buffered event. The SSE
`id`/`stream_seq` is a per-subscription stream sequence, not the domain
`event.seq`. Reconnect uses `Last-Event-ID` against that stream sequence. If
the server detects an overflow or replay gap, it sends a
`subscription_started` frame with `lossy: true` before continuing event
delivery.

Downstream clients may batch after receipt for UI or transport efficiency, but
that batching is outside the EventBus and does not delay producer-to-subscriber
delivery inside Synapse.

## Rationale

The project goal is fast local reaction. Bus-layer batching would trade lower
frame count for latency jitter and would make reflex timing harder to reason
about. A per-event frame maps directly to the SSE event model and gives clients
simple resume semantics through `id` and `Last-Event-ID`.

## Alternatives Considered

- 10 ms bus batches - rejected because batch windows consume meaningful budget
  under the 50 ms p99 event-to-subscriber target.
- SSE frames containing arrays of events - rejected because replay, loss
  marking, and per-event audit correlation become less direct.
- Adaptive batching under load - rejected for M3 because it changes latency
  semantics exactly when the system is stressed.

## Consequences

- Positive: latency and ordering are explainable one event at a time.
- Positive: SSE resume can replay from a single stream sequence without
  unpacking batch offsets.
- Negative: high-rate subscriptions produce more SSE frames.
- Trade-off accepted: consumers that need fewer UI updates or writes can batch
  downstream after receiving the individual events.

## Supersedes

- OQ-029 in `docs/computergames/16_open_questions.md`

## References

- Decision issue: #340
- Event bus: `crates/synapse-reflex/src/bus.rs`
- SSE transport: `crates/synapse-mcp/src/http/sse.rs`
- Performance budget: `docs/computergames/10_performance_budget.md` §2
- SSE model: https://html.spec.whatwg.org/dev/server-sent-events.html
