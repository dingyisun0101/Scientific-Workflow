//! Application-defined scientific observation.
//!
//! Applications declare which state fields are observed and at what iteration
//! cadence through the module root; schema-bound descriptors and encoding
//! handoffs remain crate-private. Paths, buffering, chunking,
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

pub(crate) use encoding::EncodedObservation;
pub use error::ObservationError;
pub(crate) use plan::BoundObservationPlan;
pub use plan::ObservationPlan;
pub(crate) use session::ObservationSession;
pub(crate) use stream::BoundObservationStream;
pub use stream::ObservationStream;
