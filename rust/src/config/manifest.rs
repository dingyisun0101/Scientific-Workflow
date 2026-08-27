//! Validated Workflow-owned study manifest declarations.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Map, Value};

use super::error::ConfigError;
use super::input::ResolvedTaskInput;

/// Effective policy for isolated study replicates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReplicatePolicy {
    count: u64,
    scheduling: ReplicateScheduling,
    failure_policy: FailurePolicy,
    base_seed: Option<u64>,
}

impl ReplicatePolicy {
    /// Returns the positive number of replicate executions.
    pub const fn count(self) -> u64 {
        self.count
    }

    /// Returns the effective replicate scheduling policy.
    pub const fn scheduling(self) -> ReplicateScheduling {
        self.scheduling
    }

    /// Returns the effective response to a failed replicate.
    pub const fn failure_policy(self) -> FailurePolicy {
        self.failure_policy
    }

    /// Returns the optional study seed; deterministic work need not invent one.
    pub const fn base_seed(self) -> Option<u64> {
        self.base_seed
    }
}

/// Effective scheduling mode for isolated replicates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplicateScheduling {
    /// Complete one replicate before launching the next.
    Sequential,
    /// Launch eligible replicates concurrently.
    Parallel,
}

impl ReplicateScheduling {
    /// Returns the exact study-manifest spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sequential => "sequential",
            Self::Parallel => "parallel",
        }
    }
}

/// Effective failure propagation policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailurePolicy {
    /// Prevent further admission after the first failure.
    FailFast,
    /// Allow already-declared sibling work to finish.
    FinishAll,
}

impl FailurePolicy {
    /// Returns the exact study-manifest spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FailFast => "fail_fast",
            Self::FinishAll => "finish_all",
        }
    }
}

/// The validated Workflow-owned portion of `study.json`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StudyManifest {
    replicates: ReplicatePolicy,
}

impl StudyManifest {
    /// Returns the complete effective replicate policy.
    pub const fn replicate_policy(self) -> ReplicatePolicy {
        self.replicates
    }
}

/// One validated phase with resolved task inputs and effective policy.
#[derive(Clone, Debug)]
pub struct PhaseSpecification {
    pub(crate) name: Box<str>,
    pub(crate) dependencies: Box<[Box<str>]>,
    pub(crate) tasks: Box<[ResolvedTaskInput]>,
    pub(crate) max_concurrency: usize,
    pub(crate) start_interval: Duration,
    pub(crate) timeout: Option<Duration>,
    pub(crate) failure_policy: FailurePolicy,
}

impl PhaseSpecification {
    /// Returns the manifest phase key.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Iterates dependency phase keys in declaration order.
    pub fn dependencies(&self) -> impl ExactSizeIterator<Item = &str> {
        self.dependencies.iter().map(Box::as_ref)
    }

    /// Returns fully expanded task inputs in deterministic execution order.
    pub fn tasks(&self) -> &[ResolvedTaskInput] {
        &self.tasks
    }

    /// Returns the positive effective maximum number of active tasks.
    pub const fn max_concurrency(&self) -> usize {
        self.max_concurrency
    }

    /// Returns the effective interval between task admissions.
    pub const fn start_interval(&self) -> Duration {
        self.start_interval
    }

    /// Returns the optional effective phase timeout.
    pub const fn timeout(&self) -> Option<Duration> {
        self.timeout
    }

    /// Returns the effective sibling-task failure policy.
    pub const fn failure_policy(&self) -> FailurePolicy {
        self.failure_policy
    }
}

pub(crate) struct ParsedManifest {
    pub(crate) manifest: StudyManifest,
    pub(crate) phases: Vec<ParsedPhase>,
}

pub(crate) struct ParsedPhase {
    pub(crate) name: Box<str>,
    pub(crate) dependencies: Box<[Box<str>]>,
    pub(crate) tasks: Vec<ParsedTask>,
    pub(crate) max_concurrency: usize,
    pub(crate) start_interval: Duration,
    pub(crate) timeout: Option<Duration>,
    pub(crate) failure_policy: FailurePolicy,
}

