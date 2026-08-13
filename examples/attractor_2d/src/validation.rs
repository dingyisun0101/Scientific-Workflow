use scientific_workflow::prelude::basics::*;

use std::path::Path;

use crate::{recording::CHECKPOINT_STREAM, AppResult};

/// Reconstructs the final complete checkpoint for dependent validation work.
pub(crate) fn read_final_checkpoint(
    recording_directory: &Path,
    point_field: &str,
    radius_field: &str,
) -> AppResult<SystemState> {
    let decoders = JsonPayloadDecoderRegistry::new()
        .with_json_field::<Vec<f64>>(point_field)?
        .with_json_field::<f64>(radius_field)?;
    let reader =
        StoredStateSeriesReader::open_completed_recording(recording_directory, decoders)?;
    Ok(reader.read_latest_state_from_stream(CHECKPOINT_STREAM)?)
}
