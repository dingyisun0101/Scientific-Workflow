use crate::AppResult;
use scientific_workflow::prelude::basic::*;
use scientific_workflow::prelude::study::TaskContext;

use crate::hopf_model::{POINT_FIELD, RADIUS_FIELD};
use crate::{attractor_run::AttractorConstants, attractor_run::AttractorRun};

/// Stream containing two-dimensional phase points.
pub(crate) const TRAJECTORY_STREAM: &str = "trajectory";

/// Stream containing scalar radial diagnostics.
pub(crate) const RADIUS_STREAM: &str = "radius";

/// Stream containing restart-capable state snapshots.
pub(crate) const CHECKPOINT_STREAM: &str = "checkpoint";

pub(crate) fn record_task(
    directory: &std::path::Path,
    run: &AttractorRun,
    model: &mut crate::hopf_model::HopfModel,
    context: &TaskContext,
) -> AppResult<()> {
    let constants = run.constants();
    let mut writer = build_writer(model.state(), directory, run.input(), constants)?;

    // TaskContext updates drive the terminal display only. SystemState is the
    // scientific record, and SystemStateWriter alone decides when to sample it.
    context.set_iteration(0)?;
    context.set_target_iteration(constants.step_count)?;

    // Observe the initial condition before stepping, then the completed state
    // after every step. Each stream independently filters these observations by
    // its iteration cadence.
    writer.observe_state(model.state())?;
    for _ in 0..constants.step_count {
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
    input: &scientific_workflow::config::advanced::ResolvedTaskInput,
    constants: &AttractorConstants,
) -> AppResult<SystemStateWriter> {
    // Iteration and physical time belong to the scientific record. Operational
    // UTC timestamps and active duration are added by the writer itself.
    let writer_definition = Writer::streams([
        Stream::fields(TRAJECTORY_STREAM, [POINT_FIELD])?
            .every_iterations(constants.trajectory_sampling_interval)?,
        Stream::fields(RADIUS_STREAM, [RADIUS_FIELD])?
            .every_iterations(constants.radius_sampling_interval)?,
        Stream::fields(CHECKPOINT_STREAM, [POINT_FIELD, RADIUS_FIELD])?
            .every_iterations(constants.checkpoint_sampling_interval)?,
    ])?
    .with_iteration_unit("iteration")?
    .with_physical_time_unit("dimensionless_model_time")?;

    // Writer construction is also where the task takes ownership of its I/O
    // lifecycle. Merely constructing a Phase or ExecutionScope writes no data.
    let resolved = serde_json::from_slice::<serde_json::Value>(input.resolved_json())?;
    let serde_json::Value::Object(mut metadata) = resolved else {
        return Err("attractor task input must decode as an object".into());
    };
    metadata.insert("input_ordinal".to_owned(), input.ordinal().into());

    let writer = SystemStateWriter::builder(directory.to_path_buf(), state)
        .with_writer(writer_definition)
        // Persisting resolved parameters makes each task output independently
        // interpretable without duplicating them in every state record.
        .with_user_metadata(metadata)
        // Queue bytes bound memory and apply backpressure; chunk bytes govern
        // file rollover without ever splitting one encoded state record.
        .with_shared_stream_storage(StateStreamStorage::chunked(
            constants.maximum_chunk_bytes,
            constants.storage_queue_bytes,
        ))
        .create_new_recording()?;

    Ok(writer)
}
