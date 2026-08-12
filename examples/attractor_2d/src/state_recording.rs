//! Configures sample streams and records one evolving model.
//!
//! Storage borrows the model's state only while encoding a due sample. The
//! [`HopfModel`] remains the sole state owner before, during, and after this
//! function.

use std::path::Path;

use scientific_workflow::prelude::*;

use crate::AppResult;
use crate::hopf_model::{HopfModel, POINT_FIELD, RADIUS_FIELD};
use crate::project_setup::TaskSettings;

/// Partial stream containing two-dimensional phase points.
pub(crate) const TRAJECTORY_STREAM: &str = "trajectory";

/// Partial stream containing scalar radial diagnostics.
pub(crate) const RADIUS_STREAM: &str = "radius";

/// Complete stream containing restart-capable state snapshots.
pub(crate) const CHECKPOINT_STREAM: &str = "checkpoint";

/// Evolves one model while recording all due samples, then seals the output.
pub(crate) fn record_model(
    schema: &SystemStateSchema,
    execution: &ExecutionScope,
    task: &TaskConfig,
    settings: &TaskSettings,
    model: &mut HopfModel,
    progress: &TaskProgress,
) -> AppResult<CompletedRecording> {
    // The scope derives a stable task path but does not create it. Exclusive
    // directory creation belongs to the writer, preventing accidental reuse.
    let directory = execution.task_recording_directory(task.task_ordinal());
    let mut writer = build_writer(schema, &directory, settings, task)?;

    // The initial condition is a legitimate sample at iteration zero.
    writer.observe_state(model.state())?;
    for _ in 0..settings.step_count {
        model.step()?;

        // Progress observes the model's authoritative absolute iteration. It
        // never owns or independently increments scientific time.
        progress.set_iteration(model.state().simulation_time().iteration())?;

        // Observation is intentionally unconditional. The writer owns every
        // stream's cadence and immediately returns when no sample is due.
        writer.observe_state(model.state())?;
    }

    // Completion offers the endpoint once more, ensuring a final record even
    // when the requested step count misses a configured sampling interval.
    // It then drains the bounded queue and atomically seals the recording.
    Ok(writer.complete_recording_with_final_state(model.state())?)
}

/// Creates the trajectory, diagnostic, and checkpoint streams.
fn build_writer(
    schema: &SystemStateSchema,
    directory: &Path,
    settings: &TaskSettings,
    task: &TaskConfig,
) -> Result<SystemStateWriter, StorageError> {
    // Iteration and physical time belong to the scientific record. Operational
    // UTC timestamps and active duration are added by the writer itself.
    let time_axis = TimeAxisMetadata::new("iteration")
        .with_iteration_unit("iteration")
        .with_physical_axis("physical_time", "dimensionless_model_time");

    SystemStateWriter::builder(directory, schema)
        .with_time_axis_metadata(time_axis)
        // Persisting resolved parameters makes each task output independently
        // interpretable without duplicating them in every state record.
        .with_task_parameters(task.parameters())
        // Queue bytes bound memory and apply backpressure; chunk bytes govern
        // file rollover without ever splitting one encoded state record.
        .with_shared_stream_limits(settings.maximum_chunk_bytes, settings.writer_queue_bytes)
        .add_state_stream(StateStreamConfig::new(
            TRAJECTORY_STREAM,
            [POINT_FIELD],
            settings.trajectory_sampling_interval,
            None,
        ))
        .add_state_stream(StateStreamConfig::new(
            RADIUS_STREAM,
            [RADIUS_FIELD],
            settings.radius_sampling_interval,
            None,
        ))
        .add_state_stream(StateStreamConfig::new(
            CHECKPOINT_STREAM,
            [POINT_FIELD, RADIUS_FIELD],
            settings.checkpoint_sampling_interval,
            None,
        ))
        .create_new_recording()
}
