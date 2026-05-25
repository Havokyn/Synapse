use std::{error::Error, time::Duration};

use synapse_action::{ActionHandle, ActionMessage};
use synapse_core::{Action, Backend, ComboInput, ComboStep, EventFilter, Key, KeyCode};
use synapse_reflex::{
    ComboContext, ComboController, ComboOutput, ComboParams, ComboPhase, EventBus,
    REFLEX_COMBO_COMPLETED_KIND, ReflexScheduler, ScheduledReflex, SchedulerConfig,
};
use tokio::sync::mpsc;

#[test]
fn combo_keypress_steps_dispatch_at_due_offsets() -> Result<(), Box<dyn Error>> {
    let key_a = named_key("a");
    let key_b = named_key("b");
    let mut controller = ComboController::new(
        "combo-keypress",
        ComboParams::new(
            vec![
                ComboStep {
                    at_ms: 0,
                    input: ComboInput::KeyPress {
                        key: key_a.clone(),
                        hold_ms: 33,
                    },
                },
                ComboStep {
                    at_ms: 100,
                    input: ComboInput::KeyPress {
                        key: key_b.clone(),
                        hold_ms: 33,
                    },
                },
            ],
            Backend::Software,
        ),
    );
    let bus = EventBus::default();
    let (handle, mut rx) = ActionHandle::channel();

    assert!(drain(&mut rx).is_empty());
    assert_eq!(
        controller.start_dispatch(&handle, &bus)?,
        ComboOutput::Started {
            actions: 1,
            remaining: 3
        }
    );
    assert_eq!(
        drain(&mut rx),
        vec![Action::KeyDown {
            key: key_a.clone(),
            backend: Backend::Software,
        }]
    );

    assert_eq!(
        controller.step_dispatch(&context(33), &handle, &bus)?,
        ComboOutput::Dispatched {
            actions: 1,
            elapsed_ms: 33,
            remaining: 2
        }
    );
    assert_eq!(
        drain(&mut rx),
        vec![Action::KeyUp {
            key: key_a,
            backend: Backend::Software,
        }]
    );

    assert_eq!(
        controller.step_dispatch(&context(67), &handle, &bus)?,
        ComboOutput::Dispatched {
            actions: 1,
            elapsed_ms: 100,
            remaining: 1
        }
    );
    assert_eq!(
        drain(&mut rx),
        vec![Action::KeyDown {
            key: key_b.clone(),
            backend: Backend::Software,
        }]
    );

    assert_eq!(
        controller.step_dispatch(&context(33), &handle, &bus)?,
        ComboOutput::Completed {
            scheduled_actions: 4,
            dispatched_actions: 4,
            actions: 1
        }
    );
    assert_eq!(
        drain(&mut rx),
        vec![Action::KeyUp {
            key: key_b,
            backend: Backend::Software,
        }]
    );
    assert_eq!(controller.phase(), ComboPhase::Completed);
    Ok(())
}

#[test]
fn combo_empty_steps_complete_without_dispatch_and_emit_audit() -> Result<(), Box<dyn Error>> {
    let bus = EventBus::default();
    let subscriber = bus.subscribe(
        EventFilter::Kind {
            kind: REFLEX_COMBO_COMPLETED_KIND.to_owned(),
        },
        Vec::new(),
        false,
    )?;
    let mut controller = ComboController::new(
        "combo-empty",
        ComboParams::new(Vec::new(), Backend::Software),
    );
    let (handle, mut rx) = ActionHandle::channel();

    assert_eq!(
        controller.start_dispatch(&handle, &bus)?,
        ComboOutput::Completed {
            scheduled_actions: 0,
            dispatched_actions: 0,
            actions: 0
        }
    );
    assert!(drain(&mut rx).is_empty());
    let events = subscriber.drain();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].data["status"], "completed");
    assert_eq!(events[0].data["scheduled_actions"], 0);
    assert_eq!(events[0].data["dispatched_actions"], 0);
    Ok(())
}

