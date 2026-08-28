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

/// Kind of workload completed by a task.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskRunKind {
    /// A registered scientific execution-unit invocation.
    Model,
    /// An external executable program invocation.
    Program,
}

/// Successful completion facts for one model inside an execution unit.
#[derive(Clone, Debug)]
pub struct ModelRunSummary {
    pub(crate) identity: Box<str>,
    pub(crate) final_iteration: u64,
    pub(crate) output_directory: PathBuf,
}

impl ModelRunSummary {
    /// Returns the stable model identity supplied by the execution unit.
    pub fn identity(&self) -> &str {
        &self.identity
    }

    /// Returns this model's final scientific iteration.
    pub const fn final_iteration(&self) -> u64 {
        self.final_iteration
    }

    /// Returns this model's completed recording directory.
    pub fn output_directory(&self) -> &Path {
        &self.output_directory
    }
}

/// Successful completion facts for one generic task invocation.
#[derive(Clone, Debug)]
pub struct TaskRunSummary {
    pub(crate) identity: Box<str>,
    pub(crate) kind: TaskRunKind,
    pub(crate) model: Option<Box<str>>,
    pub(crate) program: Option<PathBuf>,
    pub(crate) program_kind: Option<Box<str>>,
    pub(crate) python_script: Option<PathBuf>,
    pub(crate) final_iteration: Option<u64>,
    pub(crate) models: Box<[ModelRunSummary]>,
    pub(crate) output_directory: PathBuf,
}

impl TaskRunSummary {
    /// Returns the study-inferred task identity.
    pub fn identity(&self) -> &str {
        &self.identity
    }

    /// Returns whether this task executed a model or a program.
    pub const fn kind(&self) -> TaskRunKind {
        self.kind
    }

    /// Returns the registered model key for a model task.
    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    /// Returns the resolved launcher executable for a program task.
    ///
    /// For a nested Python declaration this is its interpreter or environment
    /// manager; durable `program.json` retains the canonical script path.
    pub fn program(&self) -> Option<&Path> {
        self.program.as_deref()
    }

    /// Returns the resolved program workload kind (`program` or `python`).
    pub fn program_kind(&self) -> Option<&str> {
        self.program_kind.as_deref()
    }

    /// Returns the canonical script path for a nested Python task.
    pub fn python_script(&self) -> Option<&Path> {
        self.python_script.as_deref()
    }

    /// Returns the maximum final model iteration for a scientific task.
    pub const fn final_iteration(&self) -> Option<u64> {
        self.final_iteration
    }

    /// Returns independently recorded model results in stable unit order.
    ///
    /// A standalone model produces one entry, an ensemble produces one entry
    /// per model, and a program task produces none.
    pub fn models(&self) -> &[ModelRunSummary] {
        &self.models
    }

    /// Returns the task output root or program workspace directory.
    ///
    /// This is also the recording directory for a one-model execution unit.
    /// Use [`Self::models`] for every per-model recording path.
    pub fn output_directory(&self) -> &Path {
        &self.output_directory
    }
}
