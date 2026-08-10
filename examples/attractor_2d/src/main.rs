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
use std::process;

use scientific_workflow::prelude::*;

use project_setup::prepare_task;
use recording_validation::validate_recording;
use state_recording::record_model;

/// Error boundary shared by the example's application modules.
///
/// Library and model-specific errors retain their concrete source chains while
/// the executable remains independent of an application error dependency.
pub(crate) type AppResult<T> = Result<T, Box<dyn Error>>;

/// Reports one terminal failure and selects an unsuccessful process status.
fn main() {
    if let Err(error) = run() {
        eprintln!("[error] {error}");
        process::exit(1);
    }
}

/// Runs every configured task and reports one minimal validation result.
fn run() -> AppResult<()> {
    // By convention the standalone crate root is also the scientific project
    // root, and ScientificProject loads all four files under `config/`.
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let project = ScientificProject::load(&project_root)?;
    let schema = project.state_schema();

    // ExecutionScope owns collision-safe run-directory organization. The
    // application supplies only the configured parent directory.
    let recording_root = project.paths().resolve_path("recording_root")?;
    let execution = ExecutionScope::create_generated(&recording_root)?;

    let mut validated_tasks = 0_u64;
    for parameters in project.parameters().tasks() {
        // Each resolved sweep task receives an independent state owner and an
        // independent bounded writer rooted under the shared execution scope.
        let (mut model, settings) = prepare_task(schema, &parameters)?;
        let recording = record_model(schema, &execution, &parameters, &settings, &mut model)?;

        validate_recording(model.state(), &recording)?;
        validated_tasks += 1;
    }

    // This is the only normal output: reaching it proves that configuration,
    // evolution, recording, decoding, and exact final-state comparison passed
    // for every task. Any failure instead exits through main's error boundary.
    println!(
        "[validation] tasks={} round_trip=true output={}",
        validated_tasks,
        execution.directory().display()
    );
    Ok(())
}
