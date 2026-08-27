//! Read-only successful runtime summaries.

use std::path::{Path, PathBuf};

/// Successful completion facts for one crate-level run.
#[derive(Clone, Debug)]
pub struct RunSummary {
    pub(crate) output_directory: PathBuf,
    pub(crate) replicates: Box<[ReplicateRunSummary]>,
}

impl RunSummary {
    /// Returns the uniquely inferred execution output directory.
    pub fn output_directory(&self) -> &Path {
        &self.output_directory
    }

    /// Returns successful replicate summaries in ascending index order.
    pub fn replicates(&self) -> &[ReplicateRunSummary] {
        &self.replicates
    }
}

/// Successful completion facts for one replicate.
#[derive(Clone, Debug)]
pub struct ReplicateRunSummary {
    pub(crate) index: u64,
    pub(crate) output_directory: PathBuf,
    pub(crate) phases: Box<[PhaseRunSummary]>,
}

impl ReplicateRunSummary {
    /// Returns the zero-based replicate index.
    pub const fn index(&self) -> u64 {
        self.index
    }

    /// Returns the isolated replicate output directory.
    pub fn output_directory(&self) -> &Path {
        &self.output_directory
    }

    /// Returns phase summaries in dependency execution order.
    pub fn phases(&self) -> &[PhaseRunSummary] {
        &self.phases
    }
}

/// Successful completion facts for one phase.
#[derive(Clone, Debug)]
pub struct PhaseRunSummary {
    pub(crate) name: Box<str>,
    pub(crate) tasks: Box<[TaskRunSummary]>,
}

impl PhaseRunSummary {
    /// Returns the stable phase key.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns task summaries in deterministic plan order.
    pub fn tasks(&self) -> &[TaskRunSummary] {
        &self.tasks
    }
}

/// Successful completion facts for one bound model invocation.
#[derive(Clone, Debug)]
pub struct TaskRunSummary {
    pub(crate) identity: Box<str>,
    pub(crate) model: Box<str>,
    pub(crate) final_iteration: u64,
    pub(crate) recording_directory: PathBuf,
}

impl TaskRunSummary {
    /// Returns the study-inferred task identity.
    pub fn identity(&self) -> &str {
        &self.identity
    }

    /// Returns the registered model key.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Returns the final scientific iteration recorded by the model.
    pub const fn final_iteration(&self) -> u64 {
        self.final_iteration
    }

    /// Returns the completed task recording directory.
    pub fn recording_directory(&self) -> &Path {
        &self.recording_directory
    }
}
