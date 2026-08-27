//! Execution of a completely validated immutable study.
//!
//! Runtime owns active mechanics only: output scopes, replicate and phase
//! scheduling, task admission, cooperative cancellation, and automatic
//! persistence lifecycle. It also publishes inferred facts to the automatic UI.
//! It accepts only a completed Study and never opens project declarations or
//! binds model keys itself.

mod error;
mod execution;
mod host;
mod output;
mod summary;

/// Ordinary application-facing runtime API.
///
/// This scope is intentionally empty. Ordinary applications call the
/// crate-level `run(&Path)` facade; Runtime itself accepts only a completed
/// Study through its Advanced API.
pub mod basic {}

/// Supported runtime API for inspection, embedding, and Workflow peer modules.
pub mod advanced {
    #[allow(unused_imports)]
    pub use super::basic::*;
    pub use super::error::RuntimeError;
    pub use super::execution::execute;
    pub use super::summary::{
        PhaseRunSummary, ReplicateRunSummary, RunSummary, TaskRunKind, TaskRunSummary,
    };
}
