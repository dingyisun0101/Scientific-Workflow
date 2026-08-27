//! Study, phase, and task orchestration.
//!
//! A [`Study`] is the largest scope. It owns ordered [`Phase`] declarations,
//! scheduling, cancellation, recording, and display. Each phase owns its
//! [`Task`] declarations and scheduling policy. Each task owns one application
//! workload and communicates through [`TaskContext`].
//!
//! # Boundary
//!
//! Study owns only orchestration concerns: task ordering, concurrency caps,
//! cancellation, failure policy, progress reporting, and task lifecycle. It does
//! not define scientific state schema, persistence formats, artifact identity, or
//! RNG strategy. Applications feed workloads and state objects into the study.
//!
//! Project declaration parsing belongs exclusively to [`crate::config`]. The
//! transitional study API receives already constructed workloads and does not
//! load, expand, or decode task inputs.
//!
//! # Phase completion
//!
//! An application may attach a read-only completion examiner with
//! [`PhaseBuilder::examine_completion`]. Workflow uses the resulting
//! [`PhaseCompletion`] verdict only for whole-phase reuse and dependency
//! satisfaction. An incomplete phase is invoked normally: recovery, reuse,
//! cleanup, and validation within that phase remain application-owned.
//! Examination is enabled by default and may be disabled for a launch with
//! [`StudyBuilder::without_completion_examination`].
//!
//! ```no_run
//! use scientific_workflow::prelude::study::*;
//!
//! # fn main() -> Result<(), StudyError> {
//! let task = Task::progress("simulation-0", "simulation 0", |context| {
//!     context.set_target_iteration(100)?;
//!     for iteration in 0..=100 {
//!         context.set_iteration(iteration)?;
//!         if context.is_cancelled() {
//!             break;
//!         }
//!     }
//!     Ok(())
//! });
//! let phase = Phase::builder(1, "simulation").task(task).build()?;
//! let summary = Study::builder("study-record.json")
//!     .phase(phase)
//!     .hidden()
//!     .build()?
//!     .run()?;
//! assert!(summary.is_success());
//! # Ok(())
//! # }
//! ```

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[path = "study/command.rs"]
mod command;
#[path = "study/completion.rs"]
mod completion;
#[path = "study/display.rs"]
mod display;
#[path = "study/error.rs"]
mod error;
#[path = "study/phase.rs"]
mod phase;
#[path = "study/plan.rs"]
mod plan;
#[path = "study/record.rs"]
mod record;
#[path = "study/renderer.rs"]
mod renderer;
#[path = "study/scheduler.rs"]
mod scheduler;
#[path = "study/task.rs"]
mod task;
#[path = "study/timing.rs"]
mod timing;
#[path = "study/tui.rs"]
mod tui;

pub use completion::PhaseCompletion;
pub use error::StudyError;
pub use phase::{
    Phase, PhaseBuilder, PhaseFailurePolicy, PhaseId, Task, TaskId, TaskKey, TaskMode, TaskSelector,
};
pub use plan::StudyPlan;
pub use record::{PhaseRecord, StudyRecord, TaskRecord};
pub use renderer::{CancellationToken, ProgressSummary, TaskIdentity, TaskStatus};
pub use task::{TaskContext, TaskResult};

use display::DisplayMode;
use renderer::StudyRenderer;

static STUDY_OWNED: AtomicBool = AtomicBool::new(false);

/// Builder for one immutable study plan.
pub struct StudyBuilder {
    phases: Vec<Phase>,
    output: DisplayMode,
    examine_completion: bool,
    record_path: std::path::PathBuf,
}

impl StudyBuilder {
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

    /// Disables all application-supplied phase completion examiners.
    ///
    /// Completion examination is enabled by default. Disabling it invokes all
    /// selected phases and makes omitted dependencies unsatisfied exactly as if
    /// no phase had an examiner. Workflow emits a warning because reuse and
    /// continuation within invoked phases remain application-owned.
    pub fn without_completion_examination(mut self) -> Self {
        self.examine_completion = false;
        self
    }

    /// Selects automatic terminal/plain output detection.
    pub fn automatic(mut self) -> Self {
        self.output = DisplayMode::Auto;
        self
    }

    /// Forces cursor-controlled interactive display.
    pub fn terminal(mut self) -> Self {
        self.output = DisplayMode::Terminal;
        self
    }

