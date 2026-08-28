//! Execution of a completely validated immutable study.
//!
//! Runtime owns active mechanics only: output scopes, replicate and phase
//! scheduling, task admission, cooperative cancellation, and automatic
//! persistence lifecycle. It also publishes inferred facts to the automatic UI.
//! It accepts only a completed Study and never opens project declarations or
//! binds execution unit keys itself.

mod error;
mod execution;
mod host;
mod output;
mod summary;

#[cfg(test)]
#[path = "runtime/tests/runtime_workflow.rs"]
mod runtime_workflow_tests;

pub use error::RuntimeError;
pub use execution::execute;
pub use summary::{
    MemberRunSummary, PhaseRunSummary, ReplicateRunSummary, RunSummary, TaskRunKind, TaskRunSummary,
};
