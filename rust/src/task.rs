//! Generic application workloads behind one uniform runtime definition.
//!
//! Application authors implement the crate-root [`ExecutionUnit`] contract and
//! register the implementation with
//! `#[scientific_workflow::execution_unit("key")]`.
//! Study creates execution-unit tasks from registrations plus resolved constants and
//! creates program tasks directly from declarative executable paths. Python
//! declarations are lowered by Config to the same program boundary. All use
//! one private Task definition; adapters and execution ports remain crate-private.

mod catalog;
mod definition;
mod execution;
mod result;
mod unit;

#[cfg(test)]
#[path = "task/tests/task_workflow.rs"]
mod task_workflow_tests;

#[doc(hidden)]
pub use catalog::ExecutionUnitRegistration;
pub(crate) use catalog::{ExecutionUnitCatalog, ExecutionUnitCatalogError};
pub(crate) use definition::{ExecutionUnitTaskProvenance, Task, TaskKind};
pub(crate) use execution::{
    MemberInitialization, ProgramTaskInvocation, TaskDefinition, TaskExecutionHost,
};
pub(crate) use result::TaskResult;
pub use result::UnitResult;
pub use unit::{ExecutionUnit, InitializationContext, MemberCompletion, MemberView, SeedError};
pub(crate) use unit::{SEED_DERIVATION_ALGORITHM, derive_program_seed};
