//! Application-defined scientific observation.
//!
//! [`basic`] lets an application declare which state fields are observed and
//! at what iteration cadence. [`advanced`] retains that same supported public
//! surface; schema-bound descriptors and encoding handoffs remain crate-private. Paths, buffering, chunking,
//! provenance, and durable lifecycle do not belong to this subsystem.

mod encoding;
mod error;
mod plan;
mod sampling;
mod session;
mod state_observation;
mod stream;

#[cfg(test)]
#[path = "observation/tests/observation_workflow.rs"]
mod observation_workflow_tests;

/// Ordinary application-facing observation definitions.
pub mod basic {
    pub use super::error::ObservationError;
    pub use super::plan::ObservationPlan;
    pub use super::stream::ObservationStream;
}

/// Supported observation API for advanced users and Workflow peer subsystems.
pub mod advanced {
    #[allow(unused_imports)]
    pub use super::basic::*;
    pub(crate) use super::plan::BoundObservationPlan;
    pub(crate) use super::session::ObservationSession;
}
