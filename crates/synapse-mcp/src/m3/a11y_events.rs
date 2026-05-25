use std::fmt;

use chrono::Utc;
use serde_json::json;
use synapse_a11y::{AccessibleEvent, AccessibleEventKind, WinEventSubscription};
use synapse_core::{Event, EventSource, ForegroundContext};
use synapse_reflex::EventBus;
use tokio::{sync::mpsc::UnboundedReceiver, task::JoinHandle};

pub struct A11yEventBridge {
    _subscription: WinEventSubscription,
    task: JoinHandle<()>,
}

impl A11yEventBridge {
    pub(crate) fn start(event_bus: EventBus) -> synapse_a11y::A11yResult<Self> {
        let (sender, receiver) = tokio::sync::mpsc::unbounded_channel();
        let subscription = synapse_a11y::subscribe_win_events(sender)?;
        let task = tokio::spawn(run_bridge(event_bus, receiver));
        tracing::info!(
            code = "M3_A11Y_EVENT_BRIDGE_STARTED",
            thread_id = subscription.readback().thread_id,
            hook_count = subscription.readback().hook_count,
            event_ids = ?subscription.readback().event_ids,
            "M3 a11y WinEvent bridge started"
        );
        Ok(Self {
            _subscription: subscription,
            task,
        })
    }
}

impl fmt::Debug for A11yEventBridge {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("A11yEventBridge")
            .field("running", &!self.task.is_finished())
            .finish_non_exhaustive()
    }
}

impl Drop for A11yEventBridge {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn run_bridge(event_bus: EventBus, mut receiver: UnboundedReceiver<AccessibleEvent>) {
    let mut next_seq = 1_u64;
    while let Some(accessible_event) = receiver.recv().await {
        let event = event_from_accessible(&accessible_event, next_seq);
        next_seq = next_seq.saturating_add(1);
        let report = event_bus.publish(event.clone());
        tracing::debug!(
            code = "M3_A11Y_EVENT_PUBLISHED",
            seq = event.seq,
            kind = %event.kind,
            window_title = %event
                .data
                .get("window_title")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default(),
            matched = report.matched,
            queued = report.queued,
            dropped = report.dropped,
            "M3 a11y event published"
        );
    }
    tracing::info!(
        code = "M3_A11Y_EVENT_BRIDGE_STOPPED",
        "M3 a11y WinEvent bridge stopped"
    );
}

fn event_from_accessible(accessible_event: &AccessibleEvent, seq: u64) -> Event {
    let foreground = event_foreground_context(accessible_event);
    let element_id = accessible_event
        .element_id
        .as_ref()
        .map(ToString::to_string);
    let window_title = foreground
        .as_ref()
        .map(|context| context.window_title.clone())
        .unwrap_or_default();
    let process_name = foreground
        .as_ref()
        .map(|context| context.process_name.clone())
        .unwrap_or_default();
    let pid = foreground.as_ref().map_or(0, |context| context.pid);
    let foreground_window_id = foreground.as_ref().map(|context| context.hwnd);

    Event {
        seq,
        at: Utc::now(),
        source: EventSource::A11yWinEvent,
        kind: event_kind_name(accessible_event.kind).to_owned(),
        data: json!({
            "window_id": accessible_event.window_id,
            "foreground_window_id": foreground_window_id,
            "element_id": element_id,
            "event_kind": accessible_event.kind,
            "window_title": window_title,
            "process_name": process_name,
            "pid": pid,
            "name": accessible_event.name.clone(),
            "value": accessible_event.value.clone(),
            "win_event_seq": accessible_event.seq,
            "win_event_at_ms": accessible_event.at_ms,
        }),
        correlations: Vec::new(),
    }
}

fn event_foreground_context(accessible_event: &AccessibleEvent) -> Option<ForegroundContext> {
    let event_context = synapse_a11y::foreground_context(accessible_event.window_id).ok();
    if event_context
        .as_ref()
        .is_some_and(|context| !context.window_title.trim().is_empty())
    {
        return event_context;
    }

    synapse_a11y::current_foreground_context()
        .ok()
        .or(event_context)
}

const fn event_kind_name(kind: AccessibleEventKind) -> &'static str {
    match kind {
        AccessibleEventKind::ForegroundChanged => "foreground-changed",
        AccessibleEventKind::FocusChanged => "focus-changed",
        AccessibleEventKind::ValueChanged => "value-changed",
        AccessibleEventKind::NameChanged => "name-changed",
        AccessibleEventKind::ElementAppeared => "element-appeared",
        AccessibleEventKind::ElementDisappeared => "element-disappeared",
        AccessibleEventKind::SelectionChanged => "selection-changed",
        AccessibleEventKind::MenuStart => "menustart",
        AccessibleEventKind::MenuEnd => "menuend",
        AccessibleEventKind::Alert => "alert",
    }
}
