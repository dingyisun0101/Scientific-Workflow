//! Generic application workloads behind one uniform runtime definition.
//!
//! Application authors import [`basic`], implement [`basic::ExecutionUnit`],
//! and register the implementation with
//! `#[scientific_workflow::execution_unit("key")]`.
//! Study creates execution-unit tasks from registrations plus resolved constants and
//! creates program tasks directly from declarative executable paths. Python
//! declarations are lowered by Config to the same program boundary. All use
//! one internal Task definition. [`advanced`] retains the same supported
//! public surface; adapters and execution ports remain crate-private.

mod catalog;
mod definition;
mod execution;
mod result;
mod unit;

#[cfg(test)]
#[path = "task/tests/task_workflow.rs"]
mod task_workflow_tests;

/// Ordinary application-facing task API.
pub mod basic {
    pub use super::result::TaskResult;
    pub use super::unit::{
        ExecutionUnit, InitializationContext, MemberCompletion, MemberView, SeedError,
    };
}

/// Supported task API for advanced users and Workflow peer subsystems.
pub mod advanced {
    #[allow(unused_imports)]
    pub use super::basic::*;
    #[doc(hidden)]
    pub use super::catalog::ExecutionUnitRegistration;
    pub(crate) use super::catalog::{ExecutionUnitCatalog, ExecutionUnitCatalogError};
    pub(crate) use super::definition::{ExecutionUnitTaskProvenance, Task, TaskKind};
    pub(crate) use super::execution::{
        MemberInitialization, ProgramTaskInvocation, TaskDefinition, TaskExecutionHost,
    };
}
