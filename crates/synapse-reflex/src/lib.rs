pub mod audit;
pub mod bus;
pub mod error;
pub mod kinds;
pub mod scheduler;

use std::{path::Path, sync::Arc};

use synapse_action::ActionHandle;
use synapse_storage::Db;

pub use audit::write_audit;
pub use bus::{
    DEFAULT_MAX_SUBSCRIPTIONS, EVENTS_DROPPED_METRIC, EventBus, EventBusError, EventBusResult,
    PublishReport, SUBSCRIBER_QUEUE_CAPACITY, SubscriberHandle,
};
pub use error::{ReflexError, ReflexResult};
pub use kinds::aim_track::{
    AimTrackContext, AimTrackController, AimTrackOutput, AimTrackParams, AimTrackTarget,
    DEFAULT_EMA_ALPHA, DEFAULT_MAX_SPEED_PX_PER_TICK, REFLEX_TRACK_LOST_KIND, ResolvedElementBox,
    TRACK_LOST_AFTER,
};
pub use kinds::combo::{
    ComboContext, ComboController, ComboOutput, ComboParams, ComboPhase,
    REFLEX_COMBO_COMPLETED_KIND,
};
pub use kinds::hold_button::{
    HoldButtonController, HoldButtonOutput, HoldButtonParams, HoldButtonPhase,
};
pub use kinds::hold_lifetime::{
    HoldLifetimeContext, HoldReleaseReason, REFLEX_LIFETIME_EXPIRED_KIND,
};
pub use kinds::hold_move::{HoldMoveController, HoldMoveOutput, HoldMoveParams, HoldMovePhase};
pub use kinds::on_event::{
    MAX_ON_EVENT_FIRINGS_PER_TICK, REFLEX_FIRED_KIND, REFLEX_RECURSION_LIMIT_KIND,
};
pub use scheduler::{
    MAX_SCHEDULED_REFLEXES, REFLEX_TICK_LATE_KIND, ReflexScheduler, ScheduledReflex,
    SchedulerConfig, SchedulerHandle, SchedulerTrigger, TickSample, p99_jitter_us,
};

/// Runtime handle for the M3 reflex subsystem.
///
/// Reflex input controllers use the shared [`synapse_action::ActionHandle`] as
/// the `synapse-action::handle` interlock authority. Held input state remains
/// owned by the private `synapse-action` emitter `BitSet`; reflex must enqueue
/// `hold_*` down/up actions through this handle and must not mirror, read, or
/// mutate held state independently.
#[derive(Debug)]
pub struct ReflexRuntime {
    db: Arc<Db>,
    action_handle: ActionHandle,
    event_bus: EventBus,
}

impl ReflexRuntime {
    /// Spawns the reflex runtime scaffold.
    ///
    /// # Errors
    ///
    /// The scaffold currently cannot fail after receiving initialized handles.
    /// Later M3 scheduler/bus work extends this result with OS-thread setup
    /// errors.
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime"))]
    pub fn spawn(
        db: Arc<Db>,
        action_handle: ActionHandle,
        event_bus: EventBus,
    ) -> ReflexResult<Self> {
        Ok(Self {
            db,
            action_handle,
            event_bus,
        })
    }

    /// Returns the storage path backing this runtime.
    #[must_use]
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime"))]
    pub fn storage_path(&self) -> &Path {
        &self.db.path
    }

    /// Returns the storage schema version backing this runtime.
    #[must_use]
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime"))]
    pub fn schema_version(&self) -> u32 {
        self.db.schema_version
    }

    /// Returns the action emitter handle used by reflex controllers.
    #[must_use]
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime"))]
    pub fn action_handle(&self) -> &ActionHandle {
        &self.action_handle
    }

    /// Returns the event bus handle used by this runtime.
    #[must_use]
    #[tracing::instrument(skip_all, fields(component = "reflex_runtime"))]
    pub fn event_bus(&self) -> &EventBus {
        &self.event_bus
    }
}

#[cfg(test)]
mod tests {
    use std::{error::Error, sync::Arc};

    use synapse_action::ActionHandle;
    use synapse_core::Action;
    use synapse_storage::Db;
    use tempfile::tempdir;
    use tokio::sync::mpsc;

    use super::{EventBus, ReflexRuntime};

    const TEST_SCHEMA_VERSION: u32 = 7;

    #[test]
    fn spawn_retains_runtime_inputs_and_action_handle() -> Result<(), Box<dyn Error>> {
        let temp = tempdir()?;
        let db = Arc::new(Db::open(&temp.path().join("db"), TEST_SCHEMA_VERSION)?);
        let (action_handle, mut action_rx) = ActionHandle::channel();
        assert!(matches!(
            action_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        let runtime = ReflexRuntime::spawn(Arc::clone(&db), action_handle, EventBus::default())?;
        runtime.action_handle().try_execute(Action::ReleaseAll)?;
        let (queued_action, _ack) = action_rx.try_recv()?;

        assert_eq!(runtime.schema_version(), TEST_SCHEMA_VERSION);
        assert_eq!(queued_action, Action::ReleaseAll);
        Ok(())
    }
}
