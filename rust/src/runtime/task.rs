//! Task-local execution context and workload contract.

use std::error::Error;
use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde_json::Value;

use super::error::RuntimeError;
use super::phase::{Task, TaskId, TaskKey};
use super::reporting::{ActivityTask, TaskProgress};
use crate::configuration::TaskConfig;

/// Error-erased result returned by one task-owned workload.
pub type TaskResult = Result<(), Box<dyn Error + Send + Sync + 'static>>;

pub(crate) type Workload = Arc<dyn Fn(&TaskContext) -> TaskResult + Send + Sync + 'static>;

enum TaskHandle {
    Progress(TaskProgress),
    Activity(ActivityTask),
}

/// Read-only task identity/configuration plus reporting and cancellation.
///
/// The context deliberately has no filesystem, storage, artifact, network,
/// subprocess, or machine-resource operations. The workload owns all such
/// effects directly.
pub struct TaskContext {
    task: Task,
    handle: TaskHandle,
}

impl TaskContext {
    pub(crate) fn progress(task: Task, progress: TaskProgress) -> Self {
        Self {
            task,
            handle: TaskHandle::Progress(progress),
        }
    }

    pub(crate) fn activity(task: Task, activity: ActivityTask) -> Self {
        Self {
            task,
            handle: TaskHandle::Activity(activity),
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

    /// Borrows the task kind/namespace.
    pub fn kind(&self) -> &str {
        self.task.kind()
    }

    /// Borrows retained configuration when this task was generated from one.
    pub fn configuration(&self) -> Option<&TaskConfig> {
        self.task.configuration()
    }

    /// Borrows one fixed, swept, or explicit task parameter.
    pub fn value(&self, key: &str) -> Option<&Value> {
        self.task.value(key)
    }

    /// Decodes one required task parameter.
    pub fn decode_value<T>(&self, key: &str) -> Result<T, RuntimeError>
    where
        T: DeserializeOwned,
    {
        self.task.decode_value(key)
    }

    /// Borrows iterative progress for a progress task.
    pub fn progress_handle(&self) -> Option<&TaskProgress> {
        match &self.handle {
            TaskHandle::Progress(progress) => Some(progress),
            TaskHandle::Activity(_) => None,
        }
    }

    /// Borrows lifecycle-only reporting for an activity task.
    pub fn activity_handle(&self) -> Option<&ActivityTask> {
        match &self.handle {
            TaskHandle::Progress(_) => None,
            TaskHandle::Activity(activity) => Some(activity),
        }
    }

    /// Reports whether the runtime requested cooperative cancellation.
    pub fn is_cancelled(&self) -> bool {
        match &self.handle {
            TaskHandle::Progress(progress) => progress.is_cancelled(),
            TaskHandle::Activity(activity) => activity.is_cancelled(),
        }
    }

    /// Updates the human-readable task detail.
    pub fn set_detail(&self, detail: impl Into<String>) {
        let detail = detail.into();
        match &self.handle {
            TaskHandle::Progress(progress) => progress.set_detail(detail),
            TaskHandle::Activity(activity) => activity.set_detail(detail),
        }
    }

    /// Sends a task-scoped display message.
    pub fn report(&self, message: impl Into<String>) -> Result<(), RuntimeError> {
        let message = message.into();
        match &self.handle {
            TaskHandle::Progress(progress) => progress.report(message),
            TaskHandle::Activity(activity) => activity.report(message),
        }
    }

    pub(crate) fn complete(self) -> Result<(), RuntimeError> {
        match self.handle {
            TaskHandle::Progress(progress) => progress.complete(None),
            TaskHandle::Activity(activity) => {
                activity.complete();
                Ok(())
            }
        }
    }

    pub(crate) fn fail(self, reason: impl Into<String>) {
        let reason = reason.into();
        match self.handle {
            TaskHandle::Progress(progress) => progress.fail(reason),
            TaskHandle::Activity(activity) => activity.fail(reason),
        }
    }
}

impl std::fmt::Debug for TaskContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TaskContext")
            .field("key", self.key())
            .field("kind", &self.kind())
            .field("display_kind", &self.task.display_kind())
            .finish_non_exhaustive()
    }
}
