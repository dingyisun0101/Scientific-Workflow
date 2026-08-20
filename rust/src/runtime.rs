//! Phase-based scheduling and centralized runtime display.
//!
//! Applications declare the complete `config -> tasks -> phases -> runtime`
//! structure before execution. [`WorkflowRuntime`] schedules only those tasks,
//! displays their lifecycle, and provides cooperative cancellation. Each task
//! workload owns all scientific I/O, artifacts, recordings, and subprocesses.
//!
//! ```no_run
//! use scientific_workflow::prelude::basics::*;
//! use scientific_workflow::prelude::runtime::*;
//!
//! # fn main() -> Result<(), RuntimeError> {
//! let project = ScientificProject::load("my-project")
//!     .map_err(|error| RuntimeError::TaskWorkload {
//!         task: "load-project".to_owned(),
//!         source: Box::new(error),
//!     })?;
//! let phase = Phase::builder(2, "simulation")
//!     .activity_tasks_from_project(&project, "prepare", |context| {
//!         context.set_detail("ready");
//!         Ok(())
//!     })
//!     .max_concurrent_workloads(1)
//!     .queue_capacity(1)
//!     .build()?;
//! let summary = WorkflowRuntime::builder()
//!     .phase(phase)
//!     .hidden()
//!     .build()?
//!     .run_phases([2])?;
//! assert!(summary.is_success());
//! # Ok(())
//! # }
//! ```

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

mod error;
mod phase;
mod renderer;
mod reporting;
mod scheduler;
mod task;
mod timing;

pub use error::RuntimeError;
pub use phase::{
    Phase, PhaseBuilder, PhaseFailurePolicy, PhaseId, Task, TaskDisplayKind, TaskId, TaskKey,
    TaskSelector,
};
pub use reporting::{
    ActivityTask, CancellationToken, ProgressSummary, TaskIdentity, TaskProgress, TaskStatus,
};
pub use task::{TaskContext, TaskResult};

use renderer::RuntimeOutput;
use reporting::RuntimeReporter;

static RUNTIME_OWNED: AtomicBool = AtomicBool::new(false);

type SatisfiedPhaseVerifier = Arc<dyn Fn(PhaseId) -> bool + Send + Sync + 'static>;

/// Builder for one immutable runtime plan.
pub struct WorkflowRuntimeBuilder {
    phases: Vec<Phase>,
    output: RuntimeOutput,
    satisfied_phase: Option<SatisfiedPhaseVerifier>,
}

impl WorkflowRuntimeBuilder {
    /// Adds one already validated nonempty phase.
    pub fn phase(mut self, phase: Phase) -> Self {
        self.phases.push(phase);
        self
    }

    /// Adds phases in deterministic declaration order.
    pub fn phases<I>(mut self, phases: I) -> Self
    where
        I: IntoIterator<Item = Phase>,
    {
        self.phases.extend(phases);
        self
    }

    /// Supplies application verification for an omitted completed dependency.
    pub fn satisfied_phase_verifier<F>(mut self, verifier: F) -> Self
    where
        F: Fn(PhaseId) -> bool + Send + Sync + 'static,
    {
        self.satisfied_phase = Some(Arc::new(verifier));
        self
    }

    /// Selects automatic terminal/plain output detection.
    pub fn automatic(mut self) -> Self {
        self.output = RuntimeOutput::Auto;
        self
    }

    /// Forces cursor-controlled interactive display.
    pub fn terminal(mut self) -> Self {
        self.output = RuntimeOutput::Terminal;
        self
    }

    /// Forces append-only uncolored line output.
    pub fn plain(mut self) -> Self {
        self.output = RuntimeOutput::Plain;
        self
    }

    /// Suppresses display while preserving scheduling and lifecycle checks.
    pub fn hidden(mut self) -> Self {
        self.output = RuntimeOutput::Hidden;
        self
    }

    /// Validates the complete phase/task/dependency plan.
    pub fn build(self) -> Result<WorkflowRuntime, RuntimeError> {
        validate_plan(&self.phases)?;
        Ok(WorkflowRuntime {
            phases: self.phases,
            output: self.output,
            satisfied_phase: self.satisfied_phase,
            cancellation: CancellationToken::new(),
        })
    }
}

