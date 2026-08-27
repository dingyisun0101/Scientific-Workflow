//! Explicit identity and durable output location for one producer run.

use std::path::{Path, PathBuf};

use scientific_workflow::config::advanced::ResolvedTaskInput;
use scientific_workflow::prelude::basic::ExecutionScope;
use serde::Deserialize;

use crate::AppResult;

/// Complete typed scientific and recording constants for one attractor run.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AttractorConstants {
    pub(crate) initial_point: [f64; 2],
    pub(crate) physical_time_increment_per_step: f64,
    pub(crate) step_count: u64,
    pub(crate) trajectory_sampling_interval: u64,
    pub(crate) radius_sampling_interval: u64,
    pub(crate) checkpoint_sampling_interval: u64,
    pub(crate) maximum_chunk_bytes: std::num::NonZeroU64,
    pub(crate) storage_queue_bytes: std::num::NonZeroU64,
    pub(crate) mu: f64,
    pub(crate) angular_frequency: f64,
}

/// One simulation input paired with its producer-owned recording path.
///
/// Validation receives this descriptor directly. It never attempts to infer a
/// producer path from another task declaration's ordinal.
#[derive(Clone)]
pub(crate) struct AttractorRun {
    input: ResolvedTaskInput,
    constants: AttractorConstants,
    task_id: Box<str>,
    recording_directory: PathBuf,
}

impl AttractorRun {
    /// Decodes one complete input and reserves a stable path for its producer.
    pub(crate) fn new(execution: &ExecutionScope, input: ResolvedTaskInput) -> AppResult<Self> {
        let constants = input.decode::<AttractorConstants>()?;
        let task_id = format!("attractor-{:06}", input.ordinal());
        let recording_directory = execution.named_task_recording_directory(&task_id)?;
        Ok(Self {
            input,
            constants,
            task_id: task_id.into_boxed_str(),
            recording_directory,
        })
    }

    /// Borrows the resolved input persisted with this recording.
    pub(crate) fn input(&self) -> &ResolvedTaskInput {
        &self.input
    }

    /// Borrows the one typed constants value used by model and writer.
    pub(crate) fn constants(&self) -> &AttractorConstants {
        &self.constants
    }

    /// Borrows the producer's phase-local task ID.
    pub(crate) fn task_id(&self) -> &str {
        &self.task_id
    }

    /// Borrows the explicit recording destination shared with consumers.
    pub(crate) fn recording_directory(&self) -> &Path {
        &self.recording_directory
    }

    /// Returns the model bifurcation parameter used in task labels.
    pub(crate) fn mu(&self) -> f64 {
        self.constants.mu
    }

    /// Returns the model angular frequency used in task labels.
    pub(crate) fn angular_frequency(&self) -> f64 {
        self.constants.angular_frequency
    }

    /// Reports whether a validation input describes this producer.
    pub(crate) fn matches_validation(&self, validation: &AttractorConstants) -> bool {
        self.constants.mu == validation.mu
            && self.constants.angular_frequency == validation.angular_frequency
    }
}