    /// Forces append-only uncolored line output.
    pub fn plain(mut self) -> Self {
        self.output = DisplayMode::Plain;
        self
    }

    /// Suppresses display while preserving scheduling and lifecycle checks.
    pub fn hidden(mut self) -> Self {
        self.output = DisplayMode::Hidden;
        self
    }

    /// Validates the complete phase/task/dependency plan.
    pub fn build(self) -> Result<Study, StudyError> {
        validate_plan(&self.phases)?;
        Ok(Study {
            phases: self.phases,
            output: self.output,
            examine_completion: self.examine_completion,
            cancellation: CancellationToken::new(),
            record_path: self.record_path,
        })
    }
}

impl fmt::Debug for StudyBuilder {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StudyBuilder")
            .field("phases", &self.phases.len())
            .field("output", &self.output)
            .field("record_path", &self.record_path)
            .field("examine_completion", &self.examine_completion)
            .finish_non_exhaustive()
    }
}

/// Non-clone scheduler and display owner for one declared study plan.
pub struct Study {
    phases: Vec<Phase>,
    output: DisplayMode,
    examine_completion: bool,
    cancellation: CancellationToken,
    record_path: std::path::PathBuf,
}

impl Study {
    /// Starts an empty builder with the mandatory study-record destination.
    pub fn builder(record_path: impl Into<std::path::PathBuf>) -> StudyBuilder {
        StudyBuilder {
            phases: Vec::new(),
            output: DisplayMode::Auto,
            examine_completion: true,
            record_path: record_path.into(),
        }
    }

    /// Borrows all registered phases in deterministic declaration order.
    pub fn phases(&self) -> &[Phase] {
        &self.phases
    }

    /// Materializes a deterministic, side-effect-free description of every
    /// registered phase and task.
    pub fn plan(&self) -> StudyPlan {
        StudyPlan::from_phases(&self.phases)
    }

    /// Writes the complete registered plan as pretty JSON without running it.
    /// An existing byte-identical file is accepted. Different existing
    /// content is rejected and never overwritten.
    pub fn write_plan_json(&self, path: impl AsRef<std::path::Path>) -> Result<(), StudyError> {
        self.plan().write_json(path)
    }

    /// Borrows one registered phase by stable ID.
    pub fn phase(&self, id: impl Into<PhaseId>) -> Option<&Phase> {
        let id = id.into();
        self.phases.iter().find(|phase| phase.id() == id)
    }

    /// Returns a cheap token that can request or observe study cancellation.
    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    /// Returns the unique task matching an exact partial selector.
    pub fn unique_task_matching(&self, selector: &TaskSelector) -> Result<&Task, StudyError> {
        let mut matches = self
            .phases
            .iter()
            .flat_map(|phase| phase.tasks())
            .filter(|task| selector.matches(task));
        let first = matches.next().ok_or_else(|| StudyError::TaskNotFound {
            selector: selector.to_string(),
        })?;
        if let Some(second) = matches.next() {
            return Err(StudyError::TaskSelectorAmbiguous {
                selector: selector.to_string(),
                first: first.key().to_string(),
                second: second.key().to_string(),
            });
        }
        Ok(first)
    }

    /// Runs every registered phase in dependency order, reusing any phase whose
    /// enabled application examiner reports complete.
    pub fn run(self) -> Result<StudySummary, StudyError> {
        let phases = self.phases.iter().map(Phase::id).collect::<Vec<_>>();
        let selected = self.select_phases(phases, false)?;
        self.execute(selected)
    }

    /// Runs exactly the selected phases; omitted dependencies must examine as
    /// complete or selection fails.
    pub fn run_phases<I, P>(self, phases: I) -> Result<StudySummary, StudyError>
    where
        I: IntoIterator<Item = P>,
        P: Into<PhaseId>,
    {
        let selected = self.select_phases(phases, false)?;
        self.execute(selected)
    }

    /// Adds only non-complete dependencies and runs the deterministic closure.
    pub fn run_phases_with_dependencies<I, P>(self, phases: I) -> Result<StudySummary, StudyError>
    where
        I: IntoIterator<Item = P>,
        P: Into<PhaseId>,
    {
        let selected = self.select_phases(phases, true)?;
        self.execute(selected)
    }