impl fmt::Debug for WorkflowRuntimeBuilder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorkflowRuntimeBuilder")
            .field("phases", &self.phases.len())
            .field("output", &self.output)
            .field(
                "has_satisfied_phase_verifier",
                &self.satisfied_phase.is_some(),
            )
            .finish_non_exhaustive()
    }
}

/// Non-clone scheduler and display owner for one declared workflow plan.
pub struct WorkflowRuntime {
    phases: Vec<Phase>,
    output: RuntimeOutput,
    satisfied_phase: Option<SatisfiedPhaseVerifier>,
    cancellation: CancellationToken,
}

impl WorkflowRuntime {
    /// Starts an empty builder. At least one phase is mandatory.
    pub fn builder() -> WorkflowRuntimeBuilder {
        WorkflowRuntimeBuilder {
            phases: Vec::new(),
            output: RuntimeOutput::Auto,
            satisfied_phase: None,
        }
    }

    /// Borrows all registered phases in deterministic declaration order.
    pub fn phases(&self) -> &[Phase] {
        &self.phases
    }

    /// Borrows one registered phase by stable ID.
    pub fn phase(&self, id: impl Into<PhaseId>) -> Option<&Phase> {
        let id = id.into();
        self.phases.iter().find(|phase| phase.id() == id)
    }

    /// Returns a cheap token that can request or observe runtime cancellation.
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    /// Returns the unique task matching an exact partial selector.
    pub fn unique_task_matching(&self, selector: &TaskSelector) -> Result<&Task, RuntimeError> {
        let mut matches = self
            .phases
            .iter()
            .flat_map(|phase| phase.tasks())
            .filter(|task| selector.matches(task));
        let first = matches
            .next()
            .ok_or_else(|| RuntimeError::ManagedTaskNotFound {
                selector: selector.to_string(),
            })?;
        if let Some(second) = matches.next() {
            return Err(RuntimeError::ManagedTaskSelectorAmbiguous {
                selector: selector.to_string(),
                first: first.key().to_string(),
                second: second.key().to_string(),
            });
        }
        Ok(first)
    }

    /// Runs exactly the selected phases; omitted unsatisfied dependencies fail.
    pub fn run_phases<I, P>(self, phases: I) -> Result<RuntimeSummary, RuntimeError>
    where
        I: IntoIterator<Item = P>,
        P: Into<PhaseId>,
    {
        self.run_phases_exact(phases)
    }

    /// Runs exactly the selected phases; omitted unsatisfied dependencies fail.
    pub fn run_phases_exact<I, P>(self, phases: I) -> Result<RuntimeSummary, RuntimeError>
    where
        I: IntoIterator<Item = P>,
        P: Into<PhaseId>,
    {
        let selected = self.select_exact(phases)?;
        self.execute(selected)
    }

    /// Adds unsatisfied dependencies and runs the deterministic closure.
    pub fn run_phases_with_dependencies<I, P>(
        self,
        phases: I,
    ) -> Result<RuntimeSummary, RuntimeError>
    where
        I: IntoIterator<Item = P>,
        P: Into<PhaseId>,
    {
        let selected = self.select_with_dependencies(phases)?;
        self.execute(selected)
    }

    fn select_exact<I, P>(&self, phases: I) -> Result<Vec<usize>, RuntimeError>
    where
        I: IntoIterator<Item = P>,
        P: Into<PhaseId>,
    {
        let requested = selected_ids(&self.phases, phases)?;
        for phase in self
            .phases
            .iter()
            .filter(|phase| requested.contains(&phase.id()))
        {
            for dependency in phase.dependencies() {
                if !requested.contains(dependency) && !self.is_satisfied(*dependency) {
                    return Err(RuntimeError::UnsatisfiedPhaseDependency {
                        phase: phase.id().get(),
                        dependency: dependency.get(),
                    });
                }
            }
        }
        Ok(topological_positions(&self.phases)?
            .into_iter()
            .filter(|position| requested.contains(&self.phases[*position].id()))
            .collect())
    }

