//! Compact configuration-to-validation scientific workflow.

mod hopf_model;
mod recording;
mod task_execution;
mod validation;

use std::error::Error;
use std::time::Duration;

use scientific_workflow::prelude::basics::*;
use scientific_workflow::prelude::runtime::*;

/// Error boundary shared by the example's application.
pub(crate) type AppResult<T> = Result<T, Box<dyn Error + Send + Sync>>;
const TASK_START_DELAY: Duration = Duration::from_secs(3);

fn main() -> AppResult<()> {
    // ScientificProject is the example's configuration boundary. It loads the
    // conventional fixed, sweep, path, and state-schema files and generates the
    // resolved TaskConfig values used below; no application settings struct or
    // hand-written task registry is needed.
    let project = ScientificProject::load(env!("CARGO_MANIFEST_DIR"))?;

    // An execution scope is only a deterministic path namespace. Creating it
    // does not create task recordings: each simulation task remains responsible
    // for opening, writing, and completing its own recording.
    let execution = ExecutionScope::create_generated(project.resolve_path("recording_root")?)?;

    // Phase workloads are stored by the runtime and may execute on worker
    // threads, so their closures are `move` closures with a `'static` lifetime.
    // Both phases need the same schema/scope after `main` finishes constructing
    // them. Cloning these inexpensive owned handles lets each closure own what it
    // needs; it does not copy a schema on disk or create another execution.
    let simulation_schema = project.state_schema().clone();
    let simulation_execution = execution.clone();

    // The shared-workload helper generates one task from every resolved
    // configuration and applies this callable to it. A per-task FnOnce factory
    // is only needed when individual tasks must capture unique owned resources.
    let simulation = Phase::builder(1, "attractor simulation")
        .progress_tasks_from_project(&project, "attractor", move |context| {
            task_execution::run_task(&simulation_schema, &simulation_execution, context)
        })
        // `mu` is a concise display selector, not a second task identity. The
        // generated task id and complete resolved parameters remain canonical.
        .display_tasks_by("attractor", ["/mu"])
        // These are phase-local scheduling bounds. Machine-level CPU and memory
        // policy belongs to the external service manager running this process.
        .max_concurrent_workloads(3)
        .queue_capacity(2)
        // Admission remains pending until each dense phase-local rank starts.
        // This staggers recording creation without sleeping inside workloads.
        .delay_per_task(TASK_START_DELAY)
        .build()?;

    // Validation receives independently generated tasks with matching ordinals.
    // It reconstructs the phase-1 path from that ordinal, making the completed
    // recording—not an in-memory runtime result—the durable phase handoff.
    let validation = Phase::builder(2, "recording validation")
        .activity_tasks_from_project(&project, "validate", move |context| {
            validation::validate_recording(&execution, context)
        })
        .display_tasks_by("validate", ["/mu"])
        .depends_on(1)
        .max_concurrent_workloads(3)
        .queue_capacity(2)
        .delay_per_task(TASK_START_DELAY)
        .build()?;

    // Requesting phase 2 with dependency expansion runs phase 1 first. Runtime
    // coordinates phase order, bounded scheduling, cancellation, and display;
    // it never owns the model's scientific I/O.
    WorkflowRuntime::builder()
        .phases([simulation, validation])
        .build()?
        .run_phases_with_dependencies([2])?;
    Ok(())
}
