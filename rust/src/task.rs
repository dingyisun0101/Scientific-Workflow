//! Generic application workloads behind one uniform runtime definition.
//!
//! Application authors import [`basic`], implement [`basic::ScientificModel`],
//! and register the implementation with `#[scientific_workflow::model("key")]`.
//! Study creates model tasks from registered models plus resolved constants and
//! creates program tasks directly from declarative executable paths. Python
//! declarations are lowered by Config to the same program boundary. All use
//! one internal Task definition. [`advanced`] retains the same supported
//! public surface; adapters and execution ports remain crate-private.

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
    pub(crate) use super::definition::{Task, TaskKind};
    pub(crate) use super::execution::{TaskDefinition, TaskExecutionHost};
}
