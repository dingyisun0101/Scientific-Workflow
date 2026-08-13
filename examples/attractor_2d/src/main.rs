//! Minimal orchestration for a complete two-dimensional attractor workflow.
//!
//! Read files in this order:
//! 1. [`main.rs`] for the orchestration sequence.
//! 2. [`task_execution.rs`] for per-task orchestration.
//! 3. [`recording.rs`] for writer creation and sample cadence.
//! 4. [`validation.rs`] for round-trip validation strategy.
//! 5. [`cross_check.rs`] for the numerical correctness check.
//! 6. [`hopf_model.rs`] for the scientific model implementation.

mod cross_check;
mod hopf_model;
mod recording;
mod task_execution;
mod validation;

use std::error::Error;
use std::path::PathBuf;

use scientific_workflow::prelude::*;
use std::sync::{Arc, Mutex};

/// Error boundary shared by the example's application.
pub(crate) type AppResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

/// Runs every configured task and reports validation and correctness checks.
fn main() -> AppResult<()> {
    // By convention the standalone crate root is also the scientific project
    // root, and ScientificProject loads all four files under `config/`.
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let project = ScientificProject::load(&project_root)?;
    let schema = project.state_schema().clone();

    // ExecutionScope owns collision-safe run-directory organization. The
    // application supplies only the configured parent directory.
    let recording_root = project.resolve_path("recording_root")?;
    let execution = ExecutionScope::create_generated(&recording_root)?;

    let task_summaries = Arc::new(Mutex::new(Vec::new()));
    let summaries = Arc::clone(&task_summaries);
    let phase = Phase::builder(1, "attractor simulation")
        .progress_workloads_from_project(&project, "attractor", move |_| {
            let schema = schema.clone();
            let execution = execution.clone();
            let summaries = Arc::clone(&summaries);
            move |context| {
                let summary = task_execution::run_task(&schema, &execution, context)?;
                summaries.lock().unwrap().push(summary);
                Ok(())
            }
        })
        .display_tasks_by("attractor", ["mu"])
        .max_concurrent_workloads(
            std::thread::available_parallelism()
                .map(usize::from)
                .unwrap_or(1),
        )
        .queue_capacity(2)
        .build()?;
    WorkflowRuntime::builder()
        .phase(phase)
        .terminal()
        .build()?
        .run_phases([1])?;

    let task_summaries = task_summaries.lock().unwrap().clone();

    cross_check::print_example_report(&task_summaries);

    Ok(())
}
