//! Supported runtime port and private task execution implementations.

use thiserror::Error;

use crate::config::advanced::ResolvedTaskInput;
use crate::observation::advanced::BoundObservationPlan;
use crate::state::advanced::{StateSchemaAccess, SystemState, SystemStateSchema};

use super::model::ScientificModel;
use super::result::TaskResult;

/// The runtime-owned services a task may use while executing.
///
/// Implementations perform automatic state observation and automatic
/// lifecycle/progress snapshot publication at each method boundary. The task
/// module deliberately exposes no user callback for formatting messages,
/// choosing channels, managing storage, or mutating lifecycle state.
pub(crate) trait TaskExecutionHost {
    /// Borrows the state schema loaded from the project's `config/state.json`.
    fn state_schema(&self) -> TaskResult<&SystemStateSchema>;

    /// Reports whether cooperative cancellation has been requested.
    fn cancellation_requested(&self) -> bool;

    /// Accepts the validated observation plan and emits the initial observation and
    /// automatic initialized snapshot for `state`.
    fn begin_model(
        &mut self,
        plan: BoundObservationPlan,
        state: &SystemState,
        target_iteration: Option<u64>,
    ) -> TaskResult;

    /// Emits the automatic observation and progress snapshot after one
    /// successful model step.
    fn observe_model_step(
        &mut self,
        state: &SystemState,
        target_iteration: Option<u64>,
    ) -> TaskResult;

    /// Emits the final observation and completion snapshot exactly once
    /// after the model reports completion.
    fn observe_model_final(
        &mut self,
        state: &SystemState,
        target_iteration: Option<u64>,
    ) -> TaskResult;
}

/// Type-erased execution contract consumed by the Workflow runtime.
///
/// Study ordinarily constructs [`super::definition::Task`] from a registered
/// model. Application code does not implement this trait. Replacement runtimes
/// may execute a task from a resolved task input through this boundary.
pub(crate) trait TaskDefinition: Send + Sync {
    /// Obtains typed constants from `input` and executes through `host`.
    ///
    /// Returning `Ok(())` while `host.cancellation_requested()` is true means
    /// cooperative cancellation, not successful completion; lifecycle status
    /// remains the runtime's responsibility.
    fn execute(
        &self,
        input: &ResolvedTaskInput,
        observation_plan: &BoundObservationPlan,
        host: &mut dyn TaskExecutionHost,
    ) -> TaskResult;
}

pub(crate) struct StatefulDefinition<M> {
    marker: std::marker::PhantomData<fn() -> M>,
}

impl<M> StatefulDefinition<M>
where
    M: ScientificModel,
{
    pub(crate) fn new() -> Self {
        Self {
            marker: std::marker::PhantomData,
        }
    }
}

impl<M> TaskDefinition for StatefulDefinition<M>
where
    M: ScientificModel,
{
    fn execute(
        &self,
        input: &ResolvedTaskInput,
        observation_plan: &BoundObservationPlan,
        host: &mut dyn TaskExecutionHost,
    ) -> TaskResult {
        if host.cancellation_requested() {
            return Ok(());
        }

        let constants: M::Constants = input.decode()?;
        let schema = host.state_schema()?.clone();

        let mut model = M::initialize(constants, &schema)?;
        let state_address = model.state() as *const SystemState;
        validate_state(model.state(), &schema, state_address)?;
        let mut target = validate_target(model.state(), model.target_iteration())?;

        host.begin_model(observation_plan.clone(), model.state(), target)?;

        while !model.is_complete() {
            if host.cancellation_requested() {
                return Ok(());
            }

            let previous_iteration = model.state().time().iteration();
            model.step()?;
            validate_state(model.state(), &schema, state_address)?;

            let next_iteration = model.state().time().iteration();
            if next_iteration <= previous_iteration {
                return Err(ModelContractError::NonAdvancingStep {
                    previous: previous_iteration,
                    next: next_iteration,
                }
                .into());
            }

            let next_target = validate_target(model.state(), model.target_iteration())?;
            validate_target_progress(target, next_target)?;
            target = next_target;
            host.observe_model_step(model.state(), target)?;
        }

        validate_state(model.state(), &schema, state_address)?;
        let final_target = validate_target(model.state(), model.target_iteration())?;
        validate_target_progress(target, final_target)?;
        host.observe_model_final(model.state(), final_target)
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
    #[error("the scientific model returned a different SystemState owner")]
    StateOwnerChanged,
    #[error("the scientific model changed its state schema at iteration {iteration}")]
    SchemaChanged { iteration: u64 },
    #[error("a successful model step did not advance iteration: {previous} -> {next}")]
    NonAdvancingStep { previous: u64, next: u64 },
    #[error("target iteration {target} is before current iteration {current}")]
    TargetBeforeCurrent { target: u64, current: u64 },
    #[error("target iteration decreased from {previous} to {next}")]
    TargetDecreased { previous: u64, next: u64 },
    #[error("target iteration disappeared after being reported as {previous}")]
    TargetRemoved { previous: u64 },
}
