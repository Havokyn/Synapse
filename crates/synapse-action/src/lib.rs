#![allow(unsafe_code)]

pub mod backend;
pub mod emitter;
pub mod error;
pub mod handle;

pub use backend::{
    ActionBackend, ResolvedBackend,
    recording::{RecordedInput, RecordingBackend},
    resolve_backend,
};
pub use emitter::{
    ActionEmitter, ActionEmitterSnapshotHandle, ActionSnapshotMessage, ActionStateSnapshot,
    EmitState,
};
pub use error::{ActionError, ActionResult};
pub use handle::{ACTION_QUEUE_CAPACITY, ActionHandle, ActionMessage, RELEASE_ALL_HANDLE};
