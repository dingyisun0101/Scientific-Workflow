//! Immutable study, phase, and task views.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::config::advanced::{
    FailurePolicy, ProjectDocument, ProjectSpecification, ReplicatePolicy, ResolvedTaskInput,
};
use crate::state::advanced::SystemStateSchema;
use crate::task::advanced::{ModelCatalog, Task, TaskDefinition};

use super::compilation;
use super::error::StudyError;

/// A complete immutable and effect-free execution declaration.
#[derive(Clone)]
pub struct Study {
    inner: Arc<StudyInner>,
}

impl Study {
    /// Loads project declarations and binds all linked `#[model]` registrations.
    ///
    /// This performs complete preflight without creating output or initializing
    /// a scientific model. Config is the only file reader and JSON parser.
    pub fn load(project_root: &Path) -> Result<Self, StudyError> {
        let catalog = ModelCatalog::discovered()?;
        Self::load_with_catalog(project_root, &catalog)
    }

    /// Loads a study against an explicit immutable model catalog.
    ///
    /// This is the deterministic injection seam for tests and embedded hosts;
    /// ordinary applications use [`Self::load`].
    pub fn load_with_catalog(
        project_root: &Path,
        catalog: &ModelCatalog,
    ) -> Result<Self, StudyError> {
        let project = ProjectSpecification::load(project_root)?;
        compilation::compile(project, catalog)
    }

    pub(crate) fn from_parts(
        project: ProjectSpecification,
        schema: SystemStateSchema,
        phases: Box<[StudyPhase]>,
    ) -> Self {
        let output_root = project.project_root().join("output");
        Self {
            inner: Arc::new(StudyInner {
                project,
                schema,
                phases,
                output_root,
            }),
        }
    }

    /// Returns the canonical project root loaded by config.
    pub fn project_root(&self) -> &Path {
        self.inner.project.project_root()
    }

    /// Returns the inferred output root, `<project-root>/output`.
    pub fn output_root(&self) -> &Path {
        &self.inner.output_root
    }

    /// Returns the semantically validated shared state schema.
    pub fn state_schema(&self) -> &SystemStateSchema {
        &self.inner.schema
    }

    /// Returns immutable phases in manifest declaration order.
    pub fn phases(&self) -> &[StudyPhase] {
        &self.inner.phases
    }

    /// Returns the effective replicate policy parsed from `study.json`.
    pub fn replicate_policy(&self) -> ReplicatePolicy {
        self.inner.project.manifest().replicate_policy()
    }

    /// Returns every exact source document in config's deterministic first-use order.
    pub fn source_documents(&self) -> &[ProjectDocument] {
        self.inner.project.documents()
    }
}

impl std::fmt::Debug for Study {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Study")
            .field("project_root", &self.project_root())
            .field("output_root", &self.output_root())
            .field("phases", &self.phases().len())
            .finish_non_exhaustive()
    }
}

struct StudyInner {
    project: ProjectSpecification,
    schema: SystemStateSchema,
    phases: Box<[StudyPhase]>,
    output_root: PathBuf,
}

/// One immutable execution phase compiled from the study manifest.
#[derive(Clone, Debug)]
pub struct StudyPhase {
    pub(crate) name: Box<str>,
    pub(crate) dependencies: Box<[Box<str>]>,
    pub(crate) tasks: Box<[StudyTask]>,
    pub(crate) max_concurrency: usize,
    pub(crate) start_interval: Duration,
    pub(crate) timeout: Option<Duration>,
    pub(crate) failure_policy: FailurePolicy,
}

impl StudyPhase {
    /// Returns the stable phase key from `study.json`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Iterates dependency keys in declaration order.
    pub fn dependencies(&self) -> impl ExactSizeIterator<Item = &str> {
        self.dependencies.iter().map(Box::as_ref)
    }

    /// Returns bound model invocations in deterministic expansion order.
    pub fn tasks(&self) -> &[StudyTask] {
        &self.tasks
    }

    /// Returns the positive effective concurrency bound.
    pub const fn max_concurrency(&self) -> usize {
        self.max_concurrency
    }

    /// Returns the effective interval between task admissions.
    pub const fn start_interval(&self) -> Duration {
        self.start_interval
    }

    /// Returns the optional phase-wide cooperative timeout.
    pub const fn timeout(&self) -> Option<Duration> {
        self.timeout
    }

    /// Returns the effective sibling failure policy.
    pub const fn failure_policy(&self) -> FailurePolicy {
        self.failure_policy
    }
}

/// One model bound to one complete config-owned constants input.
#[derive(Clone)]
pub struct StudyTask {
    pub(crate) identity: Box<str>,
    pub(crate) label: Box<str>,
    pub(crate) output_ordinal: u64,
    pub(crate) input: ResolvedTaskInput,
    pub(crate) definition: Task,
}

impl StudyTask {
    /// Returns the inferred stable identity within this study plan.
    pub fn identity(&self) -> &str {
        &self.identity
    }

    /// Returns the inferred human-readable label.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns the registered model key selected by the manifest.
    pub fn model(&self) -> &str {
        self.input.model()
    }

    /// Returns the config-owned resolved task input.
    pub fn input(&self) -> &ResolvedTaskInput {
        &self.input
    }

    /// Returns additional state fields selected for display.
    pub fn display_fields(&self) -> impl ExactSizeIterator<Item = &str> {
        self.input.display_fields()
    }

    /// Returns the optional task-specific cooperative timeout.
    pub fn timeout(&self) -> Option<Duration> {
        self.input.timeout()
    }

    pub(crate) fn output_ordinal(&self) -> u64 {
        self.output_ordinal
    }

    pub(crate) fn definition(&self) -> &dyn TaskDefinition {
        &self.definition
    }
}

impl std::fmt::Debug for StudyTask {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StudyTask")
            .field("identity", &self.identity())
            .field("label", &self.label())
            .field("model", &self.model())
            .field("input", &self.input)
            .finish_non_exhaustive()
    }
}
