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

use rayon::prelude::*;
use scientific_workflow::prelude::*;

/// Error boundary shared by the example's application.
pub(crate) type AppResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

/// Runs every configured task and reports validation and correctness checks.
fn main() -> AppResult<()> {
    // By convention the standalone crate root is also the scientific project
    // root, and ScientificProject loads all four files under `config/`.
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let project = ScientificProject::load(&project_root)?;
    let schema = project.state_schema();

    // ExecutionScope owns collision-safe run-directory organization. The
    // application supplies only the configured parent directory.
    let recording_root = project.resolve_path("recording_root")?;
    let execution = ExecutionScope::create_generated(&recording_root)?;

    let reporter = ProgressReporter::for_project(&project)
        .identify_tasks_by(["mu"])
        .terminal()
        .start()?;

    // TaskConfig is an owned Send + Sync handle over shared immutable source data.
    // par_bridge therefore feeds the lazy Cartesian iterator directly to Rayon's
    // work-stealing pool without first collecting configs.
    let task_summaries = project
        .task_configs()
        .par_bridge()
        .map(|task| task_execution::run_task(&schema, &execution, task, &reporter))
        .collect::<AppResult<Vec<_>>>()?;

    cross_check::print_example_report(&task_summaries);

    reporter.complete(format!(
        "round_trip=true output={}",
        execution.directory().display()
    ))?;
    Ok(())
}
