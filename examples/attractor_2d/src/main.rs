//! Compact project-input-to-rendering scientific study.

mod attractor_run;
mod hopf_model;
mod recording;
mod rendering;
mod task_execution;
mod validation;

use std::error::Error;
use std::path::Path;

use scientific_workflow::config::advanced::{
    PhaseSpecification, ProjectSpecification, ResolvedTaskInput,
};
use scientific_workflow::prelude::basic::*;
use scientific_workflow::prelude::study::*;
use scientific_workflow::state::advanced::StateSchemaAccess;
use scientific_workflow::study::Task as StudyTask;

use attractor_run::AttractorRun;

/// Error boundary shared by the example's application.
pub(crate) type AppResult<T> = Result<T, Box<dyn Error + Send + Sync>>;

fn main() -> AppResult<()> {
    let study_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let project = ProjectSpecification::load(study_root)?;
    let output_root = study_root.join("target/output");
    let Some(replicate) =
        ReplicateExecutor::new(project.manifest().replicate_policy(), output_root)
            .dispatch_current_executable()?
    else {
        return Ok(());
    };
    run_replicate(study_root, &replicate, &project)
}

fn run_replicate(
    study_root: &Path,
    replicate: &ReplicateContext,
    project: &ProjectSpecification,
) -> AppResult<()> {
    let simulation_specification = phase_specification(project, "simulate")?;
    let validation_specification = phase_specification(project, "validate")?;
    let state_document = project.state_schema();
    let schema = <SystemStateSchema as StateSchemaAccess>::from_json_template_value(
        state_document.path(),
        state_document.json_value(),
    )?;

    // Replicate dispatch created this isolated scope before starting the worker.
    // It does not create task recordings: each simulation task remains
    // responsible for opening, writing, and completing its own recording.
    let execution = replicate.execution_scope();
    let record = execution.directory().join("study-record.json");

    // Producer descriptors decode each resolved input once and carry the exact
    // output path into every consumer.
    let producer_runs = simulation_specification
        .tasks()
        .iter()
        .cloned()
        .map(|input| AttractorRun::new(execution, input))
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
        .max_active_tasks(simulation_specification.max_concurrency())
        .prepared_task_queue_capacity(simulation_specification.max_concurrency())
        // Admission remains pending until each dense phase-local rank starts.
        // This staggers recording creation without sleeping inside workloads.
        .delay_per_task(simulation_specification.start_interval())
        .build()?;

    // Each validation input is paired explicitly with the producer having the
    // same scientific parameter values.
    let validation_tasks = validation_tasks(&producer_runs, validation_specification.tasks())?;
    let validation = Phase::builder(2, "recording validation")
        .tasks(validation_tasks)
        .depends_on(1)
        .max_active_tasks(validation_specification.max_concurrency())
        .prepared_task_queue_capacity(validation_specification.max_concurrency())
        .delay_per_task(validation_specification.start_interval())
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
    let task_id = run.task_id().to_owned();
    let input_ordinal = run.input().ordinal();
    let resolved_input: serde_json::Value =
        serde_json::from_slice(run.input().resolved_json()).expect("config produced valid JSON");
    let label = format!(
        "attractor mu={} omega={}",
        run.mu(),
        run.angular_frequency()
    );
    StudyTask::progress(task_id, label, move |context| {
        task_execution::run_task(&schema, run.recording_directory(), &run, context)
    })
    .category("attractor")
    .metadata("input_ordinal", input_ordinal)
    .metadata("resolved_task_input", resolved_input)
}

fn validation_tasks(
    producers: &[AttractorRun],
    inputs: &[ResolvedTaskInput],
) -> AppResult<Vec<StudyTask>> {
    let mut tasks = Vec::new();
    for input in inputs {
        let constants = input.decode::<attractor_run::AttractorConstants>()?;
        let mut matched = false;
        for producer in producers
            .iter()
            .filter(|producer| producer.matches_validation(&constants))
        {
            matched = true;
            tasks.push(validation_task(producer.clone(), input.clone()));
        }
        if !matched {
            return Err(format!(
                "validation input {} has no matching producer",
                input.ordinal()
            )
            .into());
        }
    }
    Ok(tasks)
}

fn validation_task(producer: AttractorRun, input: ResolvedTaskInput) -> StudyTask {
    let task_id = format!("validate-{}-v{:06}", producer.task_id(), input.ordinal());
    let label = format!(
        "validate mu={} omega={}",
        producer.mu(),
        producer.angular_frequency()
    );
    let producer_metadata = serde_json::json!({
        "task_id": producer.task_id(),
        "input_ordinal": producer.input().ordinal(),
        "recording_directory": producer.recording_directory(),
    });
    let resolved_input: serde_json::Value =
        serde_json::from_slice(input.resolved_json()).expect("config produced valid JSON");
    let input_ordinal = input.ordinal();
    let expected_iteration = producer.constants().step_count;
    StudyTask::one_shot(task_id, label, move |context| {
        validation::validate_recording(producer.recording_directory(), expected_iteration, context)
    })
    .category("validation")
    .metadata("producer", producer_metadata)
    .metadata("input_ordinal", input_ordinal)
    .metadata("resolved_task_input", resolved_input)
}

fn phase_specification<'a>(
    project: &'a ProjectSpecification,
    name: &str,
) -> AppResult<&'a PhaseSpecification> {
    project
        .phases()
        .iter()
        .find(|phase| phase.name() == name)
        .ok_or_else(|| format!("study manifest does not declare phase `{name}`").into())
}
