//! Central parsing and resolution of declarative Workflow project inputs.
//!
//! Ordinary applications do not call this subsystem: they write a study
//! manifest plus arbitrary JSON beneath `config`. Config parses every document
//! once into one immutable namespaced graph. Study retains that graph, uses
//! typed reserved views for workflow procedure, and binds model, generic
//! program, or environment-managed Python tasks from it.

mod document;
mod error;
mod expansion;
mod input;
mod manifest;
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
    pub(crate) use super::input::{ResolvedTask, ResolvedTaskInput};
    pub(crate) use super::manifest::{FailurePolicy, ReplicatePolicy, ReplicateScheduling};
    pub(crate) use super::program::ResolvedProgramTask;
    pub(crate) use super::specification::ProjectSpecification;
    pub(crate) use super::store::Config;
}
