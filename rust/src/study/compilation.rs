//! Private effect-free project-to-study composition.

use crate::config::advanced::ProjectSpecification;
use crate::state::advanced::{StateSchemaAccess, SystemStateSchema};
use crate::task::advanced::ModelCatalog;

use super::error::StudyError;
use super::plan::{Study, StudyPhase, StudyTask};

pub(crate) fn compile(
    project: ProjectSpecification,
    catalog: &ModelCatalog,
) -> Result<Study, StudyError> {
    let state_document = project.state_schema();
    let schema = <SystemStateSchema as StateSchemaAccess>::from_json_template_value(
        state_document.path(),
        state_document.json_value(),
    )?;

    let mut output_ordinal = 0_u64;
    let mut phases = Vec::with_capacity(project.phases().len());
    for phase in project.phases() {
        let mut tasks = Vec::with_capacity(phase.tasks().len());
        for input in phase.tasks() {
            let registration =
                catalog
                    .get(input.model())
                    .ok_or_else(|| StudyError::UnknownModel {
                        phase: phase.name().to_owned(),
                        model: input.model().to_owned(),
                    })?;
            if let Err(source) = registration.preflight(input, &schema) {
                return Err(StudyError::model_preflight(
                    phase.name(),
                    input.model(),
                    input.ordinal(),
                    source,
                ));
            }
            for field in input.display_fields() {
                if !schema.contains_field(field) {
                    return Err(StudyError::UnknownDisplayField {
                        phase: phase.name().to_owned(),
                        model: input.model().to_owned(),
                        field: field.to_owned(),
                    });
                }
            }

            let identity = format!(
                "{}/{:06}/{}-{:06}",
                phase.name(),
                output_ordinal,
                input.model(),
                input.ordinal()
            );
            let label = format!("{} #{}", input.model(), input.ordinal());
            tasks.push(StudyTask {
                identity: identity.into_boxed_str(),
                label: label.into_boxed_str(),
                output_ordinal,
                input: input.clone(),
                definition: registration.make_task(),
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
