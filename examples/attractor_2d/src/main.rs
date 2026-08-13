//! Compact configuration-to-validation scientific workflow.

mod hopf_model;
mod recording;
mod task_execution;
mod validation;

use std::error::Error;
use std::thread;
use std::time::Duration;

use scientific_workflow::prelude::basics::*;
use scientific_workflow::prelude::runtime::*;

/// Error boundary shared by the example's application.
pub(crate) type AppResult<T> = Result<T, Box<dyn Error + Send + Sync>>;
const DISPLAY_PAUSE: Duration = Duration::from_secs(3);

fn main() -> AppResult<()> {
    let project = ScientificProject::load(env!("CARGO_MANIFEST_DIR"))?;
    let execution = ExecutionScope::create_generated(project.resolve_path("recording_root")?)?;

    let simulation_schema = project.state_schema().clone();
    let simulation_execution = execution.clone();
    let simulation = Phase::builder(1, "attractor simulation")
        .progress_tasks_from_project(&project, "attractor", move |context| {
            pause_for_display(context, "simulation starts in 3 seconds");
            task_execution::run_task(&simulation_schema, &simulation_execution, context)
        })
        .display_tasks_by("attractor", ["mu"])
        .max_concurrent_workloads(3)
        .queue_capacity(2)
        .build()?;

    let validation = Phase::builder(2, "recording validation")
        .activity_tasks_from_project(&project, "validate", move |context| {
            pause_for_display(context, "validation starts in 3 seconds");
            validation::validate_recording(&execution, context)
        })
        .display_tasks_by("validate", ["mu"])
        .depends_on(1)
        .max_concurrent_workloads(3)
        .queue_capacity(2)
        .build()?;

    WorkflowRuntime::builder()
        .phases([simulation, validation])
        .build()?
        .run_phases_with_dependencies([2])?;
    Ok(())
}

fn pause_for_display(context: &TaskContext, detail: &str) {
    context.set_detail(detail);
    thread::sleep(DISPLAY_PAUSE);
}
