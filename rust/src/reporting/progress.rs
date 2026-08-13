//! Thread-safe task progress state and the sole terminal renderer.
//!
//! Workers publish only atomics on the hot path. A single renderer thread polls
//! those slots at a bounded frequency and owns every human-facing terminal
//! write for the session. Progress never mutates or replaces scientific time;
//! callers synchronize it from their authoritative model state.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io::{self, IsTerminal, Write};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{
    Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use serde_json::Value;

use super::error::ReportingError;
use crate::configuration::{ProjectConfig, TaskConfig};
use crate::project::ScientificProject;

const REFRESH_INTERVAL: Duration = Duration::from_millis(100);
static TERMINAL_OWNED: AtomicBool = AtomicBool::new(false);

/// Lifecycle status of one independently executing scientific task.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TaskStatus {
    /// The task is registered but has not started.
    Pending,
    /// The task currently owns an active [`TaskProgress`] handle.
    Running,
    /// Evolution, persistence, and caller-defined validation completed.
    Completed,
    /// The task explicitly failed or dropped its active handle prematurely.
    Failed,
}

impl TaskStatus {
    fn encode(self) -> u8 {
        match self {
            Self::Pending => 0,
            Self::Running => 1,
            Self::Completed => 2,
            Self::Failed => 3,
        }
    }

    fn decode(value: u8) -> Self {
        match value {
            0 => Self::Pending,
            1 => Self::Running,
            2 => Self::Completed,
            3 => Self::Failed,
            _ => unreachable!("task status is written only through TaskStatus::encode"),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }
}

/// Exact parameter-derived identity of one task.
///
/// The identity contains only the caller-selected parameter fields. Its label
/// is deterministic compact JSON text intended for reporting, while equality
/// validation uses the retained JSON values themselves.
#[derive(Clone, Debug)]
pub struct TaskIdentity {
    fields: Arc<[(Box<str>, Value)]>,
    label: Arc<str>,
}

impl TaskIdentity {
    /// Returns the terminal label derived from exact parameter key/value pairs.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns the number of parameter fields forming this identity.
    pub fn len(&self) -> usize {
        self.fields.len()
    }

    /// Reports whether the identity contains no parameter fields.
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// Borrows one exact identity value by parameter name.
    pub fn value(&self, key: &str) -> Option<&Value> {
        self.fields
            .iter()
            .find_map(|(name, value)| (name.as_ref() == key).then_some(value))
    }

    /// Iterates identity fields in the configured display order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&str, &Value)> {
        self.fields
            .iter()
            .map(|(name, value)| (name.as_ref(), value))
    }

    fn matches(&self, task: &TaskConfig) -> bool {
        self.fields
            .iter()
            .all(|(key, value)| task.value(key) == Some(value))
    }
}

/// Immutable aggregate captured when centralized reporting ends.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProgressSummary {
    total: u64,
    pending: u64,
    running: u64,
    completed: u64,
    failed: u64,
}

impl ProgressSummary {
    /// Returns the number of registered tasks.
    pub fn total(&self) -> u64 {
        self.total
    }

    /// Returns the number of tasks that never started.
    pub fn pending(&self) -> u64 {
        self.pending
    }

    /// Returns the number of tasks still running at capture time.
    pub fn running(&self) -> u64 {
        self.running
    }

    /// Returns the number of successfully completed tasks.
    pub fn completed(&self) -> u64 {
        self.completed
    }

    /// Returns the number of failed or interrupted tasks.
    pub fn failed(&self) -> u64 {
        self.failed
    }

    /// Reports whether every registered task completed successfully.
    pub fn is_success(&self) -> bool {
        self.completed == self.total && self.pending == 0 && self.running == 0 && self.failed == 0
    }
}

/// Output policy selected before the renderer starts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutputMode {
    Auto,
    Terminal,
    Plain,
    Hidden,
}

/// Builder for one process-wide progress-reporting session.
pub struct ProgressReporterBuilder {
    configuration: ProjectConfig,
    identity_keys: Option<Vec<String>>,
    output: OutputMode,
}

/// Builder for an application-owned task registry spanning multiple projects.
pub struct RegisteredProgressReporterBuilder {
    labels: Vec<String>,
    output: OutputMode,
}

impl RegisteredProgressReporterBuilder {
    /// Forces cursor-controlled isolated-screen rendering.
    pub fn terminal(mut self) -> Self {
        self.output = OutputMode::Terminal;
        self
    }

