//! Validated Workflow-owned study manifest declarations.

use std::collections::{BTreeMap, HashSet};
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Map, Value};

use super::document::child_pointer;
use super::error::ConfigError;
use super::parameters::ResolvedTask;
use super::python::PythonTaskDeclaration;

pub(crate) const WORKFLOW_SCHEMA_VERSION: u64 = 1;

/// Effective policy for isolated study replicates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ReplicatePolicy {
    count: u64,
    scheduling: ReplicateScheduling,
    failure_policy: FailurePolicy,
}

impl ReplicatePolicy {
    /// Returns the positive number of replicate executions.
    pub(crate) const fn count(self) -> u64 {
        self.count
    }

    /// Returns the effective replicate scheduling policy.
    pub(crate) const fn scheduling(self) -> ReplicateScheduling {
        self.scheduling
    }

    /// Returns the effective response to a failed replicate.
    pub(crate) const fn failure_policy(self) -> FailurePolicy {
        self.failure_policy
    }
}

/// Effective scheduling mode for isolated replicates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReplicateScheduling {
    /// Complete one replicate before launching the next.
    Sequential,
    /// Launch eligible replicates concurrently.
    Parallel,
}

/// Effective failure propagation policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FailurePolicy {
    /// Prevent further admission after the first failure.
    FailFast,
    /// Allow already-declared sibling work to finish.
    FinishAll,
}

/// The validated Workflow-owned portion of `wf_configs/study.json`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct StudyManifest {
    workflow_schema: u64,
    threads: usize,
    master_seed: Option<u64>,
    replicates: ReplicatePolicy,
    persistence: PersistenceSpecification,
}

impl StudyManifest {
    /// Returns the validated project-configuration schema generation.
    pub(crate) const fn workflow_schema(self) -> u64 {
        self.workflow_schema
    }

    /// Returns the required study-wide compute worker count.
    pub(crate) const fn threads(self) -> usize {
        self.threads
    }

    /// Returns the optional deterministic seed for the complete study.
    pub(crate) const fn master_seed(self) -> Option<u64> {
        self.master_seed
    }

    /// Returns the complete effective replicate policy.
    pub(crate) const fn replicate_policy(self) -> ReplicatePolicy {
        self.replicates
    }

    /// Returns the complete effective local-persistence settings.
    pub(crate) const fn persistence(self) -> PersistenceSpecification {
        self.persistence
    }
}

/// Effective persistence settings parsed from the study manifest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PersistenceSpecification {
    chunk_target_bytes: NonZeroU64,
    queue_capacity_bytes: NonZeroU64,
}

impl PersistenceSpecification {
    /// Returns the positive approximate local chunk rollover target.
    pub(crate) const fn chunk_target_bytes(self) -> NonZeroU64 {
        self.chunk_target_bytes
    }

    /// Returns the positive per-stream backpressure capacity.
    pub(crate) const fn queue_capacity_bytes(self) -> NonZeroU64 {
        self.queue_capacity_bytes
    }
}

/// One validated phase with resolved generic tasks and effective policy.
#[derive(Clone, Debug)]
pub(crate) struct PhaseSpecification {
    pub(crate) name: Box<str>,
    pub(crate) dependencies: Box<[Box<str>]>,
    pub(crate) tasks: Box<[ResolvedTask]>,
    pub(crate) max_concurrency: usize,
    pub(crate) start_interval: Duration,
    pub(crate) timeout: Option<Duration>,
    pub(crate) failure_policy: FailurePolicy,
}

impl PhaseSpecification {
    /// Returns the manifest phase key.
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    /// Iterates dependency phase keys in declaration order.
    pub(crate) fn dependencies(&self) -> impl ExactSizeIterator<Item = &str> {
        self.dependencies.iter().map(Box::as_ref)
    }

    /// Returns resolved tasks in deterministic execution order.
    pub(crate) fn tasks(&self) -> &[ResolvedTask] {
        &self.tasks
    }

    /// Returns the positive effective maximum number of active tasks.
    pub(crate) const fn max_concurrency(&self) -> usize {
        self.max_concurrency
    }

    /// Returns the effective interval between task admissions.
    pub(crate) const fn start_interval(&self) -> Duration {
        self.start_interval
    }

    /// Returns the optional effective phase timeout.
    pub(crate) const fn timeout(&self) -> Option<Duration> {
        self.timeout
    }

    /// Returns the effective sibling-task failure policy.
    pub(crate) const fn failure_policy(&self) -> FailurePolicy {
        self.failure_policy
    }
}

