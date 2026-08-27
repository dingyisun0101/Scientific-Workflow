//! Opaque generic task definitions created from models or programs.

use std::fmt;
use std::sync::Arc;

use crate::config::advanced::{ResolvedProgramTask, ResolvedTaskInput};
use crate::observation::advanced::BoundObservationPlan;

use super::execution::{ProgramDefinition, StatefulDefinition, TaskDefinition, TaskExecutionHost};
use super::model::ScientificModel;
use super::result::TaskResult;

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
    Model(ResolvedTaskInput),
    Program(ResolvedProgramTask),
}

impl Task {
    pub(crate) fn for_model<M>(
        input: ResolvedTaskInput,
        observation_plan: BoundObservationPlan,
    ) -> Self
    where
        M: ScientificModel,
    {
        Self {
            definition: Arc::new(StatefulDefinition::<M>::new(
                input.clone(),
                observation_plan,
            )),
            descriptor: TaskDescriptor::Model(input),
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
            TaskDescriptor::Model(_) => TaskKind::Model,
            TaskDescriptor::Program(_) => TaskKind::Program,
        }
    }

    pub(crate) fn kind_name(&self) -> &'static str {
        match &self.descriptor {
            TaskDescriptor::Model(_) => "model",
            TaskDescriptor::Program(program) => program.kind_name(),
        }
    }

    pub(crate) fn model(&self) -> Option<&str> {
        match &self.descriptor {
            TaskDescriptor::Model(input) => Some(input.model()),
            TaskDescriptor::Program(_) => None,
        }
    }

    pub(crate) fn input(&self) -> Option<&ResolvedTaskInput> {
        match &self.descriptor {
            TaskDescriptor::Model(input) => Some(input),
            TaskDescriptor::Program(_) => None,
        }
    }

    pub(crate) fn program(&self) -> Option<&ResolvedProgramTask> {
        match &self.descriptor {
            TaskDescriptor::Model(_) => None,
            TaskDescriptor::Program(program) => Some(program),
        }
    }

    pub(crate) fn timeout(&self) -> Option<std::time::Duration> {
        match &self.descriptor {
            TaskDescriptor::Model(input) => input.timeout(),
            TaskDescriptor::Program(program) => program.timeout(),
        }
    }

    pub(crate) fn subject(&self) -> &str {
        match &self.descriptor {
            TaskDescriptor::Model(input) => input.model(),
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
