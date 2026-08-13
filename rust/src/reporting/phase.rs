//! First-class phase and task declarations observed by progress reporting.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::error::ReportingError;
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

#[derive(Clone)]
enum ParameterSource {
    Configuration(TaskConfig),
    Explicit(Arc<[(Box<str>, Value)]>),
}

/// First-class immutable task declaration owned by exactly one [`Phase`].
#[derive(Clone)]
pub struct Task {
    key: TaskKey,
    kind: Arc<str>,
    parameters: ParameterSource,
    display_kind: TaskDisplayKind,
    label: Arc<str>,
    display_keys: Option<Arc<[Box<str>]>>,
}

impl Task {
    /// Creates an explicit iterative task without project configuration.
    pub fn progress(id: impl Into<String>, kind: impl Into<String>) -> Self {
        Self::explicit(id, kind, TaskDisplayKind::Progress)
    }

    /// Creates an explicit one-shot activity task.
    pub fn activity(id: impl Into<String>, kind: impl Into<String>) -> Self {
        Self::explicit(id, kind, TaskDisplayKind::Activity)
    }

    fn explicit(
        id: impl Into<String>,
        kind: impl Into<String>,
        display_kind: TaskDisplayKind,
    ) -> Self {
        let id = TaskId::new(id);
        let kind: Arc<str> = kind.into().into();
        Self {
            key: TaskKey::new(0, id),
            label: Arc::clone(&kind),
            kind,
            parameters: ParameterSource::Explicit(Arc::from([])),
            display_kind,
            display_keys: None,
        }
    }

    /// Adds one exact structured parameter to an explicit task.
    ///
    /// Repeating a key replaces its earlier value, matching ordinary builder
    /// semantics. Configuration-derived tasks are immutable and reject this
    /// operation.
    pub fn with_parameter(
        mut self,
        key: impl Into<String>,
        value: impl Into<Value>,
    ) -> Result<Self, ReportingError> {
        let ParameterSource::Explicit(fields) = &self.parameters else {
            return Err(ReportingError::ConfiguredTaskParametersImmutable {
                task: self.key.to_string(),
            });
        };
        let key = key.into();
        if key.trim().is_empty() {
            return Err(ReportingError::InvalidTaskParameter { key });
        }
        let mut fields = fields.to_vec();
        if let Some((_, current)) = fields
            .iter_mut()
            .find(|(candidate, _)| candidate.as_ref() == key)
        {
            *current = value.into();
        } else {
            fields.push((key.into_boxed_str(), value.into()));
        }
        self.parameters = ParameterSource::Explicit(fields.into());
        Ok(self)
    }

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

    /// Returns the originating deterministic configuration ordinal, if any.
    pub fn configuration_ordinal(&self) -> Option<u64> {
        match &self.parameters {
            ParameterSource::Configuration(config) => Some(config.task_ordinal()),
            ParameterSource::Explicit(_) => None,
        }
    }

    /// Borrows the retained cheap task-configuration handle, if generated from
    /// a project or configuration.
    pub fn configuration(&self) -> Option<&TaskConfig> {
        match &self.parameters {
            ParameterSource::Configuration(config) => Some(config),
            ParameterSource::Explicit(_) => None,
        }
    }

    /// Borrows one fixed, swept, or explicit parameter by exact key.
    pub fn value(&self, key: &str) -> Option<&Value> {
        match &self.parameters {
            ParameterSource::Configuration(config) => config.value(key),
            ParameterSource::Explicit(fields) => fields
                .iter()
                .find_map(|(name, value)| (name.as_ref() == key).then_some(value)),
        }
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

    /// Iterates every fixed/swept or explicit parameter without cloning values.
    pub fn iter(&self) -> Box<dyn Iterator<Item = (&str, &Value)> + '_> {
        match &self.parameters {
            ParameterSource::Configuration(config) => Box::new(config.parameters().iter()),
            ParameterSource::Explicit(fields) => {
                Box::new(fields.iter().map(|(key, value)| (key.as_ref(), value)))
            }
        }
    }

