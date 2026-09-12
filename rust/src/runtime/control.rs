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
    manual_pause: bool,
    disk_pause: bool,
    disk_ready: bool,
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
    pub(crate) fn pause(&self, paused: bool) {
        self.set_pause(paused, false);
    }
    pub(crate) fn pause_for_disk(&self, paused: bool) {
        self.set_pause(paused, true);
    }
    pub(crate) fn disk_paused(&self) -> bool {
        self.0
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .disk_pause
    }
    /// Update the sampled safety gate without automatically releasing a pause.
    /// Returns a transition reminder for the dashboard and durable log.
    pub(crate) fn sample_disk(&self, occupied: f64, threshold: f64) -> Option<String> {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let recovery = (threshold - 2.0).max(0.0);
        let message = if !state.disk_pause && occupied >= threshold {
            state.disk_pause = true;
            state.disk_ready = false;
            Some(format!(
                "disk usage {occupied:.2}% reached {threshold:.2}%; paused. Free space to at most {recovery:.2}%, then type resume and Enter. Work will not resume automatically."
            ))
        } else if state.disk_pause {
            let ready = occupied <= recovery;
            let changed = ready != state.disk_ready;
            state.disk_ready = ready;
            changed.then(|| if ready {
                format!("disk usage {occupied:.2}% is at or below {recovery:.2}%; still paused. Type resume and Enter to continue.")
            } else {
                format!("disk usage {occupied:.2}% is above {recovery:.2}%; resume is blocked. Free space, then type resume and Enter.")
            })
        } else {
            None
        };
        Self::update_clock(&mut state);
        self.0.wake.notify_all();
        message
    }
    /// One explicit command releases both reasons only after safe disk recovery.
    pub(crate) fn resume(&self) -> bool {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.disk_pause && !state.disk_ready {
            return false;
        }
        state.manual_pause = false;
        state.disk_pause = false;
        state.disk_ready = false;
        Self::update_clock(&mut state);
        self.0.wake.notify_all();
        true
    }
    fn set_pause(&self, paused: bool, disk: bool) {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if disk {
            state.disk_pause = paused;
            state.disk_ready = false;
        } else {
            state.manual_pause = paused;
        }
        Self::update_clock(&mut state);
        self.0.wake.notify_all();
    }
    fn update_clock(state: &mut State) {
        let paused = state.disk_pause || state.manual_pause;
        if paused && !state.cancelled && state.paused_since.is_none() {
            state.paused_since = Some(Instant::now());
        }
        if !paused && let Some(start) = state.paused_since.take() {
            state.paused_total += start.elapsed();
        }
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
    pub(crate) fn status(&self) -> &'static str {
        if self.cancelled() {
            "cancelling"
        } else if self.disk_paused() {
            let state = self
                .0
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.disk_ready {
                "disk ready; type resume and Enter"
            } else {
                "disk pause; free space, then type resume"
            }
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
    fn disk_recovery_and_manual_resume_preserve_other_pause_reasons() {
        let control = RunControl::default();
        control.pause(true);
        control.pause_for_disk(true);
        control.pause(false);
        assert!(control.paused());
        control.pause(true);
        control.pause_for_disk(false);
        assert!(control.paused());
        control.pause(false);
        assert!(!control.paused());
    }
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
