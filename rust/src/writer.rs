//! Application-defined scientific observation.
//!
//! [`basic`] lets an application declare which state fields are observed and
//! at what iteration cadence. [`advanced`] adds the validated descriptors and
//! backend ports used by Workflow integrations. Paths, buffering, chunking,
//! provenance, and durable lifecycle do not belong to this subsystem.

mod definition;
mod encoding;
mod error;
mod observation;
mod sampling;
mod session;
mod stream;

pub(crate) use session::WriterSession;

/// Ordinary application-facing writer definitions.
pub mod basic {
    pub use super::definition::Writer;
    pub use super::error::WriterError;
    pub use super::stream::Stream;
}

/// Supported writer API for advanced users and Workflow peer subsystems.
pub mod advanced {
    pub use super::basic::*;
    pub use super::definition::WriterDescriptor;
    pub use super::encoding::{EncodedObservation, ObservationSink, SessionOutcome};
    pub use super::observation::Observation;
    pub use super::stream::StreamDescriptor;
}
