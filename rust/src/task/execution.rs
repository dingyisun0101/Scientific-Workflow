//! Supported runtime port and private task execution implementations.

use std::ffi::OsString;
use std::path::Path;

use thiserror::Error;

use crate::config::advanced::{ResolvedModelParameters, ResolvedProgramTask};
use crate::observation::advanced::BoundObservationPlan;
use crate::state::advanced::{StateSchemaAccess, SystemState, SystemStateSchema};

use super::result::TaskResult;
use super::unit::{ExecutionUnit, ModelView};

/// The runtime-owned services a task may use while executing.
///
/// Implementations perform automatic state observation and automatic
/// lifecycle/progress snapshot publication at each method boundary. The task
/// module deliberately exposes no user callback for formatting messages,
/// choosing channels, managing storage, or mutating lifecycle state.
pub(crate) trait TaskExecutionHost {
    /// Reports whether cooperative cancellation has been requested.
    fn cancellation_requested(&self) -> bool;

    /// Executes one validated external program in Runtime's standardized task
    /// workspace and configuration environment.
    fn execute_program(&mut self, program: ProgramTaskInvocation<'_>) -> TaskResult;

    /// Accepts the validated observation plan and emits the initial observation and
    /// automatic initialized snapshot for `state`.
    fn begin_model(
        &mut self,
        index: usize,
        model_count: usize,
        identity: &str,
        plan: BoundObservationPlan,
        state: &SystemState,
        target_iteration: Option<u64>,
    ) -> TaskResult;

    /// Emits the automatic observation and progress snapshot after one
    /// successful model step.
    fn observe_model_step(
        &mut self,
        index: usize,
        state: &SystemState,
        target_iteration: Option<u64>,
    ) -> TaskResult;

    /// Emits the final observation and completion snapshot exactly once
    /// after the model reports completion.
    fn observe_model_final(
        &mut self,
        index: usize,
        state: &SystemState,
        target_iteration: Option<u64>,
    ) -> TaskResult;
}

/// Borrowed semantic invocation facts passed from Task to Runtime.
pub(crate) struct ProgramTaskInvocation<'a> {
    executable: &'a Path,
    args: &'a [OsString],
    kind: &'a str,
    python_script: Option<&'a Path>,
    python_environment_manager: Option<&'a str>,
}

impl ProgramTaskInvocation<'_> {
    pub(crate) fn executable(&self) -> &Path {
        self.executable
    }

    pub(crate) fn args(&self) -> &[OsString] {
        self.args
    }

    pub(crate) fn kind(&self) -> &str {
        self.kind
    }

    pub(crate) fn python_script(&self) -> Option<&Path> {
        self.python_script
    }

    pub(crate) fn python_environment_manager(&self) -> Option<&str> {
        self.python_environment_manager
    }
}

impl<'a> From<&'a ResolvedProgramTask> for ProgramTaskInvocation<'a> {
    fn from(program: &'a ResolvedProgramTask) -> Self {
        Self {
            executable: program.program(),
            args: program.args(),
            kind: program.kind_name(),
            python_script: program.python_script(),
            python_environment_manager: program.python_environment_manager(),
        }
    }
}

/// Type-erased execution contract consumed by the Workflow runtime.
///
/// Study ordinarily constructs [`super::definition::Task`] from a registered
/// model. Application code does not implement this trait. Replacement runtimes
/// may execute a task from resolved model parameters through this boundary.
pub(crate) trait TaskDefinition: Send + Sync {
    /// Obtains typed constants from resolved parameters and executes through `host`.
    ///
    /// Returning `Ok(())` while `host.cancellation_requested()` is true means
    /// cooperative cancellation, not successful completion; lifecycle status
    /// remains the runtime's responsibility.
    fn execute(&self, host: &mut dyn TaskExecutionHost) -> TaskResult;
}

pub(crate) struct StatefulDefinition<M> {
    parameters: ResolvedModelParameters,
    schema: SystemStateSchema,
    observation_plan: BoundObservationPlan,
    marker: std::marker::PhantomData<fn() -> M>,
}

impl<M> StatefulDefinition<M>
where
    M: ExecutionUnit,
{
    pub(crate) fn new(
        parameters: ResolvedModelParameters,
        schema: SystemStateSchema,
        observation_plan: BoundObservationPlan,
    ) -> Self {
        Self {
            parameters,
            schema,
            observation_plan,
            marker: std::marker::PhantomData,
        }
    }
}

