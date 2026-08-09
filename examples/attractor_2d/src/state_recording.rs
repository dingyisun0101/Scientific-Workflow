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

/// Numbers of records submitted to the three streams.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SampleCounts {
    pub(crate) trajectory: u64,
    pub(crate) radius: u64,
    pub(crate) checkpoint: u64,
}

/// Storage facts returned after the writer reaches completed status.
#[derive(Debug)]
pub(crate) struct CompletedRecording {
    pub(crate) samples: SampleCounts,
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
    let writer = build_writer(schema, &directory, settings, parameters)?;
    let mut samples = SampleCounts::default();

    sample_due(model.state(), &writer, settings, &mut samples)?;
    for _ in 0..settings.total_steps {
        model.advance(settings)?;
        sample_due(model.state(), &writer, settings, &mut samples)?;
    }

    writer.complete_recording()?;
    Ok(CompletedRecording { samples, directory })
}

/// Creates the trajectory, diagnostic, and checkpoint streams.
fn build_writer(
    schema: &SystemStateSchema,
    directory: &Path,
    settings: &TaskSettings,
    parameters: &TaskParameters,
) -> Result<SystemStateWriter, StorageError> {
    let user_metadata = parameters
        .iter()
        .map(|(key, value)| (key.to_owned(), value.clone()))
        .collect();
    let time_axis = TimeAxisMetadata::new("step")
        .with_step_unit("iteration")
        .with_physical_time_name("time")
        .with_physical_time_unit("model_time");

    let trajectory = StateStreamConfig::new(
        TRAJECTORY_STREAM,
        [POINT_FIELD],
        settings.maximum_chunk_bytes,
        settings.writer_queue_bytes,
    )
    .with_cadence_description(format!("every {} steps", settings.trajectory_every));
    let radius = StateStreamConfig::new(
        RADIUS_STREAM,
        [RADIUS_FIELD],
        settings.maximum_chunk_bytes,
        settings.writer_queue_bytes,
    )
    .with_cadence_description(format!("every {} steps", settings.radius_every));
    let checkpoint = StateStreamConfig::new(
        CHECKPOINT_STREAM,
        [POINT_FIELD, RADIUS_FIELD],
        settings.maximum_chunk_bytes,
        settings.writer_queue_bytes,
    )
    .with_cadence_description(format!("every {} steps", settings.checkpoint_every));

    SystemStateWriter::builder(directory, schema)
        .with_time_axis_metadata(time_axis)
        .with_user_metadata(user_metadata)
        .add_state_stream(trajectory)
        .add_state_stream(radius)
        .add_state_stream(checkpoint)
        .create_new_recording()
}

/// Submits the current state to every stream due at its step.
fn sample_due(
    state: &SystemState,
    writer: &SystemStateWriter,
    settings: &TaskSettings,
    samples: &mut SampleCounts,
) -> Result<(), StorageError> {
    let step = state.simulation_time().step();
    if due(step, settings.trajectory_every, settings.total_steps) {
        writer.record_state_to_stream(TRAJECTORY_STREAM, state)?;
        samples.trajectory += 1;
    }
    if due(step, settings.radius_every, settings.total_steps) {
        writer.record_state_to_stream(RADIUS_STREAM, state)?;
        samples.radius += 1;
    }
    if due(step, settings.checkpoint_every, settings.total_steps) {
        writer.record_state_to_stream(CHECKPOINT_STREAM, state)?;
        samples.checkpoint += 1;
    }
    Ok(())
}

/// Includes the initial, periodic, and final state for one cadence.
fn due(step: u64, cadence: u64, final_step: u64) -> bool {
    step == 0 || step == final_step || step % cadence == 0
}
