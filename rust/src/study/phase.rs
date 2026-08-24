//! First-class phase and task declarations owned by a study.

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::error::StudyError;
use super::task::{TaskContext, TaskResult, Workload};

/// Stable numeric identity of one execution phase.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PhaseId(u64);

impl PhaseId {
    /// Creates a phase identity suitable for command-line selections such as
    /// `[2, 4, 5]`.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the exact numeric phase identity.
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl From<u64> for PhaseId {
    fn from(value: u64) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for PhaseId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Stable task identity scoped to one phase.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TaskId(Arc<str>);

impl TaskId {
    /// Creates a task ID. Empty or whitespace-only IDs are rejected when the
    /// owning phase is built, keeping task construction infallible.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into().into())
    }

    /// Borrows the exact ID text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Exact phase-qualified task lookup key.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TaskKey {
    phase: PhaseId,
    task: TaskId,
}

impl TaskKey {
    /// Creates one phase-qualified task key.
    pub fn new(phase: impl Into<PhaseId>, task: TaskId) -> Self {
        Self {
            phase: phase.into(),
            task,
        }
    }

    /// Returns the owning phase ID.
    pub const fn phase_id(&self) -> PhaseId {
        self.phase
    }

    /// Borrows the phase-local task ID.
    pub fn task_id(&self) -> &TaskId {
        &self.task
    }
}

impl fmt::Display for TaskKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/{}", self.phase, self.task)
    }
}

/// Reporting shape of one task.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum TaskMode {
    /// Work that may report iterative progress.
    Progress,
    /// One-shot work with lifecycle status only.
    OneShot,
}

/// First-class immutable task declaration owned by exactly one [`Phase`].
pub struct Task {
    key: TaskKey,
    category: Arc<str>,
    mode: TaskMode,
    label: Arc<str>,
    metadata: Arc<BTreeMap<String, Value>>,
    workload: Option<Workload>,
    completed: bool,
}

impl Task {
    /// Creates a one-shot task.
    pub fn one_shot<W>(id: impl Into<String>, label: impl Into<String>, workload: W) -> Self
    where
        W: FnOnce(&TaskContext) -> TaskResult + Send + 'static,
    {
        Self::new(
            id,
            label,
            TaskMode::OneShot,
            Some(Box::new(workload)),
            false,
        )
    }

    /// Creates a task that may report progress through its context.
    pub fn progress<W>(id: impl Into<String>, label: impl Into<String>, workload: W) -> Self
    where
        W: FnOnce(&TaskContext) -> TaskResult + Send + 'static,
    {
        Self::new(
            id,
            label,
            TaskMode::Progress,
            Some(Box::new(workload)),
            false,
        )
    }

    /// Creates an application-verified task that is already satisfied.
    pub fn completed(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self::new(id, label, TaskMode::OneShot, None, true)
    }

    fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        mode: TaskMode,
        workload: Option<Workload>,
        completed: bool,
    ) -> Self {
        Self {
            key: TaskKey::new(0, TaskId::new(id)),
            category: "task".into(),
            mode,
            label: label.into().into(),
            metadata: Arc::new(BTreeMap::new()),
            workload,
            completed,
        }
    }

    /// Sets the optional application-defined task category.
    #[must_use]
    pub fn category(mut self, category: impl Into<String>) -> Self {
        self.category = category.into().into();
        self
    }

    /// Adds application-defined metadata used for inspection and selection.
    #[must_use]
    pub fn metadata(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        Arc::make_mut(&mut self.metadata).insert(key.into(), value.into());
        self
    }

    /// Returns the exact phase-qualified task key.
    pub fn key(&self) -> &TaskKey {
        &self.key
    }

    /// Returns the phase-local task ID.
    pub fn id(&self) -> &TaskId {
        self.key.task_id()
    }

    /// Returns the application-defined task category.
    pub fn category_name(&self) -> &str {
        &self.category
    }

    /// Returns the automatically generated display label.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns whether the task reports iterative progress or one-shot status.
    pub const fn mode(&self) -> TaskMode {
        self.mode
    }

    /// Borrows one application-defined metadata value.
    pub fn metadata_value(&self, key: &str) -> Option<&Value> {
        self.metadata.get(key)
    }

    /// Borrows one required task parameter.
    pub fn require_value(&self, key: &str) -> Result<&Value, StudyError> {
        self.metadata_value(key)
            .ok_or_else(|| StudyError::UnknownTaskMetadata {
                task: self.key.to_string(),
                key: key.to_owned(),
            })
    }

    /// Decodes one required task parameter without first cloning its JSON tree.
    pub fn decode_value<T>(&self, key: &str) -> Result<T, StudyError>
    where
        T: DeserializeOwned,
    {
        T::deserialize(self.require_value(key)?).map_err(|source| StudyError::DecodeTaskMetadata {
            task: self.key.to_string(),
            key: key.to_owned(),
            source,
        })
    }

    /// Iterates application-defined metadata.
    pub fn metadata_iter(&self) -> impl Iterator<Item = (&str, &Value)> + '_ {
        self.metadata
            .iter()
            .map(|(key, value)| (key.as_str(), value))
    }

    pub(crate) fn metadata_map(&self) -> Arc<BTreeMap<String, Value>> {
        Arc::clone(&self.metadata)
    }

    pub(crate) fn take_workload(&mut self) -> Option<Workload> {
        self.workload.take()
    }

    pub(crate) fn has_workload(&self) -> bool {
        self.workload.is_some()
    }

    pub(crate) const fn is_completed(&self) -> bool {
        self.completed
    }

    fn attach_to_phase(&mut self, phase: PhaseId) {
        self.key.phase = phase;
    }
}

