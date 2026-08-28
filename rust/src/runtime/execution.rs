//! Runtime scheduling for a completed immutable Study.

use std::collections::{HashMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::config::advanced::{ConfigSnapshot, FailurePolicy, ReplicateScheduling};
use crate::persistence::advanced::{ModelRecordingProvenance, PersistencePlan};
use crate::study::advanced::{Study, StudyPhase, StudyTask};
use crate::task::advanced::InitializationContext;
use crate::task::advanced::TaskKind;
use crate::ui::advanced::{UiEvent, UiSession};

use super::error::RuntimeError;
use super::host::{RuntimeTaskEnvironment, RuntimeTaskHost};
use super::output::{create_execution, create_replicate};
use super::summary::{
    PhaseRunSummary, ReplicateRunSummary, RunSummary, TaskRunKind, TaskRunSummary,
};

const SCHEDULER_POLL: Duration = Duration::from_millis(5);

/// Executes one already validated immutable study and returns its summary.
pub fn execute(study: Study) -> Result<RunSummary, RuntimeError> {
    let output = create_execution(study.output_root())?;
    let count = study.replicate_policy().count();
    let task_count_per_replicate = study.phases().iter().map(|phase| phase.tasks().len()).sum();
    let ui = UiSession::automatic(study.ui_plan());
    for replicate in 0..count {
        for phase in study.phases() {
            for task in phase.tasks() {
                ui.publish(UiEvent::TaskPlanned {
                    replicate,
                    phase: phase.name(),
                    identity: task.identity(),
                    label: task.label(),
                    kind: task.kind_name(),
                    subject: task.subject(),
                });
            }
        }
    }
    ui.publish(UiEvent::ExecutionStarted {
        output_directory: &output,
        replicate_count: count,
        task_count_per_replicate,
    });

    let result = (|| {
        let mut scopes = Vec::new();
        for index in 0..count {
            scopes.push((index, create_replicate(&output, index)?));
        }
        match study.replicate_policy().scheduling() {
            ReplicateScheduling::Sequential => run_replicates_sequential(&study, scopes, &ui),
            ReplicateScheduling::Parallel => run_replicates_parallel(&study, scopes, &ui),
        }
    })();

    let result = if ui.cancellation_requested() {
        Err(RuntimeError::ExecutionCancelled)
    } else {
        result
    };
    let outcome = match result {
        Ok(replicates) => {
            ui.publish(UiEvent::ExecutionCompleted {
                output_directory: &output,
            });
            Ok(RunSummary {
                output_directory: output,
                replicates: replicates.into_boxed_slice(),
            })
        }
        Err(error) => {
            if matches!(error, RuntimeError::ExecutionCancelled) {
                ui.publish(UiEvent::ExecutionCancelled);
            } else {
                let reason = error.to_string();
                ui.publish(UiEvent::ExecutionFailed { reason: &reason });
            }
            Err(error)
        }
    };
    ui.finish();
    outcome
}

fn run_replicates_sequential(
    study: &Study,
    scopes: Vec<(u64, PathBuf)>,
    ui: &UiSession,
) -> Result<Vec<ReplicateRunSummary>, RuntimeError> {
    let mut summaries = Vec::with_capacity(scopes.len());
    let mut first_error = None;
    for (index, scope) in scopes {
        let cancellation = AtomicBool::new(false);
        match run_replicate(study, index, scope, ui, &cancellation) {
            Ok(summary) => summaries.push(summary),
            Err(source) => {
                first_error.get_or_insert(RuntimeError::Replicate {
                    index,
                    source: Box::new(source),
                });
                if study.replicate_policy().failure_policy() == FailurePolicy::FailFast {
                    break;
                }
            }
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(summaries),
    }
}

fn run_replicates_parallel(
    study: &Study,
    scopes: Vec<(u64, PathBuf)>,
    ui: &UiSession,
) -> Result<Vec<ReplicateRunSummary>, RuntimeError> {
    enum WorkerOutcome {
        Finished(Result<ReplicateRunSummary, RuntimeError>),
        Panicked,
    }

    let worker_count = scopes.len();
    let cancellation = Arc::new(AtomicBool::new(false));
    let (outcomes, completed) = mpsc::channel();
    let mut workers: Vec<(u64, JoinHandle<()>)> = Vec::with_capacity(worker_count);
    for (index, scope) in scopes {
        let study = study.clone();
        let ui = ui.clone();
        let outcomes = outcomes.clone();
        let worker_cancellation = Arc::clone(&cancellation);
        let worker = match thread::Builder::new()
            .name(format!("workflow-replicate-{index}"))
            .spawn(move || {
                let outcome = catch_unwind(AssertUnwindSafe(|| {
                    run_replicate(&study, index, scope, &ui, &worker_cancellation)
                }))
                .map_or(WorkerOutcome::Panicked, WorkerOutcome::Finished);
                let _ = outcomes.send((index, outcome));
            }) {
            Ok(worker) => worker,
            Err(source) => {
                cancellation.store(true, Ordering::Release);
                for (_, worker) in workers {
                    let _ = worker.join();
                }
                return Err(RuntimeError::StartWorker {
                    scope: format!("replicate {index}"),
                    source,
                });
            }
        };
        workers.push((index, worker));
    }
    drop(outcomes);

    let fail_fast = study.replicate_policy().failure_policy() == FailurePolicy::FailFast;
    let mut summaries = Vec::with_capacity(worker_count);
    let mut first_error = None;
    for _ in 0..worker_count {
        let (index, outcome) = completed
            .recv()
            .expect("replicate worker reports exactly one terminal outcome");
        let error = match outcome {
            WorkerOutcome::Finished(Ok(summary)) => {
                summaries.push(summary);
                None
            }
            WorkerOutcome::Finished(Err(RuntimeError::ExecutionCancelled))
                if fail_fast && first_error.is_some() =>
            {
                None
            }
            WorkerOutcome::Finished(Err(source)) => Some(RuntimeError::Replicate {
                index,
                source: Box::new(source),
            }),
            WorkerOutcome::Panicked => Some(RuntimeError::ReplicatePanicked { index }),
        };
        if let Some(error) = error
            && first_error.is_none()
        {
            first_error = Some(error);
            if fail_fast {
                cancellation.store(true, Ordering::Release);
            }
        }
    }
    for (index, worker) in workers {
        if worker.join().is_err() && first_error.is_none() {
            // The closure catches the replicate body. A panic here can only
            // arise while tearing down worker-owned values after reporting.
            first_error = Some(RuntimeError::ReplicatePanicked { index });
        }
    }
    summaries.sort_by_key(ReplicateRunSummary::index);
    match first_error {
        Some(error) => Err(error),
        None => Ok(summaries),
    }
}

fn run_replicate(
    study: &Study,
    index: u64,
    scope: PathBuf,
    ui: &UiSession,
    scheduler_cancellation: &AtomicBool,
) -> Result<ReplicateRunSummary, RuntimeError> {
    ui.publish(UiEvent::ReplicateStarted { index });
    let result = run_replicate_inner(study, index, scope, ui, scheduler_cancellation);
    match &result {
        Ok(_) => ui.publish(UiEvent::ReplicateCompleted { index }),
        Err(RuntimeError::ExecutionCancelled) => {
            ui.publish(UiEvent::ReplicateCancelled { index });
        }
        Err(error) => {
            let reason = error.to_string();
            ui.publish(UiEvent::ReplicateFailed {
                index,
                reason: &reason,
            });
        }
    }
    result
}

fn run_replicate_inner(
    study: &Study,
    index: u64,
    scope: PathBuf,
    ui: &UiSession,
    scheduler_cancellation: &AtomicBool,
) -> Result<ReplicateRunSummary, RuntimeError> {
    let positions = topological_positions(study.phases());
    let mut phases = Vec::with_capacity(positions.len());
    for position in positions {
        if scheduler_cancellation.load(Ordering::Acquire) {
            return Err(RuntimeError::ExecutionCancelled);
        }
        let phase = &study.phases()[position];
        let dependencies_json = dependency_snapshot(phase, &phases);
        let context = PhaseRuntime {
            study,
            replicate_directory: &scope,
            dependencies_json,
            replicate: index,
            ui,
            scheduler_cancellation,
        };
        phases.push(run_phase(phase, &context)?);
    }
    Ok(ReplicateRunSummary {
        index,
        output_directory: scope,
        phases: phases.into_boxed_slice(),
    })
}

fn topological_positions(phases: &[StudyPhase]) -> Vec<usize> {
    fn visit(
        index: usize,
        phases: &[StudyPhase],
        by_name: &HashMap<&str, usize>,
        visited: &mut [bool],
        positions: &mut Vec<usize>,
    ) {
        if visited[index] {
            return;
        }
        for dependency in phases[index].dependencies() {
            visit(by_name[dependency], phases, by_name, visited, positions);
        }
        visited[index] = true;
        positions.push(index);
    }

    let by_name = phases
        .iter()
        .enumerate()
        .map(|(index, phase)| (phase.name(), index))
        .collect::<HashMap<_, _>>();
    let mut visited = vec![false; phases.len()];
    let mut positions = Vec::with_capacity(phases.len());
    for index in 0..phases.len() {
        visit(index, phases, &by_name, &mut visited, &mut positions);
    }
    positions
}

struct ActiveTask {
    task: StudyTask,
    cancellation: Arc<AtomicBool>,
    started: Instant,
    worker: JoinHandle<TaskWorkerOutcome>,
}

struct TaskWorkerOutcome {
    finished: Instant,
    result: Result<TaskRunSummary, RuntimeError>,
}

struct PhaseRuntime<'a> {
    study: &'a Study,
    replicate_directory: &'a Path,
    dependencies_json: Box<[u8]>,
    replicate: u64,
    ui: &'a UiSession,
    scheduler_cancellation: &'a AtomicBool,
}

struct TaskRuntime {
    persistence_plan: PersistencePlan,
    config_snapshot: ConfigSnapshot,
    project_root: PathBuf,
    replicate_directory: PathBuf,
    dependencies_json: Box<[u8]>,
    replicate: u64,
    master_seed: Option<u64>,
    ui: UiSession,
}

fn run_phase(
    phase: &StudyPhase,
    context: &PhaseRuntime<'_>,
) -> Result<PhaseRunSummary, RuntimeError> {
    context.ui.publish(UiEvent::PhaseStarted {
        replicate: context.replicate,
        name: phase.name(),
        task_count: phase.tasks().len(),
    });
    let result = run_phase_inner(phase, context);
    match &result {
        Ok(_) => context.ui.publish(UiEvent::PhaseCompleted {
            replicate: context.replicate,
            name: phase.name(),
        }),
        Err(RuntimeError::ExecutionCancelled) => {
            context.ui.publish(UiEvent::PhaseCancelled {
                replicate: context.replicate,
                name: phase.name(),
            });
        }
        Err(error) => {
            let reason = error.to_string();
            context.ui.publish(UiEvent::PhaseFailed {
                replicate: context.replicate,
                name: phase.name(),
                reason: &reason,
            });
        }
    }
    result
}

fn run_phase_inner(
    phase: &StudyPhase,
    context: &PhaseRuntime<'_>,
) -> Result<PhaseRunSummary, RuntimeError> {
    let phase_started = Instant::now();
    let mut pending = phase.tasks().iter().cloned().collect::<VecDeque<_>>();
    let mut active = Vec::<ActiveTask>::new();
    let mut completed = Vec::with_capacity(pending.len());
    let mut next_admission = phase_started;
    let mut first_error = None;
    let mut phase_timed_out = false;
    let mut execution_cancelled = false;

    while !pending.is_empty() || !active.is_empty() {
        if (context.ui.cancellation_requested()
            || context.scheduler_cancellation.load(Ordering::Acquire))
            && !execution_cancelled
        {
            execution_cancelled = true;
            pending.clear();
            for task in &active {
                task.cancellation.store(true, Ordering::Release);
            }
        }
        if let Some(timeout) = phase.timeout()
            && phase_started.elapsed() >= timeout
            && (!pending.is_empty() || active.iter().any(|task| !task.worker.is_finished()))
        {
            phase_timed_out = true;
            pending.clear();
            for task in &active {
                task.cancellation.store(true, Ordering::Release);
            }
        }

        let may_admit = !phase_timed_out
            && !execution_cancelled
            && (first_error.is_none() || phase.failure_policy() == FailurePolicy::FinishAll);
        while may_admit
            && active.len() < phase.max_concurrency()
            && !pending.is_empty()
            && Instant::now() >= next_admission
        {
            let task = pending.pop_front().expect("checked nonempty task queue");
            match spawn_task(task, phase.name(), context) {
                Ok(task) => active.push(task),
                Err(error) => {
                    first_error = Some(error);
                    pending.clear();
                    for task in &active {
                        task.cancellation.store(true, Ordering::Release);
                    }
                    break;
                }
            }
            next_admission = Instant::now() + phase.start_interval();
        }

        for task in &mut active {
            if let Some(timeout) = task.task.timeout()
                && task.started.elapsed() >= timeout
                && !task.worker.is_finished()
            {
                task.cancellation.store(true, Ordering::Release);
            }
        }

        let mut position = 0;
        while position < active.len() {
            if !active[position].worker.is_finished() {
                position += 1;
                continue;
            }
            let active_task = active.swap_remove(position);
            let identity = active_task.task.identity().to_owned();
            let outcome = active_task
                .worker
                .join()
                .unwrap_or_else(|_| TaskWorkerOutcome {
                    finished: Instant::now(),
                    result: Err(RuntimeError::TaskPanicked {
                        task: identity.clone(),
                    }),
                });
            let timed_out = active_task.task.timeout().is_some_and(|timeout| {
                task_exceeded_timeout(active_task.started, outcome.finished, timeout)
            });
            let result = if timed_out {
                Err(RuntimeError::TaskTimedOut {
                    task: identity,
                    timeout: active_task.task.timeout().expect("timed task has timeout"),
                })
            } else {
                outcome.result
            };
            match result {
                Ok(summary) => {
                    context.ui.publish(UiEvent::TaskCompleted {
                        replicate: context.replicate,
                        identity: active_task.task.identity(),
                        final_iteration: summary.final_iteration(),
                        output_directory: summary.output_directory(),
                    });
                    completed.push((active_task.task.output_ordinal(), summary));
                }
                Err(error) => {
                    if matches!(error, RuntimeError::TaskCancelled { .. }) {
                        context.ui.publish(UiEvent::TaskCancelled {
                            replicate: context.replicate,
                            identity: active_task.task.identity(),
                        });
                    } else {
                        let reason = error.to_string();
                        context.ui.publish(UiEvent::TaskFailed {
                            replicate: context.replicate,
                            identity: active_task.task.identity(),
                            reason: &reason,
                        });
                    }
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                    if phase.failure_policy() == FailurePolicy::FailFast {
                        pending.clear();
                        for sibling in &active {
                            sibling.cancellation.store(true, Ordering::Release);
                        }
                    }
                }
            }
        }

        if (!pending.is_empty() || !active.is_empty())
            && active.iter().all(|task| !task.worker.is_finished())
        {
            thread::sleep(SCHEDULER_POLL);
        }
    }

    if execution_cancelled {
        return Err(RuntimeError::ExecutionCancelled);
    }
    if phase_timed_out {
        return Err(RuntimeError::PhaseTimedOut {
            phase: phase.name().to_owned(),
            timeout: phase.timeout().expect("timed phase has timeout"),
        });
    }
    if let Some(error) = first_error {
        return Err(error);
    }
    completed.sort_by_key(|(ordinal, _)| *ordinal);
    Ok(PhaseRunSummary {
        name: phase.name().into(),
        tasks: completed
            .into_iter()
            .map(|(_, summary)| summary)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    })
}

pub(super) fn task_exceeded_timeout(
    started: Instant,
    finished: Instant,
    timeout: Duration,
) -> bool {
    finished.saturating_duration_since(started) >= timeout
}

fn spawn_task(
    task: StudyTask,
    phase: &str,
    context: &PhaseRuntime<'_>,
) -> Result<ActiveTask, RuntimeError> {
    let cancellation = Arc::new(AtomicBool::new(false));
    let worker_cancellation = Arc::clone(&cancellation);
    let worker_task = task.clone();
    let output_directory = context
        .replicate_directory
        .join(format!("task-{:06}", worker_task.output_ordinal()));
    let runtime = TaskRuntime {
        persistence_plan: context.study.persistence_plan(),
        config_snapshot: context.study.config_snapshot(),
        project_root: context.study.project_root().to_path_buf(),
        replicate_directory: context.replicate_directory.to_path_buf(),
        dependencies_json: context.dependencies_json.clone(),
        replicate: context.replicate,
        master_seed: context.study.master_seed(),
        ui: context.ui.clone(),
    };
    let thread_name = format!("workflow-task-{:06}", worker_task.output_ordinal());
    let started = Instant::now();
    let worker = match thread::Builder::new().name(thread_name).spawn(move || {
        let identity = worker_task.identity().to_owned();
        let result = catch_unwind(AssertUnwindSafe(|| {
            run_task(worker_task, runtime, worker_cancellation, output_directory)
        }))
        .unwrap_or_else(|_| Err(RuntimeError::TaskPanicked { task: identity }));
        TaskWorkerOutcome {
            finished: Instant::now(),
            result,
        }
    }) {
        Ok(worker) => worker,
        Err(source) => {
            let error = RuntimeError::StartWorker {
                scope: task.identity().to_owned(),
                source,
            };
            let reason = error.to_string();
            context.ui.publish(UiEvent::TaskFailed {
                replicate: context.replicate,
                identity: task.identity(),
                reason: &reason,
            });
            return Err(error);
        }
    };
    context.ui.publish(UiEvent::TaskStarted {
        replicate: context.replicate,
        phase,
        identity: task.identity(),
        label: task.label(),
        kind: task.kind_name(),
        subject: task.subject(),
    });
    Ok(ActiveTask {
        task,
        cancellation,
        started,
        worker,
    })
}

fn run_task(
    task: StudyTask,
    runtime: TaskRuntime,
    cancellation: Arc<AtomicBool>,
    output_directory: PathBuf,
) -> Result<TaskRunSummary, RuntimeError> {
    let initialization_context = task.model().map(|execution_unit_key| {
        InitializationContext::new(
            runtime.master_seed,
            runtime.replicate,
            task.identity(),
            execution_unit_key,
        )
    });
    let provenance = task.model_provenance().map(|provenance| {
        ModelRecordingProvenance::new(
            task.identity(),
            provenance.model(),
            provenance.state(),
            provenance.parameter_ordinal(),
            provenance.parameter_source(),
            provenance.constants().clone(),
        )
    });
    let environment = RuntimeTaskEnvironment::new(
        runtime.config_snapshot,
        runtime.project_root,
        runtime.replicate_directory,
        runtime.dependencies_json,
    );
    let mut host = RuntimeTaskHost::new(
        runtime.persistence_plan,
        cancellation,
        output_directory,
        provenance,
        initialization_context,
        runtime.ui.task(runtime.replicate, task.identity()),
        environment,
    );
    match catch_unwind(AssertUnwindSafe(|| task.definition().execute(&mut host))) {
        Ok(Ok(())) => {}
        Ok(Err(source)) => {
            host.fail(&source.to_string());
            return Err(RuntimeError::Task {
                task: task.identity().to_owned(),
                source,
            });
        }
        Err(payload) => {
            let reason = panic_reason(payload.as_ref());
            host.fail(&format!("task panicked: {reason}"));
            return Err(RuntimeError::TaskPanicked {
                task: task.identity().to_owned(),
            });
        }
    }
    if host.cancellation_requested() {
        host.fail("runtime cancellation requested");
        return Err(RuntimeError::TaskCancelled {
            task: task.identity().to_owned(),
        });
    }
    let (kind, model, program, program_kind, python_script, final_iteration) = match task.kind() {
        TaskKind::Model => (
            TaskRunKind::Model,
            task.model().map(Into::into),
            None,
            None,
            None,
            host.final_iteration(),
        ),
        TaskKind::Program => {
            let program = task
                .program_path()
                .expect("program task retains its resolved invocation");
            (
                TaskRunKind::Program,
                None,
                Some(program.to_path_buf()),
                task.program_kind_name().map(Into::into),
                task.python_script().map(Path::to_path_buf),
                None,
            )
        }
    };
    Ok(TaskRunSummary {
        identity: task.identity().into(),
        kind,
        model,
        program,
        program_kind,
        python_script,
        final_iteration,
        models: host.model_summaries(),
        output_directory: host.output_directory().to_path_buf(),
    })
}

fn panic_reason(payload: &(dyn std::any::Any + Send)) -> String {
    const MAX_CHARS: usize = 1_024;

    let message = payload
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-string panic payload".to_owned());
    let mut characters = message.chars();
    let bounded = characters.by_ref().take(MAX_CHARS).collect::<String>();
    if characters.next().is_some() {
        format!("{bounded}…")
    } else {
        bounded
    }
}

fn dependency_snapshot(phase: &StudyPhase, completed: &[PhaseRunSummary]) -> Box<[u8]> {
    let dependencies = phase
        .dependencies()
        .collect::<std::collections::HashSet<_>>();
    let values = completed
        .iter()
        .filter(|summary| dependencies.contains(summary.name()))
        .map(|summary| {
            serde_json::json!({
                "phase": summary.name(),
                "tasks": summary.tasks().iter().map(|task| {
                    serde_json::json!({
                        "identity": task.identity(),
                        "kind": match task.kind() {
                            TaskRunKind::Model => "model",
                            TaskRunKind::Program => "program",
                        },
                        "model": task.model(),
                        "program": task.program().map(|path| path.to_str()
                            .expect("Config preflight requires UTF-8 program paths")),
                        "program_kind": task.program_kind(),
                        "python_script": task.python_script().map(|path| path.to_str()
                            .expect("Config preflight requires UTF-8 Python script paths")),
                        "final_iteration": task.final_iteration(),
                        "models": task.models().iter().map(|model| {
                            serde_json::json!({
                                "identity": model.identity(),
                                "final_iteration": model.final_iteration(),
                                "output_directory": model.output_directory().to_str()
                                    .expect("UTF-8 project roots produce UTF-8 output paths")
                            })
                        }).collect::<Vec<_>>(),
                        "output_directory": task.output_directory().to_str()
                            .expect("UTF-8 project roots produce UTF-8 output paths")
                    })
                }).collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>();
    serde_json::to_vec_pretty(&values)
        .expect("serializing runtime dependency summaries cannot fail")
        .into_boxed_slice()
}
