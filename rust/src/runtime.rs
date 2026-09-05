//! Execution of a completely validated immutable study.
//!
//! Runtime owns active mechanics only: output scopes, replicate and phase
//! scheduling, task admission, cooperative cancellation, and automatic
//! persistence lifecycle. It owns the lifecycle observer contract consumed by
//! the automatically composed UI.
//! It accepts only a completed Study and never opens project declarations or
//! binds execution unit keys itself.

mod control;
mod error;
mod event;
mod execution;
mod host;
mod output;
mod presentation;
mod program;
mod resource;
mod summary;

#[cfg(test)]
#[path = "runtime/tests/runtime_workflow.rs"]
mod runtime_workflow_tests;

pub use crate::composition::execute;
pub use error::RuntimeError;
pub(crate) use event::RuntimeEvent;
pub(crate) use execution::execute_with_observer;
pub(crate) use presentation::{PresentationFailure, RuntimeObserver};
pub use summary::{
    MemberRunSummary, PhaseRunSummary, ReplicateRunSummary, RunSummary, TaskRunKind, TaskRunSummary,
};

pub(crate) use control::RunControl;

#[cfg(feature = "terminal-ui")]
pub(crate) use program::force_exit;
