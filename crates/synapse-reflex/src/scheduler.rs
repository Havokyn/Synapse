use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use synapse_action::ActionHandle;
use synapse_core::{Action, EventFilter, ReflexId};
use synapse_storage::Db;

use crate::{
    EventBus, SubscriberHandle,
    error::{ReflexError, ReflexResult},
    kinds::{combo::ComboController, on_event::OnEventState},
};
use scheduler_tick::tick;

pub const MAX_SCHEDULED_REFLEXES: usize = 32;
pub const REFLEX_TICK_LATE_KIND: &str = "reflex_tick_late";
pub const DEFAULT_SAMPLE_LIMIT: usize = 4096;

#[derive(Clone, Debug)]
pub struct SchedulerConfig {
    pub target_interval: Duration,
    pub fallback_interval: Duration,
    pub late_after: Duration,
    pub sample_limit: usize,
    pub max_ticks: Option<u64>,
    pub force_degraded: bool,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        let target_interval = Duration::from_millis(1);
        Self {
            target_interval,
            fallback_interval: Duration::from_millis(2),
            late_after: target_interval.saturating_mul(2),
            sample_limit: DEFAULT_SAMPLE_LIMIT,
            max_ticks: None,
            force_degraded: false,
        }
    }
}

impl SchedulerConfig {
    #[must_use]
    pub const fn with_max_ticks(mut self, max_ticks: u64) -> Self {
        self.max_ticks = Some(max_ticks);
        self
    }