    /// Borrows the exact keys used to generate the current label.
    pub fn display_keys(&self) -> Option<&[Box<str>]> {
        self.display_keys.as_deref()
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
#[derive(Clone, Debug)]
pub struct Phase {
    id: PhaseId,
    label: Arc<str>,
    tasks: Arc<[Task]>,
    max_concurrent_workloads: usize,
    queue_capacity: usize,
}

impl Phase {
    /// Begins declaring a phase. Tasks must be added before [`PhaseBuilder::build`].
    pub fn builder(id: impl Into<PhaseId>, label: impl Into<String>) -> PhaseBuilder {
        PhaseBuilder {
            id: id.into(),
            label: label.into(),
            tasks: Vec::new(),
            display_by_kind: HashMap::new(),
            max_concurrent_workloads: 1,
            queue_capacity: 1,
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
    pub const fn max_concurrent_workloads(&self) -> usize {
        self.max_concurrent_workloads
    }

    /// Returns the prepared-but-not-running workload ceiling.
    pub const fn queue_capacity(&self) -> usize {
        self.queue_capacity
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
    max_concurrent_workloads: usize,
    queue_capacity: usize,
}

impl PhaseBuilder {
    /// Adds one explicit task to this phase.
    pub fn task(mut self, task: Task) -> Self {
        self.tasks.push(task);
        self
    }

    /// Generates one iterative task per deterministic project configuration.
    pub fn progress_tasks_from_project(
        self,
        project: &ScientificProject,
        kind: impl Into<String>,
    ) -> Self {
        self.progress_tasks_from_configuration(project.configuration(), kind)
    }

    /// Generates one iterative task per deterministic lower-level configuration.
    pub fn progress_tasks_from_configuration(
        mut self,
        configuration: &ProjectConfig,
        kind: impl Into<String>,
    ) -> Self {
        self.extend_configuration(configuration, kind.into(), TaskDisplayKind::Progress);
        self
    }

    /// Generates one activity task per deterministic project configuration.
    pub fn activity_tasks_from_project(
        self,
        project: &ScientificProject,
        kind: impl Into<String>,
    ) -> Self {
        self.activity_tasks_from_configuration(project.configuration(), kind)
    }

    /// Generates one activity task per deterministic lower-level configuration.
    pub fn activity_tasks_from_configuration(
        mut self,
        configuration: &ProjectConfig,
        kind: impl Into<String>,
    ) -> Self {
        self.extend_configuration(configuration, kind.into(), TaskDisplayKind::Activity);
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
    pub fn max_concurrent_workloads(mut self, maximum: usize) -> Self {
        self.max_concurrent_workloads = maximum;
        self
    }

    /// Sets the later scheduler's prepared-work queue capacity.
    pub fn queue_capacity(mut self, capacity: usize) -> Self {
        self.queue_capacity = capacity;
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
        if self.max_concurrent_workloads == 0 {
            return Err(ReportingError::InvalidPhaseWorkloadLimit { phase: self.id.0 });
        }
        if self.queue_capacity == 0 {
            return Err(ReportingError::InvalidPhaseQueueCapacity { phase: self.id.0 });
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
            tasks: self.tasks.into(),
            max_concurrent_workloads: self.max_concurrent_workloads,
            queue_capacity: self.queue_capacity,
        })
    }

    fn extend_configuration(
        &mut self,
        configuration: &ProjectConfig,
        kind: String,
        display_kind: TaskDisplayKind,
    ) {
        for config in configuration.task_configs() {
            let id = TaskId::new(format!("{kind}:{}", config.task_ordinal()));
            self.tasks.push(Task {
                key: TaskKey::new(self.id, id),
                kind: kind.clone().into(),
                parameters: ParameterSource::Configuration(config),
                display_kind,
                label: kind.clone().into(),
                display_keys: None,
            });
        }
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

impl fmt::Debug for PhaseBuilder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PhaseBuilder")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("tasks", &self.tasks.len())
            .field("max_concurrent_workloads", &self.max_concurrent_workloads)
            .field("queue_capacity", &self.queue_capacity)
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

    fn matches(&self, task: &Task) -> bool {
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
