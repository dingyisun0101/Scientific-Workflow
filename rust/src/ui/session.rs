//! Automatic dashboard activation, command handling, and event publication.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
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
    plain_fallback: AtomicBool,
    plan: UiPlan,
    state: Mutex<DashboardState>,
    cancellation_requested: AtomicBool,
    finished: AtomicBool,
    renderer: Mutex<Option<JoinHandle<()>>>,
}

impl UiSession {
    /// Selects the Ratatui dashboard for a terminal and plain lines otherwise.
    pub(crate) fn automatic(plan: UiPlan) -> Self {
        let interactive = terminal::interactive();
        let inner = Arc::new(UiSessionInner {
            interactive,
            plain_fallback: AtomicBool::new(false),
            plan,
            state: Mutex::new(DashboardState::new()),
            cancellation_requested: AtomicBool::new(false),
            finished: AtomicBool::new(false),
            renderer: Mutex::new(None),
        });
        if interactive {
            let renderer_inner = Arc::clone(&inner);
            let renderer = thread::Builder::new()
                .name("scientific-workflow-ui".to_owned())
                .spawn(move || render_loop(&renderer_inner));
            match renderer {
                Ok(renderer) => {
                    *lock(&inner.renderer) = Some(renderer);
                }
                Err(source) => {
                    inner.plain_fallback.store(true, Ordering::Release);
                    lock(&inner.state).push_message(format!(
                        "workflow: failed to start terminal renderer: {source}"
                    ));
                }
            }
        }
        Self { inner }
    }

    /// Publishes one event without allowing presentation failure to fail science.
    pub(crate) fn publish(&self, event: UiEvent<'_>) {
        lock(&self.inner.state).apply(&event);
        if !self.inner.interactive || self.inner.plain_fallback.load(Ordering::Acquire) {
            terminal::render_plain(&event);
        }
    }

    pub(crate) fn task(&self, replicate: u64, identity: impl Into<Box<str>>) -> TaskUi {
        TaskUi {
            session: self.clone(),
            replicate,
            identity: identity.into(),
        }
    }

    pub(crate) fn cancellation_requested(&self) -> bool {
        self.inner.cancellation_requested.load(Ordering::Acquire)
    }

    /// Stops the renderer after Runtime has published the terminal outcome.
    pub(crate) fn finish(&self) {
        if self.inner.finished.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Some(renderer) = lock(&self.inner.renderer).take() {
            let _ = renderer.join();
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

fn render_loop(inner: &Arc<UiSessionInner>) {
    let mut terminal = match DashboardTerminal::enter() {
        Ok(terminal) => terminal,
        Err(source) => {
            inner.plain_fallback.store(true, Ordering::Release);
            lock(&inner.state).push_message(format!(
                "workflow: terminal dashboard unavailable: {source}"
            ));
            eprintln!("[workflow: terminal dashboard unavailable: {source}]");
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
                inner.cancellation_requested.store(true, Ordering::Release);
                lock(&inner.state).push_message(format!(
                    "workflow: terminal input failed; cancellation requested: {source}"
                ));
            }
        }
        let snapshot = lock(&inner.state).snapshot();
        if terminal.draw(&snapshot).is_err() {
            break;
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
