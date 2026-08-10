//! Minimal typed readback validation for a completed task recording.
//!
//! This module is intentionally not an analysis layer. It demonstrates only
//! the essential persistence round trip: register payload decoders, recover
//! each stream's final record, and compare it with the live state that was
//! handed to the writer at completion.

use scientific_workflow::prelude::*;

use crate::AppResult;
use crate::hopf_model::{POINT_FIELD, RADIUS_FIELD};
use crate::state_recording::{CHECKPOINT_STREAM, RADIUS_STREAM, TRAJECTORY_STREAM};

/// Verifies exact final-time and payload equality for every recorded stream.
pub(crate) fn validate_recording(
    live_state: &SystemState,
    recording: &CompletedRecording,
) -> AppResult<()> {
    // State schemas contain keys but deliberately do not prescribe Rust
    // payload types. Readback therefore binds one decoder to each selected
    // field. Serde JSON preserves finite f64 values exactly on round trip.
    let decoders = JsonPayloadDecoderRegistry::new()
        .with_json_field::<Vec<f64>>(POINT_FIELD)?
        .with_json_field::<f64>(RADIUS_FIELD)?;
    let reader =
        StoredStateSeriesReader::open_completed_recording(recording.directory(), decoders)?;

    // The validator needs only the newest sample, so this API verifies and
    // opens the final chunk without reconstructing an entire analysis series.
    let final_trajectory = reader.read_latest_state_from_stream(TRAJECTORY_STREAM)?;
    let final_radius = reader.read_latest_state_from_stream(RADIUS_STREAM)?;
    let final_checkpoint = reader.read_latest_state_from_stream(CHECKPOINT_STREAM)?;

    // Successful completion offers the final live state to all streams. Their
    // latest timestamps must therefore match even when the requested step
    // count is not divisible by a stream's sampling interval.
    assert_eq!(
        final_trajectory.simulation_time(),
        live_state.simulation_time()
    );
    assert_eq!(final_radius.simulation_time(), live_state.simulation_time());
    assert_eq!(
        final_checkpoint.simulation_time(),
        live_state.simulation_time()
    );

    assert_eq!(
        final_trajectory.payload::<Vec<f64>>(POINT_FIELD)?,
        live_state.payload::<Vec<f64>>(POINT_FIELD)?
    );
    assert_eq!(
        final_radius.payload::<f64>(RADIUS_FIELD)?,
        live_state.payload::<f64>(RADIUS_FIELD)?
    );
    assert_eq!(
        final_checkpoint.payload::<Vec<f64>>(POINT_FIELD)?,
        live_state.payload::<Vec<f64>>(POINT_FIELD)?
    );
    assert_eq!(
        final_checkpoint.payload::<f64>(RADIUS_FIELD)?,
        live_state.payload::<f64>(RADIUS_FIELD)?
    );
    Ok(())
}