    /// Forces stable line-oriented output.
    pub fn plain(mut self) -> Self {
        self.output = OutputMode::Plain;
        self
    }

    /// Suppresses rendering while retaining progress state.
    pub fn hidden(mut self) -> Self {
        self.output = OutputMode::Hidden;
        self
    }

    /// Starts the sole reporter for the complete registered task set.
    pub fn start(self) -> Result<ProgressReporter, ReportingError> {
        let slots = build_registered_slots(&self.labels)?;
        start_reporter(slots, Arc::from([]), self.output)
    }
}

impl ProgressReporterBuilder {
    /// Selects the exact parameter combination used to identify every task.
    ///
    /// Keys may refer to fixed or swept parameters, but the complete selected
    /// tuple must be unique for every generated task. Calling this method with
    /// no keys is valid only for a one-task project.
    pub fn identify_tasks_by<I, S>(mut self, keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.identity_keys = Some(keys.into_iter().map(Into::into).collect());
        self
    }

    /// Forces cursor-controlled terminal rendering even when stderr is not a
    /// terminal. Most applications should retain automatic output selection.
    pub fn terminal(mut self) -> Self {
        self.output = OutputMode::Terminal;
        self
    }

    /// Forces stable line-oriented output suitable for logs and CI systems.
    pub fn plain(mut self) -> Self {
        self.output = OutputMode::Plain;
        self
    }

    /// Suppresses rendering while retaining counters and lifecycle validation.
    ///
    /// This is useful for tests and embedding applications with their own
    /// non-terminal presentation layer. It does not disable progress tracking.
    pub fn hidden(mut self) -> Self {
        self.output = OutputMode::Hidden;
        self
    }

    /// Validates identities, acquires exclusive terminal ownership, and starts
    /// the centralized renderer.
    pub fn start(self) -> Result<ProgressReporter, ReportingError> {
        let identity_keys: Arc<[Box<str>]> =
            validate_identity_keys(&self.configuration, self.identity_keys)?
                .into_iter()
                .map(String::into_boxed_str)
                .collect();
        let slots = build_slots(&self.configuration, Arc::clone(&identity_keys))?;
        start_reporter(slots, identity_keys, self.output)
    }
}

impl fmt::Debug for ProgressReporterBuilder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProgressReporterBuilder")
            .field("tasks", &self.configuration.task_count())
            .field("identity_keys", &self.identity_keys)
            .field("output", &self.output)
            .finish_non_exhaustive()
    }
}

/// Central progress registry and exclusive human-facing terminal owner.
pub struct ProgressReporter {
    inner: Arc<ReporterInner>,
    renderer: Option<JoinHandle<()>>,
    finished: bool,
}

