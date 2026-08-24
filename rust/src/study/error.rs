//! Errors produced while planning, scheduling, and rendering a study.

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum StudyError {
    #[error("failed to serialize study record")]
    SerializeStudyRecord {
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to write study record `{path}`")]
    WriteStudyRecord {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to format UTC timestamp while attempting to {operation}")]
    StudyRecordTimestamp {
        operation: &'static str,
        #[source]
        source: time::error::Format,
    },
    #[error("failed to serialize study plan")]
    SerializeStudyPlan {
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to write study plan `{path}`")]
    WriteStudyPlan {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("study plan destination `{path}` already contains different data")]
    StudyPlanConflict { path: PathBuf },
    #[error("study phase execution failed: {source}")]
    PhaseExecutionFailed {
        summary: super::StudySummary,
        #[source]
        source: Box<StudyError>,
    },
    #[error("another study renderer already owns the process terminal")]
    TerminalAlreadyOwned,
    #[error("phase {phase} must contain at least one task")]
    EmptyPhase { phase: u64 },
    #[error("a study must contain at least one phase")]
    EmptyPhaseSet,
    #[error("phase {phase} must have a nonempty label")]
    InvalidPhaseLabel { phase: u64 },
    #[error("phase {phase} max_active_tasks must be greater than zero")]
    InvalidPhaseWorkloadLimit { phase: u64 },
    #[error("phase {phase} prepared_task_queue_capacity must be greater than zero")]
    InvalidPhaseQueueCapacity { phase: u64 },
    #[error("phase {phase} timing setting `{setting}` must be nonzero and representable")]
    InvalidPhaseTiming { phase: u64, setting: &'static str },
    #[error("phase ID {phase} appears more than once")]
    DuplicatePhaseId { phase: u64 },
    #[error("phase {phase} depends on unknown phase {dependency}")]
    UnknownPhaseDependency { phase: u64, dependency: u64 },
    #[error("phase dependency graph contains a cycle involving phase {phase}")]
    PhaseDependencyCycle { phase: u64 },
    #[error("selected phase {phase} is not registered")]
    UnknownSelectedPhase { phase: u64 },
    #[error("selected phase {phase} requires unsatisfied phase {dependency}")]
    UnsatisfiedPhaseDependency { phase: u64, dependency: u64 },
    #[error("confirmation input ended after phase {phase} before the next phase could start")]
    PhaseConfirmationEof { phase: u64 },
    #[error("failed to read confirmation after phase {phase}")]
    PhaseConfirmationInput {
        phase: u64,
        #[source]
        source: io::Error,
    },
    #[error("task `{task}` has no workload")]
    MissingTaskWorkload { task: String },
    #[error("task `{task}` failed: {source}")]
    TaskWorkload {
        task: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },
    #[error("task `{task}` exceeded its timeout of {timeout:?}")]
    TaskTimedOut { task: String, timeout: Duration },
    #[error("phase {phase} exceeded its deadline of {deadline_after:?}")]
    PhaseDeadlineExceeded {
        phase: u64,
        deadline_after: Duration,
    },
    #[error("a study scheduler worker panicked")]
    SchedulerPanicked,
    #[error("study execution was cancelled")]
    Cancelled,
    #[error("phase {phase} contains an empty task ID")]
    InvalidTaskId { phase: u64 },
    #[error("task `{task}` must have a nonempty category")]
    InvalidTaskCategory { task: String },
    #[error("phase {phase} repeats task ID `{task}`")]
    DuplicateTaskId { phase: u64, task: String },
    #[error("task selector `{selector}` matched no task")]
    TaskNotFound { selector: String },
    #[error("task selector `{selector}` is ambiguous between `{first}` and `{second}`")]
    TaskSelectorAmbiguous {
        selector: String,
        first: String,
        second: String,
    },
    #[error("task `{task}` does not contain metadata `{key}`")]
    UnknownTaskMetadata { task: String, key: String },
    #[error("task `{task}` metadata `{key}` could not be decoded")]
    DecodeTaskMetadata {
        task: String,
        key: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("task `{task}` is not registered with this renderer")]
    UnknownTask { task: String },
    #[error("task `{task}` is declared as {actual}, not {requested}")]
    TaskModeMismatch {
        task: String,
        requested: &'static str,
        actual: &'static str,
    },
    #[error("task `{identity}` has already started or reached a terminal status")]
    TaskAlreadyStarted { identity: String },
    #[error("task `{identity}` starts at iteration {initial}, beyond target {target}")]
    InitialIterationBeyondTarget {
        identity: String,
        initial: u64,
        target: u64,
    },
    #[error("task `{identity}` cannot move progress from iteration {current} back to {attempted}")]
    IterationRegressed {
        identity: String,
        current: u64,
        attempted: u64,
    },
    #[error("task `{identity}` reported iteration {iteration}, beyond target {target}")]
    IterationBeyondTarget {
        identity: String,
        iteration: u64,
        target: u64,
    },
    #[error("task `{identity}` completed at iteration {current}, before target {target}")]
    TargetIterationNotReached {
        identity: String,
        current: u64,
        target: u64,
    },
    #[error("failed to start the study renderer")]
    StartRenderer {
        #[source]
        source: io::Error,
    },
    #[error("failed to {operation} for the study display")]
    TerminalSetup {
        operation: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("the study renderer is no longer available")]
    RendererUnavailable,
    #[error("the study renderer panicked")]
    RendererPanicked,
    #[error(
        "cannot report success with {pending} pending, {running} running, and {failed} failed tasks"
    )]
    IncompleteProgress {
        pending: u64,
        running: u64,
        failed: u64,
    },
}

impl StudyError {
    pub fn study_summary(&self) -> Option<&super::StudySummary> {
        match self {
            Self::PhaseExecutionFailed { summary, .. } => Some(summary),
            _ => None,
        }
    }

    pub fn execution_cause(&self) -> Option<&StudyError> {
        match self {
            Self::PhaseExecutionFailed { source, .. } => Some(source),
            _ => None,
        }
    }
}
