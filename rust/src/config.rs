//! Central parsing and resolution of declarative Workflow project inputs.
//!
//! Ordinary applications do not call this subsystem: they write a study
//! manifest, a state schema, and task input documents. [`advanced`] is the
//! supported boundary through which Study loads one project root and obtains
//! immutable phase specifications and resolved model inputs.

mod document;
mod error;
mod expansion;
mod input;
mod manifest;
mod specification;

#[cfg(test)]
#[path = "config/tests/config_workflow.rs"]
mod config_workflow_tests;

/// Ordinary application-facing configuration API.
///
/// This scope is intentionally empty. Configuration's user interface is the
/// documented project file grammar, while the runtime owns loading.
pub mod basic {}

/// Supported configuration API for advanced users and Workflow subsystems.
pub mod advanced {
    #[allow(unused_imports)]
    pub use super::basic::*;
    pub use super::error::ConfigError;
    pub(crate) use super::input::ResolvedTaskInput;
    pub(crate) use super::manifest::{FailurePolicy, ReplicatePolicy, ReplicateScheduling};
    pub(crate) use super::specification::ProjectSpecification;
}