impl ProgressReporter {
    /// Registers one ordered application task set independent of any one
    /// Workflow project. This is the embedding boundary for study runners.
    pub fn for_registered_tasks<I, S>(labels: I) -> RegisteredProgressReporterBuilder
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        RegisteredProgressReporterBuilder {
            labels: labels.into_iter().map(Into::into).collect(),
            output: OutputMode::Auto,
        }
    }

    /// Creates a builder from a conventional four-file scientific project.
    ///
    /// By default all sweep keys, in declaration order, form task identity.
    pub fn for_project(project: &ScientificProject) -> ProgressReporterBuilder {
        Self::for_configuration(project.configuration())
    }

    /// Creates a builder from the lower-level three-file project configuration.
    pub fn for_configuration(configuration: &ProjectConfig) -> ProgressReporterBuilder {
        ProgressReporterBuilder {
            configuration: configuration.clone(),
            identity_keys: None,
            output: OutputMode::Auto,
        }
    }

    /// Starts tracking one task using identity and ordering from `TaskConfig`.
    ///
    /// `initial_iteration` and `target_iteration` are absolute scientific
    /// coordinates. A missing target creates an indeterminate progress display.
    /// Callers never supply an ordinal or terminal label.
    pub fn start_task(
        &self,
        task: &TaskConfig,
        initial_iteration: u64,
        target_iteration: Option<u64>,
    ) -> Result<TaskProgress, ReportingError> {
        let ordinal = task.task_ordinal();
        let index = usize::try_from(ordinal)
            .ok()
            .filter(|index| *index < self.inner.slots.len())
            .ok_or(ReportingError::UnknownTaskOrdinal {
                task_ordinal: ordinal,
            })?;
        let slot = Arc::clone(&self.inner.slots[index]);
        if !slot.identity.matches(task) {
            return Err(ReportingError::TaskIdentityMismatch {
                task_ordinal: ordinal,
            });
        }
        start_slot(&self.inner, slot, initial_iteration, target_iteration)
    }

    /// Starts one application-registered task by exact stable label.
    pub fn start_registered_task(
        &self,
        label: &str,
        initial_iteration: u64,
        target_iteration: Option<u64>,
    ) -> Result<TaskProgress, ReportingError> {
        let slot = registered_slot(&self.inner.slots, label)?;
        start_slot(&self.inner, slot, initial_iteration, target_iteration)
    }

    /// Marks a registered task as already complete through verified reuse.
    pub fn mark_registered_reused(&self, label: &str) -> Result<(), ReportingError> {
        let slot = registered_slot(&self.inner.slots, label)?;
        slot.status
            .compare_exchange(
                TaskStatus::Pending.encode(),
                TaskStatus::Completed.encode(),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .map_err(|_| ReportingError::TaskAlreadyStarted {
                identity: label.to_owned(),
            })?;
        *lock(&slot.phase) = "reused".into();
        Ok(())
    }

    /// Returns the cancellation token set by Ctrl-C in interactive mode.
    pub fn cancellation_token(&self) -> CancellationToken {
        CancellationToken(Arc::clone(&self.inner.cancelled))
    }

    /// Sends one application-wide message through the sole renderer.
    pub fn report(&self, message: impl Into<String>) -> Result<(), ReportingError> {
        self.inner
            .events
            .send(RenderEvent::Message(message.into()))
            .map_err(|_| ReportingError::RendererUnavailable)
    }

    /// Returns a non-blocking snapshot of all task lifecycle counts.
    pub fn summary(&self) -> ProgressSummary {
        summarize(&self.inner.slots)
    }

    /// Finishes a successful session and emits its final summary and message.
    ///
    /// Every task must already be completed. The method stops and joins the
    /// renderer and releases exclusive terminal ownership.
    pub fn complete(
        mut self,
        message: impl Into<String>,
    ) -> Result<ProgressSummary, ReportingError> {
        let summary = self.summary();
        if !summary.is_success() {
            self.stop(false, "workflow did not complete".to_owned())?;
            return Err(ReportingError::IncompleteProgress {
                pending: summary.pending,
                running: summary.running,
                failed: summary.failed,
            });
        }
        self.stop(true, message.into())?;
        Ok(summary)
    }

    /// Finishes an unsuccessful session while preserving all task statuses.
    pub fn fail(mut self, message: impl Into<String>) -> Result<ProgressSummary, ReportingError> {
        let summary = self.summary();
        self.stop(false, message.into())?;
        Ok(summary)
    }

    /// Emits one terminal error through the reporting subsystem.
    ///
    /// This helper is intended for failures occurring before a reporting
    /// session starts or after it has released the terminal lease.
    pub fn report_error(message: impl fmt::Display) {
        eprintln!("[error] {message}");
    }

    fn stop(&mut self, success: bool, message: String) -> Result<(), ReportingError> {
        self.inner
            .events
            .send(RenderEvent::Stop { success, message })
            .map_err(|_| ReportingError::RendererUnavailable)?;
        self.finished = true;
        if self
            .renderer
            .take()
            .expect("an unfinished reporter owns one renderer")
            .join()
            .is_err()
        {
            return Err(ReportingError::RendererPanicked);
        }
        Ok(())
    }
}

impl fmt::Debug for ProgressReporter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProgressReporter")
            .field("tasks", &self.inner.slots.len())
            .field("identity_keys", &self.inner.identity_keys)
            .field("summary", &self.summary())
            .finish_non_exhaustive()
    }
}

impl Drop for ProgressReporter {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let _ = self.inner.events.send(RenderEvent::Stop {
            success: false,
            message: "progress reporter dropped before completion".to_owned(),
        });
        if let Some(renderer) = self.renderer.take() {
            let _ = renderer.join();
        }
    }
}

/// Non-clone task-local progress handle.
///
/// Dropping a running handle without calling [`TaskProgress::complete`] or
/// [`TaskProgress::fail`] marks the task failed, making ordinary `?` returns
/// safe without a separate cleanup branch.
pub struct TaskProgress {
    slot: Arc<ProgressSlot>,
    events: Sender<RenderEvent>,
    cancelled: Arc<AtomicBool>,
    active: bool,
}