pub(crate) struct ParsedTask {
    pub(crate) model: Box<str>,
    pub(crate) input: PathBuf,
    pub(crate) display_fields: Arc<[Box<str>]>,
    pub(crate) timeout: Option<Duration>,
}

pub(crate) fn parse(path: &Path, value: Value) -> Result<ParsedManifest, ConfigError> {
    let raw: RawStudy = serde_json::from_value(value)
        .map_err(|error| ConfigError::invalid(path, "/", error.to_string()))?;
    if raw.phases.is_empty() {
        return Err(ConfigError::invalid(
            path,
            "/phases",
            "at least one phase must be declared",
        ));
    }
    if raw.replicates.count == 0 {
        return Err(ConfigError::invalid(
            path,
            "/replicates/count",
            "replicate count must be positive",
        ));
    }

    let manifest = StudyManifest {
        replicates: ReplicatePolicy {
            count: raw.replicates.count,
            scheduling: raw.replicates.scheduling.into(),
            failure_policy: raw.replicates.failure_policy.into(),
            base_seed: raw.replicates.base_seed,
        },
    };

    let declared_names = raw.phases.keys().cloned().collect::<HashSet<_>>();
    let mut phases = Vec::with_capacity(raw.phases.len());
    for (name, value) in raw.phases {
        validate_identifier(path, &format!("/phases/{name}"), &name, "phase")?;
        let raw: RawPhase = serde_json::from_value(value).map_err(|error| {
            ConfigError::invalid(path, format!("/phases/{name}"), error.to_string())
        })?;
        if raw.tasks.is_empty() {
            return Err(ConfigError::invalid(
                path,
                format!("/phases/{name}/tasks"),
                "a phase must declare at least one task",
            ));
        }
        if raw.max_concurrency == 0 {
            return Err(ConfigError::invalid(
                path,
                format!("/phases/{name}/max_concurrency"),
                "maximum concurrency must be positive",
            ));
        }

        let mut dependencies = Vec::with_capacity(raw.after.len());
        let mut seen_dependencies = HashSet::with_capacity(raw.after.len());
        for dependency in raw.after {
            validate_identifier(
                path,
                &format!("/phases/{name}/after"),
                &dependency,
                "phase dependency",
            )?;
            if dependency == name {
                return Err(ConfigError::invalid(
                    path,
                    format!("/phases/{name}/after"),
                    "a phase cannot depend on itself",
                ));
            }
            if !declared_names.contains(&dependency) {
                return Err(ConfigError::UnknownDependency {
                    phase: name.clone(),
                    dependency,
                });
            }
            if !seen_dependencies.insert(dependency.clone()) {
                return Err(ConfigError::invalid(
                    path,
                    format!("/phases/{name}/after"),
                    format!("dependency `{dependency}` is repeated"),
                ));
            }
            dependencies.push(dependency.into_boxed_str());
        }

        let mut tasks = Vec::with_capacity(raw.tasks.len());
        for (index, task) in raw.tasks.into_iter().enumerate() {
            let pointer = format!("/phases/{name}/tasks/{index}");
            validate_identifier(path, &format!("{pointer}/model"), &task.model, "model")?;
            let mut display_fields = Vec::with_capacity(task.display.include.len());
            let mut seen_fields = HashSet::with_capacity(task.display.include.len());
            for field in task.display.include {
                validate_identifier(
                    path,
                    &format!("{pointer}/display/include"),
                    &field,
                    "display field",
                )?;
                if !seen_fields.insert(field.clone()) {
                    return Err(ConfigError::invalid(
                        path,
                        format!("{pointer}/display/include"),
                        format!("display field `{field}` is repeated"),
                    ));
                }
                display_fields.push(field.into_boxed_str());
            }
            tasks.push(ParsedTask {
                model: task.model.into_boxed_str(),
                input: task.input,
                display_fields: display_fields.into(),
                timeout: task.timeout_ms.map(Duration::from_millis),
            });
        }

        phases.push(ParsedPhase {
            name: name.into_boxed_str(),
            dependencies: dependencies.into_boxed_slice(),
            tasks,
            max_concurrency: raw.max_concurrency,
            start_interval: Duration::from_millis(raw.start_interval_ms),
            timeout: raw.timeout_ms.map(Duration::from_millis),
            failure_policy: raw.failure_policy.into(),
        });
    }

    validate_acyclic(path, &phases)?;
    Ok(ParsedManifest { manifest, phases })
}

