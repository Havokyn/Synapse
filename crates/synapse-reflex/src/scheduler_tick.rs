use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use chrono::Utc;
use serde_json::json;
use synapse_core::{Action, Event, EventSource, ReflexId, error_codes};

use super::{
    REFLEX_TICK_LATE_KIND, RuntimeState, SchedulerTrigger, TickSample,
    scheduler_combo::{dispatch_reflex_action, step_active_combos},
};
use crate::{
    ReflexError, ReflexResult,
    kinds::on_event::{OnEventTickGuard, publish_fired},
};

pub(super) fn tick(runtime: &mut RuntimeState, elapsed: Duration, degraded: bool) {
    let events = runtime.subscription.drain();
    let mut dispatched_actions = 0_usize;
    let mut dispatch_blocked = false;
    step_active_combos(
        runtime,
        elapsed,
        &mut dispatched_actions,
        &mut dispatch_blocked,
    );

    if !dispatch_blocked {
        dispatch_triggered_reflexes(
            runtime,
            &events,
            &mut dispatched_actions,
            &mut dispatch_blocked,
        );
    }

    record_tick_sample(
        runtime,
        elapsed,
        degraded,
        events.len(),
        dispatched_actions,
        dispatch_blocked,
    );
}

fn dispatch_triggered_reflexes(
    runtime: &mut RuntimeState,
    events: &[Event],
    dispatched_actions: &mut usize,
    dispatch_blocked: &mut bool,
) {
    let now = Instant::now();
    let mut guard = OnEventTickGuard::default();

    'reflexes: for index in 0..runtime.reflexes.len() {
        let (reflex_id, trigger, actions, debounce) = {
            let reflex = &runtime.reflexes[index];
            (
                reflex.reflex_id.clone(),
                reflex.trigger.clone(),
                reflex.then.clone(),
                reflex.debounce,
            )
        };

        match trigger {
            SchedulerTrigger::EveryTick => match dispatch_actions(runtime, &reflex_id, actions) {
                Ok(action_count) => {
                    *dispatched_actions = dispatched_actions.saturating_add(action_count);
                }
                Err(error) => {
                    *dispatch_blocked = true;
                    warn_dispatch_blocked(&reflex_id, &error);
                    break;
                }
            },
            SchedulerTrigger::OnEvent(filter) => {
                for event in events {
                    if !filter.matches(event)
                        || !runtime.on_event_states[index].allows_fire(now, debounce)
                    {
                        continue;
                    }
                    if !guard.can_fire() {
                        guard.report_limit_once(
                            &runtime.event_bus,
                            runtime.audit_db.as_deref(),
                            &reflex_id,
                            runtime.tick_index,
                            event,
                        );
                        break 'reflexes;
                    }
                    match dispatch_actions(runtime, &reflex_id, actions.clone()) {
                        Ok(action_count) => {
                            *dispatched_actions = dispatched_actions.saturating_add(action_count);
                            runtime.on_event_states[index].mark_fired(now);
                            guard.record_fire();
                            publish_fired(
                                &runtime.event_bus,
                                runtime.audit_db.as_deref(),
                                &reflex_id,
                                runtime.tick_index,
                                event,
                                &actions,
                            );
                        }
                        Err(error) => {
                            *dispatch_blocked = true;
                            warn_dispatch_blocked(&reflex_id, &error);
                            break 'reflexes;
                        }
                    }
                }
            }
        }
    }
}

fn dispatch_actions(
    runtime: &mut RuntimeState,
    reflex_id: &ReflexId,
    actions: Vec<Action>,
) -> ReflexResult<usize> {
    let mut dispatched = 0_usize;
    for action in actions {
        let action_count = dispatch_reflex_action(runtime, reflex_id, action)?;
        dispatched = dispatched.saturating_add(action_count);
    }
    Ok(dispatched)
}

fn record_tick_sample(
    runtime: &mut RuntimeState,
    elapsed: Duration,
    degraded: bool,
    event_count: usize,
    dispatched_actions: usize,
    dispatch_blocked: bool,
) {
    let elapsed_us = duration_us(elapsed);
    let target_us = duration_us(runtime.config.target_interval);
    let jitter_us = elapsed_us.abs_diff(target_us);
    let deadline_late = elapsed > runtime.config.late_after;
    let late = deadline_late || dispatch_blocked;
    if late {
        let reason = if dispatch_blocked {
            "dispatch_blocked"
        } else {
            "deadline_miss"
        };
        emit_tick_late(runtime, elapsed_us, jitter_us, reason);
    }

    let sample = TickSample {
        tick_index: runtime.tick_index,
        elapsed_us,
        jitter_us,
        target_us,
        pulled_events: event_count,
        dispatched_actions,
        late,
        degraded,
    };
    tracing::info!(
        component = "reflex_scheduler",
        tick_index = sample.tick_index,
        elapsed_us = sample.elapsed_us,
        jitter_us = sample.jitter_us,
        target_us = sample.target_us,
        pulled_events = sample.pulled_events,
        dispatched_actions = sample.dispatched_actions,
        late = sample.late,
        degraded = sample.degraded,
        "reflex scheduler tick"
    );
    push_sample(&runtime.samples, runtime.config.sample_limit, sample);
    runtime.tick_index = runtime.tick_index.saturating_add(1);
}

fn emit_tick_late(runtime: &RuntimeState, elapsed_us: u64, jitter_us: u64, reason: &str) {
    let event = Event {
        seq: runtime.tick_index,
        at: Utc::now(),
        source: EventSource::Reflex,
        kind: REFLEX_TICK_LATE_KIND.to_owned(),
        data: json!({
            "code": error_codes::REFLEX_TICK_LATE,
            "elapsed_us": elapsed_us,
            "jitter_us": jitter_us,
            "target_us": duration_us(runtime.config.target_interval),
            "reason": reason,
        }),
        correlations: Vec::new(),
    };
    let _report = runtime.event_bus.publish(event);
}

fn warn_dispatch_blocked(reflex_id: &ReflexId, error: &ReflexError) {
    tracing::warn!(
        component = "reflex_scheduler",
        reflex_id = %reflex_id,
        error_code = error.code(),
        detail = %error,
        "reflex action dispatch blocked"
    );
}

fn push_sample(
    samples: &Arc<Mutex<VecDeque<TickSample>>>,
    sample_limit: usize,
    sample: TickSample,
) {
    let mut samples = lock_samples(samples);
    if samples.len() >= sample_limit {
        let _oldest = samples.pop_front();
    }
    samples.push_back(sample);
}

fn lock_samples(
    samples: &Arc<Mutex<VecDeque<TickSample>>>,
) -> std::sync::MutexGuard<'_, VecDeque<TickSample>> {
    match samples.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn duration_us(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}
