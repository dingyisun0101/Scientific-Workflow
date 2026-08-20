//! First-class phase and task declarations owned by the runtime.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::error::ReportingError;
use super::task::{TaskContext, TaskResult, Workload};
use crate::configuration::{ProjectConfig, TaskConfig};
use crate::project::ScientificProject;

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

/// Exact runtime task lookup key.
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
pub enum TaskDisplayKind {
    /// Iterative work with an optional target supplied when execution starts.
    Progress,
    /// One-shot lifecycle work without an artificial iteration counter.
    Activity,
}

/// First-class immutable task declaration owned by exactly one [`Phase`].
pub struct Task {
    key: TaskKey,
    kind: Arc<str>,
    configuration: TaskConfig,
    display_kind: TaskDisplayKind,
    label: Arc<str>,
    display_keys: Option<Arc<[Box<str>]>>,
    workload: Option<Workload>,
    reused: bool,
}

impl Task {
    /// Returns the exact phase-qualified task key.
    pub fn key(&self) -> &TaskKey {
        &self.key
    }

    /// Returns the phase-local task ID.
    pub fn id(&self) -> &TaskId {
        self.key.task_id()
    }

    /// Returns the task kind/namespace.
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// Returns the automatically generated display label.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns whether the task renders iterative progress or activity status.
    pub const fn display_kind(&self) -> TaskDisplayKind {
        self.display_kind
    }

    /// Returns the originating deterministic configuration ordinal.
    pub fn configuration_ordinal(&self) -> u64 {
        self.configuration.task_ordinal()
    }

    /// Borrows the retained cheap task-configuration handle.
    pub fn configuration(&self) -> &TaskConfig {
        &self.configuration
    }

    /// Borrows one fixed or swept parameter by exact key.
    pub fn value(&self, key: &str) -> Option<&Value> {
        self.configuration.value(key)
    }

    /// Borrows one required task parameter.
    pub fn require_value(&self, key: &str) -> Result<&Value, ReportingError> {
        self.value(key)
            .ok_or_else(|| ReportingError::UnknownManagedTaskParameter {
                task: self.key.to_string(),
                key: key.to_owned(),
            })
    }

    /// Decodes one required task parameter without first cloning its JSON tree.
    pub fn decode_value<T>(&self, key: &str) -> Result<T, ReportingError>
    where
        T: DeserializeOwned,
    {
        T::deserialize(self.require_value(key)?).map_err(|source| {
            ReportingError::DecodeManagedTaskParameter {
                task: self.key.to_string(),
                key: key.to_owned(),
                source,
            }
        })
    }

    /// Iterates every fixed or swept parameter without cloning values.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> + '_ {
        self.configuration.parameters().iter()
    }

    /// Borrows the exact keys used to generate the current label.
    pub fn display_keys(&self) -> Option<&[Box<str>]> {
        self.display_keys.as_deref()
    }

    pub(crate) fn take_workload(&mut self) -> Option<Workload> {
        self.workload.take()
    }

    pub(crate) fn has_workload(&self) -> bool {
        self.workload.is_some()
    }

    pub(crate) const fn is_reused(&self) -> bool {
        self.reused
    }

    fn attach_to_phase(&mut self, phase: PhaseId) {
        self.key.phase = phase;
    }

    fn parameter_keys(&self) -> Vec<&str> {
        self.iter().map(|(key, _)| key).collect()
    }

    fn regenerate_label(&mut self, keys: Arc<[Box<str>]>, include_id: bool) {
        let mut parts = Vec::with_capacity(keys.len() + usize::from(include_id));
        for key in keys.iter() {
            let value = self
                .value(key)
                .expect("validated display parameters resolve for every applicable task");
            parts.push(format!("{key}={}", compact_value(value)));
        }
        if include_id {
            parts.push(format!("id={}", self.key.task_id()));
        }
        self.label = if parts.is_empty() {
            Arc::clone(&self.kind)
        } else {
            format!("{} {}", self.kind, parts.join(" ")).into()
        };
        self.display_keys = Some(keys);
    }
}

impl fmt::Debug for Task {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Task")
            .field("key", &self.key)
            .field("kind", &self.kind)
            .field("label", &self.label)
            .field("display_kind", &self.display_kind)
            .field("parameters", &self.iter().count())
            .finish_non_exhaustive()
    }
}