impl fmt::Debug for Task {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Task")
            .field("key", &self.key)
            .field("category", &self.category)
            .field("label", &self.label)
            .field("mode", &self.mode)
            .field("metadata", &self.metadata.len())
            .finish_non_exhaustive()
    }
}

/// Immutable nonempty execution phase and renderer section.
pub struct Phase {
    id: PhaseId,
    label: Arc<str>,
    tasks: Vec<Task>,
    max_active_tasks: usize,
    prepared_task_queue_capacity: usize,
    delay_per_task: Option<Duration>,
    task_timeout: Option<Duration>,
    deadline_after: Option<Duration>,
    dependencies: Vec<PhaseId>,
    failure_policy: PhaseFailurePolicy,
    require_confirm: bool,
}

/// Scheduling behavior after the first workload failure in a phase.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum PhaseFailurePolicy {
    /// Stop new work and cooperatively cancel active workloads.
    #[default]
    FailFast,
    /// Stop new work but allow already active workloads to finish.
    FinishActive,
}

impl PhaseFailurePolicy {
    /// Returns the stable uncolored policy name used by plain output.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FailFast => "fail-fast",
            Self::FinishActive => "finish-active",
        }
    }
}

impl Phase {
    /// Begins declaring a phase. Tasks must be added before [`PhaseBuilder::build`].
    pub fn builder(id: impl Into<PhaseId>, label: impl Into<String>) -> PhaseBuilder {
        PhaseBuilder {
            id: id.into(),
            label: label.into(),
            tasks: Vec::new(),
            max_active_tasks: 1,
            prepared_task_queue_capacity: 1,
            delay_per_task: None,
            task_timeout: None,
            deadline_after: None,
            dependencies: Vec::new(),
            failure_policy: PhaseFailurePolicy::FailFast,
            require_confirm: false,
        }
    }

    /// Returns the stable phase identity.
    pub const fn id(&self) -> PhaseId {
        self.id
    }

    /// Returns the human-facing phase heading.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns every task in deterministic display/execution order.
    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    /// Returns the phase-local concurrent workload ceiling.
    pub const fn max_active_tasks(&self) -> usize {
        self.max_active_tasks
    }

    /// Returns the prepared-but-not-running workload ceiling.
    pub const fn prepared_task_queue_capacity(&self) -> usize {
        self.prepared_task_queue_capacity
    }

    /// Returns the optional minimum interval between consecutive task starts.
    pub const fn delay_per_task(&self) -> Option<Duration> {
        self.delay_per_task
    }

    /// Returns the optional elapsed-time limit for each running task.
    pub const fn task_timeout(&self) -> Option<Duration> {
        self.task_timeout
    }

    /// Returns the optional phase deadline measured from phase execution start.
    pub const fn deadline_after(&self) -> Option<Duration> {
        self.deadline_after
    }

    /// Returns declared predecessor phases in declaration order.
    pub fn dependencies(&self) -> &[PhaseId] {
        &self.dependencies
    }

    /// Returns the behavior selected for the first workload failure.
    pub const fn failure_policy(&self) -> PhaseFailurePolicy {
        self.failure_policy
    }

    /// Reports whether successful completion requires confirmation before the
    /// next selected phase may start.
    pub const fn requires_confirmation(&self) -> bool {
        self.require_confirm
    }

    pub(crate) fn into_tasks(self) -> Vec<Task> {
        self.tasks
    }

    /// Returns one exact phase-local task by ID.
    pub fn task(&self, id: &TaskId) -> Option<&Task> {
        self.tasks.iter().find(|task| task.key.task_id() == id)
    }

