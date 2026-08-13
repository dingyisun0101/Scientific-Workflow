//! Errors produced by centralized progress registration and rendering.

use std::io;

use thiserror::Error;

use crate::configuration::ConfigurationError;

/// Failure while configuring, updating, or finalizing progress reporting.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ReportingError {
    /// Project configuration could not supply a required task or identity value.
    #[error(transparent)]
    Configuration(#[from] ConfigurationError),

    /// Another live reporter already owns process terminal rendering.
    #[error("another progress reporter already owns the process terminal")]
    TerminalAlreadyOwned,

    /// The validated task count cannot be represented by this platform.
    #[error("task count {task_count} exceeds this platform's addressable progress slots")]
    TaskCountTooLarge {
        /// Validated project task count.
        task_count: u64,
    },

    /// One identity key was supplied more than once.
    #[error("task identity repeats parameter key `{key}`")]
    DuplicateIdentityParameter {
        /// Repeated exact parameter name.
        key: String,
    },

    /// One requested identity key is absent from project parameters.
    #[error("task identity parameter `{key}` is not declared by the project")]
    UnknownIdentityParameter {
        /// Missing exact parameter name.
        key: String,
    },

    /// Two generated tasks have the same selected parameter identity.
    #[error(
        "task identity `{identity}` is shared by ordinals {first_ordinal} and {second_ordinal}"
    )]
    NonUniqueTaskIdentity {
        /// Deterministic rendered parameter identity.
        identity: String,
        /// First colliding automatically assigned ordinal.
        first_ordinal: u64,
        /// Second colliding automatically assigned ordinal.
        second_ordinal: u64,
    },

    /// A task handle does not belong to the reporter's configured task space.
    #[error("task ordinal {task_ordinal} is outside the reporter's task registry")]
    UnknownTaskOrdinal {
        /// Automatically assigned ordinal obtained from `TaskConfig`.
        task_ordinal: u64,
    },

    /// A directly registered application task name was not found.
    #[error("registered task `{identity}` does not exist")]
    UnknownRegisteredTask { identity: String },

    /// The same directly registered application task name appeared twice.
    #[error("registered task `{identity}` appears more than once")]
    DuplicateRegisteredTask { identity: String },

    /// A task handle's selected identity differs from the registered project.
    #[error("task ordinal {task_ordinal} does not match its registered parameter identity")]
    TaskIdentityMismatch {
        /// Automatically assigned task ordinal.
        task_ordinal: u64,
    },

    /// The same task was started more than once.
    #[error("task `{identity}` has already started or reached a terminal status")]
    TaskAlreadyStarted {
        /// Human-readable parameter identity.
        identity: String,
    },

    /// Initial progress lies beyond a known target.
    #[error("task `{identity}` starts at iteration {initial}, beyond target {target}")]
    InitialIterationBeyondTarget {
        /// Human-readable parameter identity.
        identity: String,
        /// Initial absolute simulation iteration.
        initial: u64,
        /// Target absolute simulation iteration.
        target: u64,
    },

    /// A progress update attempted to move scientific iteration backward.
    #[error("task `{identity}` cannot move progress from iteration {current} back to {attempted}")]
    IterationRegressed {
        /// Human-readable parameter identity.
        identity: String,
        /// Previously reported iteration.
        current: u64,
        /// Rejected iteration.
        attempted: u64,
    },

    /// A progress update exceeded a known target.
    #[error("task `{identity}` reported iteration {iteration}, beyond target {target}")]
    IterationBeyondTarget {
        /// Human-readable parameter identity.
        identity: String,
        /// Rejected absolute simulation iteration.
        iteration: u64,
        /// Configured absolute target iteration.
        target: u64,
    },

    /// Completion was requested before a known target was reached.
    #[error("task `{identity}` completed at iteration {current}, before target {target}")]
    TargetIterationNotReached {
        /// Human-readable parameter identity.
        identity: String,
        /// Last reported absolute simulation iteration.
        current: u64,
        /// Configured absolute target iteration.
        target: u64,
    },

    /// The sole renderer thread could not be created.
    #[error("failed to start the centralized terminal reporter")]
    StartRenderer {
        /// Underlying thread-creation failure.
        #[source]
        source: io::Error,
    },

    /// Interactive terminal isolation could not be established.
    #[error("failed to {operation} for the isolated progress screen")]
    TerminalSetup {
        operation: &'static str,
        #[source]
        source: io::Error,
    },

    /// The renderer stopped before accepting a requested message.
    #[error("the centralized terminal reporter is no longer available")]
    RendererUnavailable,

    /// The renderer thread panicked while the reporter was active.
    #[error("the centralized terminal reporter panicked")]
    RendererPanicked,

    /// Successful finalization was requested before every task completed.
    #[error(
        "cannot report success with {pending} pending, {running} running, and {failed} failed tasks"
    )]
    IncompleteProgress {
        /// Tasks that never started.
        pending: u64,
        /// Tasks that have not reached a terminal status.
        running: u64,
        /// Tasks that failed or dropped before completion.
        failed: u64,
    },
}