fn validate_identifier(
    path: &Path,
    pointer: &str,
    value: &str,
    kind: &str,
) -> Result<(), ConfigError> {
    if value.is_empty() || value.trim() != value {
        return Err(ConfigError::invalid(
            path,
            pointer,
            format!("{kind} must be nonempty and contain no surrounding whitespace"),
        ));
    }
    Ok(())
}

fn validate_acyclic(path: &Path, phases: &[ParsedPhase]) -> Result<(), ConfigError> {
    fn visit(
        index: usize,
        phases: &[ParsedPhase],
        by_name: &std::collections::HashMap<&str, usize>,
        states: &mut [u8],
    ) -> Option<String> {
        if states[index] == 1 {
            return Some(phases[index].name.to_string());
        }
        if states[index] == 2 {
            return None;
        }
        states[index] = 1;
        for dependency in &phases[index].dependencies {
            let dependency = by_name[dependency.as_ref()];
            if let Some(phase) = visit(dependency, phases, by_name, states) {
                return Some(phase);
            }
        }
        states[index] = 2;
        None
    }

    let by_name = phases
        .iter()
        .enumerate()
        .map(|(index, phase)| (phase.name.as_ref(), index))
        .collect::<std::collections::HashMap<_, _>>();
    let mut states = vec![0_u8; phases.len()];
    for index in 0..phases.len() {
        if let Some(phase) = visit(index, phases, &by_name, &mut states) {
            return Err(ConfigError::invalid(
                path,
                "/phases",
                format!("phase dependency cycle includes `{phase}`"),
            ));
        }
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStudy {
    #[serde(default)]
    replicates: RawReplicatePolicy,
    phases: Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawReplicatePolicy {
    count: u64,
    scheduling: RawReplicateScheduling,
    failure_policy: RawFailurePolicy,
    base_seed: Option<u64>,
}

impl Default for RawReplicatePolicy {
    fn default() -> Self {
        Self {
            count: 1,
            scheduling: RawReplicateScheduling::default(),
            failure_policy: RawFailurePolicy::default(),
            base_seed: None,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPhase {
    #[serde(default)]
    after: Vec<String>,
    tasks: Vec<RawTask>,
    #[serde(default = "default_max_concurrency")]
    max_concurrency: usize,
    #[serde(default)]
    start_interval_ms: u64,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    failure_policy: RawFailurePolicy,
}

const fn default_max_concurrency() -> usize {
    1
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTask {
    model: String,
    input: PathBuf,
    #[serde(default)]
    display: RawDisplay,
    #[serde(default)]
    timeout_ms: Option<u64>,
}

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawDisplay {
    include: Vec<String>,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RawReplicateScheduling {
    #[default]
    Sequential,
    Parallel,
}

impl From<RawReplicateScheduling> for ReplicateScheduling {
    fn from(value: RawReplicateScheduling) -> Self {
        match value {
            RawReplicateScheduling::Sequential => Self::Sequential,
            RawReplicateScheduling::Parallel => Self::Parallel,
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RawFailurePolicy {
    #[default]
    FailFast,
    FinishAll,
}

impl From<RawFailurePolicy> for FailurePolicy {
    fn from(value: RawFailurePolicy) -> Self {
        match value {
            RawFailurePolicy::FailFast => Self::FailFast,
            RawFailurePolicy::FinishAll => Self::FinishAll,
        }
    }
}
