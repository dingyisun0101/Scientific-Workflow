//! Typed application work behind one uniform runtime definition.
//!
//! Application authors import [`basic`], implement [`basic::ScientificModel`],
//! and register the implementation with `#[scientific_workflow::model("key")]`.
//! Study combines each registered model with config-supplied constants and
//! creates the internal task automatically. [`advanced`] supplies discovery,
//! explicit catalog, and execution ports used by Study and runtime. Scheduling,
//! identity, paths, lifecycle persistence, and display remain outside this module.

mod catalog;
mod definition;
mod execution;
mod model;
mod result;

/// Ordinary application-facing task API.
pub mod basic {
    pub use super::model::ScientificModel;
    pub use super::result::TaskResult;
}

/// Supported task API for advanced users and Workflow peer subsystems.
pub mod advanced {
    pub use super::basic::*;
    pub use super::catalog::{ModelCatalog, ModelCatalogError, ModelRegistration};
    pub use super::definition::Task;
    pub use super::execution::{TaskDefinition, TaskExecutionHost};
}