/// Immutable nonempty execution phase and reporter section.
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
            display_by_kind: HashMap::new(),
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
    pub fn unique_task_matching(&self, selector: &TaskSelector) -> Result<&Task, ReportingError> {
        let mut matches = self.tasks.iter().filter(|task| selector.matches(task));
        let first = matches
            .next()
            .ok_or_else(|| ReportingError::ManagedTaskNotFound {
                selector: selector.to_string(),
            })?;
        if let Some(second) = matches.next() {
            return Err(ReportingError::ManagedTaskSelectorAmbiguous {
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
    display_by_kind: HashMap<String, Vec<String>>,
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
    /// Adds one iterative workload backed by an already selected configuration.
    pub fn progress_workload<W>(
        mut self,
        configuration: TaskConfig,
        kind: impl Into<String>,
        workload: W,
    ) -> Self
    where
        W: FnOnce(&TaskContext) -> TaskResult + Send + 'static,
    {
        self.push_configuration_task(
            configuration,
            kind.into(),
            TaskDisplayKind::Progress,
            Some(Box::new(workload)),
            false,
        );
        self
    }

    /// Adds one activity workload backed by an already selected configuration.
    pub fn activity_workload<W>(
        mut self,
        configuration: TaskConfig,
        kind: impl Into<String>,
        workload: W,
    ) -> Self
    where
        W: FnOnce(&TaskContext) -> TaskResult + Send + 'static,
    {
        self.push_configuration_task(
            configuration,
            kind.into(),
            TaskDisplayKind::Activity,
            Some(Box::new(workload)),
            false,
        );
        self
    }

    /// Adds one application-verified reused activity task.
    pub fn reused_activity(mut self, configuration: TaskConfig, kind: impl Into<String>) -> Self {
        self.push_configuration_task(
            configuration,
            kind.into(),
            TaskDisplayKind::Activity,
            None,
            true,
        );
        self
    }

    /// Generates iterative tasks that share one thread-safe workload.
    ///
    /// Use this concise form when the callable can borrow shared captured
    /// state. Use [`Self::progress_workloads_from_project`] when each task must
    /// instead own a separately constructed resource.
    pub fn progress_tasks_from_project<W>(
        self,
        project: &ScientificProject,
        kind: impl Into<String>,
        workload: W,
    ) -> Self
    where
        W: Fn(&TaskContext) -> TaskResult + Send + Sync + 'static,
    {
        self.progress_tasks_from_configuration(project.configuration(), kind, workload)
    }

    /// Generates iterative tasks from configuration with one shared workload.
    pub fn progress_tasks_from_configuration<W>(
        mut self,
        configuration: &ProjectConfig,
        kind: impl Into<String>,
        workload: W,
    ) -> Self
    where
        W: Fn(&TaskContext) -> TaskResult + Send + Sync + 'static,
    {
        self.extend_shared_workload(
            configuration,
            kind.into(),
            TaskDisplayKind::Progress,
            workload,
        );
        self
    }

    /// Generates executable iterative tasks from every project configuration.
    pub fn progress_workloads_from_project<F, W>(
        self,
        project: &ScientificProject,
        kind: impl Into<String>,
        factory: F,
    ) -> Self
    where
        F: Fn(&TaskConfig) -> W,
        W: FnOnce(&TaskContext) -> TaskResult + Send + 'static,
    {
        self.progress_workloads_from_configuration(project.configuration(), kind, factory)
    }

    /// Generates executable iterative tasks from lower-level configuration.
    pub fn progress_workloads_from_configuration<F, W>(
        mut self,
        configuration: &ProjectConfig,
        kind: impl Into<String>,
        factory: F,
    ) -> Self
    where
        F: Fn(&TaskConfig) -> W,
        W: FnOnce(&TaskContext) -> TaskResult + Send + 'static,
    {
        self.extend_configuration_workloads(
            configuration,
            kind.into(),
            TaskDisplayKind::Progress,
            factory,
        );
        self
    }

    /// Generates activity tasks that share one thread-safe workload.
    ///
    /// Use [`Self::activity_workloads_from_project`] when each task needs a
    /// separately constructed single-owner resource.
    pub fn activity_tasks_from_project<W>(
        self,
        project: &ScientificProject,
        kind: impl Into<String>,
        workload: W,
    ) -> Self
    where
        W: Fn(&TaskContext) -> TaskResult + Send + Sync + 'static,
    {
        self.activity_tasks_from_configuration(project.configuration(), kind, workload)
    }

    /// Generates activity tasks from configuration with one shared workload.
    pub fn activity_tasks_from_configuration<W>(
        mut self,
        configuration: &ProjectConfig,
        kind: impl Into<String>,
        workload: W,
    ) -> Self
    where
        W: Fn(&TaskContext) -> TaskResult + Send + Sync + 'static,
    {
        self.extend_shared_workload(
            configuration,
            kind.into(),
            TaskDisplayKind::Activity,
            workload,
        );
        self
    }

    /// Generates executable activity tasks from every project configuration.
    pub fn activity_workloads_from_project<F, W>(
        self,
        project: &ScientificProject,
        kind: impl Into<String>,
        factory: F,
    ) -> Self
    where
        F: Fn(&TaskConfig) -> W,
        W: FnOnce(&TaskContext) -> TaskResult + Send + 'static,
    {
        self.activity_workloads_from_configuration(project.configuration(), kind, factory)
    }

    /// Generates executable activity tasks from lower-level configuration.
    pub fn activity_workloads_from_configuration<F, W>(
        mut self,
        configuration: &ProjectConfig,
        kind: impl Into<String>,
        factory: F,
    ) -> Self
    where
        F: Fn(&TaskConfig) -> W,
        W: FnOnce(&TaskContext) -> TaskResult + Send + 'static,
    {
        self.extend_configuration_workloads(
            configuration,
            kind.into(),
            TaskDisplayKind::Activity,
            factory,
        );
        self
    }

    /// Selects the exact parameter subset used to label one task kind.
    pub fn display_tasks_by<I, S>(mut self, kind: impl Into<String>, keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.display_by_kind
            .insert(kind.into(), keys.into_iter().map(Into::into).collect());
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
    /// as before. Reused tasks do not consume a delay rank.
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
    pub fn build(mut self) -> Result<Phase, ReportingError> {
        if self.label.trim().is_empty() {
            return Err(ReportingError::InvalidPhaseLabel { phase: self.id.0 });
        }
        if self.tasks.is_empty() {
            return Err(ReportingError::EmptyPhase { phase: self.id.0 });
        }
        if self.max_active_tasks == 0 {
            return Err(ReportingError::InvalidPhaseWorkloadLimit { phase: self.id.0 });
        }
        if self.prepared_task_queue_capacity == 0 {
            return Err(ReportingError::InvalidPhaseQueueCapacity { phase: self.id.0 });
        }
        for (setting, duration) in [
            ("delay_per_task", self.delay_per_task),
            ("task_timeout", self.task_timeout),
            ("deadline_after", self.deadline_after),
        ] {
            if duration.is_some_and(|duration| {
                duration.is_zero() || Instant::now().checked_add(duration).is_none()
            }) {
                return Err(ReportingError::InvalidPhaseTiming {
                    phase: self.id.0,
                    setting,
                });
            }
        }

        let mut ids = HashSet::with_capacity(self.tasks.len());
        for task in &mut self.tasks {
            if task.key.task_id().as_str().trim().is_empty() {
                return Err(ReportingError::InvalidManagedTaskId { phase: self.id.0 });
            }
            if task.kind.trim().is_empty() {
                return Err(ReportingError::InvalidManagedTaskKind {
                    task: task.key.task_id().to_string(),
                });
            }
            if !ids.insert(task.key.task_id().clone()) {
                return Err(ReportingError::DuplicateManagedTaskId {
                    phase: self.id.0,
                    task: task.key.task_id().to_string(),
                });
            }
            task.attach_to_phase(self.id);
        }

        let mut dependencies = HashSet::with_capacity(self.dependencies.len());
        for dependency in &self.dependencies {
            if !dependencies.insert(*dependency) || *dependency == self.id {
                return Err(ReportingError::PhaseDependencyCycle { phase: self.id.0 });
            }
        }

        let kinds: HashSet<_> = self
            .tasks
            .iter()
            .map(|task| task.kind.to_string())
            .collect();
        for requested in self.display_by_kind.keys() {
            if !kinds.contains(requested) {
                return Err(ReportingError::UnknownManagedTaskKind {
                    kind: requested.clone(),
                });
            }
        }
        for kind in kinds {
            self.generate_labels(&kind)?;
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

    fn extend_configuration_workloads<F, W>(
        &mut self,
        configuration: &ProjectConfig,
        kind: String,
        display_kind: TaskDisplayKind,
        factory: F,
    ) where
        F: Fn(&TaskConfig) -> W,
        W: FnOnce(&TaskContext) -> TaskResult + Send + 'static,
    {
        for config in configuration.task_configs() {
            let workload = factory(&config);
            self.push_configuration_task(
                config,
                kind.clone(),
                display_kind,
                Some(Box::new(workload)),
                false,
            );
        }
    }

    fn extend_shared_workload<W>(
        &mut self,
        configuration: &ProjectConfig,
        kind: String,
        display_kind: TaskDisplayKind,
        workload: W,
    ) where
        W: Fn(&TaskContext) -> TaskResult + Send + Sync + 'static,
    {
        let workload = Arc::new(workload);
        for config in configuration.task_configs() {
            let workload = Arc::clone(&workload);
            self.push_configuration_task(
                config,
                kind.clone(),
                display_kind,
                Some(Box::new(move |context| workload(context))),
                false,
            );
        }
    }

    fn push_configuration_task(
        &mut self,
        configuration: TaskConfig,
        kind: String,
        display_kind: TaskDisplayKind,
        workload: Option<Workload>,
        reused: bool,
    ) {
        let id = TaskId::new(format!("{kind}:{}", configuration.task_ordinal()));
        self.tasks.push(Task {
            key: TaskKey::new(self.id, id),
            kind: kind.clone().into(),
            configuration,
            display_kind,
            label: kind.into(),
            display_keys: None,
            workload,
            reused,
        });
    }

    fn generate_labels(&mut self, kind: &str) -> Result<(), ReportingError> {
        let positions: Vec<_> = self
            .tasks
            .iter()
            .enumerate()
            .filter_map(|(position, task)| (task.kind() == kind).then_some(position))
            .collect();
        validate_parameter_keys(&self.tasks, &positions, kind)?;
        let requested = self.display_by_kind.get(kind);
        let keys = if let Some(requested) = requested {
            validate_display_keys(&self.tasks, &positions, requested)?
        } else {
            varying_keys(&self.tasks, &positions)
        };
        let keys: Arc<[Box<str>]> = keys.into_iter().map(String::into_boxed_str).collect();
        let include_id = keys.is_empty() && positions.len() > 1;
        let mut labels = HashMap::<String, TaskKey>::with_capacity(positions.len());
        for position in positions {
            self.tasks[position].regenerate_label(Arc::clone(&keys), include_id);
            let task = &self.tasks[position];
            if let Some(first) = labels.insert(task.label().to_owned(), task.key().clone()) {
                return Err(ReportingError::ManagedTaskDisplayCollision {
                    label: task.label().to_owned(),
                    first: first.to_string(),
                    second: task.key().to_string(),
                });
            }
        }
        Ok(())
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

/// Exact partial selector over phase, task kind, and structured parameters.
#[derive(Clone, Debug, Default)]
pub struct TaskSelector {
    phase: Option<PhaseId>,
    kind: Option<Arc<str>>,
    parameters: Vec<(Box<str>, Value)>,
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

    /// Constrains selection to one task kind/namespace.
    pub fn kind(mut self, kind: impl Into<String>) -> Self {
        self.kind = Some(kind.into().into());
        self
    }

    /// Adds or replaces one exact parameter constraint.
    pub fn parameter(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        let key = key.into();
        if let Some((_, current)) = self
            .parameters
            .iter_mut()
            .find(|(candidate, _)| candidate.as_ref() == key)
        {
            *current = value.into();
        } else {
            self.parameters.push((key.into_boxed_str(), value.into()));
        }
        self
    }

    pub(crate) fn matches(&self, task: &Task) -> bool {
        self.phase.is_none_or(|phase| task.key.phase == phase)
            && self.kind.as_deref().is_none_or(|kind| task.kind() == kind)
            && self
                .parameters
                .iter()
                .all(|(key, value)| task.value(key) == Some(value))
    }
}

impl fmt::Display for TaskSelector {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut fields = Vec::new();
        if let Some(phase) = self.phase {
            fields.push(format!("phase={phase}"));
        }
        if let Some(kind) = &self.kind {
            fields.push(format!("kind={kind}"));
        }
        fields.extend(
            self.parameters
                .iter()
                .map(|(key, value)| format!("{key}={}", compact_value(value))),
        );
        formatter.write_str(&fields.join(", "))
    }
}

fn varying_keys(tasks: &[Task], positions: &[usize]) -> Vec<String> {
    let Some(&first) = positions.first() else {
        return Vec::new();
    };
    tasks[first]
        .parameter_keys()
        .into_iter()
        .filter(|key| {
            let first_value = tasks[first].value(key);
            positions
                .iter()
                .skip(1)
                .any(|&position| tasks[position].value(key) != first_value)
        })
        .map(str::to_owned)
        .collect()
}

fn validate_parameter_keys(
    tasks: &[Task],
    positions: &[usize],
    kind: &str,
) -> Result<(), ReportingError> {
    let Some(&first) = positions.first() else {
        return Ok(());
    };
    let expected: HashSet<_> = tasks[first].parameter_keys().into_iter().collect();
    for &position in positions.iter().skip(1) {
        let actual: HashSet<_> = tasks[position].parameter_keys().into_iter().collect();
        if actual != expected {
            return Err(ReportingError::InconsistentManagedTaskParameters {
                kind: kind.to_owned(),
                first: tasks[first].key().to_string(),
                second: tasks[position].key().to_string(),
            });
        }
    }
    Ok(())
}

fn validate_display_keys(
    tasks: &[Task],
    positions: &[usize],
    requested: &[String],
) -> Result<Vec<String>, ReportingError> {
    let mut seen = HashSet::with_capacity(requested.len());
    for key in requested {
        if !seen.insert(key.as_str()) {
            return Err(ReportingError::DuplicateIdentityParameter { key: key.clone() });
        }
        if positions
            .iter()
            .any(|&position| tasks[position].value(key).is_none())
        {
            return Err(ReportingError::UnknownIdentityParameter { key: key.clone() });
        }
    }
    Ok(requested.to_vec())
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
