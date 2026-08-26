use std::num::NonZeroU64;

use crate::AppResult;
use scientific_workflow::prelude::basic::*;
use scientific_workflow::prelude::study::TaskContext;

use crate::hopf_model::{POINT_FIELD, RADIUS_FIELD};

/// Stream containing two-dimensional phase points.
pub(crate) const TRAJECTORY_STREAM: &str = "trajectory";

/// Stream containing scalar radial diagnostics.
pub(crate) const RADIUS_STREAM: &str = "radius";

/// Stream containing restart-capable state snapshots.
pub(crate) const CHECKPOINT_STREAM: &str = "checkpoint";

pub(crate) fn record_task(
    directory: &std::path::Path,
    configuration: &ResolvedConfiguration,
    model: &mut crate::hopf_model::HopfModel,
    context: &TaskContext,
) -> AppResult<()> {
    // Operational policy is decoded from the same resolved configuration as model
    // parameters. There is intentionally no parallel recording-settings type.
    let step_count: u64 = configuration.decode_value("/step_count")?;
    let mut writer = build_writer(model.state(), directory, configuration)?;

    // TaskContext updates drive the terminal display only. SystemState is the
    // scientific record, and SystemStateWriter alone decides when to sample it.
    context.set_iteration(0)?;
    context.set_target_iteration(step_count)?;

    // Observe the initial condition before stepping, then the completed state
    // after every step. Each stream independently filters these observations by
    // its iteration cadence.
    writer.observe_state(model.state())?;
    for _ in 0..step_count {
        model.step()?;
        context.set_iteration(model.state().time().iteration())?;
        writer.observe_state(model.state())?;
    }
    // Completion flushes queued records and atomically publishes terminal
    // metadata. Downstream readers open only completed recordings.
    writer.complete_recording_with_final_state(model.state())?;
    Ok(())
}

fn build_writer(
    state: &SystemState,
    directory: &std::path::Path,
    configuration: &ResolvedConfiguration,
) -> AppResult<SystemStateWriter> {
    // Grouped decoding keeps related policy together while retaining precise
    // concrete Rust types, including non-zero limits validated at decode time.
    let (trajectory_interval, radius_interval, checkpoint_interval, chunk_bytes, queue_bytes): (
        u64,
        u64,
        u64,
        NonZeroU64,
        NonZeroU64,
    ) = configuration.decode_values((
        "/trajectory_sampling_interval",
        "/radius_sampling_interval",
        "/checkpoint_sampling_interval",
        "/maximum_chunk_bytes",
        "/storage_queue_bytes",
    ))?;
    // Iteration and physical time belong to the scientific record. Operational
    // UTC timestamps and active duration are added by the writer itself.
    let writer_definition = Writer::streams([
        Stream::fields(TRAJECTORY_STREAM, [POINT_FIELD])?
            .every_iterations(trajectory_interval)?,
        Stream::fields(RADIUS_STREAM, [RADIUS_FIELD])?
            .every_iterations(radius_interval)?,
        Stream::fields(CHECKPOINT_STREAM, [POINT_FIELD, RADIUS_FIELD])?
            .every_iterations(checkpoint_interval)?,
    ])?
    .with_iteration_unit("iteration")?
    .with_physical_time_unit("dimensionless_model_time")?;

    // Writer construction is also where the task takes ownership of its I/O
    // lifecycle. Merely constructing a Phase or ExecutionScope writes no data.
    let writer = SystemStateWriter::builder(directory.to_path_buf(), state)
        .with_writer(writer_definition)
        // Persisting resolved parameters makes each task output independently
        // interpretable without duplicating them in every state record.
        .with_configuration(configuration)
        // Queue bytes bound memory and apply backpressure; chunk bytes govern
        // file rollover without ever splitting one encoded state record.
        .with_shared_stream_storage(StateStreamStorage::chunked(chunk_bytes, queue_bytes))
        .create_new_recording()?;

    Ok(writer)
}
