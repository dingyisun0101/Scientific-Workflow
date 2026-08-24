//! Optional phase timing gates and cooperative expiration watches.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::error::StudyError;

const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(10);

pub(crate) struct StartGate {
    interval: Option<Duration>,
    state: Mutex<StartGateState>,
}

struct StartGateState {
    next_rank: usize,
    earliest_start: Option<Instant>,
}

impl StartGate {
    pub(crate) fn new(interval: Option<Duration>) -> Self {
        Self {
            interval,
            state: Mutex::new(StartGateState {
                next_rank: 0,
                earliest_start: None,
            }),
        }
    }

    /// Waits for one dense executable-task rank while it remains pending.
    pub(crate) fn wait_for_turn<'a>(
        &'a self,
        rank: usize,
        cancelled: &AtomicBool,
        stopped: &AtomicBool,
    ) -> Option<StartPermit<'a>> {
        let Some(interval) = self.interval else {
            return (!cancelled.load(Ordering::Acquire) && !stopped.load(Ordering::Acquire))
                .then_some(StartPermit {
                    interval: None,
                    state: None,
                });
        };
        loop {
            if cancelled.load(Ordering::Acquire) || stopped.load(Ordering::Acquire) {
                return None;
            }
            let state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.next_rank != rank {
                drop(state);
                thread::sleep(CANCELLATION_POLL_INTERVAL);
                continue;
            }
            let now = Instant::now();
            if let Some(earliest_start) = state.earliest_start
                && now < earliest_start
            {
                let remaining = earliest_start.duration_since(now);
                drop(state);
                thread::sleep(remaining.min(CANCELLATION_POLL_INTERVAL));
                continue;
            }
            return Some(StartPermit {
                interval: Some(interval),
                state: Some(state),
            });
        }
    }
}

/// Exclusive permission to transition one ranked task from pending to running.
pub(crate) struct StartPermit<'a> {
    interval: Option<Duration>,
    state: Option<MutexGuard<'a, StartGateState>>,
}

impl StartPermit<'_> {
    /// Records the completed running transition before allowing the next rank.
    pub(crate) fn started(mut self) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        state.next_rank += 1;
        state.earliest_start = Instant::now().checked_add(
            self.interval
                .expect("a delayed start permit always retains its interval"),
        );
    }
}

pub(crate) struct TimingFailures(Mutex<Option<StudyError>>);

impl TimingFailures {
    pub(crate) fn new() -> Self {
        Self(Mutex::new(None))
    }

    fn record(&self, error: StudyError) {
        let mut first = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if first.is_none() {
            *first = Some(error);
        }
    }

    pub(crate) fn take(&self) -> Option<StudyError> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
}

pub(crate) struct ExpirationWatch {
    completed: mpsc::Sender<()>,
    expired: Arc<AtomicBool>,
    worker: JoinHandle<()>,
}

impl ExpirationWatch {
    pub(crate) fn task(
        timeout: Duration,
        task: String,
        cancelled: Arc<AtomicBool>,
        failures: Arc<TimingFailures>,
    ) -> Self {
        Self::start(timeout, cancelled, failures, move || {
            StudyError::TaskTimedOut { task, timeout }
        })
    }

    pub(crate) fn phase(
        deadline_after: Duration,
        phase: u64,
        cancelled: Arc<AtomicBool>,
        failures: Arc<TimingFailures>,
    ) -> Self {
        Self::start(deadline_after, cancelled, failures, move || {
            StudyError::PhaseDeadlineExceeded {
                phase,
                deadline_after,
            }
        })
    }

    fn start<F>(
        duration: Duration,
        cancelled: Arc<AtomicBool>,
        failures: Arc<TimingFailures>,
        error: F,
    ) -> Self
    where
        F: FnOnce() -> StudyError + Send + 'static,
    {
        let (completed, receiver) = mpsc::channel();
        let expired = Arc::new(AtomicBool::new(false));
        let worker_expired = Arc::clone(&expired);
        let worker = thread::spawn(move || {
            if receiver.recv_timeout(duration) == Err(mpsc::RecvTimeoutError::Timeout) {
                worker_expired.store(true, Ordering::Release);
                failures.record(error());
                cancelled.store(true, Ordering::Release);
            }
        });
        Self {
            completed,
            expired,
            worker,
        }
    }

    pub(crate) fn finish(self) -> bool {
        let Self {
            completed,
            expired,
            worker,
        } = self;
        let _ = completed.send(());
        let _ = worker.join();
        expired.load(Ordering::Acquire)
    }
}
