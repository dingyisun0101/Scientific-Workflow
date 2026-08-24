//! Compact configuration-to-rendering scientific study.

mod hopf_model;
mod recording;
mod rendering;
mod task_execution;
mod validation;

use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::Duration;

use scientific_workflow::prelude::basics::*;
use scientific_workflow::prelude::study::*;

/// Error boundary shared by the example's application.
pub(crate) type AppResult<T> = Result<T, Box<dyn Error + Send + Sync>>;
const TASK_START_DELAY: Duration = Duration::from_secs(3);

fn main() -> AppResult<()> {
    let study_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let configurations = ConfigurationSpace::load(study_root.join("config"))?;
    let schema = SystemStateSchema::load_json_template(study_root.join("config/state.json"))?;
    let recording_root = recording_root(study_root)?;

    // An execution scope is only a deterministic path namespace. Creating it
    // does not create task recordings: each simulation task remains responsible
    // for opening, writing, and completing its own recording.
    let execution = ExecutionScope::create_generated(recording_root)?;
    let record = execution.directory().join("study-record.json");

    // Phase workloads are stored by the study and may execute on worker
    // threads, so their closures are `move` closures with a `'static` lifetime.
    // Both phases need the same schema/scope after `main` finishes constructing
    // them. Cloning these inexpensive owned handles lets each closure own what it
    // needs; it does not copy a schema on disk or create another execution.
    let simulation_schema = schema.clone();
    let simulation_execution = execution.clone();

    // The application explicitly maps every resolved configuration to a task.
    let simulation_tasks = configurations.combinations().map(|configuration| {
        let ordinal = configuration.ordinal();
        let mu = configuration.decode_value::<f64>("/mu").unwrap();
        let omega = configuration
            .decode_value::<f64>("/angular_frequency")
            .unwrap();
        let schema = simulation_schema.clone();
        let execution = simulation_execution.clone();
        Task::progress(
            format!("attractor-{ordinal}"),
            format!("attractor mu={mu} omega={omega}"),
            move |context| task_execution::run_task(&schema, &execution, &configuration, context),
        )
        .category("attractor")
        .metadata("configuration_ordinal", ordinal)
        .metadata("mu", mu)
        .metadata("angular_frequency", omega)
    });
    let simulation = Phase::builder(1, "attractor simulation")
        .tasks(simulation_tasks)
        // These are phase-local scheduling bounds. Machine-level CPU and memory
        // policy belongs to the external service manager running this process.
        .max_active_tasks(3)
        .prepared_task_queue_capacity(2)
        // Admission remains pending until each dense phase-local rank starts.
        // This staggers recording creation without sleeping inside workloads.
        .delay_per_task(TASK_START_DELAY)
        .build()?;

    // Validation receives independently generated tasks with matching ordinals.
    // It reconstructs the phase-1 path from that ordinal, making the completed
    // recording—not an in-memory scheduler result—the durable phase handoff.
    let validation_tasks = configurations.combinations().map(|configuration| {
        let ordinal = configuration.ordinal();
        let mu = configuration.decode_value::<f64>("/mu").unwrap();
        let omega = configuration
            .decode_value::<f64>("/angular_frequency")
            .unwrap();
        let execution = execution.clone();
        Task::one_shot(
            format!("validate-{ordinal}"),
            format!("validate mu={mu} omega={omega}"),
            move |context| validation::validate_recording(&execution, &configuration, context),
        )
        .category("validation")
        .metadata("configuration_ordinal", ordinal)
        .metadata("mu", mu)
        .metadata("angular_frequency", omega)
    });
    let validation = Phase::builder(2, "recording validation")
        .tasks(validation_tasks)
        .depends_on(1)
        .max_active_tasks(3)
        .prepared_task_queue_capacity(2)
        .delay_per_task(TASK_START_DELAY)
        .build()?;

    // Rendering is one ordinary application task. The study knows only that it
    // depends on validation; the task itself owns the Python process and files.
    let rendering_execution = execution.clone();
    let rendering_script = study_root.join("scripts/render_trajectories.py");
    let rendering_output = study_root.join("target").join("plots");
    let rendering = Phase::builder(3, "trajectory rendering")
        .task(
            Task::one_shot(
                "render-trajectories",
                "render all trajectories",
                move |context| {
                    rendering::render_trajectories(
                        &rendering_script,
                        rendering_execution.directory(),
                        &rendering_output,
                        context,
                    )
                },
            )
            .category("visualization")
            .metadata("configuration_count", configurations.combination_count()),
        )
        .depends_on(2)
        .build()?;

    // Requesting phase 3 with dependency expansion runs simulation and
    // validation first. The study coordinates ordering and display; each task
    // retains ownership of its scientific or visualization effects.
    Study::builder(record)
        .phases([simulation, validation, rendering])
        .build()?
        .run_phases_with_dependencies([3])?;
    Ok(())
}

fn recording_root(study_root: &Path) -> AppResult<PathBuf> {
    let paths: serde_json::Map<String, serde_json::Value> =
        serde_json::from_slice(&std::fs::read(study_root.join("config/paths.json"))?)?;
    let configured = paths
        .get("recording_root")
        .and_then(serde_json::Value::as_str)
        .ok_or("paths.json must contain string `recording_root`")?;
    let path = PathBuf::from(configured);
    Ok(if path.is_absolute() {
        path
    } else {
        study_root.join(path)
    })
}
