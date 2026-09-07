//! Phase-local admission, timeout, cancellation, and worker scheduling.

use std::collections::VecDeque;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::super::error::RuntimeError;
use super::super::event::RuntimeEvent;
use super::super::presentation::RuntimePresentation;
use super::super::resources::{ResourceCoordinator, TaskResourceLease, TaskResourceRequirement};
use super::super::reuse;
use super::super::summary::{PhaseRunSummary, TaskRunSummary};
use super::dependency_snapshot;
use super::task::{TaskRuntime, run_task};
use crate::config::FailurePolicy;
use crate::study::{Study, StudyPhase, StudyTask};
use crate::task::TaskKind;

const SCHEDULER_POLL: Duration = Duration::from_millis(5);

struct ActiveTask {
    task: StudyTask,
    cancellation: Arc<AtomicBool>,
    started: Instant,
    worker: TaskWorker,
}

struct TaskWorker {
    handle: Option<JoinHandle<TaskWorkerOutcome>>,
    cancellation: Arc<AtomicBool>,
}
impl TaskWorker {
    fn is_finished(&self) -> bool {
        self.handle.as_ref().expect("live worker").is_finished()
    }
    fn join(mut self) -> std::thread::Result<TaskWorkerOutcome> {
        self.handle.take().expect("live worker").join()
    }
}
impl Drop for TaskWorker {
    fn drop(&mut self) {
        if let Some(worker) = self.handle.take() {
            self.cancellation.store(true, Ordering::Release);
            let _ = worker.join();
        }
    }
}

struct TaskWorkerOutcome {
    finished: Instant,
    result: Result<TaskRunSummary, RuntimeError>,
}

pub(super) struct PhaseRuntime<'a> {
    pub(super) study: &'a Study,
    pub(super) replicate_directory: &'a Path,
    pub(super) completed_phases: &'a [PhaseRunSummary],
    pub(super) replicate: u64,
    pub(super) presentation: &'a RuntimePresentation,
    pub(super) scheduler_cancellation: &'a AtomicBool,
    pub(super) resources: &'a ResourceCoordinator,
}

pub(super) fn run_phase(
    phase: &StudyPhase,
    context: &PhaseRuntime<'_>,
) -> Result<PhaseRunSummary, RuntimeError> {
    context.presentation.publish(RuntimeEvent::PhaseStarted {
        replicate: context.replicate,
        name: phase.name(),
        task_count: phase.tasks().iter().filter(|task| task.is_active()).count(),
    })?;
    let result = run_phase_inner(phase, context).and_then(|summary| {
        reuse::persist_phase(phase, &summary, context.replicate_directory)?;
        Ok(summary)
    });
    match &result {
        Ok(_) => context.presentation.publish(RuntimeEvent::PhaseCompleted {
            replicate: context.replicate,
            name: phase.name(),
        })?,
        Err(RuntimeError::ExecutionCancelled) => {
            context.presentation.publish(RuntimeEvent::PhaseCancelled {
                replicate: context.replicate,
                name: phase.name(),
            })?;
        }
        Err(error) => {
            let reason = error.to_string();
            context.presentation.publish(RuntimeEvent::PhaseFailed {
                replicate: context.replicate,
                name: phase.name(),
                reason: &reason,
            })?;
        }
    }
    result
}

