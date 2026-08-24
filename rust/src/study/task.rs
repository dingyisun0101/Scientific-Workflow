//! Task-local execution context and workload contract.

use std::error::Error;

use serde::de::DeserializeOwned;
use serde_json::Value;

use super::error::StudyError;
use super::phase::{Task, TaskId, TaskKey};
use super::renderer::{OneShotTaskHandle, TaskProgressHandle};

/// Error-erased result returned by one task-owned workload.
pub type TaskResult = Result<(), Box<dyn Error + Send + Sync + 'static>>;

pub(crate) type Workload = Box<dyn FnOnce(&TaskContext) -> TaskResult + Send + 'static>;

enum TaskHandle {
    Progress(TaskProgressHandle),
    OneShot(OneShotTaskHandle),
}

/// Read-only task identity plus reporting and cancellation.
///
/// The context deliberately has no filesystem, storage, artifact, network,
/// subprocess, or machine-resource operations. The workload owns all such
/// effects directly.
pub struct TaskContext {
    task: Task,
    handle: TaskHandle,
}

impl TaskContext {
    pub(crate) fn progress(task: Task, progress: TaskProgressHandle) -> Self {
        Self {
            task,
            handle: TaskHandle::Progress(progress),
        }
    }

    pub(crate) fn one_shot(task: Task, one_shot: OneShotTaskHandle) -> Self {
        Self {
            task,
            handle: TaskHandle::OneShot(one_shot),
        }
    }

    /// Borrows the complete immutable task declaration.
    pub fn task(&self) -> &Task {
        &self.task
    }

    /// Borrows the exact phase-qualified task key.
    pub fn key(&self) -> &TaskKey {
        self.task.key()
    }

    /// Borrows the phase-local task ID.
    pub fn id(&self) -> &TaskId {
        self.task.id()
    }

    /// Borrows the application-defined task category.
    pub fn category(&self) -> &str {
        self.task.category_name()
    }

    /// Borrows one application-defined metadata value.
    pub fn metadata(&self, key: &str) -> Option<&Value> {
        self.task.metadata_value(key)
    }

    /// Decodes one required metadata value.
    pub fn decode_metadata<T>(&self, key: &str) -> Result<T, StudyError>
    where
        T: DeserializeOwned,
    {
        self.task.decode_metadata(key)
    }

    /// Sets or replaces the target iteration of a progress task.
    pub fn set_target_iteration(&self, target: u64) -> Result<(), StudyError> {
        self.required_progress()?.set_target_iteration(target)
    }

    /// Synchronizes a progress task to the authoritative scientific iteration.
    pub fn set_iteration(&self, iteration: u64) -> Result<(), StudyError> {
        self.required_progress()?.set_iteration(iteration)
    }

    /// Synchronizes iteration and reports whether work should continue.
    pub fn should_continue(&self, iteration: u64) -> Result<bool, StudyError> {
        self.required_progress()?.should_continue(iteration)
    }

    /// Reports whether the study requested cooperative cancellation.
    pub fn is_cancelled(&self) -> bool {
        match &self.handle {
            TaskHandle::Progress(progress) => progress.is_cancelled(),
            TaskHandle::OneShot(one_shot) => one_shot.is_cancelled(),
        }
    }

    /// Updates the human-readable task detail.
    pub fn set_detail(&self, detail: impl Into<String>) {
        let detail = detail.into();
        match &self.handle {
            TaskHandle::Progress(progress) => progress.set_detail(detail),
            TaskHandle::OneShot(one_shot) => one_shot.set_detail(detail),
        }
    }

    /// Sends a task-scoped display message.
    pub fn report(&self, message: impl Into<String>) -> Result<(), StudyError> {
        let message = message.into();
        match &self.handle {
            TaskHandle::Progress(progress) => progress.report(message),
            TaskHandle::OneShot(one_shot) => one_shot.report(message),
        }
    }

    pub(crate) fn complete(self) -> Result<(), StudyError> {
        match self.handle {
            TaskHandle::Progress(progress) => progress.complete(None),
            TaskHandle::OneShot(one_shot) => {
                one_shot.complete();
                Ok(())
            }
        }
    }

    pub(crate) fn fail(self, reason: impl Into<String>) {
        let reason = reason.into();
        match self.handle {
            TaskHandle::Progress(progress) => progress.fail(reason),
            TaskHandle::OneShot(one_shot) => one_shot.fail(reason),
        }
    }

    pub(crate) fn cancel(self, reason: impl Into<String>) {
        let reason = reason.into();
        match self.handle {
            TaskHandle::Progress(progress) => progress.cancel(reason),
            TaskHandle::OneShot(one_shot) => one_shot.cancel(reason),
        }
    }

    fn required_progress(&self) -> Result<&TaskProgressHandle, StudyError> {
        match &self.handle {
            TaskHandle::Progress(progress) => Ok(progress),
            TaskHandle::OneShot(_) => Err(StudyError::TaskModeMismatch {
                task: self.key().to_string(),
                requested: "progress",
                actual: "one-shot",
            }),
        }
    }
}

impl std::fmt::Debug for TaskContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TaskContext")
            .field("key", self.key())
            .field("category", &self.category())
            .field("mode", &self.task.mode())
            .finish_non_exhaustive()
    }
}