    fn select_with_dependencies<I, P>(&self, phases: I) -> Result<Vec<usize>, RuntimeError>
    where
        I: IntoIterator<Item = P>,
        P: Into<PhaseId>,
    {
        let mut selected = selected_ids(&self.phases, phases)?;
        let positions: HashMap<_, _> = self
            .phases
            .iter()
            .enumerate()
            .map(|(position, phase)| (phase.id(), position))
            .collect();
        let mut pending: Vec<_> = selected.iter().copied().collect();
        while let Some(id) = pending.pop() {
            let phase = &self.phases[positions[&id]];
            for dependency in phase.dependencies() {
                if !self.is_satisfied(*dependency) && selected.insert(*dependency) {
                    pending.push(*dependency);
                }
            }
        }
        Ok(topological_positions(&self.phases)?
            .into_iter()
            .filter(|position| selected.contains(&self.phases[*position].id()))
            .collect())
    }

    fn is_satisfied(&self, phase: PhaseId) -> bool {
        self.satisfied_phase
            .as_ref()
            .is_some_and(|verify| verify(phase))
    }

    fn execute(self, selected: Vec<usize>) -> Result<RuntimeSummary, RuntimeError> {
        let _lease = RuntimeLease::acquire()?;
        let total_phases = selected.len();
        let total_tasks = selected
            .iter()
            .map(|position| self.phases[*position].tasks().len())
            .sum();
        let mut summaries = Vec::with_capacity(total_phases);
        let mut phases = self.phases.into_iter().map(Some).collect::<Vec<_>>();

        for (selection_position, phase_position) in selected.iter().copied().enumerate() {
            let phase = phases[phase_position]
                .take()
                .expect("selected phase positions are unique");
            renderer::phase_start(self.output, &phase, selection_position + 1, total_phases);
            let heading = renderer::phase_heading(&phase, selection_position + 1, total_phases);
            let builder = RuntimeReporter::for_phase(&phase, &heading)?
                .cancellation_token(self.cancellation.clone());
            let reporter = match self.output {
                RuntimeOutput::Auto => builder,
                RuntimeOutput::Terminal => builder.terminal(),
                RuntimeOutput::Plain => builder.plain(),
                RuntimeOutput::Hidden => builder.hidden(),
            }
            .start()?;
            let phase_id = phase.id();
            let phase_label: Arc<str> = phase.label().into();
            let require_confirm = phase.requires_confirmation();
            let result = scheduler::execute_phase(phase, &reporter);
            let progress = if result.is_ok() {
                reporter.complete(format!("phase {phase_id} completed"))?
            } else {
                reporter.fail(format!("phase {phase_id} failed"))?
            };
            let success = result.is_ok() && progress.is_success();
            renderer::phase_complete(self.output, phase_id, &phase_label, success);
            summaries.push(PhaseSummary {
                id: phase_id,
                label: phase_label,
                progress,
            });
            if let Err(error) = result {
                renderer::runtime_complete(self.output, summaries.len(), total_tasks, false);
                return Err(RuntimeError::PhaseExecutionFailed {
                    summary: RuntimeSummary {
                        phases: summaries.into(),
                    },
                    source: Box::new(error),
                });
            }
            if require_confirm && selection_position + 1 < total_phases {
                let next = phases[selected[selection_position + 1]]
                    .as_ref()
                    .expect("the next selected phase has not executed");
                let confirmed = match renderer::confirm_transition(phase_id, next) {
                    Ok(confirmed) => confirmed,
                    Err(source) => {
                        renderer::runtime_complete(
                            self.output,
                            summaries.len(),
                            total_tasks,
                            false,
                        );
                        return Err(RuntimeError::PhaseExecutionFailed {
                            summary: RuntimeSummary {
                                phases: summaries.into(),
                            },
                            source: Box::new(RuntimeError::PhaseConfirmationInput {
                                phase: phase_id.get(),
                                source,
                            }),
                        });
                    }
                };
                if !confirmed {
                    renderer::runtime_complete(self.output, summaries.len(), total_tasks, false);
                    return Err(RuntimeError::PhaseExecutionFailed {
                        summary: RuntimeSummary {
                            phases: summaries.into(),
                        },
                        source: Box::new(RuntimeError::PhaseConfirmationEof {
                            phase: phase_id.get(),
                        }),
                    });
                }
            }
        }

        renderer::runtime_complete(self.output, summaries.len(), total_tasks, true);
        Ok(RuntimeSummary {
            phases: summaries.into(),
        })
    }
}

