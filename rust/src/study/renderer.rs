//! Private thread-safe task progress state and terminal renderer.
//!
//! Workers publish only atomics on the hot path. A single renderer thread polls
//! those slots at a bounded frequency and owns every human-facing terminal
//! write for the session. Progress never mutates or replaces scientific time;
//! callers synchronize it from their authoritative model state.

use std::collections::HashSet;
use std::fmt;
use std::io::{self, IsTerminal};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde_json::Value;

use super::command::StudyCommand;
use super::error::StudyError;
use super::phase::{Phase, TaskKey, TaskMode};
use super::tui::{RenderSnapshot, TaskView, TerminalUi};

const REFRESH_INTERVAL: Duration = Duration::from_millis(100);
const MESSAGE_CAPACITY: usize = 256;
static TERMINAL_OWNED: AtomicBool = AtomicBool::new(false);

/// Lifecycle status of one independently executing scientific task.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TaskStatus {
    /// The task is registered but has not started.
    Pending,
    /// The task currently owns an active progress-reporting handle.
    Running,
    /// Evolution, persistence, and caller-defined validation completed.
    Completed,
    /// The task explicitly failed or dropped its active handle prematurely.
    Failed,
    /// The task cooperatively stopped after cancellation was requested.
    Cancelled,
    /// The task was never started because its phase stopped admitting work.
    Skipped,
}

impl TaskStatus {
    /// Returns the stable uncolored lifecycle label used by logs and APIs.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Skipped => "skipped",
        }
    }

    fn encode(self) -> u8 {
        match self {
            Self::Pending => 0,
            Self::Running => 1,
            Self::Completed => 2,
            Self::Failed => 3,
            Self::Cancelled => 4,
            Self::Skipped => 5,
        }
    }

    fn decode(value: u8) -> Self {
        match value {
            0 => Self::Pending,
            1 => Self::Running,
            2 => Self::Completed,
            3 => Self::Failed,
            4 => Self::Cancelled,
            5 => Self::Skipped,
            _ => unreachable!("task status is written only through TaskStatus::encode"),
        }
    }

    fn label(self) -> &'static str {
        self.as_str()
    }
}

/// Exact reporting identity and immutable metadata of one task.
///
/// The label and phase-qualified key identify the task in progress output.
/// Metadata contains the complete application-defined task metadata map,
/// including configuration provenance when attached by the caller.
#[derive(Clone, Debug)]
pub struct TaskIdentity {
    label: Arc<str>,
    key: TaskKey,
    metadata: Arc<std::collections::BTreeMap<String, Value>>,
}

impl TaskIdentity {
    /// Returns the declared display label for this task.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns the number of task metadata entries.
    pub fn len(&self) -> usize {
        self.metadata.len()
    }

    /// Reports whether the task contains no metadata.
    pub fn is_empty(&self) -> bool {
        self.metadata.is_empty()
    }

    /// Borrows one exact task metadata value by key.
    pub fn value(&self, key: &str) -> Option<&Value> {
        self.metadata.get(key)
    }

    /// Iterates task metadata in deterministic key order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&str, &Value)> {
        self.metadata
            .iter()
            .map(|(key, value)| (key.as_str(), value))
    }

    /// Returns the exact first-class task key when this identity came from a
    /// phase declaration.
    pub fn task_key(&self) -> &TaskKey {
        &self.key
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
    cancelled: u64,
    skipped: u64,
}

impl ProgressSummary {
    /// Builds the aggregate used when whole-phase examination certified every
    /// declared task without entering the renderer or scheduler.
    pub(crate) const fn fully_completed(total: u64) -> Self {
        Self {
            total,
            pending: 0,
            running: 0,
            completed: total,
            failed: 0,
            cancelled: 0,
            skipped: 0,
        }
    }

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

    /// Returns the number of active tasks that cooperatively cancelled.
    pub fn cancelled(&self) -> u64 {
        self.cancelled
    }

