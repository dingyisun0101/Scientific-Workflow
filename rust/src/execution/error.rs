use std::path::PathBuf;

use thiserror::Error;

/// Failure while validating or creating an execution-directory scope.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ExecutionScopeError {
    /// A caller-supplied scope name was empty or not one safe path component.
    #[error("invalid execution scope name `{name}`")]
    InvalidName {
        /// Rejected scope name.
        name: String,
    },
    /// The host UTC clock could not be formatted for the scope identity.
    #[error("failed to format execution scope creation timestamp")]
    Timestamp {
        /// Timestamp-formatting failure.
        #[source]
        source: time::error::Format,
    },
    /// A filesystem operation failed at a scope boundary.
    #[error("failed to {operation} execution scope at `{path}`")]
    Io {
        /// Stable filesystem action.
        operation: &'static str,
        /// Affected root or scope path.
        path: PathBuf,
        /// Underlying operating-system failure.
        #[source]
        source: std::io::Error,
    },
    /// Generated identity attempts repeatedly collided with existing scopes.
    #[error("could not allocate a unique execution scope beneath `{root}`")]
    IdentityExhausted {
        /// Parent directory in which allocation was attempted.
        root: PathBuf,
    },
}