impl<M> TaskDefinition for StatefulDefinition<M>
where
    M: ExecutionUnit,
{
    fn execute(&self, host: &mut dyn TaskExecutionHost) -> TaskResult {
        if host.cancellation_requested() {
            return Ok(());
        }

        let constants: M::Constants = self.parameters.decode()?;
        let schema = self.schema.clone();

        let mut unit = M::initialize(constants, &schema)?;
        let model_count = unit.model_count();
        if model_count == 0 {
            return Err(ModelContractError::EmptyExecutionUnit.into());
        }
        if unit.model(model_count).is_some() {
            return Err(
                ModelContractError::ModelOutsideDeclaredCount { index: model_count }.into(),
            );
        }

        let mut models = inspect_initial_models(&unit, &schema, model_count)?;
        for (index, model) in models.iter().enumerate() {
            let view = required_model(&unit, index)?;
            host.begin_model(
                index,
                model_count,
                &model.identity,
                self.observation_plan.clone(),
                view.state(),
                model.target,
            )?;
            if model.complete {
                host.observe_model_final(index, view.state(), model.target)?;
            }
        }

        while models.iter().any(|model| !model.complete) {
            if host.cancellation_requested() {
                return Ok(());
            }

            unit.step()?;
            if unit.model_count() != model_count {
                return Err(ModelContractError::ModelCountChanged {
                    previous: model_count,
                    next: unit.model_count(),
                }
                .into());
            }
            let next_models = inspect_models(&unit, &schema, &models)?;
            let mut advanced = false;
            for (index, (previous, next)) in models.iter().zip(&next_models).enumerate() {
                let view = required_model(&unit, index)?;
                if previous.complete {
                    if !next.complete {
                        return Err(ModelContractError::CompletionReversed {
                            identity: next.identity.clone(),
                        }
                        .into());
                    }
                    if next.iteration != previous.iteration {
                        return Err(ModelContractError::CompletedModelAdvanced {
                            identity: next.identity.clone(),
                            previous: previous.iteration,
                            next: next.iteration,
                        }
                        .into());
                    }
                    continue;
                }
                if next.iteration < previous.iteration {
                    return Err(ModelContractError::IterationRegressed {
                        identity: next.identity.clone(),
                        previous: previous.iteration,
                        next: next.iteration,
                    }
                    .into());
                }
                if next.iteration > previous.iteration {
                    advanced = true;
                    host.observe_model_step(index, view.state(), next.target)?;
                }
                if next.complete {
                    host.observe_model_final(index, view.state(), next.target)?;
                }
            }
            if !advanced {
                return Err(ModelContractError::NonAdvancingStep.into());
            }
            models = next_models;
        }
        Ok(())
    }
}

pub(crate) struct ProgramDefinition {
    program: ResolvedProgramTask,
}

impl ProgramDefinition {
    pub(crate) fn new(program: ResolvedProgramTask) -> Self {
        Self { program }
    }
}

impl TaskDefinition for ProgramDefinition {
    fn execute(&self, host: &mut dyn TaskExecutionHost) -> TaskResult {
        if host.cancellation_requested() {
            return Ok(());
        }
        host.execute_program((&self.program).into())
    }
}

fn validate_state(
    state: &SystemState,
    schema: &SystemStateSchema,
    expected_address: *const SystemState,
) -> Result<(), ModelContractError> {
    if !std::ptr::eq(state, expected_address) {
        return Err(ModelContractError::StateOwnerChanged);
    }
    if !schema.shares_schema_instance(state.schema()) {
        return Err(ModelContractError::SchemaChanged {
            iteration: state.time().iteration(),
        });
    }
    Ok(())
}

struct ModelSnapshot {
    identity: Box<str>,
    state_address: *const SystemState,
    iteration: u64,
    complete: bool,
    target: Option<u64>,
}

fn required_model<M>(unit: &M, index: usize) -> Result<ModelView<'_>, ModelContractError>
where
    M: ExecutionUnit,
{
    unit.model(index)
        .ok_or(ModelContractError::MissingModel { index })
}

fn inspect_initial_models<M>(
    unit: &M,
    schema: &SystemStateSchema,
    model_count: usize,
) -> Result<Vec<ModelSnapshot>, ModelContractError>
where
    M: ExecutionUnit,
{
    let mut identities = std::collections::HashSet::with_capacity(model_count);
    let mut models = Vec::with_capacity(model_count);
    for index in 0..model_count {
        let model = required_model(unit, index)?;
        validate_identity(model.identity(), index)?;
        if !identities.insert(model.identity()) {
            return Err(ModelContractError::DuplicateIdentity {
                identity: model.identity().into(),
            });
        }
        validate_state(model.state(), schema, model.state() as *const SystemState)?;
        models.push(ModelSnapshot {
            identity: model.identity().into(),
            state_address: model.state() as *const SystemState,
            iteration: model.state().time().iteration(),
            complete: model.is_complete(),
            target: validate_target(model.state(), model.target_iteration())?,
        });
    }
    Ok(models)
}

