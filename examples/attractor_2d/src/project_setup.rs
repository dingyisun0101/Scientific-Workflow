//! Loads the example project and prepares its parameter-sweep tasks.
//!
//! The checked-in JSON files are part of this demonstration, so this module
//! shows the normal typed configuration path without reproducing a production
//! application's domain-validation framework.

use std::num::NonZeroU64;

use scientific_workflow::prelude::*;

use crate::AppResult;
use crate::hopf_model::HopfModel;

/// Immutable model, sampling, and storage settings for one swept task.
#[derive(Debug)]
pub(crate) struct TaskSettings {
    pub(crate) step_count: u64,
    pub(crate) trajectory_sampling_interval: SamplingInterval,
    pub(crate) radius_sampling_interval: SamplingInterval,
    pub(crate) checkpoint_sampling_interval: SamplingInterval,
    pub(crate) maximum_chunk_bytes: NonZeroU64,
    pub(crate) writer_queue_bytes: NonZeroU64,
}

/// Creates one model and its execution settings from a resolved task dictionary.
pub(crate) fn prepare_task(
    schema: &SystemStateSchema,
    task: &TaskConfig,
) -> AppResult<(HopfModel, TaskSettings)> {
    // TaskConfig is a cheap owned handle combining fixed values, the selected
    // Cartesian sweep point, and shared paths without allocating a merged map.
    let settings = TaskSettings {
        step_count: task.decode_value("step_count")?,
        trajectory_sampling_interval: task.decode_value("trajectory_sampling_interval")?,
        radius_sampling_interval: task.decode_value("radius_sampling_interval")?,
        checkpoint_sampling_interval: task.decode_value("checkpoint_sampling_interval")?,
        maximum_chunk_bytes: task.decode_value("maximum_chunk_bytes")?,
        writer_queue_bytes: task.decode_value("writer_queue_bytes")?,
    };

    // Large scientific payloads should be constructed here and moved into the
    // state once. This example's Vec is tiny, but follows the same ownership
    // pattern expected for tensors and other expensive payloads.
    let model = HopfModel::new(
        schema,
        task.decode_value("initial_point")?,
        task.decode_value("mu")?,
        task.decode_value("angular_frequency")?,
        task.decode_value("physical_time_increment_per_step")?,
    )?;
    Ok((model, settings))
}
