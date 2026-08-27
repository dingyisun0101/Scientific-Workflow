//! Active runtime failures.

use std::path::PathBuf;
use std::time::Duration;

use thiserror::Error;

use crate::execution::ExecutionScopeError;

/// A failure after an immutable study passed preflight.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RuntimeError {
    /// Runtime could not create an inferred execution or replicate scope.
    #[error("failed to create inferred output scope `{path}`")]
    OutputScope {
        /// Intended output location.
        path: PathBuf,
        /// Filesystem-scope failure.
        #[source]
        source: ExecutionScopeError,
    },

    /// A model invocation returned an application, state, writer, or record error.
    #[error("task `{task}` failed: {source}")]
    Task {
        /// Inferred task identity.
        task: String,
        /// Original task failure.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },

    /// A model invocation panicked in its runtime worker.
    #[error("task `{task}` panicked")]
    TaskPanicked {
        /// Inferred task identity.
        task: String,
    },

    /// A task exceeded its cooperative timeout.
    #[error("task `{task}` exceeded its cooperative timeout of {timeout:?}")]
    TaskTimedOut {
        /// Inferred task identity.
        task: String,
        /// Effective timeout.
        timeout: Duration,
    },

    /// A running task stopped cooperatively because runtime cancelled it.
    #[error("task `{task}` was cancelled")]
    TaskCancelled {
        /// Inferred task identity.
        task: String,
    },

    /// A phase exceeded its cooperative timeout.
    #[error("phase `{phase}` exceeded its cooperative timeout of {timeout:?}")]
    PhaseTimedOut {
        /// Stable phase key.
        phase: String,
        /// Effective timeout.
        timeout: Duration,
    },

    /// Runtime could not start an operating-system worker thread.
    #[error("failed to start runtime worker for `{scope}`")]
    StartWorker {
        /// Task or replicate identity.
        scope: String,
        /// Thread creation failure.
        #[source]
        source: std::io::Error,
    },

    /// A replicate worker panicked.
    #[error("replicate {index} panicked")]
    ReplicatePanicked {
        /// Zero-based replicate index.
        index: u64,
    },

    /// At least one replicate failed under a finish-all policy.
    #[error("replicate {index} failed: {source}")]
    Replicate {
        /// Zero-based failed replicate index.
        index: u64,
        /// Original runtime failure.
        #[source]
        source: Box<RuntimeError>,
    },
}
