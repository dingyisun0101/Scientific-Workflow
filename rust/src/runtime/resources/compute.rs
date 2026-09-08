//! Fair execution-unit compute allocation over task-private Rayon pools.

use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use serde_json::{Value, json};
use thiserror::Error;

use crate::config::ComputeMode;
use crate::task::TaskResult;

#[derive(Clone)]
pub(crate) struct ComputeCoordinator {
    inner: Arc<CoordinatorInner>,
}

struct CoordinatorInner {
    total_threads: usize,
    mode: ComputeMode,
    state: Mutex<CoordinatorState>,
    changed: Condvar,
}

#[derive(Default)]
struct CoordinatorState {
    slots: BTreeMap<(u64, u64), ComputeSlot>,
    rebalancing: bool,
    epoch: u64,
    failure: Option<String>,
}

struct ComputeSlot {
    fixed_threads: Option<usize>,
    pool: Option<Arc<rayon::ThreadPool>>,
    in_compute: bool,
    history: Vec<Allocation>,
}

#[derive(Clone, Copy)]
struct Allocation {
    epoch: u64,
    threads: usize,
}

pub(crate) struct ComputeLease {
    coordinator: ComputeCoordinator,
    key: (u64, u64),
}

#[derive(Debug, Error)]
#[error("execution-unit compute allocation failed: {reason}")]
pub(crate) struct ComputeError {
    reason: String,
}

impl ComputeCoordinator {
    pub(crate) fn new(total_threads: usize, mode: ComputeMode) -> Self {
        Self {
            inner: Arc::new(CoordinatorInner {
                total_threads,
                mode,
                state: Mutex::new(CoordinatorState::default()),
                changed: Condvar::new(),
            }),
        }
    }

    pub(crate) fn register(
        &self,
        replicate: u64,
        output_ordinal: u64,
        fixed_threads: Option<usize>,
    ) -> Result<ComputeLease, ComputeError> {
        let key = (replicate, output_ordinal);
        let mut state = self.state();
        if state.slots.contains_key(&key) {
            return Err(ComputeError {
                reason: format!("duplicate working task {replicate}/{output_ordinal}"),
            });
        }
        let required_epoch = state.epoch.checked_add(1).ok_or_else(|| ComputeError {
            reason: "automatic compute allocation epoch overflow".to_owned(),
        })?;
        state.slots.insert(
            key,
            ComputeSlot {
                fixed_threads,
                pool: None,
                in_compute: false,
                history: Vec::new(),
            },
        );
        match self.inner.mode {
            ComputeMode::Auto => {
                state.rebalancing = true;
                state = self.wait_for_epoch(state, required_epoch);
            }
            ComputeMode::Isolated => self.build_isolated(key, &mut state),
        }
        let result = self.result(&state);
        self.inner.changed.notify_all();
        result?;
        Ok(ComputeLease {
            coordinator: self.clone(),
            key,
        })
    }

