use scientific_workflow::prelude::basics::*;

use crate::{
    AppResult,
    hopf_model::{POINT_FIELD, RADIUS_FIELD},
    recording::CHECKPOINT_STREAM,
};
use scientific_workflow::prelude::runtime::TaskContext;

pub(crate) fn validate_recording(
    execution: &ExecutionScope,
    context: &TaskContext,
) -> AppResult<()> {
    context.set_detail("checking final checkpoint");
    let decoders = JsonPayloadDecoderRegistry::new()
        .with_json_field::<Vec<f64>>(POINT_FIELD)?
        .with_json_field::<f64>(RADIUS_FIELD)?;
    let directory = execution.task_recording_directory(context.configuration().task_ordinal());
    let state = StoredStateSeriesReader::open_completed_recording(directory, decoders)?
        .read_latest_state_from_stream(CHECKPOINT_STREAM)?;
    let point = state.payload::<Vec<f64>>(POINT_FIELD)?;
    let radius = state.payload::<f64>(RADIUS_FIELD)?;
    let expected_iteration: u64 = context.decode_value("step_count")?;

    if point.len() != 2 || *radius != point[0].hypot(point[1]) {
        return Err("checkpoint radius is inconsistent with its point".into());
    }
    if state.simulation_time().iteration() != expected_iteration {
        return Err("checkpoint does not contain the configured final iteration".into());
    }
    context.set_detail("checkpoint verified");
    Ok(())
}
