//! Loading one project root into an immutable resolved specification.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use super::document::{ProjectDocument, StateSchemaDocument};
use super::error::ConfigError;
use super::expansion;
use super::input::ResolvedTaskInput;
use super::manifest::{self, PhaseSpecification, StudyManifest};

const STUDY_MANIFEST: &str = "study.json";
const CONFIG_DIRECTORY: &str = "config";
const STATE_SCHEMA: &str = "state.json";
const INPUT_DIRECTORY: &str = "inputs";

/// A complete immutable project declaration compiled from one project root.
#[derive(Clone, Debug)]
pub struct ProjectSpecification {
    inner: Arc<ProjectSpecificationInner>,
}

impl ProjectSpecification {
    /// Loads and validates all declarative inputs beneath `project_root`.
    ///
    /// The root is canonicalized once. `study.json` is read from the root;
    /// `state.json` and every task input are read beneath its `config`
    /// directory. Loading creates no output and executes no task.
    pub fn load(project_root: &Path) -> Result<Self, ConfigError> {
        let project_root = canonicalize(project_root)?;
        let config_root = canonicalize(&project_root.join(CONFIG_DIRECTORY))?;

        let manifest_path = project_root.join(STUDY_MANIFEST);
        let (manifest_document, manifest_value) = ProjectDocument::read(&manifest_path)?;
        let parsed = manifest::parse(&manifest_path, manifest_value)?;

        let state_path = canonicalize(&config_root.join(STATE_SCHEMA))?;
        ensure_contained(&config_root, &state_path)?;
        let (state_document, state_value) = ProjectDocument::read(&state_path)?;
        let state_schema = StateSchemaDocument::new(state_document.clone(), state_value);

        let mut documents = vec![manifest_document, state_document];
        let mut input_documents: HashMap<PathBuf, serde_json::Value> = HashMap::new();
        let mut phases = Vec::with_capacity(parsed.phases.len());
        for phase in parsed.phases {
            let mut tasks = Vec::new();
            for task in phase.tasks {
                let input_path = resolve_input_path(&config_root, &task.input)?;
                let value = if let Some(value) = input_documents.get(&input_path) {
                    value.clone()
                } else {
                    let (document, value) = ProjectDocument::read(&input_path)?;
                    documents.push(document);
                    input_documents.insert(input_path.clone(), value.clone());
                    value
                };
                let expanded = expansion::expand(&input_path, &value)?;
                tasks
                    .try_reserve(expanded.len())
                    .map_err(|_| ConfigError::ExpansionOverflow {
                        path: input_path.clone(),
                    })?;
                for (ordinal, value) in expanded.into_iter().enumerate() {
                    let ordinal =
                        u64::try_from(ordinal).map_err(|_| ConfigError::ExpansionOverflow {
                            path: input_path.clone(),
                        })?;
                    tasks.push(ResolvedTaskInput::new(
                        task.definition.clone(),
                        input_path.clone(),
                        ordinal,
                        value,
                        Arc::clone(&task.display_fields),
                        task.timeout,
                    ));
                }
            }
            phases.push(PhaseSpecification {
                name: phase.name,
                dependencies: phase.dependencies,
                tasks: tasks.into_boxed_slice(),
                max_concurrency: phase.max_concurrency,
                start_interval: phase.start_interval,
                timeout: phase.timeout,
                failure_policy: phase.failure_policy,
            });
        }

        Ok(Self {
            inner: Arc::new(ProjectSpecificationInner {
                project_root,
                config_root,
                manifest: parsed.manifest,
                state_schema,
                phases: phases.into_boxed_slice(),
                documents: documents.into_boxed_slice(),
            }),
        })
    }

    /// Returns the canonical project root supplied to [`Self::load`].
    pub fn project_root(&self) -> &Path {
        &self.inner.project_root
    }

    /// Returns the canonical configuration directory beneath the project root.
    pub fn config_root(&self) -> &Path {
        &self.inner.config_root
    }

    /// Returns the validated Workflow-owned study manifest.
    pub fn manifest(&self) -> &StudyManifest {
        &self.inner.manifest
    }

    /// Returns the centrally parsed state-schema document.
    pub fn state_schema(&self) -> &StateSchemaDocument {
        &self.inner.state_schema
    }

    /// Returns validated phases and resolved task inputs in declaration order.
    pub fn phases(&self) -> &[PhaseSpecification] {
        &self.inner.phases
    }

    /// Returns every unique source document in first-use order.
    pub fn documents(&self) -> &[ProjectDocument] {
        &self.inner.documents
    }
}

#[derive(Debug)]
struct ProjectSpecificationInner {
    project_root: PathBuf,
    config_root: PathBuf,
    manifest: StudyManifest,
    state_schema: StateSchemaDocument,
    phases: Box<[PhaseSpecification]>,
    documents: Box<[ProjectDocument]>,
}

fn canonicalize(path: &Path) -> Result<PathBuf, ConfigError> {
    std::fs::canonicalize(path).map_err(|source| ConfigError::Read {
        path: path.to_path_buf(),
        source,
    })
}

fn resolve_input_path(config_root: &Path, authored: &Path) -> Result<PathBuf, ConfigError> {
    if authored.is_absolute()
        || authored
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        || !authored.starts_with(INPUT_DIRECTORY)
        || authored.extension().and_then(std::ffi::OsStr::to_str) != Some("json")
    {
        return Err(ConfigError::PathOutsideConfig {
            path: authored.to_path_buf(),
            config_root: config_root.to_path_buf(),
        });
    }
    let resolved = canonicalize(&config_root.join(authored))?;
    ensure_contained(config_root, &resolved)?;
    Ok(resolved)
}

fn ensure_contained(config_root: &Path, path: &Path) -> Result<(), ConfigError> {
    if path.starts_with(config_root) {
        Ok(())
    } else {
        Err(ConfigError::PathOutsideConfig {
            path: path.to_path_buf(),
            config_root: config_root.to_path_buf(),
        })
    }
}
