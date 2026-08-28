//! Automatic dashboard activation, command handling, and event publication.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};

use super::command::{CommandSubmission, UiCommand};
use super::event::UiEvent;
use super::plan::UiPlan;
use super::state::DashboardState;
use super::terminal::{self, DashboardTerminal};

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
    interactive: bool,
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

    fn assert_healthy(&self) {
        if let Some(reason) = lock(&self.failure).clone() {
            panic!("workflow UI failed: {reason}");
        }
    }
}

impl UiSession {
    /// Selects the Ratatui dashboard for a terminal and plain lines otherwise.
    pub(crate) fn automatic(plan: UiPlan) -> Self {
        let interactive = terminal::interactive();
        let inner = Arc::new(UiSessionInner {
            interactive,
            plan,
            state: Mutex::new(DashboardState::new()),
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
                .unwrap_or_else(|source| {
                    panic!("workflow UI failed to start its renderer thread: {source}")
                });
            match readiness.recv() {
                Ok(Ok(())) => *lock(&inner.renderer) = Some(renderer),
                Ok(Err(reason)) => {
                    let _ = renderer.join();
                    panic!("workflow UI failed: {reason}");
                }
                Err(_) => match renderer.join() {
                    Ok(()) => panic!("workflow UI renderer stopped before initialization"),
                    Err(payload) => std::panic::resume_unwind(payload),
                },
            }
        }
        Self { inner }
    }

    /// Publishes one event and enforces the selected UI as a healthy interface.
    pub(crate) fn publish(&self, event: UiEvent<'_>) {
        self.inner.render_health.assert_healthy();
        lock(&self.inner.state).apply(&event);
        if !self.inner.interactive
            && let Err(source) = terminal::render_plain(&event)
        {
            panic!("workflow UI failed to render plain output: {source}");
        }
        self.inner.render_health.assert_healthy();
    }

    pub(crate) fn task(&self, replicate: u64, identity: impl Into<Box<str>>) -> TaskUi {
        TaskUi {
            session: self.clone(),
            replicate,
            identity: identity.into(),
        }
    }

    pub(crate) fn cancellation_requested(&self) -> bool {
        self.inner.render_health.assert_healthy();
        self.inner.cancellation_requested.load(Ordering::Acquire)
    }

    /// Stops the renderer after Runtime has published the terminal outcome.
    pub(crate) fn finish(&self) {
        if self.inner.finished.swap(true, Ordering::AcqRel) {
            self.inner.render_health.assert_healthy();
            return;
        }
        if let Some(renderer) = lock(&self.inner.renderer).take()
            && let Err(payload) = renderer.join()
        {
            std::panic::resume_unwind(payload);
        }
        self.inner.render_health.assert_healthy();
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
    loop {
        match terminal.poll_command() {
            Ok(Some(CommandSubmission::Parsed(UiCommand::Exit))) => {
                inner.cancellation_requested.store(true, Ordering::Release);
                let mut state = lock(&inner.state);
                state.request_exit();
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
        if inner.finished.load(Ordering::Acquire) {
            break;
        }
        thread::sleep(inner.plan.refresh_interval());
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::RenderHealth;

    #[test]
    #[should_panic(expected = "workflow UI failed: terminal drawing failed")]
    fn a_recorded_render_failure_is_fatal_to_the_runtime_facing_boundary() {
        let health = RenderHealth::default();
        health.fail("terminal drawing failed".to_owned());
        health.assert_healthy();
    }
}
