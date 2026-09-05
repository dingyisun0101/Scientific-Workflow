//! Automatic dashboard activation, command handling, and event publication.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};

use thiserror::Error;

use super::command::{CommandSubmission, UiCommand};
use super::plan::UiPlan;
use super::state::DashboardState;
use super::terminal::{self, DashboardTerminal};
use crate::runtime::{PresentationFailure, RuntimeEvent, RuntimeObserver};

/// Clone-cheap, thread-safe UI session shared by Runtime workers.
#[derive(Clone)]
pub(crate) struct UiSession {
    inner: Arc<UiSessionInner>,
}

/// A structured failure of the automatically selected presentation adapter.
#[derive(Clone, Debug, Error)]
#[error("{reason}")]
pub(crate) struct UiFailure {
    reason: String,
}

impl UiFailure {
    fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
        }
    }
}

struct UiSessionInner {
    interactive: bool,
    control: crate::runtime::RunControl,
    plan: UiPlan,
    state: Mutex<DashboardState>,
    cancellation_requested: AtomicBool,
    finished: AtomicBool,
    renderer: Mutex<Option<JoinHandle<()>>>,
    render_health: RenderHealth,
}

#[derive(Default)]
struct RenderHealth {
    failure: Mutex<Option<String>>,
}

impl RenderHealth {
    fn fail(&self, reason: String) {
        lock(&self.failure).get_or_insert(reason);
    }

    fn check(&self) -> Result<(), UiFailure> {
        if let Some(reason) = lock(&self.failure).clone() {
            return Err(UiFailure::new(reason));
        }
        Ok(())
    }
}

impl UiSession {
    /// Selects the Ratatui dashboard for a terminal and plain lines otherwise.
    pub(crate) fn automatic() -> Result<Self, UiFailure> {
        let interactive = terminal::interactive();
        let control = crate::runtime::RunControl::default();
        let inner = Arc::new(UiSessionInner {
            control: control.clone(),
            interactive,
            plan: UiPlan::automatic(),
            state: Mutex::new(DashboardState::with_control(control)),
            cancellation_requested: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            renderer: Mutex::new(None),
            render_health: RenderHealth::default(),
        });
        if interactive {
            let renderer_inner = Arc::clone(&inner);
            let (ready, readiness) = mpsc::sync_channel(0);
            let renderer = thread::Builder::new()
                .name("scientific-workflow-ui".to_owned())
                .spawn(move || render_loop(&renderer_inner, ready))
                .map_err(|source| {
                    UiFailure::new(format!("failed to start renderer thread: {source}"))
                })?;
            match readiness.recv() {
                Ok(Ok(())) => *lock(&inner.renderer) = Some(renderer),
                Ok(Err(reason)) => {
                    let _ = renderer.join();
                    return Err(UiFailure::new(reason));
                }
                Err(_) => match renderer.join() {
                    Ok(()) => {
                        return Err(UiFailure::new(
                            "renderer stopped before terminal initialization",
                        ));
                    }
                    Err(_) => return Err(UiFailure::new("renderer thread panicked")),
                },
            }
        }
        Ok(Self { inner })
    }

    /// Publishes one event and enforces the selected UI as a healthy interface.
    pub(crate) fn publish(&self, event: RuntimeEvent<'_>) -> Result<(), UiFailure> {
        self.inner.render_health.check()?;
        lock(&self.inner.state).apply(&event);
        if !self.inner.interactive
            && let Err(source) = terminal::render_plain(&event)
        {
            let reason = format!("plain output failed: {source}");
            self.inner.render_health.fail(reason.clone());
            return Err(UiFailure::new(reason));
        }
        self.inner.render_health.check()
    }

    pub(crate) fn cancellation_requested(&self) -> Result<bool, UiFailure> {
        self.inner.render_health.check()?;
        Ok(self.inner.cancellation_requested.load(Ordering::Acquire))
    }

    /// Marks execution finished and waits for an interactive user to type `exit`.
    pub(crate) fn finish(&self) -> Result<(), UiFailure> {
        if self.inner.finished.swap(true, Ordering::AcqRel) {
            return self.inner.render_health.check();
        }
        if let Some(renderer) = lock(&self.inner.renderer).take()
            && renderer.join().is_err()
        {
            self.inner
                .render_health
                .fail("renderer thread panicked".to_owned());
        }
        self.inner.render_health.check()
    }
}

