use std::num::NonZeroU64;

use crate::{
    AppResult,
    hopf_model::HopfModel,
    recording,
};
use scientific_workflow::prelude::basics::*;
use scientific_workflow::prelude::runtime::*;
use std::path::PathBuf;

pub(crate) struct TaskExecutionSummary {
    pub(crate) task_ordinal: u64,
    pub(crate) recording_directory: PathBuf,
}

pub(crate) fn run_task(
    schema: &SystemStateSchema,
    execution: &ExecutionScope,
    context: &TaskContext,
) -> AppResult<TaskExecutionSummary> {
    let task = context.configuration();
    let initial_point: Vec<f64> = task.decode_value("initial_point")?;
    let mu: f64 = task.decode_value("mu")?;
    let angular_frequency: f64 = task.decode_value("angular_frequency")?;
    let physical_time_increment_per_step: f64 =
        task.decode_value("physical_time_increment_per_step")?;

    if initial_point.len() != 2 {
        return Err(format!(
            "initial_point must contain exactly two values, got {}",
            initial_point.len()
        )
        .into());
    }

    let task_ordinal = task.task_ordinal();

    let mut model = HopfModel::new(
        schema,
        initial_point,
        mu,
        angular_frequency,
        physical_time_increment_per_step,
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
    let progress = context
        .progress_handle()
        .ok_or("attractor task requires iterative progress")?;
    progress.set_iteration(initial_iteration)?;
    progress.set_target_iteration(target_iteration)?;

    let directory = execution.task_recording_directory(task_ordinal);
    let recording = recording::record_task(
        schema,
        &directory,
        task,
        step_count,
        trajectory_sampling_interval,
        radius_sampling_interval,
        checkpoint_sampling_interval,
        maximum_chunk_bytes,
        writer_queue_bytes,
        &mut model,
        &progress,
    )?;

    Ok(TaskExecutionSummary {
        task_ordinal,
        recording_directory: recording.directory().to_path_buf(),
    })
}
