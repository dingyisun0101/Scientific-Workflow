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
    parameters: &TaskParameters,
    settings: &TaskSettings,
    model: &mut HopfModel,
) -> AppResult<CompletedRecording> {
    let directory = execution.task_recording_directory(settings.task_index);
    let mut writer = build_writer(schema, &directory, settings, parameters)?;

    writer.observe_state(model.state())?;
    for _ in 0..settings.step_count {
        model.step()?;
        writer.observe_state(model.state())?;
    }

    let mut terminal_metadata = serde_json::Map::new();
    terminal_metadata.insert(
        "completed_step_count".to_owned(),
        serde_json::Value::from(settings.step_count),
    );
    terminal_metadata.insert(
        "termination_reason".to_owned(),
        serde_json::Value::from("requested_steps_completed"),
    );
    Ok(
        writer.complete_recording_with_final_state_and_terminal_metadata(
            model.state(),
            terminal_metadata,
        )?,
    )
}

/// Creates the trajectory, diagnostic, and checkpoint streams.
fn build_writer(
    schema: &SystemStateSchema,
    directory: &Path,
    settings: &TaskSettings,
    parameters: &TaskParameters,
) -> Result<SystemStateWriter, StorageError> {
    let time_axis = TimeAxisMetadata::new("iteration")
        .with_iteration_unit("iteration")
        .with_physical_axis("physical_time", "dimensionless_model_time");

    SystemStateWriter::builder(directory, schema)
        .with_time_axis_metadata(time_axis)
        .with_task_parameters(parameters)
        .with_shared_stream_limits(settings.maximum_chunk_bytes, settings.writer_queue_bytes)
        .add_sampled_state_stream(
            TRAJECTORY_STREAM,
            [POINT_FIELD],
            settings.trajectory_sampling_interval,
        )
        .add_sampled_state_stream(
            RADIUS_STREAM,
            [RADIUS_FIELD],
            settings.radius_sampling_interval,
        )
        .add_sampled_state_stream(
            CHECKPOINT_STREAM,
            [POINT_FIELD, RADIUS_FIELD],
            settings.checkpoint_sampling_interval,
        )
        .create_new_recording()
}
