use std::path::Path;

use crate::{
    AppResult,
    attractor_run::{AttractorConstants, AttractorRun},
    hopf_model::HopfModel,
    recording,
};
use scientific_workflow::prelude::basic::*;
use scientific_workflow::prelude::study::*;

pub(crate) fn run_task(
    schema: &SystemStateSchema,
    recording_directory: &Path,
    run: &AttractorRun,
    context: &TaskContext,
) -> AppResult<()> {
    let constants: &AttractorConstants = run.constants();

    // The model owns scientific state; the recording function owns evolution
    // and I/O. The study owns neither and observes only TaskContext progress.
    let mut model = HopfModel::new(
        schema,
        constants.initial_point,
        constants.mu,
        constants.angular_frequency,
        constants.physical_time_increment_per_step,
    )?;

    recording::record_task(recording_directory, run, &mut model, context)
}