    /// Returns the number of tasks that were never started.
    pub fn skipped(&self) -> u64 {
        self.skipped
    }

    /// Reports whether every registered task completed successfully.
    pub fn is_success(&self) -> bool {
        self.completed == self.total
            && self.pending == 0
            && self.running == 0
            && self.failed == 0
            && self.cancelled == 0
            && self.skipped == 0
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

/// Builder that makes the renderer observe existing first-class phases/tasks.
pub(crate) struct StudyRendererBuilder {
    slots: Arc<[Arc<ProgressSlot>]>,
    output: OutputMode,
    cancellation: Option<CancellationToken>,
}

impl StudyRendererBuilder {
    pub(crate) fn cancellation_token(mut self, cancellation: CancellationToken) -> Self {
        self.cancellation = Some(cancellation);
        self
    }
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

    /// Suppresses rendering while retaining lifecycle validation.
    pub fn hidden(mut self) -> Self {
        self.output = OutputMode::Hidden;
        self
    }

    /// Validates phase uniqueness and starts one renderer.
    pub fn start(self) -> Result<StudyRenderer, StudyError> {
        start_renderer(self.slots, self.output, self.cancellation)
    }
}

impl fmt::Debug for StudyRendererBuilder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StudyRendererBuilder")
            .field("tasks", &self.slots.len())
            .field("output", &self.output)
            .finish_non_exhaustive()
    }
}

/// Central progress registry and exclusive human-facing terminal owner.
pub(crate) struct StudyRenderer {
    inner: Arc<RendererInner>,
    renderer: Option<JoinHandle<()>>,
    finished: bool,
}

impl StudyRenderer {
    /// Observes tasks already owned and identified by first-class phases.
    pub fn for_phase(phase: &Phase, heading: &str) -> Result<StudyRendererBuilder, StudyError> {
        Ok(StudyRendererBuilder {
            slots: build_phase_slots(std::slice::from_ref(phase), Some(heading))?,
            output: OutputMode::Auto,
            cancellation: None,
        })
    }

    /// Starts one exact first-class iterative task.
    pub(crate) fn start_progress(
        &self,
        key: &TaskKey,
        initial_iteration: u64,
        target_iteration: Option<u64>,
    ) -> Result<TaskProgressHandle, StudyError> {
        let slot = managed_slot(&self.inner.slots, key)?;
        if slot.mode != TaskMode::Progress {
            return Err(mode_mismatch(&slot, "progress"));
        }
        start_slot(&self.inner, slot, initial_iteration, target_iteration)
    }

    /// Starts one exact first-class lifecycle-only one-shot task.
    pub(crate) fn start_one_shot(&self, key: &TaskKey) -> Result<OneShotTaskHandle, StudyError> {
        let slot = managed_slot(&self.inner.slots, key)?;
        if slot.mode != TaskMode::OneShot {
            return Err(mode_mismatch(&slot, "one-shot"));
        }
        start_one_shot_slot(&self.inner, slot)
    }

    /// Marks one exact first-class task complete through verified reuse.
    pub fn mark_completed(&self, key: &TaskKey) -> Result<(), StudyError> {
        let slot = managed_slot(&self.inner.slots, key)?;
        mark_slot_completed(&slot)
    }

    pub(crate) fn mark_skipped(&self, key: &TaskKey) -> Result<(), StudyError> {
        let slot = managed_slot(&self.inner.slots, key)?;
        mark_pending_terminal(&slot, TaskStatus::Skipped, "skipped")
    }

    pub(crate) fn mark_cancelled(&self, key: &TaskKey) -> Result<(), StudyError> {
        let slot = managed_slot(&self.inner.slots, key)?;
        mark_pending_terminal(&slot, TaskStatus::Cancelled, "cancelled")
    }

