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

/// Successful result of the workload completed by a task.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub enum TaskRunKind {
    /// A registered scientific execution-unit invocation and its members.
    ExecutionUnit {
        /// Registration key used to select the execution unit.
        execution_unit: Box<str>,
        /// Independently recorded member results in stable unit order.
        members: Box<[MemberRunSummary]>,
    },
    /// An external executable or Python program invocation.
    Program {
        /// Resolved launcher executable.
        executable: PathBuf,
        /// Canonical script path when this was a nested Python task.
        python_script: Option<PathBuf>,
    },
    /// Workflow's reserved conversion of prerequisite recordings to NumPy arrays.
    Npy {
        /// Resolved Python interpreter used to launch the standard converter.
        launcher: PathBuf,
        /// Standard execution-level directory containing manifests and arrays.
        processed_directory: PathBuf,
    },
}

/// Successful completion facts for one member inside an execution unit.
#[derive(Clone, Debug)]
pub struct MemberRunSummary {
    pub(crate) identity: Box<str>,
    pub(crate) final_iteration: u64,
    pub(crate) output_directory: PathBuf,
}

impl MemberRunSummary {
    /// Returns the stable member identity supplied by the execution unit.
    pub fn identity(&self) -> &str {
        &self.identity
    }

    /// Returns this member's final scientific iteration.
    pub const fn final_iteration(&self) -> u64 {
        self.final_iteration
    }

    /// Returns this member's completed recording directory.
    pub fn output_directory(&self) -> &Path {
        &self.output_directory
    }
}

/// Successful completion facts for one generic task invocation.
#[derive(Clone, Debug)]
pub struct TaskRunSummary {
    pub(crate) identity: Box<str>,
    pub(crate) kind: TaskRunKind,
    pub(crate) output_directory: PathBuf,
    pub(crate) configuration: usize,
}

impl TaskRunSummary {
    /// Returns the study-inferred task identity.
    pub fn identity(&self) -> &str {
        &self.identity
    }

    /// Returns the completed workload and its variant-specific result.
    pub const fn kind(&self) -> &TaskRunKind {
        &self.kind
    }

    /// Returns the task output root or program workspace directory.
    ///
    /// This is also the recording directory for a one-member execution unit.
    /// Match [`Self::kind`] for every per-member recording path.
    pub fn output_directory(&self) -> &Path {
        &self.output_directory
    }

    pub(crate) fn final_iteration(&self) -> Option<u64> {
        match &self.kind {
            TaskRunKind::ExecutionUnit { members, .. } => {
                members.iter().map(MemberRunSummary::final_iteration).max()
            }
            TaskRunKind::Program { .. } | TaskRunKind::Npy { .. } => None,
        }
    }

    pub(crate) const fn configuration(&self) -> usize {
        self.configuration
    }
}
