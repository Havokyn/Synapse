use std::{error::Error, io, time::Duration};

use serde_json::json;
use synapse_action::{ACTION_QUEUE_CAPACITY, ActionHandle};
use synapse_core::{Action, Event, EventFilter, EventSource, error_codes};
use synapse_reflex::{
    EventBus, REFLEX_TICK_LATE_KIND, ReflexScheduler, ScheduledReflex, SchedulerConfig,
    SchedulerTrigger, p99_jitter_us,
};

const WAIT_TIMEOUT: Duration = Duration::from_secs(3);

#[test]
fn zero_reflexes_tick_fires_without_dispatch_with_fsv() -> Result<(), Box<dyn Error>> {
    let bus = EventBus::default();
    let (action_handle, action_rx) = ActionHandle::channel();
    println!(
        "source_of_truth=reflex_scheduler case=zero_reflexes before=action_queue:{}",
        action_rx.len()
    );

    let mut scheduler = ReflexScheduler::spawn(
        bus,
        action_handle,
        Vec::new(),
        SchedulerConfig::default().with_max_ticks(24),
    )?;
    let samples = scheduler.wait_for_samples(24, WAIT_TIMEOUT);
    scheduler.stop()?;

    let p99 = p99_jitter_us(&samples);
    let last_elapsed = samples.last().map_or(0, |sample| sample.elapsed_us);
    println!(
        "source_of_truth=reflex_scheduler case=zero_reflexes after_truth=samples:{} action_queue:{} p99_jitter_us:{} final_value=elapsed_us={last_elapsed}",
        samples.len(),
        action_rx.len(),
        p99
    );

    assert_eq!(samples.len(), 24);
    assert!(samples.iter().all(|sample| sample.dispatched_actions == 0));
    assert_eq!(action_rx.len(), 0);
    Ok(())
}

#[test]
fn on_event_reflex_pulls_bus_event_and_dispatches_with_fsv() -> Result<(), Box<dyn Error>> {
    let bus = EventBus::default();
    let (action_handle, action_rx) = ActionHandle::channel();
    let reflex = ScheduledReflex::on_event(
        "reflex-on-event",
        EventFilter::Kind {
            kind: "wanted".to_owned(),
        },
        vec![Action::ReleaseAll],
    );
    println!(
        "source_of_truth=reflex_scheduler case=on_event before=action_queue:{}",
        action_rx.len()
    );
    let mut scheduler = ReflexScheduler::spawn(
        bus.clone(),
        action_handle,
        vec![reflex],
        SchedulerConfig::default().with_max_ticks(8),
    )?;
    let _report = bus.publish(event(1, "wanted"));
    let samples = scheduler.wait_for_samples(8, WAIT_TIMEOUT);
    scheduler.stop()?;

    let pulled = samples
        .iter()
        .map(|sample| sample.pulled_events)
        .sum::<usize>();
    let dispatched = samples
        .iter()
        .map(|sample| sample.dispatched_actions)
        .sum::<usize>();
    let last_elapsed = samples.last().map_or(0, |sample| sample.elapsed_us);
    println!(
        "source_of_truth=reflex_scheduler case=on_event after_truth=pulled_events:{pulled} dispatched:{dispatched} action_queue:{} final_value=elapsed_us={last_elapsed}",
        action_rx.len()
    );

    assert!(pulled >= 1);
    assert_eq!(dispatched, 1);
    assert_eq!(action_rx.len(), 1);
    Ok(())
}

#[test]
fn thirty_two_reflexes_fire_same_tick_without_tick_late_with_fsv() -> Result<(), Box<dyn Error>> {
    let bus = EventBus::default();
    let late_events = bus.subscribe(
        EventFilter::Kind {
            kind: REFLEX_TICK_LATE_KIND.to_owned(),
        },
        Vec::new(),
        false,
    )?;
    let (action_handle, action_rx) = ActionHandle::channel();
    let reflexes = (0..32)
        .map(|index| {
            ScheduledReflex::every_tick(format!("reflex-{index:02}"), vec![Action::ReleaseAll])
        })
        .collect::<Vec<_>>();
    println!(
        "source_of_truth=reflex_scheduler case=thirty_two before=reflexes:{} action_queue:{} late_events:{}",
        reflexes.len(),
        action_rx.len(),
        late_events.len()
    );

    let mut scheduler = ReflexScheduler::spawn(
        bus,
        action_handle,
        reflexes,
        SchedulerConfig::default().with_max_ticks(1),
    )?;
    let samples = scheduler.wait_for_samples(1, WAIT_TIMEOUT);
    scheduler.stop()?;

    let late = late_events.drain();
    let Some(sample) = samples.first().copied() else {
        return Err(Box::new(io::Error::other(
            "scheduler did not record the expected tick sample",
        )));
    };
    println!(
        "source_of_truth=reflex_scheduler case=thirty_two after_truth=action_queue:{} late_events:{} sample:{sample:?} final_value=elapsed_us={}",
        action_rx.len(),
        late.len(),
        sample.elapsed_us
    );

    assert_eq!(sample.dispatched_actions, 32);
    assert!(!sample.late);
    assert_eq!(action_rx.len(), 32);
    assert!(late.is_empty());
    Ok(())
}

