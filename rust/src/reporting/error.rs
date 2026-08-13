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

    /// One phase contains no tasks.
    #[error("phase {phase} must contain at least one task")]
    EmptyPhase { phase: u64 },

    /// A first-class reporter/runtime plan contains no phase.
    #[error("at least one phase is required")]
    EmptyPhaseSet,

    /// A phase label is empty or whitespace-only.
    #[error("phase {phase} must have a nonempty label")]
    InvalidPhaseLabel { phase: u64 },

    /// A phase active-workload limit is zero.
    #[error("phase {phase} max_concurrent_workloads must be greater than zero")]
    InvalidPhaseWorkloadLimit { phase: u64 },

    /// A phase prepared-work queue capacity is zero.
    #[error("phase {phase} queue_capacity must be greater than zero")]
    InvalidPhaseQueueCapacity { phase: u64 },

    /// A reporter phase list repeats one phase ID.
    #[error("phase ID {phase} appears more than once")]
    DuplicatePhaseId { phase: u64 },

    /// One task has an empty phase-local ID.
    #[error("phase {phase} contains an empty task ID")]
    InvalidManagedTaskId { phase: u64 },

    /// One task has an empty kind/namespace.
    #[error("task `{task}` must have a nonempty kind")]
    InvalidManagedTaskKind { task: String },

    /// One phase repeats the same phase-local task ID.
    #[error("phase {phase} repeats task ID `{task}`")]
    DuplicateManagedTaskId { phase: u64, task: String },

    /// Tasks of one kind do not expose one consistent parameter-key set.
    #[error("task kind `{kind}` has inconsistent parameter keys between `{first}` and `{second}`")]
    InconsistentManagedTaskParameters {
        kind: String,
        first: String,
        second: String,
    },

    /// A display projection names a task kind absent from the phase.
    #[error("task kind `{kind}` is not declared by the phase")]
    UnknownManagedTaskKind { kind: String },

    /// Two tasks receive the same requested generated label.
    #[error("generated task label `{label}` collides between `{first}` and `{second}`")]
    ManagedTaskDisplayCollision {
        label: String,
        first: String,
        second: String,
    },

    /// A partial selector matched no managed task.
    #[error("task selector `{selector}` matched no task")]
    ManagedTaskNotFound { selector: String },

    /// A partial selector matched more than one managed task.
    #[error("task selector `{selector}` is ambiguous between `{first}` and `{second}`")]
    ManagedTaskSelectorAmbiguous {
        selector: String,
        first: String,
        second: String,
    },

    /// A managed task does not contain one required parameter.
    #[error("task `{task}` does not contain parameter `{key}`")]
    UnknownManagedTaskParameter { task: String, key: String },

    /// One managed task parameter could not be decoded.
    #[error("task `{task}` parameter `{key}` could not be decoded")]
    DecodeManagedTaskParameter {
        task: String,
        key: String,
        #[source]
        source: serde_json::Error,
    },

    /// An explicit parameter key is empty.
    #[error("task parameter key `{key}` is invalid")]
    InvalidTaskParameter { key: String },

    /// Configuration-derived parameters cannot be mutated.
    #[error("configuration-derived task `{task}` has immutable parameters")]
    ConfiguredTaskParametersImmutable { task: String },

    /// The reporter does not contain one exact first-class task key.
    #[error("managed task `{task}` does not exist in this reporter")]
    UnknownManagedTask { task: String },

    /// A requested handle does not match the task's declared display kind.
    #[error("task `{task}` is declared as {actual}, not {requested}")]
    ManagedTaskKindMismatch {
        task: String,
        requested: &'static str,
        actual: &'static str,
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
