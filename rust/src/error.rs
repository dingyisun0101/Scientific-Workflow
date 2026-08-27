//! Complete-workflow error composition.
//!
//! [`basic`] exposes the ordinary crate-facade error. [`advanced`] is its
//! strict superset; detailed Study and Runtime errors remain owned by their
//! respective subsystem scopes.

mod workflow;

/// Ordinary application-facing complete-workflow error API.
pub mod basic {
    pub use super::workflow::WorkflowError;
}

/// Supported complete-workflow error API for advanced users.
pub mod advanced {
    pub use super::basic::*;
}
