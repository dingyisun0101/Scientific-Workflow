//! Runtime scheduling and the single ordinary entry point.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::{Map, Value};

use crate::WorkflowError;
use crate::config::advanced::{FailurePolicy, ReplicateScheduling};
use crate::persistence::advanced::PersistencePlan;
use crate::state::advanced::SystemStateSchema;
use crate::study::advanced::{Study, StudyPhase, StudyTask};

use super::error::RuntimeError;
use super::host::RuntimeTaskHost;
use super::output::{create_execution, create_replicate};
use super::summary::{PhaseRunSummary, ReplicateRunSummary, RunSummary, TaskRunSummary};

const SCHEDULER_POLL: Duration = Duration::from_millis(5);

/// Loads, preflights, and executes the project rooted at `project_root`.
///
/// This is the sole ordinary application entry point. Successful completion
/// returns `()`; advanced integrations may call [`execute`] to retain a
/// read-only run summary.
pub fn run(project_root: &Path) -> Result<(), WorkflowError> {
    let study = Study::load(project_root)?;
    execute(study)?;
    Ok(())
}

/// Executes one already validated immutable study and returns its summary.
pub fn execute(study: Study) -> Result<RunSummary, RuntimeError> {
    let output = create_execution(study.output_root())?;
    let count = study.replicate_policy().count();
    let mut scopes = Vec::new();
    for index in 0..count {
        scopes.push((index, create_replicate(&output, index)?));
    }

    let replicates = match study.replicate_policy().scheduling() {
        ReplicateScheduling::Sequential => run_replicates_sequential(&study, scopes)?,
        ReplicateScheduling::Parallel => run_replicates_parallel(&study, scopes)?,
    };
    Ok(RunSummary {
        output_directory: output,
        replicates: replicates.into_boxed_slice(),
    })
}