    fn state(&self) -> MutexGuard<'_, CoordinatorState> {
        self.inner
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn wait_for_epoch<'a>(
        &self,
        mut state: MutexGuard<'a, CoordinatorState>,
        required_epoch: u64,
    ) -> MutexGuard<'a, CoordinatorState> {
        while state.epoch < required_epoch && state.failure.is_none() {
            if state.slots.values().all(|slot| !slot.in_compute) {
                self.rebuild_auto(&mut state);
            } else {
                state = self
                    .inner
                    .changed
                    .wait(state)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        }
        state
    }

    fn build_isolated(&self, key: (u64, u64), state: &mut CoordinatorState) {
        let threads = state.slots[&key]
            .fixed_threads
            .expect("isolated config requires a fixed task allocation");
        state.epoch += 1;
        let epoch = state.epoch;
        match build_pool(key, threads) {
            Ok(pool) => {
                let slot = state.slots.get_mut(&key).expect("inserted compute slot");
                slot.pool = Some(pool);
                slot.history.push(Allocation { epoch, threads });
            }
            Err(error) => state.failure = Some(error.to_string()),
        }
    }

    fn rebuild_auto(&self, state: &mut CoordinatorState) {
        if state.failure.is_some() || !state.rebalancing {
            return;
        }
        debug_assert!(state.slots.values().all(|slot| !slot.in_compute));
        let epoch = state
            .epoch
            .checked_add(1)
            .expect("compute allocation epoch cannot overflow in a process lifetime");
        let count = state.slots.len();
        if count == 0 {
            state.epoch = epoch;
            state.rebalancing = false;
            return;
        }
        let quotient = self.inner.total_threads / count;
        let remainder = self.inner.total_threads % count;
        debug_assert!(quotient > 0, "resource admission bounds working auto tasks");
        let allocations = state
            .slots
            .keys()
            .copied()
            .enumerate()
            .map(|(index, key)| (key, quotient + usize::from(index < remainder)))
            .collect::<Vec<_>>();
        let mut pools = Vec::with_capacity(count);
        for (key, threads) in &allocations {
            match build_pool(*key, *threads) {
                Ok(pool) => pools.push((*key, *threads, pool)),
                Err(error) => {
                    state.failure = Some(error.to_string());
                    self.inner.changed.notify_all();
                    return;
                }
            }
        }
        for (key, threads, pool) in pools {
            let slot = state.slots.get_mut(&key).expect("planned compute slot");
            slot.pool = Some(pool);
            if slot.history.last().map(|allocation| allocation.threads) != Some(threads) {
                slot.history.push(Allocation { epoch, threads });
            }
        }
        state.epoch = epoch;
        state.rebalancing = false;
    }

    fn result(&self, state: &CoordinatorState) -> Result<(), ComputeError> {
        match &state.failure {
            Some(reason) => Err(ComputeError {
                reason: reason.clone(),
            }),
            None => Ok(()),
        }
    }
}

