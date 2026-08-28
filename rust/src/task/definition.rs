//! Opaque generic task definitions created from models or programs.

use std::fmt;
use std::path::Path;
use std::sync::Arc;

use crate::config::advanced::{ResolvedModelParameters, ResolvedProgramTask};
use crate::observation::advanced::BoundObservationPlan;
use crate::state::advanced::SystemStateSchema;

use super::execution::{ProgramDefinition, StatefulDefinition, TaskDefinition, TaskExecutionHost};
use super::result::TaskResult;
use super::unit::ExecutionUnit;

/// A reusable type-erased workload definition.
///
/// `Task` deliberately carries no user-supplied identity, output path,
/// lifecycle callback, persistence session, or scheduler policy. Study derives
/// those concerns while the descriptor records only irreducible model/program
/// invocation intent.
#[derive(Clone)]
pub(crate) struct Task {
    definition: Arc<dyn TaskDefinition>,
    descriptor: TaskDescriptor,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TaskKind {
    Model,
    Program,
}

#[derive(Clone)]
enum TaskDescriptor {
    Model {
        parameters: ResolvedModelParameters,
        state: Box<str>,
    },
    Program(ResolvedProgramTask),
}

/// Borrowed semantic provenance for one configured model invocation.
pub(crate) struct ModelTaskProvenance<'a> {
    parameters: &'a ResolvedModelParameters,
    state: &'a str,
}

impl ModelTaskProvenance<'_> {
    pub(crate) fn model(&self) -> &str {
        self.parameters.model()
    }

    pub(crate) fn state(&self) -> &str {
        self.state
    }

    pub(crate) fn parameter_ordinal(&self) -> u64 {
        self.parameters.ordinal()
    }

    pub(crate) fn parameter_source(&self) -> &Path {
        self.parameters.source_path()
    }

    pub(crate) fn constants(&self) -> &serde_json::Value {
        self.parameters.resolved_value()
    }
}

impl Task {
    pub(crate) fn for_model<M>(
        parameters: ResolvedModelParameters,
        state: Box<str>,
        schema: SystemStateSchema,
        observation_plan: BoundObservationPlan,
    ) -> Self
    where
        M: ExecutionUnit,
    {
        Self {
            definition: Arc::new(StatefulDefinition::<M>::new(
                parameters.clone(),
                schema,
                observation_plan,
            )),
            descriptor: TaskDescriptor::Model { parameters, state },
        }
    }

    pub(crate) fn for_program(program: ResolvedProgramTask) -> Self {
        Self {
            definition: Arc::new(ProgramDefinition::new(program.clone())),
            descriptor: TaskDescriptor::Program(program),
        }
    }

    pub(crate) fn kind(&self) -> TaskKind {
        match self.descriptor {
            TaskDescriptor::Model { .. } => TaskKind::Model,
            TaskDescriptor::Program(_) => TaskKind::Program,
        }
    }

    pub(crate) fn kind_name(&self) -> &'static str {
        match &self.descriptor {
            TaskDescriptor::Model { .. } => "model",
            TaskDescriptor::Program(program) => program.kind_name(),
        }
    }

    pub(crate) fn model(&self) -> Option<&str> {
        match &self.descriptor {
            TaskDescriptor::Model { parameters, .. } => Some(parameters.model()),
            TaskDescriptor::Program(_) => None,
        }
    }

    pub(crate) fn model_provenance(&self) -> Option<ModelTaskProvenance<'_>> {
        match &self.descriptor {
            TaskDescriptor::Model { parameters, state } => {
                Some(ModelTaskProvenance { parameters, state })
            }
            TaskDescriptor::Program(_) => None,
        }
    }

    fn program(&self) -> Option<&ResolvedProgramTask> {
        match &self.descriptor {
            TaskDescriptor::Model { .. } => None,
            TaskDescriptor::Program(program) => Some(program),
        }
    }

    pub(crate) fn program_path(&self) -> Option<&Path> {
        self.program().map(ResolvedProgramTask::program)
    }

    pub(crate) fn program_kind_name(&self) -> Option<&str> {
        self.program().map(ResolvedProgramTask::kind_name)
    }

    pub(crate) fn python_script(&self) -> Option<&Path> {
        self.program().and_then(ResolvedProgramTask::python_script)
    }

    pub(crate) fn timeout(&self) -> Option<std::time::Duration> {
        match &self.descriptor {
            TaskDescriptor::Model { parameters, .. } => parameters.timeout(),
            TaskDescriptor::Program(program) => program.timeout(),
        }
    }

    pub(crate) fn subject(&self) -> &str {
        match &self.descriptor {
            TaskDescriptor::Model { parameters, .. } => parameters.model(),
            TaskDescriptor::Program(program) => program.subject(),
        }
    }
}

impl TaskDefinition for Task {
    fn execute(&self, host: &mut dyn TaskExecutionHost) -> TaskResult {
        self.definition.execute(host)
    }
}

impl fmt::Debug for Task {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Task")
            .field("kind", &self.kind())
            .field("subject", &self.subject())
            .finish_non_exhaustive()
    }
}
