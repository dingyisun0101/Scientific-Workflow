//! Runtime disk guard, independent of terminal presentation and the paused clock.
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use super::presentation::RuntimePresentation;
use super::{RunControl, RuntimeError, RuntimeEvent};

pub(crate) fn usage(path: &Path) -> std::io::Result<f64> {
    let stats = fs2::statvfs(path)?;
    let total = stats.total_space();
    if total == 0 {
        return Err(std::io::Error::other("filesystem reported zero capacity"));
    }
    Ok(
        (total.saturating_sub(stats.available_space()) as f64 * 100.0 / total as f64)
            .clamp(0.0, 100.0),
    )
}

fn next_pause(paused: bool, occupied: f64, threshold: f64) -> bool {
    if paused {
        occupied > (threshold - 2.0).max(0.0)
    } else {
        occupied >= threshold
    }
}

pub(super) struct DiskGuard {
    stop: Option<mpsc::Sender<()>>,
    worker: Option<JoinHandle<Result<(), RuntimeError>>>,
    control: RunControl,
    path: PathBuf,
}

impl DiskGuard {
    pub(super) fn start(
        path: &Path,
        threshold: Option<f64>,
        presentation: &RuntimePresentation,
    ) -> Result<Self, RuntimeError> {
        let mut guard = Self {
            stop: None,
            worker: None,
            control: presentation.control.clone(),
            path: path.to_path_buf(),
        };
        let Some(threshold) = threshold else {
            return Ok(guard);
        };
        sample(path, threshold, presentation)?;
        let (stop, receiver) = mpsc::channel();
        let path = path.to_path_buf();
        let presentation = presentation.clone();
        guard.worker = Some(
            thread::Builder::new()
                .name("workflow-disk-guard".into())
                .spawn(move || {
                    while matches!(
                        receiver.recv_timeout(Duration::from_millis(250)),
                        Err(mpsc::RecvTimeoutError::Timeout)
                    ) {
                        if presentation.control.cancelled() {
                            break;
                        }
                        if let Err(error) = sample(&path, threshold, &presentation) {
                            presentation.control.cancel();
                            return Err(error);
                        }
                    }
                    Ok(())
                })
                .map_err(|source| RuntimeError::DiskMonitor {
                    path: guard.path.clone(),
                    source,
                })?,
        );
        guard.stop = Some(stop);
        Ok(guard)
    }

    pub(super) fn finish(mut self) -> Result<(), RuntimeError> {
        self.stop_and_join()
    }

    fn stop_and_join(&mut self) -> Result<(), RuntimeError> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        let result = self
            .worker
            .take()
            .map(|worker| {
                worker.join().unwrap_or_else(|_| {
                    Err(RuntimeError::DiskMonitor {
                        path: self.path.clone(),
                        source: std::io::Error::other("disk guard panicked"),
                    })
                })
            })
            .unwrap_or(Ok(()));
        self.control.pause_for_disk(false);
        result
    }
}

impl Drop for DiskGuard {
    fn drop(&mut self) {
        let _ = self.stop_and_join();
    }
}

fn sample(
    path: &Path,
    threshold: f64,
    presentation: &RuntimePresentation,
) -> Result<(), RuntimeError> {
    let occupied = usage(path).map_err(|source| RuntimeError::DiskMonitor {
        path: path.to_path_buf(),
        source,
    })?;
    let previous = presentation.control.disk_paused();
    let paused = next_pause(previous, occupied, threshold);
    if previous != paused {
        let message = if paused {
            format!("disk usage {occupied:.2}% reached {threshold:.2}%; auto-pausing")
        } else {
            format!("disk usage {occupied:.2}% recovered; disk pause released")
        };
        presentation.publish(RuntimeEvent::ResourcePolicy { message: &message })?;
        presentation.control.pause_for_disk(paused);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_threshold_and_recovery_margin() {
        assert!(!next_pause(false, 94.99, 95.0));
        assert!(next_pause(false, 95.0, 95.0));
        assert!(next_pause(true, 94.0, 95.0));
        assert!(!next_pause(true, 93.0, 95.0));
        assert!(!next_pause(true, 0.0, 1.0));
        assert!(usage(Path::new("/this-path-does-not-exist/workflow")).is_err());
    }

    #[test]
    fn monitor_errors_cancel_headless_control_and_are_returned_on_join() {
        struct Observer;
        impl super::super::RuntimeObserver for Observer {
            fn publish(
                &self,
                _: RuntimeEvent<'_>,
            ) -> Result<(), super::super::PresentationFailure> {
                Ok(())
            }
            fn cancellation_requested(&self) -> Result<bool, super::super::PresentationFailure> {
                Ok(false)
            }
            fn finish(&self) -> Result<(), super::super::PresentationFailure> {
                Ok(())
            }
        }
        let path = std::env::temp_dir().join(format!("workflow-disk-test-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        let presentation = RuntimePresentation::new(Observer);
        let guard = DiskGuard::start(&path, Some(100.0), &presentation).unwrap();
        std::fs::remove_dir(&path).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !presentation.control.cancelled() && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(presentation.control.cancelled());
        assert!(matches!(
            guard.finish(),
            Err(RuntimeError::DiskMonitor { .. })
        ));
    }
}