    fn validate(&self) -> ReflexResult<()> {
        if self.target_interval.is_zero() {
            return Err(ReflexError::ParamsInvalid {
                detail: "scheduler target interval must be non-zero".to_owned(),
            });
        }
        if self.fallback_interval.is_zero() {
            return Err(ReflexError::ParamsInvalid {
                detail: "scheduler fallback interval must be non-zero".to_owned(),
            });
        }
        if self.sample_limit == 0 {
            return Err(ReflexError::ParamsInvalid {
                detail: "scheduler sample limit must be non-zero".to_owned(),
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct ScheduledReflex {
    pub reflex_id: ReflexId,
    pub trigger: SchedulerTrigger,
    pub then: Vec<Action>,
    pub priority: u32,
    pub debounce: Duration,
}

impl ScheduledReflex {
    #[must_use]
    pub fn every_tick(reflex_id: impl Into<ReflexId>, then: Vec<Action>) -> Self {
        Self {
            reflex_id: reflex_id.into(),
            trigger: SchedulerTrigger::EveryTick,
            then,
            priority: 0,
            debounce: Duration::ZERO,
        }
    }

    #[must_use]
    pub fn on_event(
        reflex_id: impl Into<ReflexId>,
        filter: EventFilter,
        then: Vec<Action>,
    ) -> Self {
        Self {
            reflex_id: reflex_id.into(),
            trigger: SchedulerTrigger::OnEvent(filter),
            then,
            priority: 0,
            debounce: Duration::ZERO,
        }
    }

    #[must_use]
    pub fn on_event_with_debounce(
        reflex_id: impl Into<ReflexId>,
        filter: EventFilter,
        then: Vec<Action>,
        debounce: Duration,
    ) -> Self {
        Self {
            reflex_id: reflex_id.into(),
            trigger: SchedulerTrigger::OnEvent(filter),
            then,
            priority: 0,
            debounce,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SchedulerTrigger {
    EveryTick,
    OnEvent(EventFilter),
}

impl SchedulerTrigger {
    fn validate(&self) -> ReflexResult<()> {
        match self {
            Self::EveryTick => Ok(()),
            Self::OnEvent(filter) => {
                filter
                    .validate()
                    .map_err(|error| ReflexError::FilterInvalid {
                        detail: error.to_string(),
                    })
            }
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct TickSample {
    pub tick_index: u64,
    pub elapsed_us: u64,
    pub jitter_us: u64,
    pub target_us: u64,
    pub pulled_events: usize,
    pub dispatched_actions: usize,
    pub late: bool,
    pub degraded: bool,
}

pub struct SchedulerHandle {
    stop: Arc<AtomicBool>,
    join: Option<thread::JoinHandle<()>>,
    samples: Arc<Mutex<VecDeque<TickSample>>>,
}

impl SchedulerHandle {
    #[must_use]
    pub fn samples(&self) -> Vec<TickSample> {
        lock_samples(&self.samples).iter().copied().collect()
    }

    #[must_use]
    pub fn wait_for_samples(&self, count: usize, timeout: Duration) -> Vec<TickSample> {
        let deadline = Instant::now() + timeout;
        loop {
            let samples = self.samples();
            if samples.len() >= count || Instant::now() >= deadline {
                return samples;
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    /// Stops the scheduler thread.
    ///
    /// # Errors
    ///
    /// Returns an error if the scheduler thread panicked before joining.
    pub fn stop(&mut self) -> ReflexResult<()> {
        self.stop.store(true, Ordering::Release);
        if let Some(join) = self.join.take() {
            join.join().map_err(|error| ReflexError::ParamsInvalid {
                detail: format!("scheduler thread panicked: {error:?}"),
            })?;
        }
        Ok(())
    }
}

impl Drop for SchedulerHandle {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

pub struct ReflexScheduler;

impl ReflexScheduler {
    /// Spawns the dedicated reflex scheduler thread.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid timing config, invalid reflex filters, reflex cap overflow,
    /// event-bus subscription failure, or scheduler thread spawn failure.
    pub fn spawn(
        event_bus: EventBus,
        action_handle: ActionHandle,
        reflexes: Vec<ScheduledReflex>,
        config: SchedulerConfig,
    ) -> ReflexResult<SchedulerHandle> {
        Self::spawn_inner(event_bus, action_handle, reflexes, config, None)
    }

    /// Spawns the scheduler and writes reflex audit rows into `audit_db`.
    ///
    /// # Errors
    ///
    /// Returns the same setup errors as [`Self::spawn`].
    pub fn spawn_with_audit_db(
        event_bus: EventBus,
        action_handle: ActionHandle,
        reflexes: Vec<ScheduledReflex>,
        config: SchedulerConfig,
        audit_db: Arc<Db>,
    ) -> ReflexResult<SchedulerHandle> {
        Self::spawn_inner(event_bus, action_handle, reflexes, config, Some(audit_db))
    }

    fn spawn_inner(
        event_bus: EventBus,
        action_handle: ActionHandle,
        reflexes: Vec<ScheduledReflex>,
        config: SchedulerConfig,
        audit_db: Option<Arc<Db>>,
    ) -> ReflexResult<SchedulerHandle> {
        config.validate()?;
        validate_reflexes(&reflexes)?;
        let subscription = event_bus
            .subscribe(EventFilter::All, Vec::new(), false)
            .map_err(|error| ReflexError::CapReached {
                detail: format!("scheduler event subscription failed: {error}"),
            })?;
        let stop = Arc::new(AtomicBool::new(false));
        let samples = Arc::new(Mutex::new(VecDeque::with_capacity(config.sample_limit)));
        let mut reflexes = reflexes;
        reflexes.sort_by_key(|reflex| std::cmp::Reverse(reflex.priority));
        let on_event_states = reflexes
            .iter()
            .map(|_| OnEventState::default())
            .collect::<Vec<_>>();

        let runtime = RuntimeState {
            event_bus,
            action_handle,
            reflexes,
            active_combos: Vec::new(),
            on_event_states,
            subscription,
            stop: Arc::clone(&stop),
            samples: Arc::clone(&samples),
            config,
            audit_db,
            tick_index: 0,
        };

        let join = thread::Builder::new()
            .name("synapse-reflex-scheduler".to_owned())
            .spawn(move || run_scheduler_thread(runtime))
            .map_err(|error| ReflexError::ParamsInvalid {
                detail: format!("scheduler thread spawn failed: {error}"),
            })?;

        Ok(SchedulerHandle {
            stop,
            join: Some(join),
            samples,
        })
    }
}

struct RuntimeState {
    event_bus: EventBus,
    action_handle: ActionHandle,
    reflexes: Vec<ScheduledReflex>,
    active_combos: Vec<ComboController>,
    on_event_states: Vec<OnEventState>,
    subscription: SubscriberHandle,
    stop: Arc<AtomicBool>,
    samples: Arc<Mutex<VecDeque<TickSample>>>,
    config: SchedulerConfig,
    audit_db: Option<Arc<Db>>,
    tick_index: u64,
}

fn validate_reflexes(reflexes: &[ScheduledReflex]) -> ReflexResult<()> {
    if reflexes.len() > MAX_SCHEDULED_REFLEXES {
        return Err(ReflexError::CapReached {
            detail: format!(
                "scheduler reflex cap {MAX_SCHEDULED_REFLEXES} exceeded by {}",
                reflexes.len()
            ),
        });
    }
    for reflex in reflexes {
        reflex.trigger.validate()?;
    }
    Ok(())
}

#[cfg(windows)]
fn run_scheduler_thread(mut runtime: RuntimeState) {
    if runtime.config.force_degraded {
        run_degraded(runtime, "forced_degraded_config");
        return;
    }

    match windows_timer::WindowsHighResolutionTimer::start(runtime.config.target_interval) {
        Ok(timer) => {
            let mut last = Instant::now();
            while should_tick(&runtime) {
                let deadline = last + runtime.config.target_interval;
                if let Err(error) = timer.wait_until(deadline) {
                    run_degraded(runtime, &error);
                    return;
                }
                let now = Instant::now();
                let elapsed = now.duration_since(last);
                last = now;
                tick(&mut runtime, elapsed, false);
            }
        }
        Err(error) => run_degraded(runtime, &error),
    }
}

#[cfg(not(windows))]
fn run_scheduler_thread(runtime: RuntimeState) {
    run_degraded(
        runtime,
        "high-resolution waitable timer is only available on Windows",
    );
}

fn run_degraded(mut runtime: RuntimeState, reason: &str) {
    tracing::warn!(
        component = "reflex_scheduler",
        degraded = true,
        reason = %reason,
        "reflex scheduler falling back to tokio interval"
    );
    let Ok(tokio_runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
    else {
        tracing::error!(
            component = "reflex_scheduler",
            degraded = true,
            "reflex scheduler could not build fallback tokio runtime"
        );
        return;
    };
    tokio_runtime.block_on(async move {
        let mut interval = tokio::time::interval(runtime.config.fallback_interval);
        interval.tick().await;
        let mut last = Instant::now();
        while should_tick(&runtime) {
            interval.tick().await;
            let now = Instant::now();
            let elapsed = now.duration_since(last);
            last = now;
            tick(&mut runtime, elapsed, true);
        }
    });
}

fn should_tick(runtime: &RuntimeState) -> bool {
    if runtime.stop.load(Ordering::Acquire) {
        return false;
    }
    runtime
        .config
        .max_ticks
        .is_none_or(|max_ticks| runtime.tick_index < max_ticks)
}

fn lock_samples(
    samples: &Arc<Mutex<VecDeque<TickSample>>>,
) -> std::sync::MutexGuard<'_, VecDeque<TickSample>> {
    match samples.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[path = "scheduler_stats.rs"]
mod scheduler_stats;
pub use scheduler_stats::p99_jitter_us;

#[path = "scheduler_combo.rs"]
mod scheduler_combo;

#[path = "scheduler_tick.rs"]
mod scheduler_tick;

#[cfg(windows)]
#[path = "scheduler_windows.rs"]
mod windows_timer;
