# ADR-0003: Reflex Recursion Guard

## Context
M3 `on_event` reflexes can publish reflex events after firing. Chaining those
events is useful, but an `on_event` filter that matches events produced by
reflex execution can otherwise loop forever inside the scheduler.

OQ-022 and #291 require a bounded same-tick guard for this failure mode. #339's
original body used "per reflex", while OQ-022 and #291 specify the stricter
shared limit. The shared limit is the implemented behavior.

## Decision
The scheduler allows at most four successful `on_event` firings per tick across
all active reflexes. A fifth matching event in the same tick is skipped, the
remaining event-driven firings are deferred until the next tick, and the runtime
emits a `REFLEX_RECURSION_LIMIT` signal. When an audit database is configured,
the skipped fifth firing also writes a `CF_REFLEX_AUDIT` row with
`error_code = "REFLEX_RECURSION_LIMIT"`.

## Rationale
Four firings allows short composite chains while preventing unbounded loops from
monopolizing the 1 ms scheduler tick. A shared per-tick cap is easier to reason
about than a per-reflex cap because it bounds total scheduler work, not just
individual reflex work.

## Alternatives Considered
- Per-reflex limit - rejected because many mutually-triggering reflexes could
  still exceed the tick budget.
- Disable reflex-originated events - rejected because reflex composition is a
  core M3 capability.
- Unlimited chaining with operator cancellation - rejected because the operator
  hotkey is a last resort, not a normal control-flow guard.

## Consequences
- Positive: recursive `on_event` loops have a deterministic per-tick ceiling.
- Positive: audit rows and event-bus signals expose the suppressed firing.
- Negative: long same-tick chains beyond four links must resume on a later tick.
- Trade-off accepted: reflex composition stays powerful but bounded.

## References
- Issue: #291
- Issue: #339
- Open question: OQ-022
- Doctrine: #351
