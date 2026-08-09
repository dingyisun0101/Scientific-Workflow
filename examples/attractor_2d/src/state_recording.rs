//! Configures sample streams and records one evolving model.
//!
//! Storage borrows the model's state only while encoding a due sample. The
//! [`HopfModel`] remains the sole state owner before, during, and after this
//! function.

use std::path::{Path, PathBuf};

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

/// Storage facts returned after the writer reaches completed status.
#[derive(Debug)]
pub(crate) struct CompletedRecording {
    pub(crate) directory: PathBuf,
}

/// Evolves one model while recording all due samples, then seals the output.
pub(crate) fn record_model(
    schema: &SystemStateSchema,
    execution_root: &Path,
    parameters: &TaskParameters,
    settings: &TaskSettings,
    model: &mut HopfModel,
) -> AppResult<CompletedRecording> {
    let directory = execution_root.join(format!("task_{:04}", settings.task_index));
    let mut writer = build_writer(schema, &directory, settings, parameters)?;

    writer.observe_state(model.state())?;
    for _ in 0..settings.total_steps {
        model.advance()?;
        writer.observe_state(model.state())?;
    }

    writer.complete_recording_with_final_state(model.state())?;
    Ok(CompletedRecording { directory })
}

/// Creates the trajectory, diagnostic, and checkpoint streams.
fn build_writer(
    schema: &SystemStateSchema,
    directory: &Path,
    settings: &TaskSettings,
    parameters: &TaskParameters,
) -> Result<SystemStateWriter, StorageError> {
    let time_axis = TimeAxisMetadata::new("step")
        .with_step_unit("iteration")
        .with_physical_axis("time", "model_time");

    SystemStateWriter::builder(directory, schema)
        .with_time_axis_metadata(time_axis)
        .with_task_parameters(parameters)
        .with_shared_stream_limits(settings.maximum_chunk_bytes, settings.writer_queue_bytes)
        .add_periodic_state_stream(TRAJECTORY_STREAM, [POINT_FIELD], settings.trajectory_every)
        .add_periodic_state_stream(RADIUS_STREAM, [RADIUS_FIELD], settings.radius_every)
        .add_periodic_state_stream(
            CHECKPOINT_STREAM,
            [POINT_FIELD, RADIUS_FIELD],
            settings.checkpoint_every,
        )
        .create_new_recording()
}
