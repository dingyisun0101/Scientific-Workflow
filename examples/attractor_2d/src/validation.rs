use scientific_workflow::prelude::*;

use crate::{recording::CHECKPOINT_STREAM, AppResult};

/// Validates exact final-time and payload equality for the final checkpoint.
pub(crate) fn validate_recording(
    live_state: &SystemState,
    recording: &CompletedRecording,
    point_field: &str,
    radius_field: &str,
) -> AppResult<()> {
    // State schemas contain keys but deliberately do not prescribe Rust payload
    // types. Readback therefore binds one decoder to each selected field.
    let decoders = JsonPayloadDecoderRegistry::new()
        .with_json_field::<Vec<f64>>(point_field)?
        .with_json_field::<f64>(radius_field)?;
    let reader = StoredStateSeriesReader::open_completed_recording(recording.directory(), decoders)?;

    // The validator needs only the newest sample, so this API verifies and
    // opens the final chunk without reconstructing an entire analysis series.
    let final_checkpoint = reader.read_latest_state_from_stream(CHECKPOINT_STREAM)?;

    if final_checkpoint.simulation_time() != live_state.simulation_time() {
        return Err(format!(
            "checkpoint simulation time mismatch: expected {:?}, recovered {:?}",
            live_state.simulation_time(),
            final_checkpoint.simulation_time()
        )
        .into());
    }
    if final_checkpoint.payload::<Vec<f64>>(point_field)? != live_state.payload::<Vec<f64>>(point_field)? {
        return Err(
            format!("checkpoint `point` payload mismatch for {:?}", recording.directory().display()).into(),
        );
    }
    if final_checkpoint.payload::<f64>(radius_field)? != live_state.payload::<f64>(radius_field)? {
        return Err(
            format!("checkpoint `radius` payload mismatch for {:?}", recording.directory().display()).into(),
        );
    }
    Ok(())
}
