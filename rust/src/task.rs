//! Typed application work behind one uniform runtime definition.
//!
//! Application authors import [`basic`], implement [`basic::ScientificModel`],
//! and register the implementation with `#[scientific_workflow::model("key")]`.
//! Study combines each registered model with config-supplied constants and
//! creates the internal task automatically. [`advanced`] retains that same
//! supported public surface; discovery, catalogs, and execution ports remain crate-private. Scheduling,
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
    #[allow(unused_imports)]
    pub use super::basic::*;
    #[doc(hidden)]
    pub use super::catalog::ModelRegistration;
    pub(crate) use super::catalog::{ModelCatalog, ModelCatalogError};
    pub(crate) use super::definition::Task;
    pub(crate) use super::execution::{TaskDefinition, TaskExecutionHost};
}