/// Shared, cheap cancellation observation for embedded schedulers and tasks.
#[derive(Clone, Debug)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    /// Reports whether interactive Ctrl-C requested termination.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    /// Requests cooperative termination.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
}

impl TaskProgress {
    /// Reports whether the owning reporter requested cooperative termination.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    /// Borrows the exact parameter-derived task identity.
    pub fn identity(&self) -> &TaskIdentity {
        &self.slot.identity
    }

    /// Returns the latest reported absolute simulation iteration.
    pub fn current_iteration(&self) -> u64 {
        self.slot.current.load(Ordering::Relaxed)
    }

    /// Returns the known absolute target iteration, if one exists.
    pub fn target_iteration(&self) -> Option<u64> {
        if self.slot.target_known.load(Ordering::Acquire) {
            Some(self.slot.target.load(Ordering::Relaxed))
        } else {
            None
        }
    }

    /// Returns this task's current lifecycle status.
    pub fn status(&self) -> TaskStatus {
        TaskStatus::decode(self.slot.status.load(Ordering::Acquire))
    }

    /// Synchronizes progress to an authoritative absolute simulation iteration.
    ///
    /// The atomic update never allocates or locks. Regressions and movement
    /// beyond a known target are rejected without modifying the counter.
    pub fn set_iteration(&self, iteration: u64) -> Result<(), ReportingError> {
        if let Some(target) = self.target_iteration().filter(|target| iteration > *target) {
            return Err(ReportingError::IterationBeyondTarget {
                identity: self.identity().label().to_owned(),
                iteration,
                target,
            });
        }
        let previous = self.slot.current.fetch_max(iteration, Ordering::Relaxed);
        if iteration < previous {
            return Err(ReportingError::IterationRegressed {
                identity: self.identity().label().to_owned(),
                current: previous,
                attempted: iteration,
            });
        }
        Ok(())
    }

    /// Synchronizes the authoritative iteration and applies the configured
    /// continuation target.
    ///
    /// An indeterminate task returns `true`. A task with a target returns
    /// `true` below it and `false` exactly at it. Movement beyond the target or
    /// backwards is rejected through the same validation as [`Self::set_iteration`].
    pub fn should_continue(&self, iteration: u64) -> Result<bool, ReportingError> {
        self.set_iteration(iteration)?;
        Ok(!self.is_cancelled()
            && self
                .target_iteration()
                .is_none_or(|target| iteration < target))
    }

    /// Updates one infrequent human-readable phase such as `evolving` or
    /// `validating`. Phase updates may lock; iteration updates do not.
    pub fn set_phase(&self, phase: impl Into<String>) {
        *lock(&self.slot.phase) = phase.into().into_boxed_str();
    }

    /// Sends one task-scoped message through the sole renderer.
    pub fn report(&self, message: impl Into<String>) -> Result<(), ReportingError> {
        self.events
            .send(RenderEvent::TaskMessage {
                identity: self.identity().label().to_owned(),
                message: message.into(),
            })
            .map_err(|_| ReportingError::RendererUnavailable)
    }

    /// Marks the complete task workflow successful and consumes this handle.
    ///
    /// `reason == None` means the configured target must have been reached.
    /// `Some(reason)` records an intentional scientific early-completion reason
    /// and permits completion before that generic target.
    pub fn complete(mut self, reason: Option<String>) -> Result<(), ReportingError> {
        if reason.is_none()
            && let Some(target) = self.target_iteration()
        {
            let current = self.current_iteration();
            if current != target {
                *lock(&self.slot.phase) = "target not reached".into();
                self.slot
                    .status
                    .store(TaskStatus::Failed.encode(), Ordering::Release);
                self.active = false;
                return Err(ReportingError::TargetIterationNotReached {
                    identity: self.identity().label().to_owned(),
                    current,
                    target,
                });
            }
        }
        *lock(&self.slot.phase) = reason
            .unwrap_or_else(|| "completed".to_owned())
            .into_boxed_str();
        self.slot
            .status
            .store(TaskStatus::Completed.encode(), Ordering::Release);
        self.active = false;
        Ok(())
    }

    /// Marks the task failed, records a concise phase, and consumes the handle.
    pub fn fail(mut self, reason: impl Into<String>) {
        *lock(&self.slot.phase) = reason.into().into_boxed_str();
        self.slot
            .status
            .store(TaskStatus::Failed.encode(), Ordering::Release);
        self.active = false;
    }
}

