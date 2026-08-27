//! Loading one project root into an immutable resolved specification.

use std::path::Path;

use super::document::StateSchemaDocument;
use super::error::ConfigError;
use super::expansion;
use super::manifest::{self, ParsedTask, PhaseSpecification, StudyManifest};
use super::parameters::{ResolvedModelParameters, ResolvedTask};
use super::program::{ResolvedProgramTask, resolve_executable};
use super::python;
use super::store::Config;

const STATE_SCHEMA: &str = "state.json";
const PARAMETERS: &str = "parameters.json";

/// A complete immutable project declaration compiled from one project root.
#[derive(Debug)]
pub(crate) struct ProjectSpecification {
    config: Config,
    manifest: StudyManifest,
    state_schema: StateSchemaDocument,
    phases: Box<[PhaseSpecification]>,
}

impl ProjectSpecification {
    /// Loads and validates all declarations and parameters beneath `project_root`.
    ///
    /// The root is canonicalized once. `study.json` is read from the root and
    /// every JSON document beneath `config` is captured centrally. Reserved
    /// views, model parameter sections, and executable paths are resolved from that
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

        let parameters_relative = Path::new(PARAMETERS);
        let (parameters_path, parameters_value) =
            config
                .document(parameters_relative)
                .ok_or_else(|| ConfigError::Read {
                    path: config.config_root().join(parameters_relative),
                    source: std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "required project parameters document was not loaded",
                    ),
                })?;
        let parameter_sections = parameters_value.as_object().ok_or_else(|| {
            ConfigError::invalid(
                parameters_path,
                "/",
                "project parameters must be a JSON object keyed by workload",
            )
        })?;

        let mut phases = Vec::with_capacity(parsed.phases.len());
        for phase in parsed.phases {
            let mut tasks = Vec::new();
            for task in phase.tasks {
                match task {
                    ParsedTask::Model { model, timeout } => {
                        let value = parameter_sections.get(model.as_ref()).ok_or_else(|| {
                            ConfigError::invalid(
                                parameters_path,
                                format!("/{}", model),
                                format!("registered model `{model}` has no parameter section"),
                            )
                        })?;
                        let expanded = expansion::expand(parameters_path, value)?;
                        tasks.try_reserve(expanded.len()).map_err(|_| {
                            ConfigError::ExpansionOverflow {
                                path: parameters_path.to_path_buf(),
                            }
                        })?;
                        for (ordinal, value) in expanded.into_iter().enumerate() {
                            let ordinal = u64::try_from(ordinal).map_err(|_| {
                                ConfigError::ExpansionOverflow {
                                    path: parameters_path.to_path_buf(),
                                }
                            })?;
                            tasks.push(ResolvedTask::Model(ResolvedModelParameters::new(
                                model.clone(),
                                parameters_path.to_path_buf(),
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