    pub(crate) fn mark_delayed(&self, key: &TaskKey, rank: usize) -> Result<(), StudyError> {
        let slot = managed_slot(&self.inner.slots, key)?;
        if TaskStatus::decode(slot.status.load(Ordering::Acquire)) != TaskStatus::Pending {
            return Err(StudyError::TaskAlreadyStarted {
                identity: slot.identity.label().to_owned(),
            });
        }
        *lock(&slot.detail) = format!("delayed start (rank {rank})").into_boxed_str();
        Ok(())
    }

    pub(crate) fn request_cancellation(&self) {
        self.inner.cancelled.store(true, Ordering::Release);
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }

    pub(crate) fn cancellation_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.inner.cancelled)
    }

    /// Returns a non-blocking snapshot of all task lifecycle counts.
    pub fn summary(&self) -> ProgressSummary {
        summarize(&self.inner.slots)
    }

    pub(crate) fn task_execution_snapshots(&self) -> Vec<TaskExecutionSnapshot> {
        self.inner
            .slots
            .iter()
            .map(|slot| TaskExecutionSnapshot {
                key: slot.identity.task_key().clone(),
                status: TaskStatus::decode(slot.status.load(Ordering::Acquire)),
                current_iteration: (slot.mode == TaskMode::Progress)
                    .then(|| slot.current.load(Ordering::Relaxed)),
                target_iteration: (slot.mode == TaskMode::Progress
                    && slot.target_known.load(Ordering::Acquire))
                .then(|| slot.target.load(Ordering::Relaxed)),
            })
            .collect()
    }

    /// Finishes a successful session and emits its final summary and message.
    ///
    /// Every task must already be completed. The method stops and joins the
    /// renderer and releases exclusive terminal ownership.
    pub fn complete(mut self, message: impl Into<String>) -> Result<ProgressSummary, StudyError> {
        let summary = self.summary();
        if !summary.is_success() {
            self.stop(false, "study did not complete".to_owned())?;
            return Err(StudyError::IncompleteProgress {
                pending: summary.pending,
                running: summary.running,
                failed: summary.failed,
            });
        }
        self.stop(true, message.into())?;
        Ok(summary)
    }

    /// Finishes an unsuccessful session while preserving all task statuses.
    pub fn fail(mut self, message: impl Into<String>) -> Result<ProgressSummary, StudyError> {
        let summary = self.summary();
        self.stop(false, message.into())?;
        Ok(summary)
    }

    fn stop(&mut self, success: bool, message: String) -> Result<(), StudyError> {
        self.inner
            .events
            .send(RenderEvent::Stop { success, message })
            .map_err(|_| StudyError::RendererUnavailable)?;
        self.finished = true;
        if self
            .renderer
            .take()
            .expect("an unfinished study renderer owns one thread")
            .join()
            .is_err()
        {
            return Err(StudyError::RendererPanicked);
        }
        Ok(())
    }
}

pub(crate) struct TaskExecutionSnapshot {
    pub(crate) key: TaskKey,
    pub(crate) status: TaskStatus,
    pub(crate) current_iteration: Option<u64>,
    pub(crate) target_iteration: Option<u64>,
}

impl fmt::Debug for StudyRenderer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StudyRenderer")
            .field("tasks", &self.inner.slots.len())
            .field("summary", &self.summary())
            .finish_non_exhaustive()
    }
}

impl Drop for StudyRenderer {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let _ = self.inner.events.send(RenderEvent::Stop {
            success: false,
            message: "study renderer dropped before completion".to_owned(),
        });
        if let Some(renderer) = self.renderer.take() {
            let _ = renderer.join();
        }
    }
}

/// Non-clone task-local progress handle.
///
/// Dropping a running handle without calling [`TaskProgressHandle::complete`] or
/// [`TaskProgressHandle::fail`] marks the task failed, making ordinary `?` returns
/// safe without a separate cleanup branch.
pub(crate) struct TaskProgressHandle {
    slot: Arc<ProgressSlot>,
    events: SyncSender<RenderEvent>,
    cancelled: Arc<AtomicBool>,
    active: bool,
}

