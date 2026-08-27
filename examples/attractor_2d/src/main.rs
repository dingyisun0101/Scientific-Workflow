//! Compact configuration-to-rendering scientific study.

mod attractor_run;
mod hopf_model;
mod recording;
mod rendering;
mod task_execution;
mod validation;

use std::error::Error;
use std::path::Path;
use std::time::Duration;

use scientific_workflow::prelude::basic::*;
use scientific_workflow::prelude::study::*;
use scientific_workflow::study::Task as StudyTask;

use attractor_run::AttractorRun;

/// Error boundary shared by the example's application.
pub(crate) type AppResult<T> = Result<T, Box<dyn Error + Send + Sync>>;
const TASK_START_DELAY: Duration = Duration::from_secs(3);

fn main() -> AppResult<()> {
    let study_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let settings = StudySettings::load(study_root)?;
    let output_root = ProjectPaths::load(study_root)?.resolve_path("output_root")?;
    let Some(replicate) = ReplicateExecutor::new(settings.replicate_settings(), output_root)
        .dispatch_current_executable()?
    else {
        return Ok(());
    };
    run_replicate(study_root, &replicate)
}

fn run_replicate(study_root: &Path, replicate: &ReplicateContext) -> AppResult<()> {
    let study_configuration = StudyConfiguration::load(study_root)?;
    let simulation_configurations = study_configuration.workload("attractor", "dynamics")?;
    let validation_configurations = study_configuration.workload("attractor", "validation")?;
    let schema = SystemStateSchema::load_json_template(&study_root.join("config/state.json"))?;

    // Replicate dispatch created this isolated scope before starting the worker.
    // It does not create task recordings: each simulation task remains
    // responsible for opening, writing, and completing its own recording.
    let execution = replicate.execution_scope();
    let record = execution.directory().join("study-record.json");

    // Producer descriptors decode configuration once and carry the exact output
    // path into every consumer. Phase-local ordinals are never compared across
    // independently expanded workload configuration spaces.
    let producer_runs = simulation_configurations
        .combinations()
        .map(|configuration| AttractorRun::new(execution, configuration))
        .collect::<AppResult<Vec<_>>>()?;
    let producer_count = u64::try_from(producer_runs.len())?;
    let rendering_recordings = producer_runs
        .iter()
        .map(|producer| producer.recording_directory().to_path_buf())
        .collect::<Vec<_>>();
    let simulation_tasks = producer_runs
        .iter()
        .cloned()
        .map(|run| simulation_task(run, schema.clone()));
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

    // Each validation configuration is paired explicitly with every producer
    // from the same global and group-shared selection. Independent phase-local
    // sweeps may therefore change cardinality without redirecting a consumer.
    let validation_tasks = validation_tasks(&producer_runs, &validation_configurations)?;
    let validation = Phase::builder(2, "recording validation")
        .tasks(validation_tasks)
        .depends_on(1)
        .max_active_tasks(3)
        .prepared_task_queue_capacity(2)
        .delay_per_task(TASK_START_DELAY)
        .build()?;

    // Rendering is one ordinary application task. The study knows only that it
    // depends on validation; the task itself owns the Python process and files.
    let rendering_script = study_root.join("scripts/render_trajectories.py");
    let rendering_output = replicate.output_directory().join("plots");
    let rendering = Phase::builder(3, "trajectory rendering")
        .task(
            StudyTask::one_shot(
                "render-trajectories",
                "render all trajectories",
                move |context| {
                    rendering::render_trajectories(
                        &rendering_script,
                        &rendering_recordings,
                        &rendering_output,
                        context,
                    )
                },
            )
            .category("visualization")
            .metadata("configuration_count", producer_count),
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

fn simulation_task(run: AttractorRun, schema: SystemStateSchema) -> StudyTask {
    let provenance = run.configuration().clone();
    let task_id = run.task_id().to_owned();
    let label = format!(
        "attractor mu={} omega={}",
        run.mu(),
        run.angular_frequency()
    );
    StudyTask::progress(task_id, label, move |context| {
        task_execution::run_task(
            &schema,
            run.recording_directory(),
            run.configuration(),
            context,
        )
    })
    .category("attractor")
    .with_configuration(&provenance)
}

fn validation_tasks(
    producers: &[AttractorRun],
    configurations: &WorkloadConfiguration,
) -> AppResult<Vec<StudyTask>> {
    let mut tasks = Vec::new();
    for configuration in configurations.combinations() {
        let mut matched = false;
        for producer in producers
            .iter()
            .filter(|producer| producer.matches_validation(&configuration))
        {
            matched = true;
            tasks.push(validation_task(producer.clone(), configuration.clone()));
        }
        if !matched {
            return Err(format!(
                "validation configuration {} has no producer in the same global/group selection",
                configuration.ordinal()
            )
            .into());
        }
    }
    Ok(tasks)
}

fn validation_task(producer: AttractorRun, configuration: ResolvedConfiguration) -> StudyTask {
    let provenance = configuration.clone();
    let task_id = format!(
        "validate-{}-v{:06}",
        producer.task_id(),
        configuration.workload_ordinal()
    );
    let label = format!(
        "validate mu={} omega={}",
        producer.mu(),
        producer.angular_frequency()
    );
    let producer_metadata = serde_json::json!({
        "task_id": producer.task_id(),
        "configuration_ordinal": producer.configuration().ordinal(),
        "recording_directory": producer.recording_directory(),
    });
    StudyTask::one_shot(task_id, label, move |context| {
        validation::validate_recording(
            producer.recording_directory(),
            producer.configuration(),
            context,
        )
    })
    .category("validation")
    .metadata("producer", producer_metadata)
    .with_configuration(&provenance)
}
