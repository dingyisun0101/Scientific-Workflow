//! Execution of a completely validated immutable study.
//!
//! Runtime owns active mechanics only: output scopes, replicate and phase
//! scheduling, task admission, cooperative cancellation, and automatic
//! persistence lifecycle. It owns the lifecycle observer contract consumed by
//! the automatically composed UI.
//! It accepts only a completed Study and never opens project declarations or
//! binds execution unit keys itself.

mod control;
mod disk;
mod error;
mod event;
mod execution;
mod output;
mod presentation;
mod program;
mod resources;
mod reuse;
mod summary;

#[cfg(test)]
#[path = "runtime/tests/runtime_workflow.rs"]
pub(crate) mod runtime_workflow_tests;

pub use crate::composition::execute;
pub use error::RuntimeError;
pub(crate) use event::RuntimeEvent;
#[cfg(test)]
pub(crate) use execution::execute_with_observer;
pub(crate) use execution::execute_with_observer_options;
pub(crate) use presentation::{PresentationFailure, RuntimeObserver};
pub use summary::{
    MemberRunSummary, PhaseRunSummary, ReplicateRunSummary, RunSummary, TaskRunKind, TaskRunSummary,
};

pub(crate) use control::RunControl;
pub(crate) use disk::usage as disk_usage;

pub(crate) use program::force_exit;

#[cfg(test)]
#[path = "runtime/tests/reuse_npy_filters.rs"]
mod reuse_npy_filter_tests;