fn run_phase_inner(
    phase: &StudyPhase,
    context: &PhaseRuntime<'_>,
) -> Result<PhaseRunSummary, RuntimeError> {
    let phase_started = context.presentation.control.now();
    let mut pending = phase
        .tasks()
        .iter()
        .filter(|task| task.is_active())
        .cloned()
        .collect::<VecDeque<_>>();
    let mut active = Vec::<ActiveTask>::new();
    let mut completed = Vec::with_capacity(pending.len());
    let mut next_admission = phase_started;
    let mut first_error = None;
    let mut phase_timed_out = false;
    let mut execution_cancelled = false;

    while !pending.is_empty() || !active.is_empty() {
        if (context.presentation.cancellation_requested()?
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
            && context
                .presentation
                .control
                .now()
                .saturating_duration_since(phase_started)
                >= timeout
            && (!pending.is_empty() || active.iter().any(|task| !task.worker.is_finished()))
        {
            phase_timed_out = true;
            pending.clear();
            for task in &active {
                task.cancellation.store(true, Ordering::Release);
            }
        }

        let may_admit = !context.presentation.control.paused()
            && !phase_timed_out
            && !execution_cancelled
            && (first_error.is_none() || phase.failure_policy() == FailurePolicy::FinishAll);
        while may_admit
            && !context.presentation.control.paused()
            && active.len() < phase.max_concurrency()
            && !pending.is_empty()
            && context.presentation.control.now() >= next_admission
        {
            let task = pending.front().expect("checked nonempty task queue");
            let requirement = if task.is_npy() {
                let bytes = dependency_snapshot(
                    phase,
                    context.study.phases(),
                    context.completed_phases,
                    task.configuration(),
                    true,
                );
                let raw = serde_json::from_slice(&bytes).expect("Runtime dependency JSON");
                let deps = crate::task::dependencies::Dependencies::from_json(raw)
                    .expect("Runtime dependencies");
                let count = deps
                    .recordings()
                    .iter()
                    .map(|r| r.directory())
                    .collect::<std::collections::BTreeSet<_>>()
                    .len();
                TaskResourceRequirement::External {
                    threads: context.study.threads().min(count.max(1)),
                }
            } else {
                task_resource_requirement(task, context.study.compute_mode())
            };
            let Some(resource_lease) = context.resources.try_acquire(requirement) else {
                break;
            };
            let task = pending.pop_front().expect("checked nonempty task queue");
            match spawn_task(task, phase, context, resource_lease) {
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
            next_admission = context.presentation.control.now() + phase.start_interval();
        }

        for task in &mut active {
            if let Some(timeout) = task.task.timeout()
                && context
                    .presentation
                    .control
                    .now()
                    .saturating_duration_since(task.started)
                    >= timeout
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
                    finished: context.presentation.control.now(),
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
                    context.presentation.publish(RuntimeEvent::TaskCompleted {
                        replicate: context.replicate,
                        identity: active_task.task.identity(),
                        final_iteration: summary.final_iteration(),
                        output_directory: summary.output_directory(),
                    })?;
                    completed.push((active_task.task.output_ordinal(), summary));
                }
                Err(error) => {
                    if matches!(error, RuntimeError::TaskCancelled { .. }) {
                        context.presentation.publish(RuntimeEvent::TaskCancelled {
                            replicate: context.replicate,
                            identity: active_task.task.identity(),
                        })?;
                    } else {
                        let reason = error.to_string();
                        context.presentation.publish(RuntimeEvent::TaskFailed {
                            replicate: context.replicate,
                            identity: active_task.task.identity(),
                            reason: &reason,
                        })?;
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
        reused: false,
        name: phase.name().into(),
        tasks: completed
            .into_iter()
            .map(|(_, summary)| summary)
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    })
}

pub(crate) fn task_exceeded_timeout(
    started: Instant,
    finished: Instant,
    timeout: Duration,
) -> bool {
    finished.saturating_duration_since(started) >= timeout
}

fn spawn_task(
    task: StudyTask,
    phase: &StudyPhase,
    context: &PhaseRuntime<'_>,
    resource_lease: TaskResourceLease,
) -> Result<ActiveTask, RuntimeError> {
    let cancellation = Arc::new(AtomicBool::new(false));
    let worker_cancellation = Arc::clone(&cancellation);
    let worker_task = task.clone();
    let output_directory = context
        .replicate_directory
        .join(format!("task-{:06}", worker_task.output_ordinal()));
    let processed_directory = worker_task.is_npy().then(|| {
        context
            .replicate_directory
            .parent()
            .expect("a replicate directory always belongs to an execution")
            .join("processed")
            .join(
                context
                    .replicate_directory
                    .file_name()
                    .expect("a replicate directory always has a stable name"),
            )
    });
    let dependencies_json = dependency_snapshot(
        phase,
        context.study.phases(),
        context.completed_phases,
        worker_task.configuration(),
        worker_task.is_npy(),
    );
    let threads = resource_lease.threads();
    let runtime = TaskRuntime {
        persistence_plan: context.study.persistence_plan(),
        config_snapshot: worker_task.config_snapshot(),
        project_root: context.study.project_root().to_path_buf(),
        replicate_directory: context.replicate_directory.to_path_buf(),
        dependencies_json,
        processed_directory,
        configuration: worker_task.configuration(),
        replicate: context.replicate,
        master_seed: context.study.master_seed(),
        threads,
        resources: resource_lease,
        presentation: context.presentation.clone(),
    };
    let thread_name = format!("workflow-task-{:06}", worker_task.output_ordinal());
    let started = context.presentation.control.now();
    let worker_control = context.presentation.control.clone();
    let activity = worker_control.activity();
    let (ready, start) = mpsc::sync_channel::<()>(0);
    let worker = match thread::Builder::new().name(thread_name).spawn(move || {
        let _activity = activity;
        if start.recv().is_err() {
            return TaskWorkerOutcome {
                finished: worker_control.now(),
                result: Err(RuntimeError::ExecutionCancelled),
            };
        }
        let identity = worker_task.identity().to_owned();
        let result = catch_unwind(AssertUnwindSafe(|| {
            run_task(worker_task, runtime, worker_cancellation, output_directory)
        }))
        .unwrap_or_else(|_| Err(RuntimeError::TaskPanicked { task: identity }));
        TaskWorkerOutcome {
            finished: worker_control.now(),
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
            context.presentation.publish(RuntimeEvent::TaskFailed {
                replicate: context.replicate,
                identity: task.identity(),
                reason: &reason,
            })?;
            return Err(error);
        }
    };
    if let Err(error) = context.presentation.publish(RuntimeEvent::TaskStarted {
        replicate: context.replicate,
        phase: phase.name(),
        identity: task.identity(),
        label: task.label(),
        kind: task.kind_name(),
        subject: task.subject(),
    }) {
        drop(ready);
        let _ = worker.join();
        return Err(error);
    }
    let _ = ready.send(());
    Ok(ActiveTask {
        task,
        cancellation: Arc::clone(&cancellation),
        started,
        worker: TaskWorker {
            handle: Some(worker),
            cancellation: Arc::clone(&cancellation),
        },
    })
}

fn task_resource_requirement(
    task: &StudyTask,
    compute_mode: crate::config::ComputeMode,
) -> TaskResourceRequirement {
    match task.kind() {
        TaskKind::ExecutionUnit => match compute_mode {
            crate::config::ComputeMode::Auto => TaskResourceRequirement::AutoInProcess,
            crate::config::ComputeMode::Isolated => TaskResourceRequirement::IsolatedInProcess {
                threads: task
                    .execution_unit_threads()
                    .expect("isolated execution unit retains its thread request"),
            },
        },
        TaskKind::Program => TaskResourceRequirement::External {
            threads: task.program_threads(),
        },
    }
}