impl RuntimeObserver for UiSession {
    fn control(&self) -> crate::runtime::RunControl {
        self.inner.control.clone()
    }
    fn publish(&self, event: RuntimeEvent<'_>) -> Result<(), PresentationFailure> {
        UiSession::publish(self, event).map_err(|source| Box::new(source) as _)
    }

    fn cancellation_requested(&self) -> Result<bool, PresentationFailure> {
        UiSession::cancellation_requested(self).map_err(|source| Box::new(source) as _)
    }

    fn finish(&self) -> Result<(), PresentationFailure> {
        UiSession::finish(self).map_err(|source| Box::new(source) as _)
    }
}

fn render_loop(inner: &Arc<UiSessionInner>, ready: mpsc::SyncSender<Result<(), String>>) {
    let mut terminal = match DashboardTerminal::enter() {
        Ok(terminal) => {
            let _ = ready.send(Ok(()));
            terminal
        }
        Err(source) => {
            let _ = ready.send(Err(format!(
                "terminal dashboard initialization failed: {source}"
            )));
            return;
        }
    };
    let mut close_requested = false;
    loop {
        match terminal.poll_command() {
            Ok(Some(CommandSubmission::Parsed(UiCommand::Pause))) => {
                if !inner.finished.load(Ordering::Acquire) {
                    inner.control.pause(true);
                    lock(&inner.state)
                        .push_message("workflow: pause requested; execution timers frozen".into());
                }
                terminal.clear_command();
            }
            Ok(Some(CommandSubmission::Parsed(UiCommand::Resume))) => {
                if !inner.finished.load(Ordering::Acquire) {
                    inner.control.pause(false);
                    lock(&inner.state).push_message("workflow: resumed".into());
                }
                terminal.clear_command();
            }
            Ok(Some(CommandSubmission::Parsed(UiCommand::Exit))) => {
                close_requested = true;
                if !inner.finished.load(Ordering::Acquire) {
                    inner.control.cancel();
                    inner.cancellation_requested.store(true, Ordering::Release);
                    lock(&inner.state).request_exit();
                }
                terminal.clear_command();
            }
            Ok(Some(CommandSubmission::Parsed(UiCommand::ForceExit))) => {
                if !inner.finished.load(Ordering::Acquire) {
                    inner.control.cancel();
                    inner.cancellation_requested.store(true, Ordering::Release);
                    lock(&inner.state).request_exit();
                }
                drop(terminal);
                crate::runtime::force_exit();
            }
            Ok(Some(CommandSubmission::Parsed(UiCommand::Interrupt))) => {
                if inner.finished.load(Ordering::Acquire) {
                    lock(&inner.state).push_message(
                        "workflow: finished; type exit then Enter to close".to_owned(),
                    );
                } else {
                    inner.control.cancel();
                    inner.cancellation_requested.store(true, Ordering::Release);
                    lock(&inner.state).request_interrupt();
                }
                terminal.clear_command();
            }
            Ok(Some(CommandSubmission::Unknown(command))) => {
                lock(&inner.state).push_message(format!("unknown command: {command}"));
            }
            Ok(Some(CommandSubmission::Empty)) | Ok(None) => {}
            Err(source) => {
                inner
                    .render_health
                    .fail(format!("terminal input failed: {source}"));
                return;
            }
        }
        let snapshot = lock(&inner.state).snapshot();
        if let Err(source) = terminal.draw(&snapshot) {
            inner
                .render_health
                .fail(format!("terminal drawing failed: {source}"));
            return;
        }
        if renderer_should_close(inner.finished.load(Ordering::Acquire), close_requested) {
            break;
        }
        thread::sleep(inner.plan.refresh_interval());
    }
}

const fn renderer_should_close(execution_finished: bool, exit_submitted: bool) -> bool {
    execution_finished && exit_submitted
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::{RenderHealth, renderer_should_close};

    #[test]
    fn a_recorded_render_failure_is_returned_to_the_runtime_facing_boundary() {
        let health = RenderHealth::default();
        health.fail("terminal drawing failed".to_owned());
        let error = health.check().unwrap_err();
        assert_eq!(error.to_string(), "terminal drawing failed");
    }

    #[test]
    fn interactive_renderer_requires_both_completion_and_explicit_exit() {
        assert!(!renderer_should_close(false, false));
        assert!(!renderer_should_close(true, false));
        assert!(!renderer_should_close(false, true));
        assert!(renderer_should_close(true, true));
    }
}