/// Shared, cheap cancellation observation for embedded schedulers and tasks.
#[derive(Clone, Debug)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub(crate) fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    pub(crate) fn shared(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.0)
    }

    /// Reports whether interactive Ctrl-C requested termination.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    /// Requests cooperative termination.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
}

impl TaskProgressHandle {
    /// Reports whether the owning renderer requested cooperative termination.
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

    /// Sets or replaces the absolute target for this running task.
    pub fn set_target_iteration(&self, target: u64) -> Result<(), StudyError> {
        let current = self.current_iteration();
        if current > target {
            return Err(StudyError::InitialIterationBeyondTarget {
                identity: self.identity().label().to_owned(),
                initial: current,
                target,
            });
        }
        self.slot.target.store(target, Ordering::Relaxed);
        self.slot.target_known.store(true, Ordering::Release);
        Ok(())
    }

    /// Synchronizes progress to an authoritative absolute simulation iteration.
    ///
    /// The atomic update never allocates or locks. Regressions and movement
    /// beyond a known target are rejected without modifying the counter.
    pub fn set_iteration(&self, iteration: u64) -> Result<(), StudyError> {
        if let Some(target) = self.target_iteration().filter(|target| iteration > *target) {
            return Err(StudyError::IterationBeyondTarget {
                identity: self.identity().label().to_owned(),
                iteration,
                target,
            });
        }
        let previous = self.slot.current.fetch_max(iteration, Ordering::Relaxed);
        if iteration < previous {
            return Err(StudyError::IterationRegressed {
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
    pub fn should_continue(&self, iteration: u64) -> Result<bool, StudyError> {
        self.set_iteration(iteration)?;
        Ok(!self.is_cancelled()
            && self
                .target_iteration()
                .is_none_or(|target| iteration < target))
    }

    /// Updates one infrequent human-readable detail such as `evolving` or
    /// `validating`. Detail updates may lock; iteration updates do not.
    pub fn set_detail(&self, detail: impl Into<String>) {
        *lock(&self.slot.detail) = detail.into().into_boxed_str();
    }

    /// Sends one task-scoped message through the sole renderer.
    pub fn report(&self, message: impl Into<String>) -> Result<(), StudyError> {
        self.events
            .send(RenderEvent::TaskMessage {
                identity: self.identity().label().to_owned(),
                message: message.into(),
            })
            .map_err(|_| StudyError::RendererUnavailable)
    }

    /// Marks the task successful and consumes this handle.
    ///
    /// `reason == None` means the configured target must have been reached.
    /// `Some(reason)` records an intentional scientific early-completion reason
    /// and permits completion before that generic target.
    pub fn complete(mut self, reason: Option<String>) -> Result<(), StudyError> {
        if reason.is_none()
            && let Some(target) = self.target_iteration()
        {
            let current = self.current_iteration();
            if current != target {
                *lock(&self.slot.detail) = "target not reached".into();
                self.slot
                    .status
                    .store(TaskStatus::Failed.encode(), Ordering::Release);
                self.active = false;
                return Err(StudyError::TargetIterationNotReached {
                    identity: self.identity().label().to_owned(),
                    current,
                    target,
                });
            }
        }
        *lock(&self.slot.detail) = reason
            .unwrap_or_else(|| "completed".to_owned())
            .into_boxed_str();
        self.slot
            .status
            .store(TaskStatus::Completed.encode(), Ordering::Release);
        self.active = false;
        Ok(())
    }

    /// Marks the task failed, records a concise detail, and consumes the handle.
    pub fn fail(mut self, reason: impl Into<String>) {
        *lock(&self.slot.detail) = reason.into().into_boxed_str();
        self.slot
            .status
            .store(TaskStatus::Failed.encode(), Ordering::Release);
        self.active = false;
    }

    pub(crate) fn cancel(mut self, reason: impl Into<String>) {
        *lock(&self.slot.detail) = reason.into().into_boxed_str();
        self.slot
            .status
            .store(TaskStatus::Cancelled.encode(), Ordering::Release);
        self.active = false;
    }
}

impl fmt::Debug for TaskProgressHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TaskProgressHandle")
            .field("identity", &self.identity().label())
            .field("current_iteration", &self.current_iteration())
            .field("target_iteration", &self.target_iteration())
            .field("active", &self.active)
            .finish_non_exhaustive()
    }
}

impl Drop for TaskProgressHandle {
    fn drop(&mut self) {
        if self.active {
            *lock(&self.slot.detail) = "interrupted".into();
            self.slot
                .status
                .store(TaskStatus::Failed.encode(), Ordering::Release);
        }
    }
}

/// Non-clone task-local handle for lifecycle-only work.
///
/// One-shot tasks deliberately expose no iteration or target operations. Dropping
/// an active handle marks only its reporting task failed.
pub(crate) struct OneShotTaskHandle {
    slot: Arc<ProgressSlot>,
    events: SyncSender<RenderEvent>,
    cancelled: Arc<AtomicBool>,
    active: bool,
}

impl OneShotTaskHandle {
    /// Reports whether the owning renderer requested cooperative termination.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    /// Borrows the task identity supplied by the owning phase.
    pub fn identity(&self) -> &TaskIdentity {
        &self.slot.identity
    }

    /// Returns the current lifecycle status.
    pub fn status(&self) -> TaskStatus {
        TaskStatus::decode(self.slot.status.load(Ordering::Acquire))
    }

    /// Updates one infrequent human-readable execution detail.
    pub fn set_detail(&self, detail: impl Into<String>) {
        *lock(&self.slot.detail) = detail.into().into_boxed_str();
    }

    /// Sends one task-scoped message through the sole renderer.
    pub fn report(&self, message: impl Into<String>) -> Result<(), StudyError> {
        self.events
            .send(RenderEvent::TaskMessage {
                identity: self.identity().label().to_owned(),
                message: message.into(),
            })
            .map_err(|_| StudyError::RendererUnavailable)
    }

    /// Marks this one-shot successful and consumes its handle.
    pub fn complete(mut self) {
        *lock(&self.slot.detail) = "completed".into();
        self.slot
            .status
            .store(TaskStatus::Completed.encode(), Ordering::Release);
        self.active = false;
    }

    /// Marks this one-shot failed and consumes its handle.
    pub fn fail(mut self, reason: impl Into<String>) {
        *lock(&self.slot.detail) = reason.into().into_boxed_str();
        self.slot
            .status
            .store(TaskStatus::Failed.encode(), Ordering::Release);
        self.active = false;
    }

    pub(crate) fn cancel(mut self, reason: impl Into<String>) {
        *lock(&self.slot.detail) = reason.into().into_boxed_str();
        self.slot
            .status
            .store(TaskStatus::Cancelled.encode(), Ordering::Release);
        self.active = false;
    }
}

impl fmt::Debug for OneShotTaskHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OneShotTaskHandle")
            .field("identity", &self.identity().label())
            .field("status", &self.status())
            .field("active", &self.active)
            .finish_non_exhaustive()
    }
}