#[test]
fn combo_single_step_dispatches_one_primitive_action() -> Result<(), Box<dyn Error>> {
    let mut controller = ComboController::new(
        "combo-single",
        ComboParams::new(
            vec![ComboStep {
                at_ms: 0,
                input: ComboInput::MouseMoveRel { dx: 3.0, dy: -2.0 },
            }],
            Backend::Software,
        ),
    );
    let bus = EventBus::default();
    let (handle, mut rx) = ActionHandle::channel();

    assert_eq!(
        controller.start_dispatch(&handle, &bus)?,
        ComboOutput::Completed {
            scheduled_actions: 1,
            dispatched_actions: 1,
            actions: 1
        }
    );
    assert_eq!(
        drain(&mut rx),
        vec![Action::MouseMoveRelative {
            dx: 3.0,
            dy: -2.0,
            backend: Backend::Software,
        }]
    );
    Ok(())
}

#[test]
fn combo_hundred_steps_fire_in_due_order() -> Result<(), Box<dyn Error>> {
    let steps = (0..100_u16)
        .map(|index| ComboStep {
            at_ms: u32::from(index),
            input: ComboInput::KeyDown {
                key: named_key(&format!("k{index:03}")),
            },
        })
        .collect::<Vec<_>>();
    let mut controller =
        ComboController::new("combo-hundred", ComboParams::new(steps, Backend::Software));
    let bus = EventBus::default();
    let (handle, mut rx) = ActionHandle::channel();

    controller.start_dispatch(&handle, &bus)?;
    assert_eq!(
        drain(&mut rx),
        vec![Action::KeyDown {
            key: named_key("k000"),
            backend: Backend::Software,
        }]
    );
    assert_eq!(
        controller.step_dispatch(&context(99), &handle, &bus)?,
        ComboOutput::Completed {
            scheduled_actions: 100,
            dispatched_actions: 100,
            actions: 99
        }
    );

    let observed = drain(&mut rx);
    assert_eq!(observed.len(), 99);
    for (offset, action) in observed.iter().enumerate() {
        assert_eq!(
            action,
            &Action::KeyDown {
                key: named_key(&format!("k{:03}", offset + 1)),
                backend: Backend::Software,
            }
        );
    }
    Ok(())
}

#[test]
fn scheduler_starts_combo_actions_when_trigger_fires() -> Result<(), Box<dyn Error>> {
    let bus = EventBus::default();
    let (action_handle, mut action_rx) = ActionHandle::channel();
    let key = named_key("s");
    let reflex = ScheduledReflex::on_event(
        "scheduler-combo",
        EventFilter::Kind {
            kind: "combo-trigger".to_owned(),
        },
        vec![Action::Combo {
            steps: vec![ComboStep {
                at_ms: 0,
                input: ComboInput::KeyDown { key: key.clone() },
            }],
            backend: Backend::Software,
        }],
    );

    let mut scheduler = ReflexScheduler::spawn(
        bus.clone(),
        action_handle,
        vec![reflex],
        SchedulerConfig::default().with_max_ticks(8),
    )?;
    let _report = bus.publish(synapse_core::Event {
        seq: 1,
        at: chrono::Utc::now(),
        source: synapse_core::EventSource::System,
        kind: "combo-trigger".to_owned(),
        data: serde_json::json!({ "case": "combo" }),
        correlations: Vec::new(),
    });
    let samples = scheduler.wait_for_samples(8, Duration::from_secs(3));
    scheduler.stop()?;

    assert_eq!(
        drain(&mut action_rx),
        vec![Action::KeyDown {
            key,
            backend: Backend::Software,
        }]
    );
    assert!(samples.iter().any(|sample| sample.dispatched_actions == 1));
    Ok(())
}

const fn context(tick_ms: u64) -> ComboContext {
    ComboContext {
        tick_elapsed: Duration::from_millis(tick_ms),
    }
}

fn drain(rx: &mut mpsc::Receiver<ActionMessage>) -> Vec<Action> {
    let mut actions = Vec::new();
    while let Ok((action, _ack)) = rx.try_recv() {
        actions.push(action);
    }
    actions
}

fn named_key(value: &str) -> Key {
    Key {
        code: KeyCode::Named {
            value: value.to_owned(),
        },
        use_scancode: false,
    }
}
