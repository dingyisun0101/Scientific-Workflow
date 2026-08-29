//! Opaque generic task definitions created from execution units or programs.

use std::fmt;
use std::path::Path;
use std::sync::Arc;

use crate::config::{ResolvedExecutionUnitParameters, ResolvedProgramTask};
use crate::observation::BoundObservationPlan;
use crate::state::SystemStateSchema;

use super::execution::{
    ExecutionUnitDefinition, ProgramDefinition, TaskDefinition, TaskExecutionHost,
};
use super::result::TaskResult;
use super::unit::ExecutionUnit;

/// A reusable type-erased workload definition.
///
/// `Task` deliberately carries no user-supplied identity, output path,
/// lifecycle callback, persistence session, or scheduler policy. Study derives
/// those concerns while the descriptor records only irreducible execution-unit/program
/// invocation intent.
#[derive(Clone)]
pub(crate) struct Task {
    definition: Arc<dyn TaskDefinition>,
    descriptor: TaskDescriptor,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TaskKind {
    ExecutionUnit,
    Program,
}

#[derive(Clone)]
enum TaskDescriptor {
    ExecutionUnit {
        parameters: ResolvedExecutionUnitParameters,
        state: Box<str>,
    },
    Program(ResolvedProgramTask),
}

/// Borrowed semantic provenance for one configured execution-unit invocation.
pub(crate) struct ExecutionUnitTaskProvenance<'a> {
    parameters: &'a ResolvedExecutionUnitParameters,
    state: &'a str,
}

impl ExecutionUnitTaskProvenance<'_> {
    pub(crate) fn execution_unit(&self) -> &str {
        self.parameters.execution_unit()
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
    pub(crate) fn for_execution_unit<U>(
        parameters: ResolvedExecutionUnitParameters,
        state: Box<str>,
        schema: SystemStateSchema,
        observation_plan: BoundObservationPlan,
    ) -> Self
    where
        U: ExecutionUnit,
    {
        Self {
            definition: Arc::new(ExecutionUnitDefinition::<U>::new(
                parameters.clone(),
                schema,
                observation_plan,
            )),
            descriptor: TaskDescriptor::ExecutionUnit { parameters, state },
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
            TaskDescriptor::ExecutionUnit { .. } => TaskKind::ExecutionUnit,
            TaskDescriptor::Program(_) => TaskKind::Program,
        }
    }

    pub(crate) fn kind_name(&self) -> &'static str {
        match &self.descriptor {
            TaskDescriptor::ExecutionUnit { .. } => "execution_unit",
            TaskDescriptor::Program(program) => program.kind_name(),
        }
    }

    pub(crate) fn execution_unit(&self) -> Option<&str> {
        match &self.descriptor {
            TaskDescriptor::ExecutionUnit { parameters, .. } => Some(parameters.execution_unit()),
            TaskDescriptor::Program(_) => None,
        }
    }

    pub(crate) fn execution_unit_provenance(&self) -> Option<ExecutionUnitTaskProvenance<'_>> {
        match &self.descriptor {
            TaskDescriptor::ExecutionUnit { parameters, state } => {
                Some(ExecutionUnitTaskProvenance { parameters, state })
            }
            TaskDescriptor::Program(_) => None,
        }
    }

    fn program(&self) -> Option<&ResolvedProgramTask> {
        match &self.descriptor {
            TaskDescriptor::ExecutionUnit { .. } => None,
            TaskDescriptor::Program(program) => Some(program),
        }
    }

    pub(crate) fn program_path(&self) -> Option<&Path> {
        self.program().map(ResolvedProgramTask::program)
    }

    pub(crate) fn python_script(&self) -> Option<&Path> {
        self.program().and_then(ResolvedProgramTask::python_script)
    }

    pub(crate) fn program_seed_purpose(&self) -> Option<&str> {
        self.program().and_then(ResolvedProgramTask::seed_purpose)
    }

    pub(crate) fn timeout(&self) -> Option<std::time::Duration> {
        match &self.descriptor {
            TaskDescriptor::ExecutionUnit { parameters, .. } => parameters.timeout(),
            TaskDescriptor::Program(program) => program.timeout(),
        }
    }

    pub(crate) fn subject(&self) -> &str {
        match &self.descriptor {
            TaskDescriptor::ExecutionUnit { parameters, .. } => parameters.execution_unit(),
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