pub(crate) struct ParsedManifest {
    pub(crate) manifest: StudyManifest,
    pub(crate) state_paths: BTreeMap<Box<str>, PathBuf>,
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

pub(crate) enum ParsedTask {
    ExecutionUnit {
        execution_unit: Box<str>,
        state: Option<Box<str>>,
        timeout: Option<Duration>,
    },
    Program {
        program: PathBuf,
        args: Box<[Box<str>]>,
        seed_purpose: Option<Box<str>>,
        timeout: Option<Duration>,
    },
    Python {
        declaration: PythonTaskDeclaration,
        seed_purpose: Option<Box<str>>,
        timeout: Option<Duration>,
    },
}

pub(crate) fn parse(path: &Path, value: Value) -> Result<ParsedManifest, ConfigError> {
    let raw: RawStudy = serde_json::from_value(value)
        .map_err(|error| ConfigError::invalid(path, "/", error.to_string()))?;
    if raw.workflow_schema != WORKFLOW_SCHEMA_VERSION {
        return Err(ConfigError::invalid(
            path,
            "/workflow_schema",
            format!(
                "unsupported Workflow configuration schema {}; this release requires schema {}",
                raw.workflow_schema, WORKFLOW_SCHEMA_VERSION
            ),
        ));
    }
    if raw.phases.is_empty() {
        return Err(ConfigError::invalid(
            path,
            "/phases",
            "at least one phase must be declared",
        ));
    }
    if raw.threads == 0 {
        return Err(ConfigError::invalid(
            path,
            "/threads",
            "study thread count must be positive",
        ));
    }
    if raw.replicates.count == 0 {
        return Err(ConfigError::invalid(
            path,
            "/replicates/count",
            "replicate count must be positive",
        ));
    }
    let chunk_target_bytes = megabytes_to_bytes(
        path,
        "/persistence/chunk_target_mb",
        raw.persistence.chunk_target_mb,
    )?;
    let queue_capacity_bytes = megabytes_to_bytes(
        path,
        "/persistence/queue_capacity_mb",
        raw.persistence.queue_capacity_mb,
    )?;

    let manifest = StudyManifest {
        workflow_schema: raw.workflow_schema,
        threads: raw.threads,
        master_seed: raw.seed,
        replicates: ReplicatePolicy {
            count: raw.replicates.count,
            scheduling: raw.replicates.scheduling.into(),
            failure_policy: raw.replicates.failure_policy.into(),
        },
        persistence: PersistenceSpecification {
            chunk_target_bytes,
            queue_capacity_bytes,
        },
    };

    let mut state_paths = BTreeMap::new();
    for (name, state_path) in raw.paths.states {
        let pointer = child_pointer("/paths/states", &name);
        validate_identifier(path, &pointer, &name, "state")?;
        if state_path.as_os_str().is_empty() {
            return Err(ConfigError::invalid(
                path,
                &pointer,
                "state schema path must be nonempty",
            ));
        }
        if state_path.is_absolute() {
            return Err(ConfigError::invalid(
                path,
                &pointer,
                "state schema path must be relative to the project root",
            ));
        }
        if state_path.extension().and_then(std::ffi::OsStr::to_str) != Some("json") {
            return Err(ConfigError::invalid(
                path,
                pointer,
                "state schema path must use the `.json` extension",
            ));
        }
        state_paths.insert(name.into_boxed_str(), state_path);
    }

    let declared_names = raw.phases.keys().cloned().collect::<HashSet<_>>();
    let mut phases = Vec::with_capacity(raw.phases.len());
    for (name, value) in raw.phases {
        let phase_pointer = child_pointer("/phases", &name);
        validate_identifier(path, &phase_pointer, &name, "phase")?;
        let raw: RawPhase = serde_json::from_value(value)
            .map_err(|error| ConfigError::invalid(path, &phase_pointer, error.to_string()))?;
        if raw.tasks.is_empty() {
            return Err(ConfigError::invalid(
                path,
                format!("{phase_pointer}/tasks"),
                "a phase must declare at least one task",
            ));
        }
        if raw.max_concurrency == 0 {
            return Err(ConfigError::invalid(
                path,
                format!("{phase_pointer}/max_concurrency"),
                "maximum concurrency must be positive",
            ));
        }

        let mut dependencies = Vec::with_capacity(raw.after.len());
        let mut seen_dependencies = HashSet::with_capacity(raw.after.len());
        for dependency in raw.after {
            validate_identifier(
                path,
                &format!("{phase_pointer}/after"),
                &dependency,
                "phase dependency",
            )?;
            if dependency == name {
                return Err(ConfigError::invalid(
                    path,
                    format!("{phase_pointer}/after"),
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
                    format!("{phase_pointer}/after"),
                    format!("dependency `{dependency}` is repeated"),
                ));
            }
            dependencies.push(dependency.into_boxed_str());
        }

        let mut tasks = Vec::with_capacity(raw.tasks.len());
        for (index, task) in raw.tasks.into_iter().enumerate() {
            let pointer = format!("{phase_pointer}/tasks/{index}");
            let timeout = task.timeout_ms.map(Duration::from_millis);
            let seed_purpose = task
                .seed
                .map(|seed| {
                    validate_identifier(
                        path,
                        &format!("{pointer}/seed/purpose"),
                        &seed.purpose,
                        "seed purpose",
                    )?;
                    if manifest.master_seed().is_none() {
                        return Err(ConfigError::invalid(
                            path,
                            format!("{pointer}/seed"),
                            "a program seed request requires top-level `seed`",
                        ));
                    }
                    Ok(seed.purpose.into_boxed_str())
                })
                .transpose()?;
            match (task.execution_unit, task.program, task.python) {
                (Some(execution_unit), None, None)
                    if task.args.is_empty() && seed_purpose.is_none() =>
                {
                    validate_identifier(
                        path,
                        &format!("{pointer}/execution_unit"),
                        &execution_unit,
                        "execution unit",
                    )?;
                    if let Some(state) = task.state.as_deref() {
                        validate_identifier(path, &format!("{pointer}/state"), state, "state")?;
                    }
                    tasks.push(ParsedTask::ExecutionUnit {
                        execution_unit: execution_unit.into_boxed_str(),
                        state: task.state.map(String::into_boxed_str),
                        timeout,
                    });
                }
                (None, Some(program), None) if task.state.is_none() => {
                    if program.as_os_str().is_empty() {
                        return Err(ConfigError::invalid(
                            path,
                            format!("{pointer}/program"),
                            "program path must be nonempty",
                        ));
                    }
                    tasks.push(ParsedTask::Program {
                        program,
                        args: task.args.into_iter().map(String::into_boxed_str).collect(),
                        seed_purpose,
                        timeout,
                    });
                }
                (None, None, Some(declaration)) if task.state.is_none() && task.args.is_empty() => {
                    tasks.push(ParsedTask::Python {
                        declaration,
                        seed_purpose,
                        timeout,
                    });
                }
                _ => {
                    return Err(ConfigError::invalid(
                        path,
                        pointer,
                        "a task must declare exactly `execution_unit`, `program`, or `python`; optional `state` is valid only for an execution unit, top-level `args` only for a program, and `seed` only for a program or Python task",
                    ));
                }
            }
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
    Ok(ParsedManifest {
        manifest,
        state_paths,
        phases,
    })
}

fn megabytes_to_bytes(
    path: &Path,
    pointer: &'static str,
    megabytes: NonZeroU64,
) -> Result<NonZeroU64, ConfigError> {
    megabytes
        .get()
        .checked_mul(BYTES_PER_MEGABYTE)
        .and_then(NonZeroU64::new)
        .ok_or_else(|| {
            ConfigError::invalid(
                path,
                pointer,
                "persistence size is too large to represent as a byte count",
            )
        })
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
    workflow_schema: u64,
    threads: usize,
    #[serde(default)]
    paths: RawPaths,
    #[serde(default)]
    seed: Option<u64>,
    #[serde(default)]
    replicates: RawReplicatePolicy,
    #[serde(default)]
    persistence: RawPersistence,
    phases: Map<String, Value>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPaths {
    #[serde(default)]
    states: BTreeMap<String, PathBuf>,
}

const BYTES_PER_MEGABYTE: u64 = 1_000_000;
const DEFAULT_PERSISTENCE_MB: u64 = 64;

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawPersistence {
    chunk_target_mb: NonZeroU64,
    queue_capacity_mb: NonZeroU64,
}

impl Default for RawPersistence {
    fn default() -> Self {
        Self {
            chunk_target_mb: NonZeroU64::new(DEFAULT_PERSISTENCE_MB)
                .expect("the built-in persistence megabyte setting is positive"),
            queue_capacity_mb: NonZeroU64::new(DEFAULT_PERSISTENCE_MB)
                .expect("the built-in persistence megabyte setting is positive"),
        }
    }
}

#[derive(Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawReplicatePolicy {
    count: u64,
    scheduling: RawReplicateScheduling,
    failure_policy: RawFailurePolicy,
}

impl Default for RawReplicatePolicy {
    fn default() -> Self {
        Self {
            count: 1,
            scheduling: RawReplicateScheduling::default(),
            failure_policy: RawFailurePolicy::default(),
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
    #[serde(default)]
    execution_unit: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    program: Option<PathBuf>,
    #[serde(default)]
    python: Option<PythonTaskDeclaration>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    seed: Option<RawProgramSeed>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProgramSeed {
    purpose: String,
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
