//! Private effect-free project-to-study composition.

use std::collections::BTreeMap;

use crate::config::{PhaseSpecification, ProjectSpecification, ResolvedTask, StateSchemaDocument};
use crate::state::{SystemStateSchema, schema_from_json_value};
use crate::task::{ExecutionUnitCatalog, Task};

use super::error::StudyError;
use super::plan::{Study, StudyPhase, StudyTask};

pub(crate) fn compile(
    project: ProjectSpecification,
    catalog: &ExecutionUnitCatalog,
) -> Result<Study, StudyError> {
    let mut schemas = BTreeMap::new();
    for (name, document) in project.state_schemas() {
        let document: &StateSchemaDocument = document;
        let schema = schema_from_json_value(document.path(), document.json_value())
            .map_err(|source| StudyError::state_schema(name, document.path(), source))?;
        schemas.insert(name.as_ref(), schema);
    }
    let mut provided_schemas: BTreeMap<&'static str, (&'static [u8], SystemStateSchema)> =
        BTreeMap::new();

    let mut output_ordinal = 0_u64;
    let mut phases = Vec::with_capacity(project.phases().len());
    for phase in project.phases() {
        let phase: &PhaseSpecification = phase;
        let mut tasks = Vec::with_capacity(phase.tasks().len());
        for resolved in phase.tasks() {
            let (identity_suffix, label, task) = match resolved {
                ResolvedTask::ExecutionUnit { parameters, state } => {
                    let registration =
                        catalog.get(parameters.execution_unit()).ok_or_else(|| {
                            StudyError::UnknownExecutionUnit {
                                phase: phase.name().to_owned(),
                                execution_unit: parameters.execution_unit().to_owned(),
                            }
                        })?;
                    let (state, schema) = if let Some(state) = state {
                        let schema = schemas
                            .get(state.as_ref())
                            .expect("config validated every explicit execution-unit state");
                        (state.clone(), schema.clone())
                    } else {
                        let provider = registration.standard_state_schema().ok_or_else(|| {
                            StudyError::MissingStateSchema {
                                phase: phase.name().to_owned(),
                                execution_unit: parameters.execution_unit().to_owned(),
                            }
                        })?;
                        let id = provider.id();
                        if id.is_empty() || id.trim() != id {
                            return Err(StudyError::InvalidStateSchemaProvider {
                                provider: id.to_owned(),
                                reason: "provider ID must be nonempty and contain no surrounding whitespace"
                                    .to_owned(),
                            });
                        }
                        let schema = if let Some((document, schema)) = provided_schemas.get(id) {
                            if *document != provider.document() {
                                return Err(StudyError::InvalidStateSchemaProvider {
                                    provider: id.to_owned(),
                                    reason:
                                        "the same provider ID supplied different JSON documents"
                                            .to_owned(),
                                });
                            }
                            schema.clone()
                        } else {
                            let schema =
                                provider
                                    .resolve()
                                    .map_err(|source| StudyError::ProvidedState {
                                        provider: id.to_owned(),
                                        source,
                                    })?;
                            provided_schemas.insert(id, (provider.document(), schema.clone()));
                            schema
                        };
                        (id.into(), schema)
                    };
                    let observation_plan =
                        registration
                            .preflight(parameters, &schema)
                            .map_err(|source| {
                                StudyError::execution_unit_preflight(
                                    phase.name(),
                                    parameters.execution_unit(),
                                    parameters.ordinal(),
                                    source,
                                )
                            })?;
                    (
                        format!(
                            "{}-{:06}",
                            parameters.execution_unit(),
                            parameters.ordinal()
                        ),
                        format!("{} #{}", parameters.execution_unit(), parameters.ordinal()),
                        registration.make_task(parameters.clone(), state, schema, observation_plan),
                    )
                }
                ResolvedTask::Program(program) => {
                    let name = program.subject();
                    let kind = program.kind_name();
                    (
                        format!("{kind}-{name}"),
                        format!("{kind} {name}"),
                        Task::for_program(program.clone()),
                    )
                }
            };
            let identity = format!("{}/{output_ordinal:06}/{identity_suffix}", phase.name());
            tasks.push(StudyTask {
                identity: identity.into_boxed_str(),
                label: label.into_boxed_str(),
                output_ordinal,
                task,
            });
            output_ordinal = output_ordinal
                .checked_add(1)
                .ok_or(StudyError::TaskIdentityOverflow)?;
        }
        phases.push(StudyPhase {
            name: phase.name().into(),
            dependencies: phase.dependencies().map(Into::into).collect(),
            tasks: tasks.into_boxed_slice(),
            max_concurrency: phase.max_concurrency(),
            start_interval: phase.start_interval(),
            timeout: phase.timeout(),
            failure_policy: phase.failure_policy(),
        });
    }
    Ok(Study::from_parts(project, phases.into_boxed_slice()))
}
