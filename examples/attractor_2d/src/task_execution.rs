use std::path::Path;

use crate::{AppResult, hopf_model::HopfModel, recording};
use scientific_workflow::prelude::basic::*;
use scientific_workflow::prelude::study::*;

pub(crate) fn run_task(
    schema: &SystemStateSchema,
    recording_directory: &Path,
    task: &ResolvedConfiguration,
    context: &TaskContext,
) -> AppResult<()> {
    // The application captured this resolved fixed × sweep combination in the task.
    // Tuple decoding binds heterogeneous values directly where they are used,
    // replacing an application-specific configuration struct without falling
    // back to repeated untyped JSON access throughout the model.
    let (initial_point, mu, omega, dt): ([f64; 2], f64, f64, f64) = task.decode_values((
        "/initial_point",
        "/mu",
        "/angular_frequency",
        "/physical_time_increment_per_step",
    ))?;

    // The model owns scientific state; the recording function owns evolution
    // and I/O. The study owns neither and observes only TaskContext progress.
    let mut model = HopfModel::new(schema, initial_point, mu, omega, dt)?;

    recording::record_task(recording_directory, task, &mut model, context)
}
