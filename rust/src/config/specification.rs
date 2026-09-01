//! Loading one project root into an immutable resolved specification.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::Value;

use super::document::{StateSchemaDocument, child_pointer};
use super::error::ConfigError;
use super::expansion;
use super::manifest::{self, ParsedTask, PhaseSpecification, StudyManifest};
use super::parameters::{ResolvedExecutionUnitParameters, ResolvedTask};
use super::program::{ResolvedProgramTask, resolve_executable};
use super::python;
use super::store::Config;

const PARAMETERS: &str = "parameters.json";

/// A complete immutable project declaration compiled from one project root.
#[derive(Debug)]
pub(crate) struct ProjectSpecification {
    config: Config,
    manifest: StudyManifest,
    state_schemas: BTreeMap<Box<str>, StateSchemaDocument>,
    phases: Box<[PhaseSpecification]>,
}

impl ProjectSpecification {
    /// Loads and validates all declarations and parameters beneath `project_root`.
    ///
    /// The root is canonicalized once. `wf_configs/study.json` and every other
    /// JSON document beneath `wf_configs` are captured centrally. Reserved views,
    /// execution-unit parameter sections, and executable paths are resolved from that
    /// snapshot. Loading creates no output and executes no task.
    pub(crate) fn load(project_root: &Path) -> Result<Self, ConfigError> {
        let config = Config::load(project_root)?;
        let parsed = manifest::parse(config.study_path(), config.study_value().clone())?;

        let mut state_schemas = BTreeMap::new();
        for (name, authored_path) in &parsed.state_paths {
            let (state_path, state_value) = config
                .project_document(authored_path)?
                .ok_or_else(|| missing_document(config.project_root(), authored_path))?;
            state_schemas.insert(
                name.clone(),
                StateSchemaDocument::new(state_path.to_path_buf(), state_value.clone()),
            );
        }

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

        let execution_units = parsed
            .phases
            .iter()
            .flat_map(|phase| phase.tasks.iter())
            .filter_map(|task| match task {
                ParsedTask::ExecutionUnit { execution_unit, .. } => {
                    Some(execution_unit.to_string())
                }
                ParsedTask::Program { .. } | ParsedTask::Python { .. } => None,
            })
            .collect::<BTreeSet<_>>();
        let shared = parameter_sections
            .iter()
            .filter(|(key, _)| !execution_units.contains(key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        let expanded_shared = expansion::expand(parameters_path, &Value::Object(shared))?;
        let mut resolved_parameters = Vec::with_capacity(expanded_shared.len());
        for shared in expanded_shared {
            let Value::Object(mut resolved) = shared else {
                unreachable!("expanding an object produces objects");
            };
            for key in &execution_units {
                if let Some(value) = parameter_sections.get(key) {
                    resolved.insert(key.clone(), value.clone());
                }
            }
            let value = Value::Object(resolved);
            let snapshot = config.snapshot_with_parameters(&value);
            resolved_parameters.push((value, snapshot));
        }

        let mut phases = Vec::with_capacity(parsed.phases.len());
        for phase in parsed.phases {
            let mut tasks = Vec::new();
            for task in phase.tasks {
                match task {
                    ParsedTask::ExecutionUnit {
                        execution_unit,
                        state,
                        timeout,
                    } => {
                        if let Some(state) = state.as_deref()
                            && !state_schemas.contains_key(state)
                        {
                            return Err(ConfigError::UnknownState {
                                phase: phase.name.to_string(),
                                execution_unit: execution_unit.to_string(),
                                state: state.to_owned(),
                            });
                        }
                        for (configuration, (resolved, snapshot)) in
                            resolved_parameters.iter().enumerate()
                        {
                            let value = resolved
                                .get(execution_unit.as_ref())
                                .ok_or_else(|| {
                                    ConfigError::invalid(
                                        parameters_path,
                                        child_pointer("/", &execution_unit),
                                        format!("registered execution unit `{execution_unit}` has no parameter section"),
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
                                tasks.push(ResolvedTask::ExecutionUnit {
                                    configuration,
                                    snapshot: snapshot.clone(),
                                    parameters: ResolvedExecutionUnitParameters::new(
                                        execution_unit.clone(),
                                        parameters_path.to_path_buf(),
                                        ordinal,
                                        value,
                                        timeout,
                                    ),
                                    state: state.clone(),
                                });
                            }
                        }
                    }
                    ParsedTask::Program {
                        program,
                        args,
                        seed_purpose,
                        timeout,
                        threads,
                    } => {
                        let program = resolve_executable(config.project_root(), &program)?;
                        let program =
                            ResolvedProgramTask::new(program, args, seed_purpose, timeout, threads);
                        for (configuration, (_, snapshot)) in resolved_parameters.iter().enumerate()
                        {
                            tasks.push(ResolvedTask::Program {
                                configuration,
                                snapshot: snapshot.clone(),
                                program: program.clone(),
                            });
                        }
                    }
                    ParsedTask::Python {
                        declaration,
                        seed_purpose,
                        timeout,
                        threads,
                    } => {
                        let program = python::resolve(
                            config.project_root(),
                            declaration,
                            seed_purpose,
                            timeout,
                            threads,
                        )?;
                        for (configuration, (_, snapshot)) in resolved_parameters.iter().enumerate()
                        {
                            tasks.push(ResolvedTask::Program {
                                configuration,
                                snapshot: snapshot.clone(),
                                program: program.clone(),
                            });
                        }
                    }
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
            state_schemas,
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

    /// Returns centrally parsed state-schema documents by manifest key.
    pub(crate) fn state_schemas(&self) -> &BTreeMap<Box<str>, StateSchemaDocument> {
        &self.state_schemas
    }

    /// Returns validated phases and resolved generic tasks in declaration order.
    pub(crate) fn phases(&self) -> &[PhaseSpecification] {
        &self.phases
    }
}

fn missing_document(project_root: &Path, authored_path: &Path) -> ConfigError {
    let path = if authored_path.is_absolute() {
        authored_path.to_path_buf()
    } else {
        project_root.join(authored_path)
    };
    ConfigError::Read {
        path,
        source: std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "declared state schema was not captured as a JSON config document",
        ),
    }
}
