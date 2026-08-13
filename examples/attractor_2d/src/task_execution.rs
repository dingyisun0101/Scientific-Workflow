use crate::{AppResult, hopf_model::HopfModel, recording};
use scientific_workflow::prelude::basics::*;
use scientific_workflow::prelude::runtime::*;

pub(crate) fn run_task(
    schema: &SystemStateSchema,
    execution: &ExecutionScope,
    context: &TaskContext,
) -> AppResult<()> {
    let task = context.configuration();
    let (initial_point, mu, omega, dt): ([f64; 2], f64, f64, f64) = task.decode_values((
        "initial_point",
        "mu",
        "angular_frequency",
        "physical_time_increment_per_step",
    ))?;
    let mut model = HopfModel::new(schema, initial_point, mu, omega, dt)?;
    recording::record_task(
        schema,
        &execution.task_recording_directory(task.task_ordinal()),
        task,
        &mut model,
        context,
    )
}