#[test]
fn blocked_dispatch_path_emits_reflex_tick_late_with_fsv() -> Result<(), Box<dyn Error>> {
    let bus = EventBus::default();
    let late_events = bus.subscribe(
        EventFilter::Kind {
            kind: REFLEX_TICK_LATE_KIND.to_owned(),
        },
        Vec::new(),
        false,
    )?;
    let (action_handle, action_rx) = ActionHandle::channel();
    for _ in 0..ACTION_QUEUE_CAPACITY {
        action_handle.try_execute(Action::ReleaseAll)?;
    }
    println!(
        "source_of_truth=reflex_scheduler case=blocked_dispatch before=action_queue:{} late_events:{}",
        action_rx.len(),
        late_events.len()
    );

    let reflex = ScheduledReflex::every_tick("reflex-blocked", vec![Action::ReleaseAll]);
    let mut scheduler = ReflexScheduler::spawn(
        bus,
        action_handle,
        vec![reflex],
        SchedulerConfig::default().with_max_ticks(1),
    )?;
    let samples = scheduler.wait_for_samples(1, WAIT_TIMEOUT);
    scheduler.stop()?;

    let late = late_events.drain();
    let Some(sample) = samples.first().copied() else {
        return Err(Box::new(io::Error::other(
            "scheduler did not record blocked-dispatch sample",
        )));
    };
    let reasons = late
        .iter()
        .map(|event| event.data["reason"].as_str().unwrap_or("<missing>"))
        .collect::<Vec<_>>();
    println!(
        "source_of_truth=reflex_scheduler case=blocked_dispatch after_truth=action_queue:{} late_events:{} reasons:{reasons:?} sample:{sample:?} final_value=elapsed_us={}",
        action_rx.len(),
        late.len(),
        sample.elapsed_us
    );

    assert_eq!(action_rx.len(), ACTION_QUEUE_CAPACITY);
    assert_eq!(sample.dispatched_actions, 0);
    assert!(sample.late);
    assert_eq!(late.len(), 1);
    assert_eq!(late[0].data["code"], error_codes::REFLEX_TICK_LATE);
    assert_eq!(late[0].data["reason"], "dispatch_blocked");
    Ok(())
}

#[test]
fn scheduler_rejects_invalid_trigger_filter_with_fsv() {
    let bus = EventBus::default();
    let (action_handle, action_rx) = ActionHandle::channel();
    let reflex = ScheduledReflex {
        reflex_id: "reflex-invalid-filter".to_owned(),
        trigger: SchedulerTrigger::OnEvent(EventFilter::And { args: Vec::new() }),
        then: vec![Action::ReleaseAll],
        priority: 0,
    };
    println!(
        "source_of_truth=reflex_scheduler case=invalid_filter before=action_queue:{}",
        action_rx.len()
    );

    let error = match ReflexScheduler::spawn(
        bus,
        action_handle,
        vec![reflex],
        SchedulerConfig::default(),
    ) {
        Ok(_scheduler) => panic!("invalid event filter must prevent scheduler spawn"),
        Err(error) => error,
    };
    println!(
        "source_of_truth=reflex_scheduler case=invalid_filter after_truth=code:{} action_queue:{} final_value={error:?}",
        error.code(),
        action_rx.len()
    );

    assert_eq!(error.code(), error_codes::REFLEX_FILTER_INVALID);
    assert_eq!(action_rx.len(), 0);
}

fn event(seq: u64, kind: &str) -> Event {
    Event {
        seq,
        at: chrono::Utc::now(),
        source: EventSource::System,
        kind: kind.to_owned(),
        data: json!({ "seq": seq, "kind": kind }),
        correlations: Vec::new(),
    }
}
