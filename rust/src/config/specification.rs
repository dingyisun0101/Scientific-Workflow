//! Loading one project root into an immutable resolved specification.

use std::path::{Component, Path, PathBuf};

use super::document::StateSchemaDocument;
use super::error::ConfigError;
use super::expansion;
use super::input::{ResolvedTask, ResolvedTaskInput};
use super::manifest::{self, ParsedTask, PhaseSpecification, StudyManifest};
use super::program::{ResolvedProgramTask, resolve_executable};
use super::python;
use super::store::{Config, canonicalize, ensure_contained};

const STATE_SCHEMA: &str = "state.json";
const INPUT_DIRECTORY: &str = "inputs";

/// A complete immutable project declaration compiled from one project root.
#[derive(Debug)]
pub(crate) struct ProjectSpecification {
    config: Config,
    manifest: StudyManifest,
    state_schema: StateSchemaDocument,
    phases: Box<[PhaseSpecification]>,
}

impl ProjectSpecification {
    /// Loads and validates all declarative inputs beneath `project_root`.
    ///
    /// The root is canonicalized once. `study.json` is read from the root and
    /// every JSON document beneath `config` is captured centrally. Reserved
    /// views, model inputs, and executable paths are then resolved from that
    /// snapshot. Loading creates no output and executes no task.
    pub(crate) fn load(project_root: &Path) -> Result<Self, ConfigError> {
        let config = Config::load(project_root)?;
        let parsed = manifest::parse(config.study_path(), config.study_value().clone())?;

        let state_relative = Path::new(STATE_SCHEMA);
        let (state_path, state_value) =
            config
                .document(state_relative)
                .ok_or_else(|| ConfigError::Read {
                    path: config.config_root().join(state_relative),
                    source: std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "required state schema document was not loaded",
                    ),
                })?;
        let state_schema = StateSchemaDocument::new(state_path.to_path_buf(), state_value.clone());

        let mut phases = Vec::with_capacity(parsed.phases.len());
        for phase in parsed.phases {
            let mut tasks = Vec::new();
            for task in phase.tasks {
                match task {
                    ParsedTask::Model {
                        model,
                        input,
                        timeout,
                    } => {
                        let input_path = resolve_input_path(config.config_root(), &input)?;
                        let relative = input_path
                            .strip_prefix(config.config_root())
                            .expect("contained input has a relative config path");
                        let (_, value) =
                            config.document(relative).ok_or_else(|| ConfigError::Read {
                                path: input_path.clone(),
                                source: std::io::Error::new(
                                    std::io::ErrorKind::NotFound,
                                    "referenced task input was not loaded by central Config",
                                ),
                            })?;
                        let expanded = expansion::expand(&input_path, value)?;
                        tasks.try_reserve(expanded.len()).map_err(|_| {
                            ConfigError::ExpansionOverflow {
                                path: input_path.clone(),
                            }
                        })?;
                        for (ordinal, value) in expanded.into_iter().enumerate() {
                            let ordinal = u64::try_from(ordinal).map_err(|_| {
                                ConfigError::ExpansionOverflow {
                                    path: input_path.clone(),
                                }
                            })?;
                            tasks.push(ResolvedTask::Model(ResolvedTaskInput::new(
                                model.clone(),
                                input_path.clone(),
                                ordinal,
                                value,
                                timeout,
                            )));
                        }
                    }
                    ParsedTask::Program {
                        program,
                        args,
                        timeout,
                    } => {
                        let program = resolve_executable(config.project_root(), &program)?;
                        tasks.push(ResolvedTask::Program(ResolvedProgramTask::new(
                            program, args, timeout,
                        )));
                    }
                    ParsedTask::Python {
                        declaration,
                        timeout,
                    } => tasks.push(ResolvedTask::Program(python::resolve(
                        config.project_root(),
                        declaration,
                        timeout,
                    )?)),
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
            config,
            manifest: parsed.manifest,
            state_schema,
            phases: phases.into_boxed_slice(),
        })
    }

    /// Returns the canonical project root supplied to [`Self::load`].
    pub(crate) fn project_root(&self) -> &Path {
        self.config.project_root()
    }

    /// Returns the immutable central configuration snapshot.
    pub(crate) fn config(&self) -> &Config {
        &self.config
    }

    /// Returns the validated Workflow-owned study manifest.
    pub(crate) fn manifest(&self) -> &StudyManifest {
        &self.manifest
    }

    /// Returns the centrally parsed state-schema document.
    pub(crate) fn state_schema(&self) -> &StateSchemaDocument {
        &self.state_schema
    }

    /// Returns validated phases and resolved generic tasks in declaration order.
    pub(crate) fn phases(&self) -> &[PhaseSpecification] {
        &self.phases
    }
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
