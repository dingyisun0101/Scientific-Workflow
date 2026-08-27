//! Central parsing and resolution of declarative Workflow project parameters.
//!
//! Ordinary applications do not call this subsystem: they write a study
//! manifest, one state schema, and one arbitrary `parameters.json`. Config
//! parses them once into one immutable namespaced graph. Study retains that graph, uses
//! typed reserved views for workflow procedure, and binds model, generic
//! program, or environment-managed Python tasks from it.

mod document;
mod error;
mod expansion;
mod manifest;
mod parameters;
mod program;
mod python;
mod specification;
mod store;

#[cfg(test)]
#[path = "config/tests/config_workflow.rs"]
mod config_workflow_tests;

/// Ordinary application-facing configuration API.
///
/// This scope is intentionally empty. Configuration's user interface is the
/// documented project file grammar, while Study owns coordinated loading.
pub mod basic {}

/// Supported configuration API for advanced users and Workflow subsystems.
pub mod advanced {
    #[allow(unused_imports)]
    pub use super::basic::*;
    pub use super::error::ConfigError;
    pub(crate) use super::manifest::{FailurePolicy, ReplicatePolicy, ReplicateScheduling};
    pub(crate) use super::parameters::{ResolvedModelParameters, ResolvedTask};
    pub(crate) use super::program::ResolvedProgramTask;
    pub(crate) use super::specification::ProjectSpecification;
    pub(crate) use super::store::Config;
}
