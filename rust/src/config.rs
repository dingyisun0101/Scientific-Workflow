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

/// Ordinary application-facing configuration API.
///
/// This scope is intentionally empty. Configuration's user interface is the
/// documented project file grammar, while the runtime owns loading.
pub mod basic {}

/// Supported configuration API for advanced users and Workflow subsystems.
pub mod advanced {
    #[allow(unused_imports)]
    pub use super::basic::*;
    pub use super::document::{ProjectDocument, StateSchemaDocument};
    pub use super::error::ConfigError;
    pub use super::input::ResolvedTaskInput;
    pub use super::manifest::{
        FailurePolicy, PhaseSpecification, ReplicatePolicy, ReplicateScheduling, StudyManifest,
    };
    pub use super::specification::ProjectSpecification;
}
