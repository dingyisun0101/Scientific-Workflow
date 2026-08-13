//! Centralized, parallel-safe progress and terminal reporting.
//!
//! First-class [`Phase`] values own [`Task`] declarations before reporting
//! begins. Configuration helpers generate those tasks from complete fixed and
//! swept parameter views, while [`TaskSelector`] supports exact partial lookup.
//! [`ProgressReporter`] observes the supplied identities and becomes the sole
//! human-facing terminal writer for its lifetime; it does not create or own
//! phases or tasks. Worker threads receive non-clone [`TaskProgress`] or
//! [`ActivityTask`] handles.
//!
//! Iteration updates use per-task atomics. One renderer thread polls them at a
//! bounded frequency, so numerical workers never draw progress bars or contend
//! on terminal locks. Interactive stderr receives a multi-progress display;
//! redirected stderr receives stable lifecycle lines. Tests and embedding
//! applications may retain tracking while selecting hidden output.
//!
//! # Minimal parallel use
//!
//! ```no_run
//! use scientific_workflow::prelude::*;
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let project = ScientificProject::load("project-root")?;
//! let reporter = ProgressReporter::for_project(&project)
//!     .identify_tasks_by(["temperature", "seed"])
//!     .start()?;
//! # let task = project.task_config(0)?;
//! let progress = reporter.start_task(&task, 0, Some(1_000))?;
//! progress.set_iteration(1_000)?;
//! progress.complete(None)?;
//! # for task in project.task_configs().skip(1) {
//! #     reporter.start_task(&task, 0, Some(0))?.complete(None)?;
//! # }
//! let summary = reporter.complete("scientific work completed")?;
//! assert!(summary.is_success());
//! # Ok(())
//! # }
//! ```

mod error;
mod phase;
mod progress;

pub use error::ReportingError;
pub use phase::{
    Phase, PhaseBuilder, PhaseId, Task, TaskDisplayKind, TaskId, TaskKey, TaskSelector,
};
pub use progress::{
    ActivityTask, CancellationToken, PhaseProgressReporterBuilder, ProgressReporter,
    ProgressReporterBuilder, ProgressSummary, RegisteredProgressReporterBuilder, TaskIdentity,
    TaskProgress, TaskStatus,
};
