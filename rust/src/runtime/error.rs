//! Active runtime failures.

use std::path::PathBuf;
use std::time::Duration;

use thiserror::Error;

/// A failure after an immutable study passed preflight.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RuntimeError {
    /// Interactive UI requested cooperative cancellation of the execution.
    #[error("workflow execution was cancelled by the user")]
    ExecutionCancelled,

    /// Runtime could not create an inferred execution or replicate scope.
    #[error("failed to create inferred output scope `{path}`")]
    OutputScope {
        /// Intended output location.
        path: PathBuf,
        /// Underlying filesystem failure.
        #[source]
        source: std::io::Error,
    },

    /// An execution-unit or program invocation returned an application, process, state,
    /// observation, or persistence error.
    #[error("task `{task}` failed: {source}")]
    Task {
        /// Inferred task identity.
        task: String,
        /// Original task failure.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },

    /// A task invocation panicked. Runtime fails any active member recording
    /// before reporting this stable task identity.
    #[error("task `{task}` panicked")]
    TaskPanicked {
        /// Inferred task identity.
        task: String,
    },

    /// A task exceeded its timeout; execution units stop cooperatively while external
    /// programs are terminated by Runtime.
    #[error("task `{task}` exceeded its timeout of {timeout:?}")]
    TaskTimedOut {
        /// Inferred task identity.
        task: String,
        /// Effective timeout.
        timeout: Duration,
    },

    /// A running task stopped because Runtime cancelled it. Execution units stop
    /// cooperatively while external programs are terminated.
    #[error("task `{task}` was cancelled")]
    TaskCancelled {
        /// Inferred task identity.
        task: String,
    },

    /// A phase exceeded its timeout. Its active execution-unit tasks stop cooperatively
    /// and its active external programs are terminated.
    #[error("phase `{phase}` exceeded its timeout of {timeout:?}")]
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

    /// A replicate failed under either failure policy.
    #[error("replicate {index} failed: {source}")]
    Replicate {
        /// Zero-based failed replicate index.
        index: u64,
        /// Original runtime failure.
        #[source]
        source: Box<RuntimeError>,
    },
}
