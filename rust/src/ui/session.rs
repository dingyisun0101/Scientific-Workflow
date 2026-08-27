//! Automatic activation and progress-event throttling.

use std::collections::HashMap;
use std::io::{IsTerminal, stderr};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::event::UiEvent;
use super::plan::UiPlan;
use super::terminal;

/// Clone-cheap, thread-safe UI session shared by Runtime workers.
#[derive(Clone)]
pub(crate) struct UiSession {
    inner: Arc<UiSessionInner>,
}

/// Task-scoped publisher supplied to Runtime's task host.
pub(crate) struct TaskUi {
    session: UiSession,
    replicate: u64,
    identity: Box<str>,
}

struct UiSessionInner {
    enabled: bool,
    plan: UiPlan,
    progress: Mutex<HashMap<u64, HashMap<Box<str>, Instant>>>,
}

impl UiSession {
    /// Activates only when standard error is attached to an interactive terminal.
    pub(crate) fn automatic(plan: UiPlan) -> Self {
        Self {
            inner: Arc::new(UiSessionInner {
                enabled: stderr().is_terminal(),
                plan,
                progress: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Publishes one event without allowing presentation failure to fail science.
    pub(crate) fn publish(&self, event: UiEvent<'_>) {
        if !self.inner.enabled || !self.should_render(&event) {
            return;
        }
        terminal::render(&event);
    }

    pub(crate) fn task(&self, replicate: u64, identity: impl Into<Box<str>>) -> TaskUi {
        TaskUi {
            session: self.clone(),
            replicate,
            identity: identity.into(),
        }
    }

    fn should_render(&self, event: &UiEvent<'_>) -> bool {
        let mut progress = self
            .inner
            .progress
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match event {
            UiEvent::TaskProgress {
                replicate,
                identity,
                ..
            } => {
                let now = Instant::now();
                let tasks = progress.entry(*replicate).or_default();
                match tasks.get_mut(*identity) {
                    Some(last) if last.elapsed() < self.inner.plan.refresh_interval() => false,
                    Some(last) => {
                        *last = now;
                        true
                    }
                    None => {
                        tasks.insert((*identity).into(), now);
                        true
                    }
                }
            }
            UiEvent::TaskCompleted {
                replicate,
                identity,
                ..
            }
            | UiEvent::TaskFailed {
                replicate,
                identity,
                ..
            } => {
                if let Some(tasks) = progress.get_mut(replicate) {
                    tasks.remove(*identity);
                    if tasks.is_empty() {
                        progress.remove(replicate);
                    }
                }
                true
            }
            _ => true,
        }
    }
}

impl TaskUi {
    pub(crate) fn progress(&self, iteration: u64, target_iteration: Option<u64>) {
        self.session.publish(UiEvent::TaskProgress {
            replicate: self.replicate,
            identity: &self.identity,
            iteration,
            target_iteration,
        });
    }
}