impl fmt::Debug for TaskProgress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TaskProgress")
            .field("identity", &self.identity().label())
            .field("current_iteration", &self.current_iteration())
            .field("target_iteration", &self.target_iteration())
            .field("active", &self.active)
            .finish_non_exhaustive()
    }
}

impl Drop for TaskProgress {
    fn drop(&mut self) {
        if self.active {
            *lock(&self.slot.phase) = "interrupted".into();
            self.slot
                .status
                .store(TaskStatus::Failed.encode(), Ordering::Release);
        }
    }
}

struct ReporterInner {
    slots: Arc<[Arc<ProgressSlot>]>,
    identity_keys: Arc<[Box<str>]>,
    events: Sender<RenderEvent>,
    cancelled: Arc<AtomicBool>,
}

struct ProgressSlot {
    identity: TaskIdentity,
    current: AtomicU64,
    target: AtomicU64,
    target_known: AtomicBool,
    status: AtomicU8,
    phase: Mutex<Box<str>>,
}

enum RenderEvent {
    Message(String),
    TaskMessage { identity: String, message: String },
    Stop { success: bool, message: String },
}

struct TerminalLease;

impl Drop for TerminalLease {
    fn drop(&mut self) {
        TERMINAL_OWNED.store(false, Ordering::Release);
    }
}

fn acquire_terminal() -> Result<(), ReportingError> {
    TERMINAL_OWNED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map(|_| ())
        .map_err(|_| ReportingError::TerminalAlreadyOwned)
}

fn resolve_output(output: OutputMode) -> OutputMode {
    match output {
        OutputMode::Auto if io::stderr().is_terminal() && io::stdin().is_terminal() => {
            OutputMode::Terminal
        }
        OutputMode::Auto => OutputMode::Plain,
        explicit => explicit,
    }
}

fn start_reporter(
    slots: Arc<[Arc<ProgressSlot>]>,
    identity_keys: Arc<[Box<str>]>,
    requested_output: OutputMode,
) -> Result<ProgressReporter, ReportingError> {
    acquire_terminal()?;
    let lease = TerminalLease;
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut output = resolve_output(requested_output);
    let terminal = if output == OutputMode::Terminal {
        match TerminalSession::enter(Arc::clone(&cancelled)) {
            Ok(terminal) => Some(terminal),
            Err(_) if requested_output == OutputMode::Auto => {
                output = OutputMode::Plain;
                None
            }
            Err(error) => return Err(error),
        }
    } else {
        None
    };
    let (events, receiver) = mpsc::channel();
    let renderer_slots = Arc::clone(&slots);
    let renderer = match thread::Builder::new()
        .name("scientific-workflow-progress".to_owned())
        .spawn(move || render(receiver, renderer_slots, output, terminal, lease))
    {
        Ok(renderer) => renderer,
        Err(source) => return Err(ReportingError::StartRenderer { source }),
    };
    Ok(ProgressReporter {
        inner: Arc::new(ReporterInner {
            slots,
            identity_keys,
            events,
            cancelled,
        }),
        renderer: Some(renderer),
        finished: false,
    })
}

fn registered_slot(
    slots: &[Arc<ProgressSlot>],
    label: &str,
) -> Result<Arc<ProgressSlot>, ReportingError> {
    slots
        .iter()
        .find(|slot| slot.identity.label() == label)
        .cloned()
        .ok_or_else(|| ReportingError::UnknownRegisteredTask {
            identity: label.to_owned(),
        })
}

