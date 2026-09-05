//! Runtime-owned presentation port and task-scoped progress publisher.

use std::sync::Arc;

use super::{RuntimeError, RuntimeEvent};

pub(crate) type PresentationFailure = Box<dyn std::error::Error + Send + Sync + 'static>;

/// Downstream adapter for Runtime-owned lifecycle facts and cancellation input.
pub(crate) trait RuntimeObserver: Send + Sync + 'static {
    fn publish(&self, event: RuntimeEvent<'_>) -> Result<(), PresentationFailure>;
    fn cancellation_requested(&self) -> Result<bool, PresentationFailure>;
    fn control(&self) -> super::RunControl {
        super::RunControl::default()
    }
    fn finish(&self) -> Result<(), PresentationFailure>;
}

/// Clone-cheap Runtime handle around one selected presentation adapter.
#[derive(Clone)]
pub(crate) struct RuntimePresentation {
    observer: Arc<dyn RuntimeObserver>,
    pub(crate) control: super::RunControl,
}

impl RuntimePresentation {
    pub(crate) fn new(observer: impl RuntimeObserver) -> Self {
        let control = observer.control();
        Self {
            control,
            observer: Arc::new(observer),
        }
    }

    pub(crate) fn publish(&self, event: RuntimeEvent<'_>) -> Result<(), RuntimeError> {
        self.observer
            .publish(event)
            .map_err(RuntimeError::presentation_boxed)
    }

    pub(crate) fn cancellation_requested(&self) -> Result<bool, RuntimeError> {
        match self.observer.cancellation_requested() {
            Ok(requested) => {
                if requested {
                    self.control.cancel();
                }
                Ok(requested || self.control.cancelled())
            }
            Err(error) => {
                self.control.cancel();
                Err(RuntimeError::presentation_boxed(error))
            }
        }
    }

    pub(crate) fn finish(&self) -> Result<(), RuntimeError> {
        self.observer
            .finish()
            .map_err(RuntimeError::presentation_boxed)
    }

    pub(crate) fn task(&self, replicate: u64, identity: impl Into<Box<str>>) -> TaskPresentation {
        TaskPresentation {
            presentation: self.clone(),
            replicate,
            identity: identity.into(),
            progress: Arc::new(std::sync::Mutex::new((None, None))),
        }
    }
}

/// Task-scoped progress publisher retained by Runtime's execution host.
#[derive(Clone)]
pub(crate) struct TaskPresentation {
    presentation: RuntimePresentation,
    replicate: u64,
    identity: Box<str>,
    progress: Arc<std::sync::Mutex<ProgressState>>,
}

type ProgressState = (Option<std::time::Instant>, Option<(u64, Option<u64>)>);

impl TaskPresentation {
    pub(crate) fn control(&self) -> &super::RunControl {
        &self.presentation.control
    }

    pub(crate) fn log(&self, level: &str, message: &str) {
        let _ = self.presentation.publish(RuntimeEvent::ProgramLog {
            replicate: self.replicate,
            identity: &self.identity,
            level,
            message,
        });
    }
    pub(crate) fn program_progress(
        &self,
        stage: &str,
        completed: u64,
        total: Option<u64>,
        unit: &str,
    ) {
        let _ = self.presentation.publish(RuntimeEvent::ProgramProgress {
            replicate: self.replicate,
            identity: &self.identity,
            stage,
            completed,
            total,
            unit,
        });
    }
    pub(crate) fn progress(&self, iteration: u64, target_iteration: Option<u64>) {
        let mut state = self
            .progress
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.1 = Some((iteration, target_iteration));
        if state
            .0
            .is_none_or(|last| last.elapsed() >= std::time::Duration::from_millis(50))
        {
            let (iteration, target_iteration) = state.1.take().unwrap();
            state.0 = Some(std::time::Instant::now());
            let _ = self.presentation.publish(RuntimeEvent::TaskProgress {
                replicate: self.replicate,
                identity: &self.identity,
                iteration,
                target_iteration,
            });
        }
    }
    pub(crate) fn flush(&self) {
        let mut state = self
            .progress
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((iteration, target_iteration)) = state.1.take() {
            let _ = self.presentation.publish(RuntimeEvent::TaskProgress {
                replicate: self.replicate,
                identity: &self.identity,
                iteration,
                target_iteration,
            });
        }
    }
}