    fn select_phases<I, P>(
        &self,
        phases: I,
        include_dependencies: bool,
    ) -> Result<PhaseSelection, StudyError>
    where
        I: IntoIterator<Item = P>,
        P: Into<PhaseId>,
    {
        let mut selected = selected_ids(&self.phases, phases)?;
        let mut examination = completion::CompletionExamination::new(self.examine_completion);
        let positions: HashMap<_, _> = self
            .phases
            .iter()
            .enumerate()
            .map(|(position, phase)| (phase.id(), position))
            .collect();
        for position in topological_positions(&self.phases)? {
            let phase = &self.phases[position];
            if selected.contains(&phase.id()) {
                examination.examine(phase)?;
            }
        }
        if include_dependencies {
            let mut pending = topological_positions(&self.phases)?
                .into_iter()
                .map(|position| self.phases[position].id())
                .filter(|phase| selected.contains(phase))
                .collect::<VecDeque<_>>();
            while let Some(id) = pending.pop_front() {
                let phase = &self.phases[positions[&id]];
                if examination.examine(phase)?.is_complete() {
                    continue;
                }
                for dependency in phase.dependencies() {
                    let dependency_phase = &self.phases[positions[dependency]];
                    if !examination.examine(dependency_phase)?.is_complete()
                        && selected.insert(*dependency)
                    {
                        pending.push_back(*dependency);
                    }
                }
            }
        } else {
            for phase in self
                .phases
                .iter()
                .filter(|phase| selected.contains(&phase.id()))
            {
                if examination.examine(phase)?.is_complete() {
                    continue;
                }
                for dependency in phase.dependencies() {
                    let dependency_phase = &self.phases[positions[dependency]];
                    if !selected.contains(dependency)
                        && !examination.examine(dependency_phase)?.is_complete()
                    {
                        return Err(StudyError::UnsatisfiedPhaseDependency {
                            phase: phase.id().get(),
                            dependency: dependency.get(),
                        });
                    }
                }
            }
        }
        let positions = topological_positions(&self.phases)?
            .into_iter()
            .filter(|position| selected.contains(&self.phases[*position].id()))
            .collect();
        let examination_enabled = examination.is_enabled();
        Ok(PhaseSelection {
            positions,
            completion: examination.into_outcomes(),
            examination_enabled,
        })
    }

