//! Runtime-owned cooperative control and one pause-aware execution clock.
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Default)]
pub(crate) struct RunControl(Arc<Inner>);
#[derive(Default)]
struct Inner {
    state: Mutex<State>,
    wake: Condvar,
    active: AtomicUsize,
    parked: AtomicUsize,
}
#[derive(Default)]
struct State {
    paused_since: Option<Instant>,
    paused_total: Duration,
    cancelled: bool,
}
impl RunControl {
    pub(crate) fn now(&self) -> Instant {
        let state = self
            .0
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.paused_since.unwrap_or_else(Instant::now) - state.paused_total
    }
    #[cfg(any(feature = "terminal-ui", test))]
    pub(crate) fn pause(&self, paused: bool) {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if paused && !state.cancelled && state.paused_since.is_none() {
            state.paused_since = Some(Instant::now());
        }
        if !paused && let Some(start) = state.paused_since.take() {
            state.paused_total += start.elapsed();
        }
        self.0.wake.notify_all();
    }
    pub(crate) fn paused(&self) -> bool {
        self.0
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .paused_since
            .is_some()
    }
    pub(crate) fn cancel(&self) {
        self.0
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .cancelled = true;
        self.0.wake.notify_all();
    }
    pub(crate) fn cancelled(&self) -> bool {
        self.0
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .cancelled
    }
    #[cfg(any(feature = "terminal-ui", test))]
    pub(crate) fn status(&self) -> &'static str {
        if self.cancelled() {
            "cancelling"
        } else if !self.paused() {
            "running"
        } else if self.0.parked.load(Ordering::Acquire) >= self.0.active.load(Ordering::Acquire) {
            "paused"
        } else {
            "pausing; waiting for active calls/programs"
        }
    }
    pub(crate) fn activity(&self) -> Activity {
        self.0.active.fetch_add(1, Ordering::AcqRel);
        Activity(self.clone())
    }
    pub(crate) fn parked(&self) -> Parked {
        self.0.parked.fetch_add(1, Ordering::AcqRel);
        Parked(self.clone())
    }
    pub(crate) fn checkpoint(&self, cancellation: &AtomicBool) {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut parked = None;
        while state.paused_since.is_some()
            && !state.cancelled
            && !cancellation.load(Ordering::Acquire)
        {
            parked.get_or_insert_with(|| self.parked());
            state = self
                .0
                .wake
                .wait_timeout(state, Duration::from_millis(10))
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
    }
}
pub(crate) struct Activity(RunControl);
impl Drop for Activity {
    fn drop(&mut self) {
        self.0.0.active.fetch_sub(1, Ordering::AcqRel);
    }
}
pub(crate) struct Parked(RunControl);
impl Drop for Parked {
    fn drop(&mut self) {
        self.0.0.parked.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clock_freezes_immediately_and_cancellation_wakes_a_parked_worker() {
        let control = RunControl::default();
        let _activity = control.activity();
        control.pause(true);
        let frozen = control.now();
        let worker_control = control.clone();
        let worker = std::thread::spawn(move || worker_control.checkpoint(&AtomicBool::new(false)));
        let deadline = Instant::now() + Duration::from_secs(1);
        while control.status() != "paused" && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(control.status(), "paused");
        assert_eq!(control.now(), frozen);
        control.pause(true);
        assert_eq!(control.now(), frozen);
        control.cancel();
        worker.join().unwrap();
        assert_eq!(control.status(), "cancelling");
        control.pause(false);
        assert!(control.now() >= frozen);
    }
}
