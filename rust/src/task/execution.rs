//! Supported runtime port and private task execution implementations.

use std::ffi::OsString;
use std::path::Path;

use serde_json::{Map, Value};
use thiserror::Error;

use crate::config::{ResolvedExecutionUnitParameters, ResolvedProgramTask};
use crate::observation::BoundObservationPlan;
use crate::state::{SystemState, SystemStateSchema};

use super::result::TaskResult;
use super::unit::{ExecutionUnit, InitializationContext, MemberView};

/// The runtime-owned services a task may use while executing.
///
/// Implementations perform automatic state observation and automatic
/// lifecycle/progress snapshot publication at each method boundary. The task
/// module deliberately exposes no user callback for formatting messages,
/// choosing channels, managing storage, or mutating lifecycle state.
pub(crate) trait TaskExecutionHost {
    /// Reports whether cooperative cancellation has been requested.
    fn cancellation_requested(&self) -> bool;

    /// Returns initialization facts for an execution-unit task.
    fn initialization_context(&self) -> Option<&InitializationContext>;

    /// Executes one validated external program in Runtime's standardized task
    /// workspace and configuration environment.
    fn execute_program(&mut self, program: ProgramTaskInvocation<'_>) -> TaskResult;

    /// Accepts the validated observation plan and emits the initial observation and
    /// automatic initialized snapshot for `state`.
    fn begin_member(&mut self, member: MemberInitialization<'_>) -> TaskResult;

    /// Emits the automatic observation and progress snapshot after one
    /// successful member step.
    fn observe_member_step(
        &mut self,
        index: usize,
        state: &SystemState,
        target_iteration: Option<u64>,
    ) -> TaskResult;

    /// Emits the final observation and completion snapshot exactly once
    /// after the member reports completion.
    fn observe_member_final(
        &mut self,
        index: usize,
        state: &SystemState,
        target_iteration: Option<u64>,
        completion_reason: Option<Map<String, Value>>,
    ) -> TaskResult;
}

/// Complete semantic handoff when one member recording begins.
pub(crate) struct MemberInitialization<'a> {
    pub(crate) index: usize,
    pub(crate) member_count: usize,
    pub(crate) identity: &'a str,
    pub(crate) seed_derivation: Option<serde_json::Value>,
    pub(crate) plan: BoundObservationPlan,
    pub(crate) state: &'a SystemState,
    pub(crate) target_iteration: Option<u64>,
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
/// execution unit. Application code does not implement this trait. Replacement runtimes
/// may execute a task from resolved execution-unit parameters through this boundary.
pub(crate) trait TaskDefinition: Send + Sync {
    /// Obtains typed constants from resolved parameters and executes through `host`.
    ///
    /// Returning `Ok(())` while `host.cancellation_requested()` is true means
    /// cooperative cancellation, not successful completion; lifecycle status
    /// remains the runtime's responsibility.
    fn execute(&self, host: &mut dyn TaskExecutionHost) -> TaskResult;
}

pub(crate) struct ExecutionUnitDefinition<U> {
    parameters: ResolvedExecutionUnitParameters,
    schema: SystemStateSchema,
    observation_plan: BoundObservationPlan,
    marker: std::marker::PhantomData<fn() -> U>,
}

impl<U> ExecutionUnitDefinition<U>
where
    U: ExecutionUnit,
{
    pub(crate) fn new(
        parameters: ResolvedExecutionUnitParameters,
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

impl<U> TaskDefinition for ExecutionUnitDefinition<U>
where
    U: ExecutionUnit,
{
    fn execute(&self, host: &mut dyn TaskExecutionHost) -> TaskResult {
        if host.cancellation_requested() {
            return Ok(());
        }

        let constants: U::Constants = self.parameters.decode()?;
        let schema = self.schema.clone();

        let context = host
            .initialization_context()
            .expect("an execution-unit task retains an initialization context");
        let mut unit = U::initialize(constants, &schema, context)?;
        let member_count = unit.member_count();
        if member_count == 0 {
            return Err(MemberContractError::EmptyExecutionUnit.into());
        }
        if unit.member(member_count).is_some() {
            return Err(MemberContractError::MemberOutsideDeclaredCount {
                index: member_count,
            }
            .into());
        }

        let mut members = inspect_initial_members(&unit, &schema, member_count)?;
        context
            .validate_member_identities(members.iter().map(|member| member.identity.as_ref()))?;
        let seed_derivations = members
            .iter()
            .map(|member| context.metadata_for_member(&member.identity))
            .collect::<Vec<_>>();
        for (index, member) in members.iter().enumerate() {
            let view = required_member(&unit, index)?;
            host.begin_member(MemberInitialization {
                index,
                member_count,
                identity: &member.identity,
                seed_derivation: seed_derivations[index].clone(),
                plan: self.observation_plan.clone(),
                state: view.state(),
                target_iteration: member.target,
            })?;
            if member.complete {
                host.observe_member_final(
                    index,
                    view.state(),
                    member.target,
                    completion_reason(view),
                )?;
            }
        }

        while members.iter().any(|member| !member.complete) {
            if host.cancellation_requested() {
                return Ok(());
            }

            unit.step()?;
            if unit.member_count() != member_count {
                return Err(MemberContractError::MemberCountChanged {
                    previous: member_count,
                    next: unit.member_count(),
                }
                .into());
            }
            let next_members = inspect_members(&unit, &schema, &members)?;
            let mut advanced = false;
            for (index, (previous, next)) in members.iter().zip(&next_members).enumerate() {
                let view = required_member(&unit, index)?;
                if previous.complete {
                    if !next.complete {
                        return Err(MemberContractError::CompletionReversed {
                            identity: next.identity.clone(),
                        }
                        .into());
                    }
                    if next.iteration != previous.iteration {
                        return Err(MemberContractError::CompletedMemberAdvanced {
                            identity: next.identity.clone(),
                            previous: previous.iteration,
                            next: next.iteration,
                        }
                        .into());
                    }
                    continue;
                }
                if next.iteration < previous.iteration {
                    return Err(MemberContractError::IterationRegressed {
                        identity: next.identity.clone(),
                        previous: previous.iteration,
                        next: next.iteration,
                    }
                    .into());
                }
                if next.iteration > previous.iteration {
                    advanced = true;
                    host.observe_member_step(index, view.state(), next.target)?;
                }
                if next.complete {
                    host.observe_member_final(
                        index,
                        view.state(),
                        next.target,
                        completion_reason(view),
                    )?;
                }
            }
            if !advanced {
                return Err(MemberContractError::NonAdvancingStep.into());
            }
            members = next_members;
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
) -> Result<(), MemberContractError> {
    if !std::ptr::eq(state, expected_address) {
        return Err(MemberContractError::StateOwnerChanged);
    }
    if !schema.shares_schema_instance(state.schema()) {
        return Err(MemberContractError::SchemaChanged {
            iteration: state.time().iteration(),
        });
    }
    Ok(())
}

struct MemberSnapshot {
    identity: Box<str>,
    state_address: *const SystemState,
    iteration: u64,
    complete: bool,
    target: Option<u64>,
}

fn required_member<M>(unit: &M, index: usize) -> Result<MemberView<'_>, MemberContractError>
where
    M: ExecutionUnit,
{
    unit.member(index)
        .ok_or(MemberContractError::MissingMember { index })
}

fn inspect_initial_members<M>(
    unit: &M,
    schema: &SystemStateSchema,
    member_count: usize,
) -> Result<Vec<MemberSnapshot>, MemberContractError>
where
    M: ExecutionUnit,
{
    let mut identities = std::collections::HashSet::with_capacity(member_count);
    let mut members = Vec::with_capacity(member_count);
    for index in 0..member_count {
        let member = required_member(unit, index)?;
        validate_identity(member.identity(), index)?;
        if !identities.insert(member.identity()) {
            return Err(MemberContractError::DuplicateIdentity {
                identity: member.identity().into(),
            });
        }
        validate_state(member.state(), schema, member.state() as *const SystemState)?;
        members.push(MemberSnapshot {
            identity: member.identity().into(),
            state_address: member.state() as *const SystemState,
            iteration: member.state().time().iteration(),
            complete: member.completion().is_some(),
            target: validate_target(member.state(), member.target_iteration())?,
        });
    }
    Ok(members)
}

fn inspect_members<M>(
    unit: &M,
    schema: &SystemStateSchema,
    previous: &[MemberSnapshot],
) -> Result<Vec<MemberSnapshot>, MemberContractError>
where
    M: ExecutionUnit,
{
    let mut members = Vec::with_capacity(previous.len());
    for (index, expected) in previous.iter().enumerate() {
        let member = required_member(unit, index)?;
        if member.identity() != expected.identity.as_ref() {
            return Err(MemberContractError::IdentityChanged {
                index,
                previous: expected.identity.clone(),
                next: member.identity().into(),
            });
        }
        validate_state(member.state(), schema, expected.state_address)?;
        let target = validate_target(member.state(), member.target_iteration())?;
        validate_target_progress(expected.target, target)?;
        members.push(MemberSnapshot {
            identity: expected.identity.clone(),
            state_address: expected.state_address,
            iteration: member.state().time().iteration(),
            complete: member.completion().is_some(),
            target,
        });
    }
    Ok(members)
}

fn completion_reason(view: MemberView<'_>) -> Option<Map<String, Value>> {
    view.completion()
        .expect("a completed member exposes completion details")
        .reason()
        .cloned()
}

fn validate_identity(identity: &str, index: usize) -> Result<(), MemberContractError> {
    if identity.is_empty() || identity.trim() != identity {
        Err(MemberContractError::InvalidIdentity {
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
) -> Result<Option<u64>, MemberContractError> {
    if let Some(target) = target {
        let current = state.time().iteration();
        if target < current {
            return Err(MemberContractError::TargetBeforeCurrent { target, current });
        }
    }
    Ok(target)
}

fn validate_target_progress(
    previous: Option<u64>,
    next: Option<u64>,
) -> Result<(), MemberContractError> {
    match (previous, next) {
        (Some(previous), None) => Err(MemberContractError::TargetRemoved { previous }),
        (Some(previous), Some(next)) if next < previous => {
            Err(MemberContractError::TargetDecreased { previous, next })
        }
        _ => Ok(()),
    }
}

#[derive(Debug, Error)]
enum MemberContractError {
    #[error("a scientific execution unit must expose at least one member")]
    EmptyExecutionUnit,
    #[error("execution unit did not expose declared member index {index}")]
    MissingMember { index: usize },
    #[error("execution unit exposed member index {index} outside its declared count")]
    MemberOutsideDeclaredCount { index: usize },
    #[error("execution unit member count changed from {previous} to {next}")]
    MemberCountChanged { previous: usize, next: usize },
    #[error("member identity `{identity}` at index {index} is empty or has surrounding whitespace")]
    InvalidIdentity { index: usize, identity: Box<str> },
    #[error("member identity `{identity}` appears more than once in one execution unit")]
    DuplicateIdentity { identity: Box<str> },
    #[error("member identity at index {index} changed from `{previous}` to `{next}`")]
    IdentityChanged {
        index: usize,
        previous: Box<str>,
        next: Box<str>,
    },
    #[error("the scientific member returned a different SystemState owner")]
    StateOwnerChanged,
    #[error("the scientific member changed its state schema at iteration {iteration}")]
    SchemaChanged { iteration: u64 },
    #[error("a successful execution-unit step did not advance any incomplete member")]
    NonAdvancingStep,
    #[error("member `{identity}` iteration regressed from {previous} to {next}")]
    IterationRegressed {
        identity: Box<str>,
        previous: u64,
        next: u64,
    },
    #[error("completed member `{identity}` advanced from {previous} to {next}")]
    CompletedMemberAdvanced {
        identity: Box<str>,
        previous: u64,
        next: u64,
    },
    #[error("completed member `{identity}` became incomplete")]
    CompletionReversed { identity: Box<str> },
    #[error("target iteration {target} is before current iteration {current}")]
    TargetBeforeCurrent { target: u64, current: u64 },
    #[error("target iteration decreased from {previous} to {next}")]
    TargetDecreased { previous: u64, next: u64 },
    #[error("target iteration disappeared after being reported as {previous}")]
    TargetRemoved { previous: u64 },
}
