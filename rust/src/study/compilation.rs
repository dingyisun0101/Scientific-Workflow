//! Private effect-free project-to-study composition.

use crate::config::advanced::{ProjectSpecification, ResolvedTask};
use crate::state::advanced::SystemStateSchema;
use crate::task::advanced::{ModelCatalog, Task};

use super::error::StudyError;
use super::plan::{Study, StudyPhase, StudyTask};

pub(crate) fn compile(
    project: ProjectSpecification,
    catalog: &ModelCatalog,
) -> Result<Study, StudyError> {
    let state_document = project.state_schema();
    let schema = SystemStateSchema::from_json_template_value(
        state_document.path(),
        state_document.json_value(),
    )?;

    let mut output_ordinal = 0_u64;
    let mut phases = Vec::with_capacity(project.phases().len());
    for phase in project.phases() {
        let mut tasks = Vec::with_capacity(phase.tasks().len());
        for resolved in phase.tasks() {
            let (identity_suffix, label, task) = match resolved {
                ResolvedTask::Model(input) => {
                    let registration =
                        catalog
                            .get(input.model())
                            .ok_or_else(|| StudyError::UnknownModel {
                                phase: phase.name().to_owned(),
                                model: input.model().to_owned(),
                            })?;
                    let observation_plan =
                        registration.preflight(input, &schema).map_err(|source| {
                            StudyError::model_preflight(
                                phase.name(),
                                input.model(),
                                input.ordinal(),
                                source,
                            )
                        })?;
                    (
                        format!("{}-{:06}", input.model(), input.ordinal()),
                        format!("{} #{}", input.model(), input.ordinal()),
                        registration.make_task(input.clone(), observation_plan),
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
    Ok(Study::from_parts(
        project,
        schema,
        phases.into_boxed_slice(),
    ))
}
