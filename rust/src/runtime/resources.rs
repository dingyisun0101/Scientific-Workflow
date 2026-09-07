//! Unified ownership of task admission and task-private compute pools.

mod admission;
mod compute;

use serde_json::Value;

use crate::config::ComputeMode;
use crate::task::TaskResult;

use admission::{ResourceBudget, ResourceLease};
use compute::{ComputeCoordinator, ComputeError, ComputeLease};

/// One process-wide resource authority shared by every scheduler.
#[derive(Clone)]
pub(super) struct ResourceCoordinator {
    total_threads: usize,
    admission: ResourceBudget,
    compute: ComputeCoordinator,
}

/// The complete resource request made before a task becomes working.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TaskResourceRequirement {
    AutoInProcess,
    IsolatedInProcess { threads: usize },
    External { threads: usize },
}

/// One lifetime lease coupling task admission with any task-private compute pool.
pub(super) struct TaskResourceLease {
    coordinator: ResourceCoordinator,
    requirement: TaskResourceRequirement,
    compute: Option<ComputeLease>,
    // Declared after `compute` so admission is released only after the pool
    // unregisters during field destruction.
    _admission: ResourceLease,
}

impl ResourceCoordinator {
    pub(super) fn new(total_threads: usize, mode: ComputeMode) -> Self {
        Self {
            total_threads,
            admission: ResourceBudget::new(total_threads),
            compute: ComputeCoordinator::new(total_threads, mode),
        }
    }

    pub(super) fn try_acquire(
        &self,
        requirement: TaskResourceRequirement,
    ) -> Option<TaskResourceLease> {
        let admission_requirement = match requirement {
            TaskResourceRequirement::AutoInProcess => admission::ResourceRequirement::AutoInProcess,
            TaskResourceRequirement::IsolatedInProcess { threads } => {
                admission::ResourceRequirement::IsolatedInProcess { threads }
            }
            TaskResourceRequirement::External { threads } => {
                admission::ResourceRequirement::External { threads }
            }
        };
        self.admission
            .try_acquire(admission_requirement)
            .map(|admission| TaskResourceLease {
                coordinator: self.clone(),
                requirement,
                compute: None,
                _admission: admission,
            })
    }
}

impl TaskResourceLease {
    pub(super) fn activate_compute(
        &mut self,
        replicate: u64,
        output_ordinal: u64,
    ) -> Result<(), ComputeError> {
        let fixed_threads = match self.requirement {
            TaskResourceRequirement::AutoInProcess => None,
            TaskResourceRequirement::IsolatedInProcess { threads } => Some(threads),
            TaskResourceRequirement::External { .. } => return Ok(()),
        };
        self.compute = Some(self.coordinator.compute.register(
            replicate,
            output_ordinal,
            fixed_threads,
        )?);
        Ok(())
    }

    pub(super) const fn threads(&self) -> usize {
        match self.requirement {
            TaskResourceRequirement::AutoInProcess => self.coordinator.total_threads,
            TaskResourceRequirement::IsolatedInProcess { threads }
            | TaskResourceRequirement::External { threads } => threads,
        }
    }

    pub(super) fn run(&self, operation: &mut (dyn FnMut() -> TaskResult + Send)) -> TaskResult {
        self.compute
            .as_ref()
            .expect("execution-unit task activated its compute allocation")
            .run(operation)
    }

    pub(super) fn compute_provenance(&self) -> Option<Value> {
        self.compute.as_ref().map(ComputeLease::provenance)
    }
}
