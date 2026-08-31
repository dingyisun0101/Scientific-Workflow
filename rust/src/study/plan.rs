//! Immutable study, phase, and task views.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::config::{
    Config, ConfigSnapshot, FailurePolicy, PersistenceSpecification, ProjectSpecification,
    ReplicatePolicy, ReplicateScheduling, StudyManifest,
};
use crate::persistence::PersistencePlan;
use crate::task::{
    ExecutionUnitCatalog, ExecutionUnitTaskProvenance, Task, TaskDefinition, TaskKind,
};

use super::compilation;
use super::error::StudyError;

/// A complete immutable and effect-free execution declaration.
#[derive(Clone)]
pub struct Study {
    inner: Arc<StudyInner>,
}

impl Study {
    /// Loads project declarations and binds all linked `#[execution_unit]` registrations.
    ///
    /// This performs complete preflight without creating output or initializing
    /// an execution unit. Config is the only file reader and JSON parser.
    pub fn load(project_root: &Path) -> Result<Self, StudyError> {
        let project = ProjectSpecification::load(project_root)?;
        let catalog = ExecutionUnitCatalog::discovered()?;
        compilation::compile(project, &catalog)
    }

    pub(crate) fn from_parts(project: ProjectSpecification, phases: Box<[StudyPhase]>) -> Self {
        let project_root = project.project_root().to_path_buf();
        let config = project.config().clone();
        let output_root = project_root.join("output");
        let manifest: StudyManifest = *project.manifest();
        let workflow_schema = manifest.workflow_schema();
        let threads = manifest.threads();
        let replicate_policy = manifest.replicate_policy();
        let master_seed = manifest.master_seed();
        let persistence: PersistenceSpecification = manifest.persistence();
        let persistence_plan = PersistencePlan::local(
            persistence.chunk_target_bytes(),
            persistence.queue_capacity_bytes(),
        );
        Self {
            inner: Arc::new(StudyInner {
                project_root,
                config,
                phases,
                output_root,
                workflow_schema,
                threads,
                master_seed,
                replicate_policy,
                persistence_plan,
            }),
        }
    }

    /// Returns the canonical project root loaded by config.
    pub fn project_root(&self) -> &Path {
        &self.inner.project_root
    }

    /// Returns the inferred output root, `<project-root>/output`.
    pub fn output_root(&self) -> &Path {
        &self.inner.output_root
    }

    /// Returns the required study-wide compute worker count.
    pub fn threads(&self) -> usize {
        self.inner.threads
    }

    /// Returns a read-only view of the fully compiled deterministic plan.
    ///
    /// The view borrows this Study and exposes planning facts only. It does not
    /// expose constants payloads, executable task handles, or mutable policy.
    pub fn plan_summary(&self) -> PlanSummary<'_> {
        PlanSummary { study: self }
    }

    /// Returns the frozen language-neutral configuration supplied to programs.
    pub(crate) fn config_snapshot(&self) -> ConfigSnapshot {
        self.inner.config.snapshot()
    }

    /// Returns immutable phases in manifest declaration order.
    pub(crate) fn phases(&self) -> &[StudyPhase] {
        &self.inner.phases
    }

    /// Returns the effective replicate policy parsed from `wf_configs/study.json`.
    pub(crate) fn replicate_policy(&self) -> ReplicatePolicy {
        self.inner.replicate_policy
    }

    /// Returns the study-wide optional deterministic seed.
    pub(crate) fn master_seed(&self) -> Option<u64> {
        self.inner.master_seed
    }

    /// Returns the immutable effective persistence plan compiled from
    /// `wf_configs/study.json`.
    pub(crate) fn persistence_plan(&self) -> PersistencePlan {
        self.inner.persistence_plan
    }
}

impl std::fmt::Debug for Study {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Study")
            .field("project_root", &self.project_root())
            .field("output_root", &self.output_root())
            .field("threads", &self.threads())
            .field("config", &self.inner.config)
            .field("persistence", &self.persistence_plan())
            .field("phases", &self.phases().len())
            .finish_non_exhaustive()
    }
}

