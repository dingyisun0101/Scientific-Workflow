use std::num::NonZeroU64;

use crate::AppResult;
use scientific_workflow::prelude::basics::*;
use scientific_workflow::prelude::runtime::TaskProgress;

use crate::hopf_model::{POINT_FIELD, RADIUS_FIELD};

/// Stream containing two-dimensional phase points.
pub(crate) const TRAJECTORY_STREAM: &str = "trajectory";

/// Stream containing scalar radial diagnostics.
pub(crate) const RADIUS_STREAM: &str = "radius";

/// Stream containing restart-capable state snapshots.
pub(crate) const CHECKPOINT_STREAM: &str = "checkpoint";

pub(crate) fn record_task(
    schema: &SystemStateSchema,
    directory: &std::path::Path,
    task: &TaskConfig,
    step_count: u64,
    trajectory_sampling_interval: SamplingInterval,
    radius_sampling_interval: SamplingInterval,
    checkpoint_sampling_interval: SamplingInterval,
    maximum_chunk_bytes: NonZeroU64,
    writer_queue_bytes: NonZeroU64,
    model: &mut crate::hopf_model::HopfModel,
    progress: &TaskProgress,
) -> AppResult<CompletedRecording> {
    // The scope derives a stable task path but does not create it. Exclusive
    // directory creation belongs to the writer, preventing accidental reuse.
    let mut writer = build_writer(
        schema,
        directory,
        task,
        trajectory_sampling_interval,
        radius_sampling_interval,
        checkpoint_sampling_interval,
        maximum_chunk_bytes,
        writer_queue_bytes,
    )?;

    // The initial condition is a legitimate sample at iteration zero.
    writer.observe_state(model.state())?;
    for _ in 0..step_count {
        model.step()?;

        // Progress observes the model's authoritative absolute iteration. It
        // never owns or independently increments scientific time.
        progress.set_iteration(model.state().simulation_time().iteration())?;

        // Observation is intentionally unconditional. Writer owns every stream's
        // cadence and returns immediately when no sample is due.
        writer.observe_state(model.state())?;
    }

    // Completion offers the endpoint once more, ensuring a final sample is always
    // recorded even when the requested step count misses a configured interval.
    Ok(writer.complete_recording_with_final_state(model.state())?)
}

#[allow(clippy::too_many_arguments)]
fn build_writer(
    schema: &SystemStateSchema,
    directory: &std::path::Path,
    task: &TaskConfig,
    trajectory_sampling_interval: SamplingInterval,
    radius_sampling_interval: SamplingInterval,
    checkpoint_sampling_interval: SamplingInterval,
    maximum_chunk_bytes: NonZeroU64,
    writer_queue_bytes: NonZeroU64,
) -> AppResult<SystemStateWriter> {
    // Iteration and physical time belong to the scientific record. Operational
    // UTC timestamps and active duration are added by the writer itself.
    let time_axis = TimeAxisMetadata::new("iteration")
        .with_iteration_unit("iteration")
        .with_physical_axis("physical_time", "dimensionless_model_time");

    let writer = SystemStateWriter::builder(directory, schema)
        .with_time_axis_metadata(time_axis)
        // Persisting resolved parameters makes each task output independently
        // interpretable without duplicating them in every state record.
        .with_task_parameters(task.parameters())
        // Queue bytes bound memory and apply backpressure; chunk bytes govern
        // file rollover without ever splitting one encoded state record.
        .add_state_stream(StateStreamConfig::new(
            TRAJECTORY_STREAM,
            [POINT_FIELD],
            trajectory_sampling_interval,
            None,
        ))
        .add_state_stream(StateStreamConfig::new(
            RADIUS_STREAM,
            [RADIUS_FIELD],
            radius_sampling_interval,
            None,
        ))
        .add_state_stream(StateStreamConfig::new(
            CHECKPOINT_STREAM,
            [POINT_FIELD, RADIUS_FIELD],
            checkpoint_sampling_interval,
            None,
        ))
        .with_shared_stream_limits(maximum_chunk_bytes, writer_queue_bytes)
        .create_new_recording()?;

    Ok(writer)
}
