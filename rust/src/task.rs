//! Typed application work behind one uniform runtime definition.
//!
//! Application authors normally import [`basic`] and implement
//! [`basic::ScientificModel`] for stateful work or use [`basic::Task::one_shot`]
//! for work without state. [`advanced`] supplies the read-only descriptor and
//! execution port used by Workflow's runtime. Scheduling, identity, paths,
//! lifecycle persistence, and display rendering remain outside this module.

mod definition;
mod execution;
mod model;
mod result;

/// Ordinary application-facing task API.
pub mod basic {
    pub use super::definition::Task;
    pub use super::model::ScientificModel;
    pub use super::result::TaskResult;
}

/// Supported task API for advanced users and Workflow peer subsystems.
pub mod advanced {
    pub use super::basic::*;
    pub use super::execution::{TaskDefinition, TaskDescriptor, TaskExecutionHost, TaskKind};
}
