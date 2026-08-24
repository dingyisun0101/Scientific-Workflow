//! Explicit identity and durable output location for one producer run.

use std::path::{Path, PathBuf};

use scientific_workflow::prelude::basics::{ExecutionScope, ResolvedConfiguration};

use crate::AppResult;

/// One simulation configuration paired with its producer-owned recording path.
///
/// Validation receives this descriptor directly. It never attempts to infer a
/// producer path from a validation configuration's unrelated flat ordinal.
#[derive(Clone)]
pub(crate) struct AttractorRun {
    configuration: ResolvedConfiguration,
    task_id: Box<str>,
    recording_directory: PathBuf,
    mu: f64,
    angular_frequency: f64,
}

impl AttractorRun {
    /// Decodes display fields and reserves a stable semantic path for a producer.
    pub(crate) fn new(
        execution: &ExecutionScope,
        configuration: ResolvedConfiguration,
    ) -> AppResult<Self> {
        let (mu, angular_frequency): (f64, f64) =
            configuration.decode_values(("/mu", "/angular_frequency"))?;
        let task_id = format!(
            "attractor-g{:06}-s{:06}-p{:06}",
            configuration.global_ordinal(),
            configuration.group_ordinal(),
            configuration.phase_ordinal(),
        );
        let recording_directory = execution.named_task_recording_directory(&task_id)?;
        Ok(Self {
            configuration,
            task_id: task_id.into_boxed_str(),
            recording_directory,
            mu,
            angular_frequency,
        })
    }

    /// Borrows the exact producer configuration persisted with its recording.
    pub(crate) fn configuration(&self) -> &ResolvedConfiguration {
        &self.configuration
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
        self.mu
    }

    /// Returns the model angular frequency used in task labels.
    pub(crate) fn angular_frequency(&self) -> f64 {
        self.angular_frequency
    }

    /// Reports whether validation settings belong to this producer's shared scopes.
    pub(crate) fn matches_validation(&self, validation: &ResolvedConfiguration) -> bool {
        self.configuration.phase_group() == validation.phase_group()
            && self.configuration.global_ordinal() == validation.global_ordinal()
            && self.configuration.group_ordinal() == validation.group_ordinal()
    }
}
