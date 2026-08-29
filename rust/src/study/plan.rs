//! Immutable study, phase, and task views.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::config::{
    Config, ConfigSnapshot, FailurePolicy, PersistenceSpecification, ProjectSpecification,
    ReplicatePolicy, StudyManifest,
};
use crate::persistence::PersistencePlan;
use crate::task::{
    ExecutionUnitCatalog, ExecutionUnitTaskProvenance, Task, TaskDefinition, TaskKind,
};
use crate::ui::UiPlan;

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
        let threads = manifest.threads();
        let replicate_policy = manifest.replicate_policy();
        let master_seed = manifest.master_seed();
        let persistence: PersistenceSpecification = manifest.persistence();
        let persistence_plan = PersistencePlan::local(
            persistence.chunk_target_bytes(),
            persistence.queue_capacity_bytes(),
        );
        let ui_plan = UiPlan::automatic();
        Self {
            inner: Arc::new(StudyInner {
                project_root,
                config,
                phases,
                output_root,
                threads,
                master_seed,
                replicate_policy,
                persistence_plan,
                ui_plan,
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

    /// Returns the immutable inferred UI plan.
    pub(crate) fn ui_plan(&self) -> UiPlan {
        self.inner.ui_plan
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
            .field("ui", &self.ui_plan())
            .field("phases", &self.phases().len())
            .finish_non_exhaustive()
    }
}

struct StudyInner {
    project_root: PathBuf,
    config: Config,
    phases: Box<[StudyPhase]>,
    output_root: PathBuf,
    threads: usize,
    master_seed: Option<u64>,
    replicate_policy: ReplicatePolicy,
    persistence_plan: PersistencePlan,
    ui_plan: UiPlan,
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