    fn execute(self, selected: PhaseSelection) -> Result<StudySummary, StudyError> {
        let _lease = StudyLease::acquire()?;
        let total_phases = selected.positions.len();
        let total_tasks = selected
            .positions
            .iter()
            .map(|position| self.phases[*position].tasks().len())
            .sum();
        let execution = {
            let selected_phases = selected
                .positions
                .iter()
                .map(|position| &self.phases[*position])
                .collect::<Vec<_>>();
            record::StudyRecorder::start(self.record_path.clone(), &selected_phases)?
        };
        let mut summaries = Vec::with_capacity(total_phases);
        let mut phases = self.phases.into_iter().map(Some).collect::<Vec<_>>();
        let executable_phases = selected
            .positions
            .iter()
            .filter(|position| {
                !selected.completion[&phases[**position].as_ref().unwrap().id()].is_complete()
            })
            .count();
        let mut execution_position = 0;
        let mut reused_phases = 0;
        if !selected.examination_enabled {
            display::completion_examination_disabled(self.output);
        }

        for (selection_position, phase_position) in selected.positions.iter().copied().enumerate() {
            let phase = phases[phase_position]
                .take()
                .expect("selected phase positions are unique");
            let completion = &selected.completion[&phase.id()];
            if completion.is_complete() {
                execution.phase_reused(phase.id())?;
                display::phase_reused(self.output, phase.id(), phase.label());
                summaries.push(PhaseSummary {
                    id: phase.id(),
                    label: phase.label().into(),
                    progress: ProgressSummary::fully_completed(phase.tasks().len() as u64),
                    reused: true,
                });
                reused_phases += 1;
                continue;
            }
            if let PhaseCompletion::Incomplete(detail) = completion {
                display::phase_incomplete(self.output, phase.id(), phase.label(), detail);
            }
            execution_position += 1;
            execution.phase_started(phase.id())?;
            display::phase_start(self.output, &phase, execution_position, executable_phases);
            let heading = display::phase_heading(&phase, execution_position, executable_phases);
            let builder = StudyRenderer::for_phase(&phase, &heading)?
                .cancellation_token(self.cancellation.clone());
            let renderer = match self.output {
                DisplayMode::Auto => builder,
                DisplayMode::Terminal => builder.terminal(),
                DisplayMode::Plain => builder.plain(),
                DisplayMode::Hidden => builder.hidden(),
            }
            .start()?;
            let phase_id = phase.id();
            let phase_label: Arc<str> = phase.label().into();
            let require_confirm = phase.requires_confirmation();
            let result = scheduler::execute_phase(phase, &renderer, &execution);
            let task_execution = renderer.task_execution_snapshots();
            let progress = if result.is_ok() {
                renderer.complete(format!("phase {phase_id} completed"))?
            } else {
                renderer.fail(format!("phase {phase_id} failed"))?
            };
            let success = result.is_ok() && progress.is_success();
            execution.phase_finished(phase_id, success, &progress, task_execution)?;
            display::phase_complete(self.output, phase_id, &phase_label, success);
            summaries.push(PhaseSummary {
                id: phase_id,
                label: phase_label,
                progress,
                reused: false,
            });
            if let Err(error) = result {
                return Err(Self::fail_phase_with_summary(
                    self.output,
                    &execution,
                    summaries,
                    reused_phases,
                    total_tasks,
                    error,
                )?);
            }
            let next = selected.positions[selection_position + 1..]
                .iter()
                .filter_map(|position| phases[*position].as_ref())
                .find(|phase| !selected.completion[&phase.id()].is_complete());
            if require_confirm && let Some(next) = next {
                let confirmed = match display::confirm_transition(phase_id, next) {
                    Ok(confirmed) => confirmed,
                    Err(source) => {
                        return Err(Self::fail_phase_with_summary(
                            self.output,
                            &execution,
                            summaries,
                            reused_phases,
                            total_tasks,
                            StudyError::PhaseConfirmationInput {
                                phase: phase_id.get(),
                                source,
                            },
                        )?);
                    }
                };
                if !confirmed {
                    return Err(Self::fail_phase_with_summary(
                        self.output,
                        &execution,
                        summaries,
                        reused_phases,
                        total_tasks,
                        StudyError::PhaseConfirmationEof {
                            phase: phase_id.get(),
                        },
                    )?);
                }
            }
        }

        display::study_complete(
            self.output,
            summaries.len(),
            reused_phases,
            total_tasks,
            true,
        );
        let record = execution.finish(true)?;
        Ok(StudySummary {
            phases: summaries.into(),
            record: Box::new(record),
        })
    }

    fn fail_phase_with_summary(
        output: DisplayMode,
        execution: &record::StudyRecorder,
        summaries: Vec<PhaseSummary>,
        reused_phases: usize,
        total_tasks: usize,
        source: StudyError,
    ) -> Result<StudyError, StudyError> {
        display::study_complete(output, summaries.len(), reused_phases, total_tasks, false);
        let record = execution.finish(false)?;
        Ok(StudyError::PhaseExecutionFailed {
            summary: StudySummary {
                phases: summaries.into(),
                record: Box::new(record),
            },
            source: Box::new(source),
        })
    }
}

/// Prepared phase selection plus its single launch-local completion snapshot.
struct PhaseSelection {
    positions: Vec<usize>,
    completion: HashMap<PhaseId, PhaseCompletion>,
    examination_enabled: bool,
}

impl fmt::Debug for Study {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Study")
            .field("phases", &self.phases.len())
            .field(
                "tasks",
                &self.phases.iter().map(|p| p.tasks().len()).sum::<usize>(),
            )
            .field("output", &self.output)
            .field("examine_completion", &self.examine_completion)
            .finish_non_exhaustive()
    }
}

/// Terminal summary for one selected phase.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PhaseSummary {
    id: PhaseId,
    label: Arc<str>,
    progress: ProgressSummary,
    reused: bool,
}

impl PhaseSummary {
    /// Returns the completed phase's stable identity.
    pub const fn id(&self) -> PhaseId {
        self.id
    }

    /// Borrows the completed phase's display label.
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Borrows aggregate task progress for this phase outcome.
    ///
    /// For a reused phase, every declared task is represented as completed by
    /// the application's whole-phase verdict. Workflow did not examine those
    /// tasks individually; [`Self::was_reused`] preserves that distinction.
    pub fn progress(&self) -> &ProgressSummary {
        &self.progress
    }

    /// Reports whether Workflow reused a whole application-verified phase
    /// without entering its task scheduler.
    pub const fn was_reused(&self) -> bool {
        self.reused
    }