fn run_replicates_sequential(
    study: &Study,
    scopes: Vec<(u64, PathBuf)>,
) -> Result<Vec<ReplicateRunSummary>, RuntimeError> {
    let mut summaries = Vec::with_capacity(scopes.len());
    let mut first_error = None;
    for (index, scope) in scopes {
        match run_replicate(study, index, scope) {
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
) -> Result<Vec<ReplicateRunSummary>, RuntimeError> {
    let mut workers: Vec<(u64, JoinHandle<Result<ReplicateRunSummary, RuntimeError>>)> =
        Vec::with_capacity(scopes.len());
    for (index, scope) in scopes {
        let study = study.clone();
        let worker = match thread::Builder::new()
            .name(format!("workflow-replicate-{index}"))
            .spawn(move || run_replicate(&study, index, scope))
        {
            Ok(worker) => worker,
            Err(source) => {
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
    let mut summaries = Vec::with_capacity(workers.len());
    let mut first_error = None;
    for (index, worker) in workers {
        match worker.join() {
            Ok(Ok(summary)) => summaries.push(summary),
            Ok(Err(source)) => {
                first_error.get_or_insert(RuntimeError::Replicate {
                    index,
                    source: Box::new(source),
                });
            }
            Err(_) => {
                first_error.get_or_insert(RuntimeError::ReplicatePanicked { index });
            }
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
) -> Result<ReplicateRunSummary, RuntimeError> {
    let positions = topological_positions(study.phases());
    let mut phases = Vec::with_capacity(positions.len());
    for position in positions {
        phases.push(run_phase(
            &study.phases()[position],
            study.state_schema(),
            study.persistence_plan(),
            &scope,
        )?);
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
    timed_out: bool,
    worker: JoinHandle<Result<TaskRunSummary, RuntimeError>>,
}

fn run_phase(
    phase: &StudyPhase,
    schema: &SystemStateSchema,
    persistence_plan: PersistencePlan,
    replicate_directory: &Path,
) -> Result<PhaseRunSummary, RuntimeError> {
    let phase_started = Instant::now();
    let mut pending = phase.tasks().iter().cloned().collect::<VecDeque<_>>();
    let mut active = Vec::<ActiveTask>::new();
    let mut completed = Vec::with_capacity(pending.len());
    let mut next_admission = phase_started;
    let mut first_error = None;
    let mut phase_timed_out = false;

    while !pending.is_empty() || !active.is_empty() {
        if let Some(timeout) = phase.timeout()
            && phase_started.elapsed() >= timeout
        {
            phase_timed_out = true;
            pending.clear();
            for task in &active {
                task.cancellation.store(true, Ordering::Release);
            }
        }

        let may_admit = !phase_timed_out
            && (first_error.is_none() || phase.failure_policy() == FailurePolicy::FinishAll);
        while may_admit
            && active.len() < phase.max_concurrency()
            && !pending.is_empty()
            && Instant::now() >= next_admission
        {
            let task = pending.pop_front().expect("checked nonempty task queue");
            match spawn_task(task, schema.clone(), persistence_plan, replicate_directory) {
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
            {
                task.timed_out = true;
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
            let result = active_task
                .worker
                .join()
                .map_err(|_| RuntimeError::TaskPanicked {
                    task: identity.clone(),
                })?;
            let result = if active_task.timed_out {
                Err(RuntimeError::TaskTimedOut {
                    task: identity,
                    timeout: active_task.task.timeout().expect("timed task has timeout"),
                })
            } else {
                result
            };
            match result {
                Ok(summary) => completed.push((active_task.task.output_ordinal(), summary)),
                Err(error) => {
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

fn spawn_task(
    task: StudyTask,
    schema: SystemStateSchema,
    persistence_plan: PersistencePlan,
    replicate_directory: &Path,
) -> Result<ActiveTask, RuntimeError> {
    let cancellation = Arc::new(AtomicBool::new(false));
    let worker_cancellation = Arc::clone(&cancellation);
    let worker_task = task.clone();
    let recording_directory =
        replicate_directory.join(format!("task-{:06}", worker_task.output_ordinal()));
    let thread_name = format!("workflow-task-{:06}", worker_task.output_ordinal());
    let worker = thread::Builder::new()
        .name(thread_name)
        .spawn(move || {
            run_task(
                worker_task,
                schema,
                persistence_plan,
                worker_cancellation,
                recording_directory,
            )
        })
        .map_err(|source| RuntimeError::StartWorker {
            scope: task.identity().to_owned(),
            source,
        })?;
    Ok(ActiveTask {
        task,
        cancellation,
        started: Instant::now(),
        timed_out: false,
        worker,
    })
}

fn run_task(
    task: StudyTask,
    schema: SystemStateSchema,
    persistence_plan: PersistencePlan,
    cancellation: Arc<AtomicBool>,
    recording_directory: PathBuf,
) -> Result<TaskRunSummary, RuntimeError> {
    let metadata = task_metadata(&task, persistence_plan);
    let mut host = RuntimeTaskHost::new(
        schema,
        persistence_plan,
        cancellation,
        recording_directory,
        metadata,
    );
    if let Err(source) = task
        .definition()
        .execute(task.input(), task.observation_plan(), &mut host)
    {
        host.fail(&source.to_string());
        return Err(RuntimeError::Task {
            task: task.identity().to_owned(),
            source,
        });
    }
    if host.cancellation_requested() {
        host.fail("runtime cancellation requested");
        return Err(RuntimeError::TaskCancelled {
            task: task.identity().to_owned(),
        });
    }
    let final_iteration = host.final_iteration().unwrap_or(0);
    Ok(TaskRunSummary {
        identity: task.identity().into(),
        model: task.model().into(),
        final_iteration,
        recording_directory: host.recording_directory().to_path_buf(),
    })
}

fn task_metadata(task: &StudyTask, persistence_plan: PersistencePlan) -> Map<String, Value> {
    let constants = serde_json::from_slice(task.input().resolved_json())
        .expect("config retains valid resolved JSON");
    let workflow = Value::Object(Map::from_iter([
        ("task_identity".to_owned(), task.identity().into()),
        ("model".to_owned(), task.model().into()),
        ("input_ordinal".to_owned(), task.input().ordinal().into()),
        (
            "input_source".to_owned(),
            task.input()
                .source_path()
                .to_string_lossy()
                .into_owned()
                .into(),
        ),
        (
            "persistence".to_owned(),
            Value::Object(Map::from_iter([
                ("backend".to_owned(), "local".into()),
                (
                    "chunk_target_bytes".to_owned(),
                    persistence_plan.chunk_target().get().into(),
                ),
                (
                    "queue_capacity_bytes".to_owned(),
                    persistence_plan.queue_capacity().get().into(),
                ),
            ])),
        ),
    ]));
    Map::from_iter([
        ("model_constants".to_owned(), constants),
        ("workflow".to_owned(), workflow),
    ])
}
