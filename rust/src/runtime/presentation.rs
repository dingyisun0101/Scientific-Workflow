//! Runtime-owned presentation port and task-scoped progress publisher.

use std::sync::Arc;

use super::{RuntimeError, RuntimeEvent};

pub(crate) type PresentationFailure = Box<dyn std::error::Error + Send + Sync + 'static>;

/// Downstream adapter for Runtime-owned lifecycle facts and cancellation input.
pub(crate) trait RuntimeObserver: Send + Sync + 'static {
    fn publish(&self, event: RuntimeEvent<'_>) -> Result<(), PresentationFailure>;
    fn cancellation_requested(&self) -> Result<bool, PresentationFailure>;
    fn finish(&self) -> Result<(), PresentationFailure>;
}

/// Clone-cheap Runtime handle around one selected presentation adapter.
#[derive(Clone)]
pub(crate) struct RuntimePresentation {
    observer: Arc<dyn RuntimeObserver>,
}

impl RuntimePresentation {
    pub(crate) fn new(observer: impl RuntimeObserver) -> Self {
        Self {
            observer: Arc::new(observer),
        }
    }

    pub(crate) fn publish(&self, event: RuntimeEvent<'_>) -> Result<(), RuntimeError> {
        self.observer
            .publish(event)
            .map_err(RuntimeError::presentation_boxed)
    }

    pub(crate) fn cancellation_requested(&self) -> Result<bool, RuntimeError> {
        self.observer
            .cancellation_requested()
            .map_err(RuntimeError::presentation_boxed)
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
        }
    }
}

/// Task-scoped progress publisher retained by Runtime's execution host.
pub(crate) struct TaskPresentation {
    presentation: RuntimePresentation,
    replicate: u64,
    identity: Box<str>,
}

impl TaskPresentation {
    pub(crate) fn progress(&self, iteration: u64, target_iteration: Option<u64>) {
        let _ = self.presentation.publish(RuntimeEvent::TaskProgress {
            replicate: self.replicate,
            identity: &self.identity,
            iteration,
            target_iteration,
        });
    }
}