struct StudyInner {
    project_root: PathBuf,
    config: Config,
    phases: Box<[StudyPhase]>,
    output_root: PathBuf,
    workflow_schema: u64,
    threads: usize,
    master_seed: Option<u64>,
    replicate_policy: ReplicatePolicy,
    persistence_plan: PersistencePlan,
}

/// A read-only view of one fully compiled [`Study`].
#[derive(Clone, Copy)]
pub struct PlanSummary<'a> {
    study: &'a Study,
}

impl std::fmt::Debug for PlanSummary<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PlanSummary")
            .field("workflow_schema", &self.workflow_schema())
            .field("threads", &self.threads())
            .field("replicate_count", &self.replicate_count())
            .field("phases", &self.study.phases().len())
            .finish_non_exhaustive()
    }
}

impl<'a> PlanSummary<'a> {
    /// Returns the validated project-configuration schema generation.
    pub fn workflow_schema(self) -> u64 {
        self.study.inner.workflow_schema
    }

    /// Returns the study-wide authored compute-thread count.
    pub fn threads(self) -> usize {
        self.study.threads()
    }

    /// Returns the positive number of isolated replicates.
    pub fn replicate_count(self) -> u64 {
        self.study.replicate_policy().count()
    }

    /// Returns the effective replicate admission mode.
    pub fn replicate_scheduling(self) -> PlanReplicateScheduling {
        match self.study.replicate_policy().scheduling() {
            ReplicateScheduling::Sequential => PlanReplicateScheduling::Sequential,
            ReplicateScheduling::Parallel => PlanReplicateScheduling::Parallel,
        }
    }

    /// Returns the effective failed-replicate response.
    pub fn replicate_failure_policy(self) -> PlanFailurePolicy {
        public_failure_policy(self.study.replicate_policy().failure_policy())
    }

    /// Reports whether this plan retains a study-wide master seed.
    ///
    /// The value itself remains outside the inspection view so diagnostics do
    /// not accidentally publish it.
    pub fn has_master_seed(self) -> bool {
        self.study.master_seed().is_some()
    }

    /// Returns the effective local chunk rollover target in bytes.
    pub fn persistence_chunk_target_bytes(self) -> u64 {
        self.study.persistence_plan().chunk_target().get()
    }

    /// Returns the effective bounded persistence queue capacity in bytes.
    pub fn persistence_queue_capacity_bytes(self) -> u64 {
        self.study.persistence_plan().queue_capacity().get()
    }

    /// Iterates compiled phases in manifest declaration order.
    pub fn phases(self) -> impl ExactSizeIterator<Item = PhasePlanSummary<'a>> {
        self.study
            .phases()
            .iter()
            .map(|phase| PhasePlanSummary { phase })
    }
}

/// The effective replicate scheduling mode reported by [`PlanSummary`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanReplicateScheduling {
    /// Complete one replicate before admitting the next.
    Sequential,
    /// Admit eligible replicates concurrently.
    Parallel,
}

/// An effective fail-fast or finish-all decision in a compiled plan.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanFailurePolicy {
    /// Stop admitting sibling work after the first failure.
    FailFast,
    /// Allow declared sibling work to finish after a failure.
    FinishAll,
}

/// A read-only view of one compiled phase.
#[derive(Clone, Copy)]
pub struct PhasePlanSummary<'a> {
    phase: &'a StudyPhase,
}

impl std::fmt::Debug for PhasePlanSummary<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PhasePlanSummary")
            .field("name", &self.name())
            .field("dependencies", &self.phase.dependencies.len())
            .field("tasks", &self.phase.tasks.len())
            .finish_non_exhaustive()
    }
}