fn inspect_models<M>(
    unit: &M,
    schema: &SystemStateSchema,
    previous: &[ModelSnapshot],
) -> Result<Vec<ModelSnapshot>, ModelContractError>
where
    M: ExecutionUnit,
{
    let mut models = Vec::with_capacity(previous.len());
    for (index, expected) in previous.iter().enumerate() {
        let model = required_model(unit, index)?;
        if model.identity() != expected.identity.as_ref() {
            return Err(ModelContractError::IdentityChanged {
                index,
                previous: expected.identity.clone(),
                next: model.identity().into(),
            });
        }
        validate_state(model.state(), schema, expected.state_address)?;
        let target = validate_target(model.state(), model.target_iteration())?;
        validate_target_progress(expected.target, target)?;
        models.push(ModelSnapshot {
            identity: expected.identity.clone(),
            state_address: expected.state_address,
            iteration: model.state().time().iteration(),
            complete: model.is_complete(),
            target,
        });
    }
    Ok(models)
}

fn validate_identity(identity: &str, index: usize) -> Result<(), ModelContractError> {
    if identity.is_empty() || identity.trim() != identity {
        Err(ModelContractError::InvalidIdentity {
            index,
            identity: identity.into(),
        })
    } else {
        Ok(())
    }
}

fn validate_target(
    state: &SystemState,
    target: Option<u64>,
) -> Result<Option<u64>, ModelContractError> {
    if let Some(target) = target {
        let current = state.time().iteration();
        if target < current {
            return Err(ModelContractError::TargetBeforeCurrent { target, current });
        }
    }
    Ok(target)
}

fn validate_target_progress(
    previous: Option<u64>,
    next: Option<u64>,
) -> Result<(), ModelContractError> {
    match (previous, next) {
        (Some(previous), None) => Err(ModelContractError::TargetRemoved { previous }),
        (Some(previous), Some(next)) if next < previous => {
            Err(ModelContractError::TargetDecreased { previous, next })
        }
        _ => Ok(()),
    }
}

#[derive(Debug, Error)]
enum ModelContractError {
    #[error("a scientific execution unit must expose at least one model")]
    EmptyExecutionUnit,
    #[error("execution unit did not expose declared model index {index}")]
    MissingModel { index: usize },
    #[error("execution unit exposed model index {index} outside its declared count")]
    ModelOutsideDeclaredCount { index: usize },
    #[error("execution unit model count changed from {previous} to {next}")]
    ModelCountChanged { previous: usize, next: usize },
    #[error("model identity `{identity}` at index {index} is empty or has surrounding whitespace")]
    InvalidIdentity { index: usize, identity: Box<str> },
    #[error("model identity `{identity}` appears more than once in one execution unit")]
    DuplicateIdentity { identity: Box<str> },
    #[error("model identity at index {index} changed from `{previous}` to `{next}`")]
    IdentityChanged {
        index: usize,
        previous: Box<str>,
        next: Box<str>,
    },
    #[error("the scientific model returned a different SystemState owner")]
    StateOwnerChanged,
    #[error("the scientific model changed its state schema at iteration {iteration}")]
    SchemaChanged { iteration: u64 },
    #[error("a successful execution-unit step did not advance any incomplete model")]
    NonAdvancingStep,
    #[error("model `{identity}` iteration regressed from {previous} to {next}")]
    IterationRegressed {
        identity: Box<str>,
        previous: u64,
        next: u64,
    },
    #[error("completed model `{identity}` advanced from {previous} to {next}")]
    CompletedModelAdvanced {
        identity: Box<str>,
        previous: u64,
        next: u64,
    },
    #[error("completed model `{identity}` became incomplete")]
    CompletionReversed { identity: Box<str> },
    #[error("target iteration {target} is before current iteration {current}")]
    TargetBeforeCurrent { target: u64, current: u64 },
    #[error("target iteration decreased from {previous} to {next}")]
    TargetDecreased { previous: u64, next: u64 },
    #[error("target iteration disappeared after being reported as {previous}")]
    TargetRemoved { previous: u64 },
}