    /// Reports whether every task in the phase completed successfully.
    pub fn is_success(&self) -> bool {
        self.progress.is_success()
    }
}

/// Immutable aggregate for a completed study execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StudySummary {
    phases: Arc<[PhaseSummary]>,
    record: Box<StudyRecord>,
}

impl StudySummary {
    /// Borrows completed phase summaries in execution order.
    pub fn phases(&self) -> &[PhaseSummary] {
        &self.phases
    }

    /// Borrows the always-on durable record for this study execution.
    pub fn record(&self) -> &StudyRecord {
        self.record.as_ref()
    }

    /// Returns the total number of tasks across the completed phase summaries.
    pub fn total_tasks(&self) -> u64 {
        self.phases.iter().map(|phase| phase.progress.total()).sum()
    }

    /// Reports whether at least one phase was selected and every selected phase
    /// either executed successfully or was reused after examination.
    pub fn is_success(&self) -> bool {
        !self.phases.is_empty() && self.phases.iter().all(PhaseSummary::is_success)
    }
}

struct StudyLease;

impl StudyLease {
    fn acquire() -> Result<Self, StudyError> {
        STUDY_OWNED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| Self)
            .map_err(|_| StudyError::TerminalAlreadyOwned)
    }
}

impl Drop for StudyLease {
    fn drop(&mut self) {
        STUDY_OWNED.store(false, Ordering::Release);
    }
}

fn selected_ids<I, P>(phases: &[Phase], requested: I) -> Result<HashSet<PhaseId>, StudyError>
where
    I: IntoIterator<Item = P>,
    P: Into<PhaseId>,
{
    let known: HashSet<_> = phases.iter().map(Phase::id).collect();
    let selected: HashSet<_> = requested.into_iter().map(Into::into).collect();
    if selected.is_empty() {
        return Err(StudyError::EmptyPhaseSet);
    }
    if let Some(unknown) = selected.iter().find(|id| !known.contains(id)) {
        return Err(StudyError::UnknownSelectedPhase {
            phase: unknown.get(),
        });
    }
    Ok(selected)
}

fn validate_plan(phases: &[Phase]) -> Result<(), StudyError> {
    if phases.is_empty() {
        return Err(StudyError::EmptyPhaseSet);
    }
    let mut ids = HashSet::with_capacity(phases.len());
    for phase in phases {
        if !ids.insert(phase.id()) {
            return Err(StudyError::DuplicatePhaseId {
                phase: phase.id().get(),
            });
        }
    }
    for phase in phases {
        for dependency in phase.dependencies() {
            if !ids.contains(dependency) {
                return Err(StudyError::UnknownPhaseDependency {
                    phase: phase.id().get(),
                    dependency: dependency.get(),
                });
            }
        }
        for task in phase.tasks() {
            if !task.has_workload() && !task.is_completed() {
                return Err(StudyError::MissingTaskWorkload {
                    task: task.key().to_string(),
                });
            }
        }
    }
    topological_positions(phases).map(|_| ())
}

fn topological_positions(phases: &[Phase]) -> Result<Vec<usize>, StudyError> {
    let positions: HashMap<_, _> = phases
        .iter()
        .enumerate()
        .map(|(position, phase)| (phase.id(), position))
        .collect();
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); phases.len()];
    let mut indegree = vec![0_usize; phases.len()];

    for (position, phase) in phases.iter().enumerate() {
        for dependency in phase.dependencies() {
            let dependency_position = positions[dependency];
            dependents[dependency_position].push(position);
            indegree[position] += 1;
        }
    }

    let mut queue = VecDeque::new();
    for (position, degree) in indegree.iter().enumerate() {
        if *degree == 0 {
            queue.push_back(position);
        }
    }

    let mut ordered = Vec::with_capacity(phases.len());
    while let Some(position) = queue.pop_front() {
        ordered.push(position);
        for dependent in dependents[position].drain(..) {
            indegree[dependent] -= 1;
            if indegree[dependent] == 0 {
                queue.push_back(dependent);
            }
        }
    }

    if ordered.len() != phases.len() {
        let phase_position = indegree
            .iter()
            .position(|degree| *degree > 0)
            .expect("cyclic dependency must leave at least one phase with indegree");
        return Err(StudyError::PhaseDependencyCycle {
            phase: phases[phase_position].id().get(),
        });
    }

    Ok(ordered)
}
