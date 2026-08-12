use std::num::NonZeroU64;

use crate::{recording, validation, AppResult, hopf_model::HopfModel};
use scientific_workflow::prelude::*;

pub(crate) fn run_task(
    schema: &SystemStateSchema,
    execution: &ExecutionScope,
    task: TaskConfig,
    reporter: &ProgressReporter,
) -> AppResult<()> {
    let mut model = HopfModel::new(
        schema,
        task.decode_value("initial_point")?,
        task.decode_value("mu")?,
        task.decode_value("angular_frequency")?,
        task.decode_value("physical_time_increment_per_step")?,
    )?;

    let step_count: u64 = task.decode_value("step_count")?;
    let trajectory_sampling_interval: SamplingInterval =
        task.decode_value("trajectory_sampling_interval")?;
    let radius_sampling_interval: SamplingInterval = task.decode_value("radius_sampling_interval")?;
    let checkpoint_sampling_interval: SamplingInterval =
        task.decode_value("checkpoint_sampling_interval")?;
    let maximum_chunk_bytes: NonZeroU64 = task.decode_value("maximum_chunk_bytes")?;
    let writer_queue_bytes: NonZeroU64 = task.decode_value("writer_queue_bytes")?;

    let initial_iteration = model.state().simulation_time().iteration();
    let target_iteration = initial_iteration + step_count;
    let progress = reporter.start_task(&task, initial_iteration, Some(target_iteration))?;

    let directory = execution.task_recording_directory(task.task_ordinal());
    let recording = recording::record_task(
        schema,
        &directory,
        &task,
        step_count,
        trajectory_sampling_interval,
        radius_sampling_interval,
        checkpoint_sampling_interval,
        maximum_chunk_bytes,
        writer_queue_bytes,
        &mut model,
        &progress,
    )?;

    validation::validate_recording(
        model.state(),
        &recording,
        hopf_model::POINT_FIELD,
        hopf_model::RADIUS_FIELD,
    )?;

    progress.complete(None)?;
    Ok(())
}