fn start_slot(
    inner: &ReporterInner,
    slot: Arc<ProgressSlot>,
    initial_iteration: u64,
    target_iteration: Option<u64>,
) -> Result<TaskProgress, ReportingError> {
    if let Some(target) = target_iteration.filter(|target| initial_iteration > *target) {
        return Err(ReportingError::InitialIterationBeyondTarget {
            identity: slot.identity.label().to_owned(),
            initial: initial_iteration,
            target,
        });
    }
    slot.status
        .compare_exchange(
            TaskStatus::Pending.encode(),
            TaskStatus::Running.encode(),
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .map_err(|_| ReportingError::TaskAlreadyStarted {
            identity: slot.identity.label().to_owned(),
        })?;
    slot.current.store(initial_iteration, Ordering::Relaxed);
    if let Some(target) = target_iteration {
        slot.target.store(target, Ordering::Relaxed);
        slot.target_known.store(true, Ordering::Release);
    } else {
        slot.target_known.store(false, Ordering::Release);
    }
    *lock(&slot.phase) = "running".into();
    Ok(TaskProgress {
        slot,
        events: inner.events.clone(),
        cancelled: Arc::clone(&inner.cancelled),
        active: true,
    })
}

fn validate_identity_keys(
    configuration: &ProjectConfig,
    requested: Option<Vec<String>>,
) -> Result<Vec<String>, ReportingError> {
    let keys = requested.unwrap_or_else(|| {
        configuration
            .parameters()
            .sweep_keys()
            .map(str::to_owned)
            .collect()
    });
    let mut seen = HashSet::with_capacity(keys.len());
    for key in &keys {
        if !seen.insert(key.as_str()) {
            return Err(ReportingError::DuplicateIdentityParameter { key: key.clone() });
        }
        if !configuration.parameters().contains_parameter(key) {
            return Err(ReportingError::UnknownIdentityParameter { key: key.clone() });
        }
    }
    Ok(keys)
}

fn build_slots(
    configuration: &ProjectConfig,
    keys: Arc<[Box<str>]>,
) -> Result<Arc<[Arc<ProgressSlot>]>, ReportingError> {
    let capacity = usize::try_from(configuration.task_count()).map_err(|_| {
        ReportingError::TaskCountTooLarge {
            task_count: configuration.task_count(),
        }
    })?;
    let mut slots = Vec::with_capacity(capacity);
    let mut identities = HashMap::<String, u64>::with_capacity(capacity);
    for task in configuration.task_configs() {
        let label = render_identity(&task, &keys);
        if let Some(first_ordinal) = identities.insert(label.clone(), task.task_ordinal()) {
            return Err(ReportingError::NonUniqueTaskIdentity {
                identity: label,
                first_ordinal,
                second_ordinal: task.task_ordinal(),
            });
        }
        slots.push(Arc::new(ProgressSlot {
            identity: TaskIdentity {
                fields: keys
                    .iter()
                    .map(|key| {
                        (
                            key.clone(),
                            task.value(key)
                                .expect("validated identity keys resolve for every task")
                                .clone(),
                        )
                    })
                    .collect::<Vec<_>>()
                    .into(),
                label: label.into(),
            },
            current: AtomicU64::new(0),
            target: AtomicU64::new(0),
            target_known: AtomicBool::new(false),
            status: AtomicU8::new(TaskStatus::Pending.encode()),
            phase: Mutex::new("pending".into()),
        }));
    }
    Ok(slots.into())
}

fn build_registered_slots(labels: &[String]) -> Result<Arc<[Arc<ProgressSlot>]>, ReportingError> {
    let mut seen = HashSet::with_capacity(labels.len());
    let mut slots = Vec::with_capacity(labels.len());
    for label in labels {
        if !seen.insert(label.as_str()) {
            return Err(ReportingError::DuplicateRegisteredTask {
                identity: label.clone(),
            });
        }
        slots.push(Arc::new(ProgressSlot {
            identity: TaskIdentity {
                fields: Arc::from([]),
                label: label.clone().into(),
            },
            current: AtomicU64::new(0),
            target: AtomicU64::new(0),
            target_known: AtomicBool::new(false),
            status: AtomicU8::new(TaskStatus::Pending.encode()),
            phase: Mutex::new("pending".into()),
        }));
    }
    Ok(slots.into())
}

fn render_identity(task: &TaskConfig, keys: &[Box<str>]) -> String {
    if keys.is_empty() {
        return "task".to_owned();
    }
    keys.iter()
        .map(|key| {
            let value = task
                .value(key)
                .expect("validated identity keys resolve for every task");
            let value = serde_json::to_string(value)
                .expect("serde_json::Value always serializes to valid JSON");
            format!("{key}={value}")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn summarize(slots: &[Arc<ProgressSlot>]) -> ProgressSummary {
    let mut summary = ProgressSummary {
        total: u64::try_from(slots.len()).expect("slot count originated from a u64 task count"),
        pending: 0,
        running: 0,
        completed: 0,
        failed: 0,
    };
    for slot in slots {
        match TaskStatus::decode(slot.status.load(Ordering::Acquire)) {
            TaskStatus::Pending => summary.pending += 1,
            TaskStatus::Running => summary.running += 1,
            TaskStatus::Completed => summary.completed += 1,
            TaskStatus::Failed => summary.failed += 1,
        }
    }
    summary
}

struct TerminalSession {
    stop: Arc<AtomicBool>,
    input: Option<JoinHandle<()>>,
}

impl TerminalSession {
    fn enter(cancelled: Arc<AtomicBool>) -> Result<Self, ReportingError> {
        enable_raw_mode().map_err(|source| ReportingError::TerminalSetup {
            operation: "enable raw input mode",
            source,
        })?;
        let mut stderr = io::stderr();
        if let Err(source) = execute!(
            stderr,
            EnterAlternateScreen,
            Clear(ClearType::All),
            MoveTo(0, 0),
            Hide,
            EnableMouseCapture
        ) {
            let _ = execute!(stderr, DisableMouseCapture, Show, LeaveAlternateScreen);
            let _ = disable_raw_mode();
            return Err(ReportingError::TerminalSetup {
                operation: "enter the isolated terminal screen",
                source,
            });
        }
        let stop = Arc::new(AtomicBool::new(false));
        let input_stop = Arc::clone(&stop);
        let input = match thread::Builder::new()
            .name("scientific-workflow-input".to_owned())
            .spawn(move || drain_terminal_input(input_stop, cancelled))
        {
            Ok(input) => input,
            Err(source) => {
                let _ = execute!(stderr, DisableMouseCapture, Show, LeaveAlternateScreen);
                let _ = disable_raw_mode();
                return Err(ReportingError::TerminalSetup {
                    operation: "start the isolated-screen input drain",
                    source,
                });
            }
        };
        Ok(Self {
            stop,
            input: Some(input),
        })
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(input) = self.input.take() {
            let _ = input.join();
        }
        let mut stderr = io::stderr();
        let _ = execute!(stderr, DisableMouseCapture, Show, LeaveAlternateScreen);
        let _ = stderr.flush();
        let _ = disable_raw_mode();
    }
}

fn drain_terminal_input(stop: Arc<AtomicBool>, cancelled: Arc<AtomicBool>) {
    while !stop.load(Ordering::Acquire) {
        match event::poll(Duration::from_millis(50)) {
            Ok(true) => match event::read() {
                Ok(Event::Key(key))
                    if key.kind == KeyEventKind::Press
                        && key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    cancelled.store(true, Ordering::Release);
                }
                Ok(_) => {}
                Err(_) => {
                    cancelled.store(true, Ordering::Release);
                    break;
                }
            },
            Ok(false) => {}
            Err(_) => {
                cancelled.store(true, Ordering::Release);
                break;
            }
        }
    }
}

fn render(
    receiver: Receiver<RenderEvent>,
    slots: Arc<[Arc<ProgressSlot>]>,
    output: OutputMode,
    mut terminal_session: Option<TerminalSession>,
    _lease: TerminalLease,
) {
    let mut terminal = (output == OutputMode::Terminal).then(|| TerminalDisplay::new(&slots));
    let mut last_statuses = vec![TaskStatus::Pending; slots.len()];
    loop {
        if let Some(display) = &mut terminal {
            display.refresh(&slots);
        }
        match receiver.recv_timeout(REFRESH_INTERVAL) {
            Ok(RenderEvent::Message(message)) => write_message(output, terminal.as_ref(), &message),
            Ok(RenderEvent::TaskMessage { identity, message }) => {
                write_message(output, terminal.as_ref(), &format!("{identity}: {message}"));
            }
            Ok(RenderEvent::Stop { success, message }) => {
                if let Some(display) = &mut terminal {
                    display.refresh(&slots);
                    display.finish(&slots);
                }
                if output == OutputMode::Plain {
                    write_plain_transitions(&slots, &mut last_statuses);
                }
                drop(terminal.take());
                drop(terminal_session.take());
                write_final(output, &slots, success, &message);
                break;
            }
            Err(RecvTimeoutError::Timeout) => {
                if output == OutputMode::Plain {
                    write_plain_transitions(&slots, &mut last_statuses);
                }
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
}

struct TerminalDisplay {
    multi: MultiProgress,
    bars: Vec<ProgressBar>,
    statuses: Vec<TaskStatus>,
    known_style: ProgressStyle,
    unknown_style: ProgressStyle,
}

impl TerminalDisplay {
    fn new(slots: &[Arc<ProgressSlot>]) -> Self {
        let multi = MultiProgress::with_draw_target(ProgressDrawTarget::stderr());
        let known_style = ProgressStyle::with_template(
            "{prefix:.bold} [{msg}] {wide_bar:.cyan/blue} {pos}/{len} elapsed {elapsed_precise} ETA {eta_precise}",
        )
        .expect("hard-coded progress template is valid");
        let unknown_style = ProgressStyle::with_template(
            "{prefix:.bold} [{msg}] {spinner:.cyan} iteration {pos} elapsed {elapsed_precise} ETA unknown",
        )
        .expect("hard-coded spinner template is valid");
        let bars: Vec<_> = slots
            .iter()
            .map(|slot| {
                let bar = multi.add(ProgressBar::new_spinner());
                bar.set_prefix(slot.identity.label().to_owned());
                bar.set_style(unknown_style.clone());
                bar.set_message("pending");
                bar
            })
            .collect();

        // MultiProgress throttles draws globally. Force each bar's initial
        // state once so pending tasks are materialized instead of being
        // starved by earlier rows updated in the same renderer pass.
        for bar in &bars {
            bar.force_draw();
        }

        Self {
            multi,
            bars,
            statuses: vec![TaskStatus::Pending; slots.len()],
            known_style,
            unknown_style,
        }
    }

    fn refresh(&mut self, slots: &[Arc<ProgressSlot>]) {
        for ((bar, previous_status), slot) in self.bars.iter().zip(&mut self.statuses).zip(slots) {
            if !slot.target_known.load(Ordering::Acquire) {
                bar.set_style(self.unknown_style.clone());
            } else {
                let target = slot.target.load(Ordering::Relaxed);
                bar.set_style(self.known_style.clone());
                bar.set_length(target);
            }
            bar.set_position(slot.current.load(Ordering::Relaxed));
            let status = TaskStatus::decode(slot.status.load(Ordering::Acquire));
            if *previous_status == TaskStatus::Pending && status == TaskStatus::Running {
                // Pending time is queueing time, not task execution time. Start
                // elapsed and ETA measurement from the first running state.
                bar.reset_elapsed();
            }
            let phase = lock(&slot.phase);
            if phase.is_empty() || phase.as_ref() == status.label() {
                bar.set_message(status.label());
            } else {
                bar.set_message(format!("{}: {}", status.label(), phase.as_ref()));
            }
            bar.tick();
            *previous_status = status;
        }
    }

    fn finish(&self, slots: &[Arc<ProgressSlot>]) {
        for (bar, slot) in self.bars.iter().zip(slots) {
            let status = TaskStatus::decode(slot.status.load(Ordering::Acquire));
            bar.finish_with_message(status.label());
        }
        let _ = self.multi.clear();
    }
}

fn write_message(output: OutputMode, terminal: Option<&TerminalDisplay>, message: &str) {
    match output {
        OutputMode::Terminal => {
            if let Some(display) = terminal {
                let _ = display.multi.println(message);
            }
        }
        OutputMode::Plain => eprintln!("[progress] {message}"),
        OutputMode::Hidden | OutputMode::Auto => {}
    }
}

fn write_plain_transitions(slots: &[Arc<ProgressSlot>], previous: &mut [TaskStatus]) {
    for (slot, old) in slots.iter().zip(previous) {
        let status = TaskStatus::decode(slot.status.load(Ordering::Acquire));
        if status != *old {
            let phase = lock(&slot.phase);
            eprintln!(
                "[task] identity={} status={} phase={} iteration={} target={}",
                slot.identity.label(),
                status.label(),
                phase.as_ref(),
                slot.current.load(Ordering::Relaxed),
                format_target(slot)
            );
            *old = status;
        }
    }
}

fn write_final(output: OutputMode, slots: &[Arc<ProgressSlot>], success: bool, message: &str) {
    if output == OutputMode::Hidden {
        return;
    }
    let summary = summarize(slots);
    eprintln!(
        "[workflow] status={} tasks={} completed={} failed={} pending={} message={}",
        if success { "completed" } else { "failed" },
        summary.total,
        summary.completed,
        summary.failed,
        summary.pending,
        message
    );
}

fn format_target(slot: &ProgressSlot) -> String {
    if slot.target_known.load(Ordering::Acquire) {
        slot.target.load(Ordering::Relaxed).to_string()
    } else {
        "unknown".to_owned()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
