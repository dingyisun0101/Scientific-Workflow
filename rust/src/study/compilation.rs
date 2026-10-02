//! Private effect-free project-to-study composition.

use std::collections::BTreeMap;

use crate::config::{
    ComputeMode, PhaseSpecification, ProjectSpecification, ResolvedTask, StateSchemaDocument,
};
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
        let auto_labels = phase
            .auto_name_prefix
            .as_deref()
            .map(|prefix| automatic_labels(phase.tasks(), prefix));
        for (task_index, resolved) in phase.tasks().iter().enumerate() {
            let configuration = resolved.configuration();
            let config_snapshot = resolved.snapshot().clone();
            let (identity_suffix, label, task, threads) = match resolved {
                ResolvedTask::ExecutionUnit {
                    parameters,
                    state,
                    threads,
                    ..
                } => {
                    let registration =
                        catalog.get(parameters.execution_unit()).ok_or_else(|| {
                            StudyError::UnknownExecutionUnit {
                                phase: phase.name().to_owned(),
                                execution_unit: parameters.execution_unit().to_owned(),
                            }
                        })?;
                    if project.manifest().compute_mode() == ComputeMode::Auto
                        && !registration.thread_count_invariant()
                    {
                        return Err(StudyError::AutoComputeRequiresInvariantUnit {
                            phase: phase.name().to_owned(),
                            execution_unit: parameters.execution_unit().to_owned(),
                        });
                    }
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
                        *threads,
                    )
                }
                ResolvedTask::Program { program, .. } => {
                    let name = program.subject();
                    let kind = program.kind_name();
                    (
                        format!("{kind}-{name}"),
                        format!("{kind} {name}"),
                        Task::for_program(program.clone()),
                        Some(program.threads()),
                    )
                }
            };
            let identity = format!("{}/{output_ordinal:06}/{identity_suffix}", phase.name());
            tasks.push(StudyTask {
                active: match resolved {
                    ResolvedTask::ExecutionUnit { active, .. } => *active,
                    ResolvedTask::Program { .. } => true,
                },
                identity: identity.into_boxed_str(),
                label: auto_labels
                    .as_ref()
                    .map_or(label, |labels| labels[task_index].clone())
                    .into_boxed_str(),
                output_ordinal,
                configuration,
                config_snapshot,
                task,
                threads,
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

/// Presentation labels use only varying resolved values; identity stays stable.
fn automatic_labels(tasks: &[ResolvedTask], prefix: &str) -> Vec<String> {
    use serde_json::Value;
    use std::collections::BTreeSet;

    fn flatten(prefix: &str, value: &Value, fields: &mut BTreeMap<String, Value>) {
        if let Value::Object(object) = value
            && !object.is_empty()
        {
            for (key, value) in object {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten(&path, value, fields);
            }
        } else {
            fields.insert(prefix.to_owned(), value.clone());
        }
    }
    let unit_keys: BTreeSet<_> = tasks
        .iter()
        .filter_map(|task| match task {
            ResolvedTask::ExecutionUnit { parameters, .. } => Some(parameters.execution_unit()),
            _ => None,
        })
        .collect();
    let fields: Vec<_> = tasks
        .iter()
        .map(|task| {
            let mut fields = BTreeMap::new();
            if let Value::Object(shared) = task.snapshot().parameters() {
                for (key, value) in shared {
                    if !unit_keys.contains(key.as_str()) {
                        flatten(key, value, &mut fields);
                    }
                }
            }
            if let ResolvedTask::ExecutionUnit { parameters, .. } = task {
                // Namespace local fields internally, then shorten unambiguous names.
                flatten(
                    parameters.execution_unit(),
                    parameters.resolved_value(),
                    &mut fields,
                );
            }
            fields
        })
        .collect();
    let keys: BTreeSet<_> = fields
        .iter()
        .flat_map(|fields| fields.keys().cloned())
        .collect();
    let varying: Vec<_> = keys
        .into_iter()
        .filter(|key| {
            fields
                .iter()
                .skip(1)
                .any(|row| row.get(key) != fields[0].get(key))
        })
        .collect();
    let mut leaf_counts = BTreeMap::new();
    for key in &varying {
        *leaf_counts
            .entry(key.rsplit('.').next().unwrap())
            .or_insert(0_usize) += 1;
    }
    let mut labels: Vec<_> = fields
        .iter()
        .map(|row| {
            let mut parts = Vec::new();
            if !prefix.is_empty() {
                parts.push(prefix.to_owned());
            }
            for key in &varying {
                if let Some(value) = row.get(key) {
                    let leaf = key.rsplit('.').next().unwrap();
                    let name = if leaf_counts[leaf] == 1 { leaf } else { key };
                    parts.push(format!("{name}={value}"));
                }
            }
            parts.join(" ")
        })
        .collect();
    // Duplicate configurations and parameter-free tasks still need distinct labels.
    let mut counts = BTreeMap::new();
    for label in &labels {
        *counts.entry(label.clone()).or_insert(0_usize) += 1;
    }
    for (index, label) in labels.iter_mut().enumerate() {
        if label.is_empty() {
            *label = format!("task {}", index + 1);
        } else if counts[label] > 1 {
            label.push_str(&format!(" #{}", index + 1));
        }
    }
    labels
}