    /// Returns the sole task matching an exact partial selector.
    pub fn unique_task_matching(&self, selector: &TaskSelector) -> Result<&Task, StudyError> {
        let mut matches = self.tasks.iter().filter(|task| selector.matches(task));
        let first = matches.next().ok_or_else(|| StudyError::TaskNotFound {
            selector: selector.to_string(),
        })?;
        if let Some(second) = matches.next() {
            return Err(StudyError::TaskSelectorAmbiguous {
                selector: selector.to_string(),
                first: first.key.to_string(),
                second: second.key.to_string(),
            });
        }
        Ok(first)
    }
}

/// Builder for one nonempty first-class phase.
pub struct PhaseBuilder {
    id: PhaseId,
    label: String,
    tasks: Vec<Task>,
    max_active_tasks: usize,
    prepared_task_queue_capacity: usize,
    delay_per_task: Option<Duration>,
    task_timeout: Option<Duration>,
    deadline_after: Option<Duration>,
    dependencies: Vec<PhaseId>,
    failure_policy: PhaseFailurePolicy,
    require_confirm: bool,
}

impl PhaseBuilder {
    /// Registers one task with this phase.
    pub fn task(mut self, task: Task) -> Self {
        self.tasks.push(task);
        self
    }

    /// Registers tasks in deterministic declaration order.
    pub fn tasks<I>(mut self, tasks: I) -> Self
    where
        I: IntoIterator<Item = Task>,
    {
        self.tasks.extend(tasks);
        self
    }

    /// Sets the later scheduler's phase-local active workload ceiling.
    pub fn max_active_tasks(mut self, maximum: usize) -> Self {
        self.max_active_tasks = maximum;
        self
    }

    /// Sets the later scheduler's prepared-work queue capacity.
    pub fn prepared_task_queue_capacity(mut self, capacity: usize) -> Self {
        self.prepared_task_queue_capacity = capacity;
        self
    }

    /// Sets a minimum start-to-start interval for executable tasks.
    ///
    /// This policy is optional. Without this call, tasks are admitted exactly
    /// as before. Completed tasks do not consume a delay rank.
    pub fn delay_per_task(mut self, delay: Duration) -> Self {
        self.delay_per_task = Some(delay);
        self
    }

    /// Sets the maximum elapsed time for each task after it starts.
    ///
    /// Expiration requests cooperative cancellation; Rust workloads cannot be
    /// forcibly terminated while they are blocked in user or system code.
    pub fn task_timeout(mut self, timeout: Duration) -> Self {
        self.task_timeout = Some(timeout);
        self
    }

    /// Sets a phase-wide deadline relative to the phase execution start.
    ///
    /// Once reached, no additional tasks start and active tasks receive a
    /// cooperative cancellation request.
    pub fn deadline_after(mut self, deadline: Duration) -> Self {
        self.deadline_after = Some(deadline);
        self
    }

    /// Declares one phase that must be satisfied before this phase starts.
    pub fn depends_on(mut self, dependency: impl Into<PhaseId>) -> Self {
        self.dependencies.push(dependency.into());
        self
    }

    /// Selects behavior after the first workload failure.
    pub fn failure_policy(mut self, policy: PhaseFailurePolicy) -> Self {
        self.failure_policy = policy;
        self
    }

    /// Requires the user to type `yes` before advancing to the next phase.
    ///
    /// The default is `false`. This setting has no effect when this phase is
    /// the final selected phase because there is no transition to confirm.
    pub fn require_confirm(mut self, require: bool) -> Self {
        self.require_confirm = require;
        self
    }

    /// Validates and creates one immutable nonempty phase.
    pub fn build(mut self) -> Result<Phase, StudyError> {
        if self.label.trim().is_empty() {
            return Err(StudyError::InvalidPhaseLabel { phase: self.id.0 });
        }
        if self.tasks.is_empty() {
            return Err(StudyError::EmptyPhase { phase: self.id.0 });
        }
        if self.max_active_tasks == 0 {
            return Err(StudyError::InvalidPhaseWorkloadLimit { phase: self.id.0 });
        }
        if self.prepared_task_queue_capacity == 0 {
            return Err(StudyError::InvalidPhaseQueueCapacity { phase: self.id.0 });
        }
        for (setting, duration) in [
            ("delay_per_task", self.delay_per_task),
            ("task_timeout", self.task_timeout),
            ("deadline_after", self.deadline_after),
        ] {
            if duration.is_some_and(|duration| {
                duration.is_zero() || Instant::now().checked_add(duration).is_none()
            }) {
                return Err(StudyError::InvalidPhaseTiming {
                    phase: self.id.0,
                    setting,
                });
            }
        }

        let mut ids = HashSet::with_capacity(self.tasks.len());
        for task in &mut self.tasks {
            if task.key.task_id().as_str().trim().is_empty() {
                return Err(StudyError::InvalidTaskId { phase: self.id.0 });
            }
            if task.category.trim().is_empty() {
                return Err(StudyError::InvalidTaskCategory {
                    task: task.key.task_id().to_string(),
                });
            }
            if !ids.insert(task.key.task_id().clone()) {
                return Err(StudyError::DuplicateTaskId {
                    phase: self.id.0,
                    task: task.key.task_id().to_string(),
                });
            }
            task.attach_to_phase(self.id);
        }

        let mut dependencies = HashSet::with_capacity(self.dependencies.len());
        for dependency in &self.dependencies {
            if !dependencies.insert(*dependency) || *dependency == self.id {
                return Err(StudyError::PhaseDependencyCycle { phase: self.id.0 });
            }
        }

        Ok(Phase {
            id: self.id,
            label: self.label.into(),
            tasks: self.tasks,
            max_active_tasks: self.max_active_tasks,
            prepared_task_queue_capacity: self.prepared_task_queue_capacity,
            delay_per_task: self.delay_per_task,
            task_timeout: self.task_timeout,
            deadline_after: self.deadline_after,
            dependencies: self.dependencies,
            failure_policy: self.failure_policy,
            require_confirm: self.require_confirm,
        })
    }
}

