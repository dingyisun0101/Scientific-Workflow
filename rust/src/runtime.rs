//! Execution of a completely validated immutable study.
//!
//! Runtime owns active mechanics only: output scopes, replicate and phase
//! scheduling, task admission, cooperative cancellation, and automatic
//! persistence lifecycle. It never opens project declarations or binds model
//! keys itself.

mod error;
mod execution;
mod host;
mod output;
mod summary;

/// Ordinary application-facing runtime API.
pub mod basic {
    pub use super::execution::run;
}

/// Supported runtime API for inspection, embedding, and Workflow peer modules.
pub mod advanced {
    pub use super::basic::*;
    pub use super::error::RuntimeError;
    pub use super::execution::execute;
    pub use super::summary::{PhaseRunSummary, ReplicateRunSummary, RunSummary, TaskRunSummary};
}