impl ComputeLease {
    pub(crate) fn run(&self, operation: &mut (dyn FnMut() -> TaskResult + Send)) -> TaskResult {
        let pool = {
            let mut state = self.coordinator.state();
            while state.rebalancing && state.failure.is_none() {
                state = self
                    .coordinator
                    .inner
                    .changed
                    .wait(state)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
            self.coordinator.result(&state)?;
            let slot = state.slots.get_mut(&self.key).ok_or_else(|| ComputeError {
                reason: "working task lost its compute allocation".to_owned(),
            })?;
            slot.in_compute = true;
            Arc::clone(slot.pool.as_ref().expect("allocated task pool"))
        };
        let result = pool.install(operation);
        let mut state = self.coordinator.state();
        if let Some(slot) = state.slots.get_mut(&self.key) {
            slot.in_compute = false;
        }
        self.coordinator.inner.changed.notify_all();
        if state.rebalancing && state.slots.values().all(|slot| !slot.in_compute) {
            self.coordinator.rebuild_auto(&mut state);
            self.coordinator.inner.changed.notify_all();
        }
        self.coordinator.result(&state)?;
        result
    }

    pub(crate) fn provenance(&self) -> Value {
        let state = self.coordinator.state();
        let history = state
            .slots
            .get(&self.key)
            .map(|slot| {
                slot.history
                    .iter()
                    .map(|allocation| {
                        json!({"epoch": allocation.epoch, "threads": allocation.threads})
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        json!({
            "mode": match self.coordinator.inner.mode {
                ComputeMode::Auto => "auto",
                ComputeMode::Isolated => "isolated",
            },
            "allocations": history,
        })
    }
}

impl Drop for ComputeLease {
    fn drop(&mut self) {
        let mut state = self.coordinator.state();
        state.slots.remove(&self.key);
        if self.coordinator.inner.mode == ComputeMode::Auto {
            state.rebalancing = true;
            if state.slots.values().all(|slot| !slot.in_compute) {
                self.coordinator.rebuild_auto(&mut state);
            }
        }
        self.coordinator.inner.changed.notify_all();
    }
}

fn build_pool(
    (replicate, output_ordinal): (u64, u64),
    threads: usize,
) -> Result<Arc<rayon::ThreadPool>, rayon::ThreadPoolBuildError> {
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .thread_name(move |index| {
            format!("workflow-r{replicate}-t{output_ordinal}-compute-{index}")
        })
        .build()
        .map(Arc::new)
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::{ComputeCoordinator, ComputeSlot};
    use crate::config::ComputeMode;

    fn observed_threads(lease: &super::ComputeLease) -> usize {
        let mut observed = 0;
        let mut operation = || {
            observed = rayon::current_num_threads();
            Ok(())
        };
        lease.run(&mut operation).unwrap();
        observed
    }

    #[test]
    fn auto_allocation_tracks_only_registered_working_tasks_and_rebalances_equally() {
        let coordinator = ComputeCoordinator::new(4, ComputeMode::Auto);
        let first = coordinator.register(0, 0, None).unwrap();
        assert_eq!(observed_threads(&first), 4);

        let second = coordinator.register(0, 1, None).unwrap();
        assert_eq!(observed_threads(&first), 2);
        assert_eq!(observed_threads(&second), 2);
        assert_eq!(
            first.provenance(),
            serde_json::json!({
                "mode": "auto",
                "allocations": [
                    {"epoch": 1, "threads": 4},
                    {"epoch": 2, "threads": 2}
                ]
            })
        );

        drop(second);
        assert_eq!(observed_threads(&first), 4);
        assert_eq!(first.provenance()["allocations"][2]["threads"], 4);
    }

    #[test]
    fn isolated_allocation_remains_fixed_as_other_tasks_start_and_finish() {
        let coordinator = ComputeCoordinator::new(8, ComputeMode::Isolated);
        let first = coordinator.register(0, 0, Some(3)).unwrap();
        let second = coordinator.register(0, 1, Some(2)).unwrap();
        assert_eq!(observed_threads(&first), 3);
        assert_eq!(observed_threads(&second), 2);
        drop(second);
        assert_eq!(observed_threads(&first), 3);
        assert_eq!(first.provenance()["mode"], "isolated");
    }

    #[test]
    fn auto_start_waits_for_an_active_step_boundary_before_replacing_pools() {
        let coordinator = ComputeCoordinator::new(2, ComputeMode::Auto);
        let first = coordinator.register(0, 0, None).unwrap();
        let (entered_tx, entered_rx) = mpsc::sync_channel(0);
        let (release_tx, release_rx) = mpsc::sync_channel(0);

        std::thread::scope(|scope| {
            let active = scope.spawn(|| {
                let mut operation = move || {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok(())
                };
                first.run(&mut operation).unwrap();
            });
            entered_rx.recv().unwrap();

            let registration_coordinator = coordinator.clone();
            let (attempting_tx, attempting_rx) = mpsc::sync_channel(0);
            let (registered_tx, registered_rx) = mpsc::sync_channel(0);
            let registering = scope.spawn(move || {
                attempting_tx.send(()).unwrap();
                let second = registration_coordinator.register(0, 1, None).unwrap();
                registered_tx.send(()).unwrap();
                second
            });
            attempting_rx.recv().unwrap();
            let deadline = Instant::now() + Duration::from_secs(1);
            while Instant::now() < deadline {
                if coordinator.state().rebalancing {
                    break;
                }
                std::thread::yield_now();
            }
            assert!(coordinator.state().rebalancing);
            assert!(matches!(
                registered_rx.try_recv(),
                Err(mpsc::TryRecvError::Empty)
            ));

            release_tx.send(()).unwrap();
            active.join().unwrap();
            registered_rx.recv().unwrap();
            let second = registering.join().unwrap();
            assert_eq!(observed_threads(&first), 1);
            assert_eq!(observed_threads(&second), 1);
        });
    }

    #[test]
    fn completed_registration_epoch_is_not_blocked_by_resumed_compute() {
        let coordinator = ComputeCoordinator::new(2, ComputeMode::Auto);
        let first = coordinator.register(0, 0, None).unwrap();
        let newcomer = (0, 1);
        let required_epoch;
        {
            let mut state = coordinator.state();
            required_epoch = state.epoch + 1;
            state.slots.insert(
                newcomer,
                ComputeSlot {
                    fixed_threads: None,
                    pool: None,
                    in_compute: false,
                    history: Vec::new(),
                },
            );
            state.rebalancing = true;
            coordinator.rebuild_auto(&mut state);

            // Force the reported race: an existing task begins its next step
            // before the registering waiter reacquires the coordinator lock.
            state.slots.get_mut(&(0, 0)).unwrap().in_compute = true;
            assert_eq!(state.epoch, required_epoch);
            assert!(!state.rebalancing);
        }

        let state = coordinator.wait_for_epoch(coordinator.state(), required_epoch);
        assert!(state.slots[&newcomer].pool.is_some());
        assert!(state.slots[&(0, 0)].in_compute);
        drop(state);
        drop(first);
    }
}
