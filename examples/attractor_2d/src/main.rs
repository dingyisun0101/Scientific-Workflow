//! Minimal orchestration for a complete two-dimensional attractor workflow.
//!
//! Scientific evolution, storage configuration, and typed validation live in
//! small focused modules. This entry point demonstrates only the application
//! sequence that users repeat in a workflow-backed project.

mod hopf_model;
mod project_setup;
mod recording_validation;
mod state_recording;

use std::error::Error;
use std::path::PathBuf;

use rayon::prelude::*;
use scientific_workflow::prelude::*;

use project_setup::prepare_task;
use recording_validation::validate_recording;
use state_recording::record_model;

/// Error boundary shared by the example's application modules.
///
/// Library and model-specific errors retain their concrete source chains while
/// the executable remains independent of an application error dependency.
pub(crate) type AppResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

/// Runs every configured task and reports one minimal validation result.
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
        .start()?;

    // TaskConfig is an owned Send + Sync handle over shared immutable source
    // data. par_bridge therefore feeds the lazy Cartesian iterator directly to
    // Rayon's work-stealing pool without first collecting or cloning configs.
    project
        .task_configs()
        .par_bridge()
        .try_for_each(|task| -> AppResult<()> {
            // Every worker creates an independent model and recording writer;
            // only immutable schema and execution-scope handles are shared.
            let (mut model, settings) = prepare_task(schema, &task)?;
            let initial_iteration = model.state().simulation_time().iteration();
            let target_iteration = initial_iteration + settings.step_count;
            let progress = reporter.start_task(&task, initial_iteration, Some(target_iteration))?;
            let recording =
                record_model(schema, &execution, &task, &settings, &mut model, &progress)?;
            validate_recording(model.state(), &recording)?;
            progress.complete()?;
            Ok(())
        })?;

    reporter.complete(format!(
        "round_trip=true output={}",
        execution.directory().display()
    ))?;
    Ok(())
}
