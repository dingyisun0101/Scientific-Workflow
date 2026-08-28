//! Observation declaration, binding, and encoding failures.

use thiserror::Error;

use crate::state::StateError;

/// A failure while defining, binding, or applying a scientific observation plan.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ObservationError {
    /// A multi-stream observation plan contained no streams.
    #[error("an observation plan must declare at least one observation stream")]
    EmptyPlan,

    /// A stream name normalized to an empty value.
    #[error("observation stream name must not be empty")]
    EmptyStreamName,

    /// Two streams used the same normalized scientific name.
    #[error("observation stream `{stream}` is declared more than once")]
    DuplicateStreamName {
        /// Repeated stream name.
        stream: String,
    },

    /// A selected field name normalized to an empty value.
    #[error("observation stream `{stream}` contains an empty field name")]
    EmptyFieldName {
        /// Stream containing the invalid selection.
        stream: String,
    },

    /// A stream selected no fields.
    #[error("observation stream `{stream}` must select at least one state field")]
    EmptyFieldSelection {
        /// Stream with no selected fields.
        stream: String,
    },

    /// A stream selected one normalized field more than once.
    #[error("observation stream `{stream}` selects field `{field}` more than once")]
    DuplicateField {
        /// Stream containing the duplicate selection.
        stream: String,
        /// Repeated field name.
        field: String,
    },

    /// A stream selected a field absent from the bound state schema.
    #[error("observation stream `{stream}` selects unknown state field `{field}`")]
    UnknownField {
        /// Stream containing the unknown selection.
        stream: String,
        /// Unknown field name.
        field: String,
    },

    /// An iteration sampling interval was zero.
    #[error("observation stream `{stream}` sampling interval must be greater than zero")]
    InvalidSamplingInterval {
        /// Stream receiving the invalid interval.
        stream: String,
    },

    /// A scientific axis unit normalized to an empty value.
    #[error("observation {axis} unit must not be empty")]
    EmptyAxisUnit {
        /// Stable axis name: `iteration` or `physical_time`.
        axis: &'static str,
    },

    /// An observation did not share the descriptor's schema allocation.
    #[error("state at iteration {iteration} does not share the observation-plan schema")]
    SchemaMismatch {
        /// Iteration carried by the rejected observation.
        iteration: u64,
    },

    /// A selected payload could not be borrowed from the observed state.
    #[error(
        "cannot observe field `{field}` for stream `{stream}` at iteration {iteration}: {source}"
    )]
    StateAccess {
        /// Stream being encoded.
        stream: String,
        /// State iteration being observed.
        iteration: u64,
        /// Selected field that could not be borrowed.
        field: String,
        /// Original typed state-access failure.
        #[source]
        source: StateError,
    },

    /// Serde rejected a selected scientific payload.
    #[error("failed to encode field `{field}` for stream `{stream}` at iteration {iteration}")]
    EncodeField {
        /// Stream being encoded.
        stream: String,
        /// State iteration being observed.
        iteration: u64,
        /// Field active when encoding failed.
        field: String,
        /// Original JSON serialization failure.
        #[source]
        source: serde_json::Error,
    },

    /// A state observation moved backward relative to an accepted stream value.
    #[error(
        "observation stream `{stream}` observed iteration {next} after accepted iteration {previous}"
    )]
    NonIncreasingObservation {
        /// Stream whose ordering would be violated.
        stream: String,
        /// Most recently accepted iteration.
        previous: u64,
        /// Rejected decreasing iteration.
        next: u64,
    },
}
