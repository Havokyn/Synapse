pub mod audit;
pub mod error;

use std::{path::Path, sync::Arc};

use synapse_action::ActionHandle;
use synapse_storage::Db;

pub use audit::write_audit;
pub use error::{ReflexError, ReflexResult};

/// Minimal event bus handle; expanded by the M3 event-bus work item.
#[derive(Clone, Debug, Default)]
pub struct EventBus;

/// Runtime handle for the M3 reflex subsystem.
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
    fn spawn_retains_runtime_inputs_and_action_handle_with_fsv() -> Result<(), Box<dyn Error>> {
        let temp = tempdir()?;
        let db = Arc::new(Db::open(&temp.path().join("db"), TEST_SCHEMA_VERSION)?);
        let (action_handle, mut action_rx) = ActionHandle::channel();
        assert!(matches!(
            action_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));
        println!(
            "source_of_truth=reflex_runtime_state before=db_schema:{} action_queue:empty",
            db.schema_version
        );

        let runtime = ReflexRuntime::spawn(Arc::clone(&db), action_handle, EventBus)?;
        runtime.action_handle().try_execute(Action::ReleaseAll)?;
        let (queued_action, _ack) = action_rx.try_recv()?;

        println!(
            "source_of_truth=reflex_runtime_state after_truth=path:{} schema:{} queued_action:{queued_action:?} bus:{:?} final_value=spawned:true",
            runtime.storage_path().display(),
            runtime.schema_version(),
            runtime.event_bus()
        );
        assert_eq!(runtime.schema_version(), TEST_SCHEMA_VERSION);
        assert_eq!(queued_action, Action::ReleaseAll);
        Ok(())
    }
}
