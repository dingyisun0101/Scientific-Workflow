//! Unified ownership of task admission and task-private compute pools.

mod admission;
mod compute;

use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::config::ComputeMode;
use crate::task::TaskResult;

use admission::{ResourceBudget, ResourceLease};
use compute::{ComputeCoordinator, ComputeError, ComputeLease};

/// One process-wide resource authority shared by every scheduler.
#[derive(Clone)]
pub(super) struct ResourceCoordinator {
    working: Arc<Mutex<BTreeMap<(u64, u64), WorkingTask>>>,
    reported: Arc<Mutex<ReportedAllocations>>,
    total_threads: usize,
    admission: ResourceBudget,
    compute: ComputeCoordinator,
}

struct WorkingTask {
    identity: Box<str>,
    external_threads: Option<usize>,
}
#[derive(Default)]
struct ReportedAllocations {
    at: Option<Instant>,
    values: Vec<(u64, Box<str>, usize)>,
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
    key: Option<(u64, u64)>,
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
            working: Arc::default(),
            reported: Arc::default(),
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
                key: None,
                coordinator: self.clone(),
                requirement,
                compute: None,
                _admission: admission,
            })
    }

    pub(super) fn publish_allocations(
        &self,
        presentation: &super::presentation::RuntimePresentation,
        force: bool,
    ) -> Result<(), super::RuntimeError> {
        let mut reported = self
            .reported
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !force
            && reported
                .at
                .is_some_and(|at| at.elapsed() < Duration::from_millis(50))
        {
            return Ok(());
        }
        let working = self
            .working
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let compute = self.compute.allocations();
        let values = working
            .iter()
            .map(|(key, task)| {
                (
                    key.0,
                    task.identity.clone(),
                    task.external_threads
                        .unwrap_or_else(|| compute.get(key).copied().unwrap_or(0)),
                )
            })
            .collect::<Vec<_>>();
        drop(working);
        if reported.at.is_none() || values != reported.values {
            presentation.publish(super::RuntimeEvent::ThreadAllocations {
                allocations: &values,
                budget: self.total_threads,
            })?;
            reported.values = values;
        }
        reported.at = Some(Instant::now());
        Ok(())
    }
}

impl TaskResourceLease {
    pub(super) fn activate(
        &mut self,
        replicate: u64,
        output_ordinal: u64,
        identity: &str,
    ) -> Result<(), ComputeError> {
        self.key = Some((replicate, output_ordinal));
        self.coordinator
            .working
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                (replicate, output_ordinal),
                WorkingTask {
                    identity: identity.into(),
                    external_threads: match self.requirement {
                        TaskResourceRequirement::External { threads } => Some(threads),
                        _ => None,
                    },
                },
            );
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

    pub(super) fn publish_allocations(
        &self,
        presentation: &super::presentation::RuntimePresentation,
    ) -> Result<(), super::RuntimeError> {
        self.coordinator.publish_allocations(presentation, true)
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

impl Drop for TaskResourceLease {
    fn drop(&mut self) {
        if let Some(key) = self.key {
            self.coordinator
                .working
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{PresentationFailure, RuntimeEvent, RuntimeObserver};
    #[test]
    fn allocation_reports_follow_rebalancing_and_external_release_across_replicates() {
        type Reports = Arc<Mutex<Vec<Vec<usize>>>>;
        struct Observer(Reports);
        impl RuntimeObserver for Observer {
            fn publish(&self, event: RuntimeEvent<'_>) -> Result<(), PresentationFailure> {
                if let RuntimeEvent::ThreadAllocations {
                    allocations,
                    budget,
                } = event
                {
                    let values: Vec<_> = allocations.iter().map(|item| item.2).collect();
                    assert!(values.iter().sum::<usize>() <= budget);
                    self.0.lock().unwrap().push(values);
                }
                Ok(())
            }
            fn cancellation_requested(&self) -> Result<bool, PresentationFailure> {
                Ok(false)
            }
            fn finish(&self) -> Result<(), PresentationFailure> {
                Ok(())
            }
        }
        let reports = Reports::default();
        let presentation =
            super::super::presentation::RuntimePresentation::new(Observer(reports.clone()));
        let coordinator = ResourceCoordinator::new(4, ComputeMode::Auto);
        coordinator
            .publish_allocations(&presentation, true)
            .unwrap();
        let mut first = coordinator
            .try_acquire(TaskResourceRequirement::AutoInProcess)
            .unwrap();
        first.activate(0, 0, "first").unwrap();
        first.publish_allocations(&presentation).unwrap();
        let mut second = coordinator
            .try_acquire(TaskResourceRequirement::AutoInProcess)
            .unwrap();
        second.activate(1, 0, "second").unwrap();
        second.publish_allocations(&presentation).unwrap();
        drop(first);
        second.publish_allocations(&presentation).unwrap();
        drop(second);
        let mut external = coordinator
            .try_acquire(TaskResourceRequirement::External { threads: 3 })
            .unwrap();
        external.activate(0, 1, "external").unwrap();
        external.publish_allocations(&presentation).unwrap();
        drop(external);
        coordinator
            .publish_allocations(&presentation, true)
            .unwrap();
        assert_eq!(
            *reports.lock().unwrap(),
            vec![vec![], vec![4], vec![2, 2], vec![4], vec![3], vec![]]
        );
    }
}