impl Drop for OneShotTaskHandle {
    fn drop(&mut self) {
        if self.active {
            *lock(&self.slot.detail) = "interrupted".into();
            self.slot
                .status
                .store(TaskStatus::Failed.encode(), Ordering::Release);
        }
    }
}

struct RendererInner {
    slots: Arc<[Arc<ProgressSlot>]>,
    events: SyncSender<RenderEvent>,
    cancelled: Arc<AtomicBool>,
}

struct ProgressSlot {
    identity: TaskIdentity,
    phase_label: Option<Arc<str>>,
    mode: TaskMode,
    current: AtomicU64,
    target: AtomicU64,
    target_known: AtomicBool,
    started: AtomicBool,
    status: AtomicU8,
    detail: Mutex<Box<str>>,
}

enum RenderEvent {
    TaskMessage { identity: String, message: String },
    Stop { success: bool, message: String },
}

struct TerminalLease;

impl Drop for TerminalLease {
    fn drop(&mut self) {
        TERMINAL_OWNED.store(false, Ordering::Release);
    }
}

fn acquire_terminal() -> Result<(), StudyError> {
    TERMINAL_OWNED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .map(|_| ())
        .map_err(|_| StudyError::TerminalAlreadyOwned)
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

fn start_renderer(
    slots: Arc<[Arc<ProgressSlot>]>,
    requested_output: OutputMode,
    cancellation: Option<CancellationToken>,
) -> Result<StudyRenderer, StudyError> {
    acquire_terminal()?;
    let lease = TerminalLease;
    let cancelled = cancellation
        .map(|token| token.shared())
        .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
    let mut output = resolve_output(requested_output);
    let terminal = if output == OutputMode::Terminal {
        match TerminalUi::enter(slots.len()) {
            Ok(terminal) => Some(terminal),
            Err(_) if requested_output == OutputMode::Auto => {
                output = OutputMode::Plain;
                None
            }
            Err(error) => {
                return Err(StudyError::TerminalSetup {
                    operation: error.operation,
                    source: error.source,
                });
            }
        }
    } else {
        None
    };
    let (events, receiver) = mpsc::sync_channel(MESSAGE_CAPACITY);
    let renderer_slots = Arc::clone(&slots);
    let renderer_cancelled = Arc::clone(&cancelled);
    let renderer = match thread::Builder::new()
        .name("scientific-workflow-progress".to_owned())
        .spawn(move || {
            render(
                receiver,
                renderer_slots,
                output,
                terminal,
                renderer_cancelled,
                lease,
            )
        }) {
        Ok(renderer) => renderer,
        Err(source) => return Err(StudyError::StartRenderer { source }),
    };
    Ok(StudyRenderer {
        inner: Arc::new(RendererInner {
            slots,
            events,
            cancelled,
        }),
        renderer: Some(renderer),
        finished: false,
    })
}

fn managed_slot(
    slots: &[Arc<ProgressSlot>],
    key: &TaskKey,
) -> Result<Arc<ProgressSlot>, StudyError> {
    slots
        .iter()
        .find(|slot| slot.identity.task_key() == key)
        .cloned()
        .ok_or_else(|| StudyError::UnknownTask {
            task: key.to_string(),
        })
}

fn mode_mismatch(slot: &ProgressSlot, requested: &'static str) -> StudyError {
    StudyError::TaskModeMismatch {
        task: slot.identity.task_key().to_string(),
        requested,
        actual: match slot.mode {
            TaskMode::Progress => "progress",
            TaskMode::OneShot => "one-shot",
        },
    }
}

fn mark_slot_completed(slot: &ProgressSlot) -> Result<(), StudyError> {
    slot.status
        .compare_exchange(
            TaskStatus::Pending.encode(),
            TaskStatus::Completed.encode(),
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .map_err(|_| StudyError::TaskAlreadyStarted {
            identity: slot.identity.label().to_owned(),
        })?;
    *lock(&slot.detail) = "already completed".into();
    Ok(())
}

fn mark_pending_terminal(
    slot: &ProgressSlot,
    status: TaskStatus,
    detail: &'static str,
) -> Result<(), StudyError> {
    slot.status
        .compare_exchange(
            TaskStatus::Pending.encode(),
            status.encode(),
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .map_err(|_| StudyError::TaskAlreadyStarted {
            identity: slot.identity.label().to_owned(),
        })?;
    *lock(&slot.detail) = detail.into();
    Ok(())
}

fn start_slot(
    inner: &RendererInner,
    slot: Arc<ProgressSlot>,
    initial_iteration: u64,
    target_iteration: Option<u64>,
) -> Result<TaskProgressHandle, StudyError> {
    if let Some(target) = target_iteration.filter(|target| initial_iteration > *target) {
        return Err(StudyError::InitialIterationBeyondTarget {
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
        .map_err(|_| StudyError::TaskAlreadyStarted {
            identity: slot.identity.label().to_owned(),
        })?;
    slot.started.store(true, Ordering::Release);
    slot.current.store(initial_iteration, Ordering::Relaxed);
    if let Some(target) = target_iteration {
        slot.target.store(target, Ordering::Relaxed);
        slot.target_known.store(true, Ordering::Release);
    } else {
        slot.target_known.store(false, Ordering::Release);
    }
    *lock(&slot.detail) = "running".into();
    Ok(TaskProgressHandle {
        slot,
        events: inner.events.clone(),
        cancelled: Arc::clone(&inner.cancelled),
        active: true,
    })
}

fn start_one_shot_slot(
    inner: &RendererInner,
    slot: Arc<ProgressSlot>,
) -> Result<OneShotTaskHandle, StudyError> {
    slot.status
        .compare_exchange(
            TaskStatus::Pending.encode(),
            TaskStatus::Running.encode(),
            Ordering::AcqRel,
            Ordering::Acquire,
        )
        .map_err(|_| StudyError::TaskAlreadyStarted {
            identity: slot.identity.label().to_owned(),
        })?;
    slot.started.store(true, Ordering::Release);
    slot.target_known.store(false, Ordering::Release);
    *lock(&slot.detail) = "running".into();
    Ok(OneShotTaskHandle {
        slot,
        events: inner.events.clone(),
        cancelled: Arc::clone(&inner.cancelled),
        active: true,
    })
}

fn build_phase_slots(
    phases: &[Phase],
    heading: Option<&str>,
) -> Result<Arc<[Arc<ProgressSlot>]>, StudyError> {
    if phases.is_empty() {
        return Err(StudyError::EmptyPhaseSet);
    }
    let mut phase_ids = HashSet::with_capacity(phases.len());
    let capacity = phases.iter().map(|phase| phase.tasks().len()).sum();
    let mut slots = Vec::with_capacity(capacity);
    for phase in phases {
        if !phase_ids.insert(phase.id()) {
            return Err(StudyError::DuplicatePhaseId {
                phase: phase.id().get(),
            });
        }
        let phase_label: Arc<str> = heading.unwrap_or_else(|| phase.label()).into();
        for task in phase.tasks() {
            slots.push(Arc::new(ProgressSlot {
                identity: TaskIdentity {
                    label: task.label().into(),
                    key: task.key().clone(),
                    metadata: task.metadata_map(),
                },
                phase_label: Some(Arc::clone(&phase_label)),
                mode: task.mode(),
                current: AtomicU64::new(0),
                target: AtomicU64::new(0),
                target_known: AtomicBool::new(false),
                started: AtomicBool::new(false),
                status: AtomicU8::new(TaskStatus::Pending.encode()),
                detail: Mutex::new("pending".into()),
            }));
        }
    }
    Ok(slots.into())
}

fn summarize(slots: &[Arc<ProgressSlot>]) -> ProgressSummary {
    let mut summary = ProgressSummary {
        total: u64::try_from(slots.len()).expect("slot count originated from a u64 task count"),
        pending: 0,
        running: 0,
        completed: 0,
        failed: 0,
        cancelled: 0,
        skipped: 0,
    };
    for slot in slots {
        match TaskStatus::decode(slot.status.load(Ordering::Acquire)) {
            TaskStatus::Pending => summary.pending += 1,
            TaskStatus::Running => summary.running += 1,
            TaskStatus::Completed => summary.completed += 1,
            TaskStatus::Failed => summary.failed += 1,
            TaskStatus::Cancelled => summary.cancelled += 1,
            TaskStatus::Skipped => summary.skipped += 1,
        }
    }
    summary
}

fn render(
    receiver: Receiver<RenderEvent>,
    slots: Arc<[Arc<ProgressSlot>]>,
    output: OutputMode,
    mut terminal: Option<TerminalUi>,
    cancelled: Arc<AtomicBool>,
    _lease: TerminalLease,
) {
    let mut last_statuses = vec![TaskStatus::Pending; slots.len()];
    let mut input_failed = false;
    loop {
        if let Some(display) = &mut terminal {
            if !input_failed {
                match display.poll_command() {
                    Ok(Some(StudyCommand::Exit)) => {
                        cancelled.store(true, Ordering::Release);
                        display.mark_exit_requested();
                    }
                    Ok(None) => {}
                    Err(error) => {
                        cancelled.store(true, Ordering::Release);
                        display.push_message(format!("study: terminal input failed: {error}"));
                        display.mark_exit_requested();
                        input_failed = true;
                    }
                }
            }
            let snapshot = render_snapshot(&slots);
            let _ = display.draw(&snapshot);
        }
        match receiver.recv_timeout(REFRESH_INTERVAL) {
            Ok(RenderEvent::TaskMessage { identity, message }) => {
                let message = format!("{identity}: {message}");
                if let Some(display) = &mut terminal {
                    display.push_message(message);
                } else {
                    write_message(output, &message);
                }
            }
            Ok(RenderEvent::Stop { success, message }) => {
                if let Some(display) = &mut terminal {
                    let snapshot = render_snapshot(&slots);
                    let _ = display.draw(&snapshot);
                }
                if output == OutputMode::Plain {
                    write_plain_transitions(&slots, &mut last_statuses);
                }
                drop(terminal.take());
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

fn render_snapshot(slots: &[Arc<ProgressSlot>]) -> RenderSnapshot {
    RenderSnapshot {
        heading: slots
            .first()
            .and_then(|slot| slot.phase_label.as_deref())
            .unwrap_or("Study")
            .to_owned(),
        summary: summarize(slots),
        tasks: terminal_task_views(slots),
    }
}

fn terminal_task_views(slots: &[Arc<ProgressSlot>]) -> Vec<TaskView> {
    slots
        .iter()
        .map(|slot| TaskView {
            label: slot.identity.label().to_owned(),
            mode: slot.mode,
            current: slot.current.load(Ordering::Relaxed),
            target: slot
                .target_known
                .load(Ordering::Acquire)
                .then(|| slot.target.load(Ordering::Relaxed)),
            started: slot.started.load(Ordering::Acquire),
            status: TaskStatus::decode(slot.status.load(Ordering::Acquire)),
            detail: lock(&slot.detail).to_string(),
        })
        .collect()
}

fn write_message(output: OutputMode, message: &str) {
    match output {
        OutputMode::Plain => eprintln!("[progress] {message}"),
        OutputMode::Terminal | OutputMode::Hidden | OutputMode::Auto => {}
    }
}

fn write_plain_transitions(slots: &[Arc<ProgressSlot>], previous: &mut [TaskStatus]) {
    for (slot, old) in slots.iter().zip(previous) {
        let status = TaskStatus::decode(slot.status.load(Ordering::Acquire));
        if status != *old {
            let detail = lock(&slot.detail);
            eprintln!(
                "[task] identity={} status={} detail={} iteration={} target={}",
                slot.identity.task_key(),
                status.label(),
                detail.as_ref(),
                slot.current.load(Ordering::Relaxed),
                format_target(slot)
            );
            *old = status;
        }
    }
}

fn write_final(output: OutputMode, slots: &[Arc<ProgressSlot>], success: bool, message: &str) {
    if output == OutputMode::Hidden || (output == OutputMode::Terminal && success) {
        return;
    }
    let summary = summarize(slots);
    if !success {
        for slot in slots {
            let detail = lock(&slot.detail);
            eprintln!(
                "[task-final] task={} status={} detail={}",
                slot.identity.task_key(),
                TaskStatus::decode(slot.status.load(Ordering::Acquire)).label(),
                detail.as_ref(),
            );
        }
    }
    eprintln!(
        "[study] status={} tasks={} completed={} failed={} cancelled={} skipped={} pending={} message={}",
        if success { "completed" } else { "failed" },
        summary.total,
        summary.completed,
        summary.failed,
        summary.cancelled,
        summary.skipped,
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