impl<'a> PhasePlanSummary<'a> {
    /// Returns the stable manifest phase key.
    pub fn name(self) -> &'a str {
        self.phase.name()
    }

    /// Iterates dependency phase keys in declaration order.
    pub fn dependencies(self) -> impl ExactSizeIterator<Item = &'a str> {
        self.phase.dependencies()
    }

    /// Returns the positive maximum number of concurrently admitted tasks.
    pub fn max_concurrency(self) -> usize {
        self.phase.max_concurrency()
    }

    /// Returns the minimum interval between successive task admissions.
    pub fn start_interval(self) -> Duration {
        self.phase.start_interval()
    }

    /// Returns the optional phase-wide timeout.
    pub fn timeout(self) -> Option<Duration> {
        self.phase.timeout()
    }

    /// Returns the effective sibling-task failure policy.
    pub fn failure_policy(self) -> PlanFailurePolicy {
        public_failure_policy(self.phase.failure_policy())
    }

    /// Iterates compiled tasks in deterministic plan order.
    pub fn tasks(self) -> impl ExactSizeIterator<Item = TaskPlanSummary<'a>> {
        self.phase
            .tasks()
            .iter()
            .map(|task| TaskPlanSummary { task })
    }
}

/// A read-only view of one compiled task.
#[derive(Clone, Copy)]
pub struct TaskPlanSummary<'a> {
    task: &'a StudyTask,
}

impl std::fmt::Debug for TaskPlanSummary<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TaskPlanSummary")
            .field("identity", &self.identity())
            .field("label", &self.label())
            .field("kind", &self.kind())
            .finish_non_exhaustive()
    }
}

impl<'a> TaskPlanSummary<'a> {
    /// Returns the deterministic study-wide task identity.
    pub fn identity(self) -> &'a str {
        self.task.identity()
    }

    /// Returns the inferred human-readable task label.
    pub fn label(self) -> &'a str {
        self.task.label()
    }

    /// Returns the global deterministic output ordinal.
    pub fn output_ordinal(self) -> u64 {
        self.task.output_ordinal()
    }

    /// Returns the optional task-specific timeout.
    pub fn timeout(self) -> Option<Duration> {
        self.task.timeout()
    }

    /// Returns the optional semantic seed purpose declared by an external task.
    pub fn seed_purpose(self) -> Option<&'a str> {
        self.task.program_seed_purpose()
    }

    /// Returns the workload-specific immutable planning facts.
    pub fn kind(self) -> PlannedTaskKind<'a> {
        if let Some(provenance) = self.task.execution_unit_provenance() {
            PlannedTaskKind::ExecutionUnit {
                execution_unit: provenance.execution_unit(),
                state: provenance.state(),
                parameter_ordinal: provenance.parameter_ordinal(),
                parameter_source: provenance.parameter_source(),
            }
        } else if let Some(script) = self.task.python_script() {
            PlannedTaskKind::Python {
                launcher: self
                    .task
                    .program_path()
                    .expect("a compiled Python task has a resolved launcher"),
                script,
            }
        } else {
            PlannedTaskKind::Program {
                executable: self
                    .task
                    .program_path()
                    .expect("a compiled program task has a resolved executable"),
            }
        }
    }
}

/// Workload-specific facts for one task in a [`PlanSummary`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlannedTaskKind<'a> {
    /// A linked Rust execution-unit invocation.
    ExecutionUnit {
        /// Stable linked registration key.
        execution_unit: &'a str,
        /// Selected project-state key or standard-provider ID.
        state: &'a str,
        /// Zero-based ordinal of the expanded parameter combination.
        parameter_ordinal: u64,
        /// Canonical source of the selected parameter namespace.
        parameter_source: &'a Path,
    },
    /// A directly invoked external executable.
    Program {
        /// Canonical executable selected during preflight.
        executable: &'a Path,
    },
    /// A Python script invoked through a resolved environment launcher.
    Python {
        /// Canonical executable used to launch the environment or interpreter.
        launcher: &'a Path,
        /// Canonical Python script selected during preflight.
        script: &'a Path,
    },
}

