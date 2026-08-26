//! Errors produced while planning, scheduling, and rendering a study.

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use thiserror::Error;

/// Failure while validating, executing, recording, or displaying a study.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum StudyError {
    /// A completed study record could not be encoded as JSON.
    #[error("failed to serialize study record")]
    SerializeStudyRecord {
        /// Underlying Serde serialization failure.
        #[source]
        source: serde_json::Error,
    },
    /// A serialized study record could not be written durably.
    #[error("failed to write study record `{path}`")]
    WriteStudyRecord {
        /// Destination study-record path.
        path: PathBuf,
        /// Underlying filesystem failure.
        #[source]
        source: io::Error,
    },
    /// A UTC timestamp required by a study record could not be formatted.
    #[error("failed to format UTC timestamp while attempting to {operation}")]
    StudyRecordTimestamp {
        /// Record operation that requested the timestamp.
        operation: &'static str,
        /// Underlying time-formatting failure.
        #[source]
        source: time::error::Format,
    },
    /// A study plan could not be encoded as JSON.
    #[error("failed to serialize study plan")]
    SerializeStudyPlan {
        /// Underlying Serde serialization failure.
        #[source]
        source: serde_json::Error,
    },
    /// A serialized study plan could not be written durably.
    #[error("failed to write study plan `{path}`")]
    WriteStudyPlan {
        /// Destination study-plan path.
        path: PathBuf,
        /// Underlying filesystem failure.
        #[source]
        source: io::Error,
    },
    /// A plan destination already contains a different immutable declaration.
    #[error("study plan destination `{path}` already contains different data")]
    StudyPlanConflict {
        /// Path containing the conflicting plan.
        path: PathBuf,
    },
    /// A phase failed after producing a durable partial study summary.
    #[error("study phase execution failed: {source}")]
    PhaseExecutionFailed {
        /// Summary and record containing all completed phase outcomes.
        summary: super::StudySummary,
        /// Original phase execution failure.
        #[source]
        source: Box<StudyError>,
    },
    /// Another study currently owns the process-wide terminal renderer.
    #[error("another study renderer already owns the process terminal")]
    TerminalAlreadyOwned,
    /// A phase was declared without tasks.
    #[error("phase {phase} must contain at least one task")]
    EmptyPhase {
        /// Invalid phase identity.
        phase: u64,
    },
    /// A study was declared without phases.
    #[error("a study must contain at least one phase")]
    EmptyPhaseSet,
    /// A phase label is empty or whitespace-only.
    #[error("phase {phase} must have a nonempty label")]
    InvalidPhaseLabel {
        /// Invalid phase identity.
        phase: u64,
    },
    /// A phase declares a zero active-task limit.
    #[error("phase {phase} max_active_tasks must be greater than zero")]
    InvalidPhaseWorkloadLimit {
        /// Invalid phase identity.
        phase: u64,
    },
    /// A phase declares a zero prepared-task queue capacity.
    #[error("phase {phase} prepared_task_queue_capacity must be greater than zero")]
    InvalidPhaseQueueCapacity {
        /// Invalid phase identity.
        phase: u64,
    },
    /// A phase timing setting is zero or cannot be represented internally.
    #[error("phase {phase} timing setting `{setting}` must be nonzero and representable")]
    InvalidPhaseTiming {
        /// Invalid phase identity.
        phase: u64,
        /// Name of the rejected timing setting.
        setting: &'static str,
    },
    /// Two phases have the same stable identity.
    #[error("phase ID {phase} appears more than once")]
    DuplicatePhaseId {
        /// Repeated phase identity.
        phase: u64,
    },
    /// A phase depends on an undeclared phase.
    #[error("phase {phase} depends on unknown phase {dependency}")]
    UnknownPhaseDependency {
        /// Depending phase identity.
        phase: u64,
        /// Missing dependency identity.
        dependency: u64,
    },
    /// The declared phase dependency graph is cyclic.
    #[error("phase dependency graph contains a cycle involving phase {phase}")]
    PhaseDependencyCycle {
        /// One phase involved in the cycle.
        phase: u64,
    },
    /// Execution selected a phase that is not declared.
    #[error("selected phase {phase} is not registered")]
    UnknownSelectedPhase {
        /// Missing selected phase identity.
        phase: u64,
    },
    /// Execution omitted a required dependency of a selected phase.
    #[error("selected phase {phase} requires unsatisfied phase {dependency}")]
    UnsatisfiedPhaseDependency {
        /// Selected phase identity.
        phase: u64,
        /// Omitted dependency identity.
        dependency: u64,
    },
    /// Application examination found invalid whole-phase state.
    #[error("phase {phase} completion is invalid: {reason}")]
    InvalidPhaseCompletion {
        /// Phase whose application-owned result was rejected.
        phase: u64,
        /// Concise application-supplied validation reason.
        reason: String,
    },
    /// A phase completion examiner panicked before execution started.
    #[error("phase {phase} completion examination panicked")]
    PhaseCompletionExaminationPanicked {
        /// Phase whose examiner panicked.
        phase: u64,
    },
    /// Interactive confirmation input ended before another phase could start.
    #[error("confirmation input ended after phase {phase} before the next phase could start")]
    PhaseConfirmationEof {
        /// Last completed phase identity.
        phase: u64,
    },
    /// Interactive confirmation input could not be read.
    #[error("failed to read confirmation after phase {phase}")]
    PhaseConfirmationInput {
        /// Last completed phase identity.
        phase: u64,
        /// Underlying input failure.
        #[source]
        source: io::Error,
    },
    /// An executable task declaration has no workload.
    #[error("task `{task}` has no workload")]
    MissingTaskWorkload {
        /// Phase-qualified task identity.
        task: String,
    },
    /// An application-owned task workload returned an error.
    #[error("task `{task}` failed: {source}")]
    TaskWorkload {
        /// Phase-qualified task identity.
        task: String,
        /// Application-owned workload failure.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },
    /// A task did not stop cooperatively before its timeout.
    #[error("task `{task}` exceeded its timeout of {timeout:?}")]
    TaskTimedOut {
        /// Phase-qualified task identity.
        task: String,
        /// Configured task timeout.
        timeout: Duration,
    },
    /// A phase did not stop cooperatively before its deadline.
    #[error("phase {phase} exceeded its deadline of {deadline_after:?}")]
    PhaseDeadlineExceeded {
        /// Expired phase identity.
        phase: u64,
        /// Configured phase deadline relative to its start.
        deadline_after: Duration,
    },
    /// A scheduler worker thread panicked.
    #[error("a study scheduler worker panicked")]
    SchedulerPanicked,
    /// Study execution was cancelled cooperatively.
    #[error("study execution was cancelled")]
    Cancelled,
    /// A task ID is empty or whitespace-only.
    #[error("phase {phase} contains an empty task ID")]
    InvalidTaskId {
        /// Phase containing the invalid task.
        phase: u64,
    },
    /// A task category is empty or whitespace-only.
    #[error("task `{task}` must have a nonempty category")]
    InvalidTaskCategory {
        /// Phase-qualified invalid task identity.
        task: String,
    },
    /// A phase repeats a task ID.
    #[error("phase {phase} repeats task ID `{task}`")]
    DuplicateTaskId {
        /// Phase containing the duplicate.
        phase: u64,
        /// Repeated phase-local task ID.
        task: String,
    },
    /// A task selector matched no declared task.
    #[error("task selector `{selector}` matched no task")]
    TaskNotFound {
        /// Unmatched selector text.
        selector: String,
    },
    /// An unqualified selector matched more than one task.
    #[error("task selector `{selector}` is ambiguous between `{first}` and `{second}`")]
    TaskSelectorAmbiguous {
        /// Ambiguous selector text.
        selector: String,
        /// First matching phase-qualified task identity.
        first: String,
        /// Second matching phase-qualified task identity.
        second: String,
    },
    /// A task does not contain requested metadata.
    #[error("task `{task}` does not contain metadata `{key}`")]
    UnknownTaskMetadata {
        /// Phase-qualified task identity.
        task: String,
        /// Missing metadata key.
        key: String,
    },
    /// Task metadata could not be decoded as the requested type.
    #[error("task `{task}` metadata `{key}` could not be decoded")]
    DecodeTaskMetadata {
        /// Phase-qualified task identity.
        task: String,
        /// Metadata key being decoded.
        key: String,
        /// Underlying Serde conversion failure.
        #[source]
        source: serde_json::Error,
    },
    /// A progress operation referenced an unregistered task.
    #[error("task `{task}` is not registered with this renderer")]
    UnknownTask {
        /// Unregistered phase-qualified task identity.
        task: String,
    },
    /// A progress operation is incompatible with the task's declared mode.
    #[error("task `{task}` is declared as {actual}, not {requested}")]
    TaskModeMismatch {
        /// Phase-qualified task identity.
        task: String,
        /// Mode required by the attempted operation.
        requested: &'static str,
        /// Mode declared by the task.
        actual: &'static str,
    },
    /// A task was started after already leaving its pending state.
    #[error("task `{identity}` has already started or reached a terminal status")]
    TaskAlreadyStarted {
        /// Phase-qualified task identity.
        identity: String,
    },
    /// Initial progress is greater than the declared target iteration.
    #[error("task `{identity}` starts at iteration {initial}, beyond target {target}")]
    InitialIterationBeyondTarget {
        /// Phase-qualified task identity.
        identity: String,
        /// Rejected initial iteration.
        initial: u64,
        /// Declared target iteration.
        target: u64,
    },
    /// A task attempted to report a lower iteration than previously observed.
    #[error("task `{identity}` cannot move progress from iteration {current} back to {attempted}")]
    IterationRegressed {
        /// Phase-qualified task identity.
        identity: String,
        /// Last accepted iteration.
        current: u64,
        /// Rejected lower iteration.
        attempted: u64,
    },
    /// A task reported progress beyond its target iteration.
    #[error("task `{identity}` reported iteration {iteration}, beyond target {target}")]
    IterationBeyondTarget {
        /// Phase-qualified task identity.
        identity: String,
        /// Rejected reported iteration.
        iteration: u64,
        /// Declared target iteration.
        target: u64,
    },
    /// A task reported completion before reaching its target iteration.
    #[error("task `{identity}` completed at iteration {current}, before target {target}")]
    TargetIterationNotReached {
        /// Phase-qualified task identity.
        identity: String,
        /// Last accepted iteration.
        current: u64,
        /// Declared target iteration.
        target: u64,
    },
    /// The renderer thread could not be created.
    #[error("failed to start the study renderer")]
    StartRenderer {
        /// Underlying thread-creation failure.
        #[source]
        source: io::Error,
    },
    /// The process terminal could not be prepared or restored.
    #[error("failed to {operation} for the study display")]
    TerminalSetup {
        /// Stable terminal operation description.
        operation: &'static str,
        /// Underlying terminal IO failure.
        #[source]
        source: io::Error,
    },
    /// The renderer stopped accepting progress commands unexpectedly.
    #[error("the study renderer is no longer available")]
    RendererUnavailable,
    /// The renderer thread panicked.
    #[error("the study renderer panicked")]
    RendererPanicked,
    /// A success summary was requested while non-success task states remain.
    #[error(
        "cannot report success with {pending} pending, {running} running, and {failed} failed tasks"
    )]
    IncompleteProgress {
        /// Number of tasks that never started.
        pending: u64,
        /// Number of tasks still running.
        running: u64,
        /// Number of failed tasks.
        failed: u64,
    },
}

impl StudyError {
    /// Returns the durable partial summary attached to a phase execution failure.
    pub fn study_summary(&self) -> Option<&super::StudySummary> {
        match self {
            Self::PhaseExecutionFailed { summary, .. } => Some(summary),
            _ => None,
        }
    }

    /// Returns the original cause attached to a phase execution failure.
    pub fn execution_cause(&self) -> Option<&StudyError> {
        match self {
            Self::PhaseExecutionFailed { source, .. } => Some(source),
            _ => None,
        }
    }
}
