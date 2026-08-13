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

use std::collections::HashMap;
use std::error::Error;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use scientific_workflow::prelude::basics::*;
use scientific_workflow::prelude::runtime::*;

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

    let pending_validation = Arc::new(Mutex::new(HashMap::new()));
    let simulation_results = Arc::clone(&pending_validation);
    let simulation = Phase::builder(1, "attractor simulation")
        .progress_workloads_from_project(&project, "attractor", move |_| {
            let schema = schema.clone();
            let execution = execution.clone();
            let results = Arc::clone(&simulation_results);
            move |context| {
                let summary = task_execution::run_task(&schema, &execution, context)?;
                results
                    .lock()
                    .unwrap()
                    .insert(summary.task_ordinal, summary);
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

    let validated_tasks = Arc::new(Mutex::new(Vec::new()));
    let validation_inputs = Arc::clone(&pending_validation);
    let validation_results = Arc::clone(&validated_tasks);
    let validation = Phase::builder(2, "recording validation")
        .activity_workloads_from_project(&project, "validate", move |task_config| {
            let ordinal = task_config.task_ordinal();
            let inputs = Arc::clone(&validation_inputs);
            let results = Arc::clone(&validation_results);
            move |context| {
                context.set_detail("reading final checkpoint");
                let summary = inputs
                    .lock()
                    .unwrap()
                    .remove(&ordinal)
                    .ok_or_else(|| format!("simulation result for task {ordinal} is missing"))?;
                let final_state = validation::read_final_checkpoint(
                    &summary.recording_directory,
                    hopf_model::POINT_FIELD,
                    hopf_model::RADIUS_FIELD,
                )?;
                let initial_point: Vec<f64> = context.decode_value("initial_point")?;
                let initial_point: [f64; 2] = initial_point.try_into().map_err(|point: Vec<f64>| {
                    format!("initial_point must contain two values, got {}", point.len())
                })?;
                cross_check::assert_matches_reference(
                    &final_state,
                    initial_point,
                    context.decode_value("mu")?,
                    context.decode_value("angular_frequency")?,
                    context.decode_value("physical_time_increment_per_step")?,
                    context.decode_value("step_count")?,
                )?;
                context.set_detail("checkpoint and reference verified");
                results.lock().unwrap().push(summary);
                Ok(())
            }
        })
        .display_tasks_by("validate", ["mu"])
        .depends_on(1)
        .max_concurrent_workloads(2)
        .queue_capacity(1)
        .build()?;

    WorkflowRuntime::builder()
        .phases([simulation, validation])
        .automatic()
        .build()?
        .run_phases_with_dependencies([2])?;

    let task_summaries = validated_tasks.lock().unwrap();

    cross_check::print_example_report(&task_summaries);

    Ok(())
}