fn public_failure_policy(policy: FailurePolicy) -> PlanFailurePolicy {
    match policy {
        FailurePolicy::FailFast => PlanFailurePolicy::FailFast,
        FailurePolicy::FinishAll => PlanFailurePolicy::FinishAll,
    }
}

/// One immutable execution phase compiled from the study manifest.
#[derive(Clone, Debug)]
pub(crate) struct StudyPhase {
    pub(crate) name: Box<str>,
    pub(crate) dependencies: Box<[Box<str>]>,
    pub(crate) tasks: Box<[StudyTask]>,
    pub(crate) max_concurrency: usize,
    pub(crate) start_interval: Duration,
    pub(crate) timeout: Option<Duration>,
    pub(crate) failure_policy: FailurePolicy,
}

impl StudyPhase {
    /// Returns the stable phase key from `wf_configs/study.json`.
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    /// Iterates dependency keys in declaration order.
    pub(crate) fn dependencies(&self) -> impl ExactSizeIterator<Item = &str> {
        self.dependencies.iter().map(Box::as_ref)
    }

    /// Returns bound execution-unit/program invocations in deterministic plan order.
    pub(crate) fn tasks(&self) -> &[StudyTask] {
        &self.tasks
    }

    /// Returns the positive effective concurrency bound.
    pub(crate) const fn max_concurrency(&self) -> usize {
        self.max_concurrency
    }

    /// Returns the effective interval between task admissions.
    pub(crate) const fn start_interval(&self) -> Duration {
        self.start_interval
    }

    /// Returns the optional phase-wide timeout.
    pub(crate) const fn timeout(&self) -> Option<Duration> {
        self.timeout
    }

    /// Returns the effective sibling failure policy.
    pub(crate) const fn failure_policy(&self) -> FailurePolicy {
        self.failure_policy
    }
}

/// One generic execution-unit or program task compiled from project configuration.
#[derive(Clone)]
pub(crate) struct StudyTask {
    pub(crate) identity: Box<str>,
    pub(crate) label: Box<str>,
    pub(crate) output_ordinal: u64,
    pub(crate) task: Task,
}

impl StudyTask {
    /// Returns the inferred stable identity within this study plan.
    pub(crate) fn identity(&self) -> &str {
        &self.identity
    }

    /// Returns the inferred human-readable label.
    pub(crate) fn label(&self) -> &str {
        &self.label
    }

    pub(crate) fn kind(&self) -> TaskKind {
        self.task.kind()
    }

    pub(crate) fn kind_name(&self) -> &'static str {
        self.task.kind_name()
    }

    /// Returns the registration key when this is an execution-unit task.
    pub(crate) fn execution_unit(&self) -> Option<&str> {
        self.task.execution_unit()
    }

    /// Returns the generic task subject used in lifecycle presentation.
    pub(crate) fn subject(&self) -> &str {
        self.task.subject()
    }

    /// Returns the optional task-specific cooperative timeout.
    pub(crate) fn timeout(&self) -> Option<Duration> {
        self.task.timeout()
    }

    pub(crate) fn output_ordinal(&self) -> u64 {
        self.output_ordinal
    }

    pub(crate) fn definition(&self) -> &dyn TaskDefinition {
        &self.task
    }

    pub(crate) fn execution_unit_provenance(&self) -> Option<ExecutionUnitTaskProvenance<'_>> {
        self.task.execution_unit_provenance()
    }

    pub(crate) fn program_path(&self) -> Option<&Path> {
        self.task.program_path()
    }

    pub(crate) fn python_script(&self) -> Option<&Path> {
        self.task.python_script()
    }

    pub(crate) fn program_seed_purpose(&self) -> Option<&str> {
        self.task.program_seed_purpose()
    }
}

impl std::fmt::Debug for StudyTask {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StudyTask")
            .field("identity", &self.identity())
            .field("label", &self.label())
            .field("kind", &self.kind())
            .field("subject", &self.subject())
            .finish_non_exhaustive()
    }
}