impl fmt::Debug for Phase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Phase")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("tasks", &self.tasks.len())
            .field("max_active_tasks", &self.max_active_tasks)
            .field(
                "prepared_task_queue_capacity",
                &self.prepared_task_queue_capacity,
            )
            .field("delay_per_task", &self.delay_per_task)
            .field("task_timeout", &self.task_timeout)
            .field("deadline_after", &self.deadline_after)
            .field("dependencies", &self.dependencies)
            .field("failure_policy", &self.failure_policy)
            .field("require_confirm", &self.require_confirm)
            .finish()
    }
}

impl fmt::Debug for PhaseBuilder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PhaseBuilder")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("tasks", &self.tasks.len())
            .field("max_active_tasks", &self.max_active_tasks)
            .field(
                "prepared_task_queue_capacity",
                &self.prepared_task_queue_capacity,
            )
            .field("delay_per_task", &self.delay_per_task)
            .field("task_timeout", &self.task_timeout)
            .field("deadline_after", &self.deadline_after)
            .field("dependencies", &self.dependencies)
            .field("require_confirm", &self.require_confirm)
            .finish_non_exhaustive()
    }
}

/// Exact partial selector over phase, task category, and metadata.
#[derive(Clone, Debug, Default)]
pub struct TaskSelector {
    phase: Option<PhaseId>,
    category: Option<Arc<str>>,
    metadata: Vec<(Box<str>, Value)>,
}

impl TaskSelector {
    /// Creates an unconstrained selector.
    pub fn new() -> Self {
        Self::default()
    }

    /// Constrains selection to one phase.
    pub fn phase(mut self, phase: impl Into<PhaseId>) -> Self {
        self.phase = Some(phase.into());
        self
    }

    /// Constrains selection to one task category.
    pub fn category(mut self, category: impl Into<String>) -> Self {
        self.category = Some(category.into().into());
        self
    }

    /// Adds or replaces one exact metadata constraint.
    pub fn metadata(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        let key = key.into();
        if let Some((_, current)) = self
            .metadata
            .iter_mut()
            .find(|(candidate, _)| candidate.as_ref() == key)
        {
            *current = value.into();
        } else {
            self.metadata.push((key.into_boxed_str(), value.into()));
        }
        self
    }

    pub(crate) fn matches(&self, task: &Task) -> bool {
        self.phase.is_none_or(|phase| task.key.phase == phase)
            && self
                .category
                .as_deref()
                .is_none_or(|category| task.category_name() == category)
            && self
                .metadata
                .iter()
                .all(|(key, value)| task.metadata_value(key) == Some(value))
    }
}

impl fmt::Display for TaskSelector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut fields = Vec::new();
        if let Some(phase) = self.phase {
            fields.push(format!("phase={phase}"));
        }
        if let Some(category) = &self.category {
            fields.push(format!("category={category}"));
        }
        fields.extend(
            self.metadata
                .iter()
                .map(|(key, value)| format!("{key}={}", compact_value(value))),
        );
        formatter.write_str(&fields.join(", "))
    }
}

fn compact_value(value: &Value) -> String {
    match value {
        Value::Array(values) => format!("<array:{}:{}>", values.len(), short_hash(value)),
        Value::Object(values) => format!("<object:{}:{}>", values.len(), short_hash(value)),
        _ => serde_json::to_string(value)
            .expect("serde_json::Value always serializes to valid compact JSON"),
    }
}

fn short_hash(value: &Value) -> String {
    let bytes = serde_json::to_vec(value)
        .expect("serde_json::Value always serializes to valid compact JSON");
    let digest = Sha256::digest(bytes);
    digest[..4]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join("")
}