impl fmt::Debug for WorkflowRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorkflowRuntime")
            .field("phases", &self.phases.len())
            .field(
                "tasks",
                &self.phases.iter().map(|p| p.tasks().len()).sum::<usize>(),
            )
            .field("output", &self.output)
            .finish_non_exhaustive()
    }
}

/// Terminal summary for one selected phase.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhaseSummary {
    id: PhaseId,
    label: Arc<str>,
    progress: ProgressSummary,
}

impl PhaseSummary {
    pub const fn id(&self) -> PhaseId {
        self.id
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    pub fn progress(&self) -> &ProgressSummary {
        &self.progress
    }

    pub fn is_success(&self) -> bool {
        self.progress.is_success()
    }
}

/// Immutable aggregate for a completed selected runtime plan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeSummary {
    phases: Arc<[PhaseSummary]>,
}

impl RuntimeSummary {
    pub fn phases(&self) -> &[PhaseSummary] {
        &self.phases
    }

    pub fn total_tasks(&self) -> u64 {
        self.phases.iter().map(|phase| phase.progress.total()).sum()
    }

    pub fn is_success(&self) -> bool {
        !self.phases.is_empty() && self.phases.iter().all(PhaseSummary::is_success)
    }
}

struct RuntimeLease;

impl RuntimeLease {
    fn acquire() -> Result<Self, RuntimeError> {
        RUNTIME_OWNED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| Self)
            .map_err(|_| RuntimeError::TerminalAlreadyOwned)
    }
}

impl Drop for RuntimeLease {
    fn drop(&mut self) {
        RUNTIME_OWNED.store(false, Ordering::Release);
    }
}

fn selected_ids<I, P>(phases: &[Phase], requested: I) -> Result<HashSet<PhaseId>, RuntimeError>
where
    I: IntoIterator<Item = P>,
    P: Into<PhaseId>,
{
    let known: HashSet<_> = phases.iter().map(Phase::id).collect();
    let selected: HashSet<_> = requested.into_iter().map(Into::into).collect();
    if selected.is_empty() {
        return Err(RuntimeError::EmptyPhaseSet);
    }
    if let Some(unknown) = selected.iter().find(|id| !known.contains(id)) {
        return Err(RuntimeError::UnknownSelectedPhase {
            phase: unknown.get(),
        });
    }
    Ok(selected)
}

fn validate_plan(phases: &[Phase]) -> Result<(), RuntimeError> {
    if phases.is_empty() {
        return Err(RuntimeError::EmptyPhaseSet);
    }
    let mut ids = HashSet::with_capacity(phases.len());
    for phase in phases {
        if !ids.insert(phase.id()) {
            return Err(RuntimeError::DuplicatePhaseId {
                phase: phase.id().get(),
            });
        }
    }
    for phase in phases {
        for dependency in phase.dependencies() {
            if !ids.contains(dependency) {
                return Err(RuntimeError::UnknownPhaseDependency {
                    phase: phase.id().get(),
                    dependency: dependency.get(),
                });
            }
        }
        for task in phase.tasks() {
            if !task.has_workload() && !task.is_reused() {
                return Err(RuntimeError::MissingTaskWorkload {
                    task: task.key().to_string(),
                });
            }
        }
    }
    topological_positions(phases).map(|_| ())
}

fn topological_positions(phases: &[Phase]) -> Result<Vec<usize>, RuntimeError> {
    let positions: HashMap<_, _> = phases
        .iter()
        .enumerate()
        .map(|(position, phase)| (phase.id(), position))
        .collect();
    let mut states = vec![0_u8; phases.len()];
    let mut ordered = Vec::with_capacity(phases.len());
    fn visit(
        position: usize,
        phases: &[Phase],
        positions: &HashMap<PhaseId, usize>,
        states: &mut [u8],
        ordered: &mut Vec<usize>,
    ) -> Result<(), RuntimeError> {
        match states[position] {
            2 => return Ok(()),
            1 => {
                return Err(RuntimeError::PhaseDependencyCycle {
                    phase: phases[position].id().get(),
                });
            }
            _ => {}
        }
        states[position] = 1;
        for dependency in phases[position].dependencies() {
            visit(positions[dependency], phases, positions, states, ordered)?;
        }
        states[position] = 2;
        ordered.push(position);
        Ok(())
    }
    for position in 0..phases.len() {
        visit(position, phases, &positions, &mut states, &mut ordered)?;
    }
    Ok(ordered)
}
