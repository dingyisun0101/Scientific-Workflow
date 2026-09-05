//! Process-wide compute permits shared by every replicate scheduler.

use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Clone)]
pub(super) struct ResourceBudget {
    inner: Arc<BudgetInner>,
}

struct BudgetInner {
    total: usize,
    state: Mutex<BudgetState>,
}

#[derive(Default)]
struct BudgetState {
    in_process_tasks: usize,
    external_threads: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ResourceRequirement {
    InProcess,
    External { threads: usize },
}

pub(super) struct ResourceLease {
    budget: ResourceBudget,
    requirement: ResourceRequirement,
}

impl ResourceBudget {
    pub(super) fn new(total: usize) -> Self {
        debug_assert!(total > 0);
        Self {
            inner: Arc::new(BudgetInner {
                total,
                state: Mutex::new(BudgetState::default()),
            }),
        }
    }

    pub(super) fn try_acquire(&self, requirement: ResourceRequirement) -> Option<ResourceLease> {
        let mut state = self.state();
        let available = self.inner.total - state.external_threads;
        let admitted = match requirement {
            ResourceRequirement::InProcess if state.external_threads == 0 => {
                state.in_process_tasks += 1;
                true
            }
            ResourceRequirement::External { threads }
                if state.in_process_tasks == 0 && threads <= available =>
            {
                state.external_threads += threads;
                true
            }
            ResourceRequirement::InProcess | ResourceRequirement::External { .. } => false,
        };
        drop(state);
        admitted.then(|| ResourceLease {
            budget: self.clone(),
            requirement,
        })
    }

    fn state(&self) -> MutexGuard<'_, BudgetState> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl ResourceLease {
    pub(super) fn threads(&self) -> Option<usize> {
        match self.requirement {
            ResourceRequirement::External { threads } => Some(threads),
            ResourceRequirement::InProcess => None,
        }
    }
}

impl Drop for ResourceLease {
    fn drop(&mut self) {
        let mut state = self.budget.state();
        match self.requirement {
            ResourceRequirement::InProcess => {
                debug_assert!(state.in_process_tasks > 0);
                state.in_process_tasks -= 1;
            }
            ResourceRequirement::External { threads } => {
                debug_assert!(state.external_threads >= threads);
                state.external_threads -= threads;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ResourceBudget, ResourceRequirement};

    #[test]
    fn in_process_and_external_work_are_mutually_exclusive() {
        let budget = ResourceBudget::new(4);
        let first = budget
            .try_acquire(ResourceRequirement::InProcess)
            .expect("in-process work fits an idle budget");
        let second = budget
            .try_acquire(ResourceRequirement::InProcess)
            .expect("in-process tasks may share the fixed pool");
        assert!(
            budget
                .try_acquire(ResourceRequirement::External { threads: 1 })
                .is_none()
        );
        drop((first, second));

        let external = budget
            .try_acquire(ResourceRequirement::External { threads: 3 })
            .expect("external request fits the idle budget");
        assert!(
            budget
                .try_acquire(ResourceRequirement::External { threads: 2 })
                .is_none()
        );
        let remaining = budget
            .try_acquire(ResourceRequirement::External { threads: 1 })
            .expect("external requests may consume the remaining permits");
        assert!(budget.try_acquire(ResourceRequirement::InProcess).is_none());
        drop((external, remaining));
        assert!(budget.try_acquire(ResourceRequirement::InProcess).is_some());
    }
}
