//! Runtime scheduling for a completed immutable Study.

use std::collections::{HashMap, VecDeque};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::error::RuntimeError;
use super::event::RuntimeEvent;
use super::host::{ProgramSeed, RuntimeTaskEnvironment, RuntimeTaskHost, RuntimeTaskLaunch};
use super::output::{create_execution, create_replicate};
use super::presentation::{PresentationFailure, RuntimeObserver, RuntimePresentation};
use super::resource::{ResourceBudget, ResourceLease, ResourceRequirement};
use super::summary::{
    PhaseRunSummary, ReplicateRunSummary, RunSummary, TaskRunKind, TaskRunSummary,
};
use crate::config::{ConfigSnapshot, FailurePolicy, ReplicateScheduling};
use crate::persistence::{MemberRecordingProvenance, PersistencePlan};
use crate::study::{Study, StudyPhase, StudyTask};
use crate::task::{
    InitializationContext, SEED_DERIVATION_ALGORITHM, TaskKind, derive_program_seed,
};

const SCHEDULER_POLL: Duration = Duration::from_millis(5);

pub(crate) fn execute_with_observer<O, F>(
    study: Study,
    create_observer: F,
) -> Result<RunSummary, RuntimeError>
where
    O: RuntimeObserver,
    F: FnOnce() -> Result<O, PresentationFailure>,
{
    let compute_pool = Arc::new(
        rayon::ThreadPoolBuilder::new()
            .num_threads(study.threads())
            .thread_name(|index| format!("workflow-compute-{index}"))
            .build()
            .map_err(|source| RuntimeError::ComputePool {
                threads: study.threads(),
                source,
            })?,
    );
    let output = create_execution(study.output_root())?;
    let resource_budget = ResourceBudget::new(study.threads());
    let observer = create_observer().map_err(RuntimeError::presentation_boxed)?;
    let presentation = RuntimePresentation::new(observer);
    let outcome =
        execute_with_presentation(study, compute_pool, resource_budget, output, &presentation);
    let finish = presentation.finish();
    finish?;
    outcome
}

fn execute_with_presentation(
    study: Study,
    compute_pool: Arc<rayon::ThreadPool>,
    resource_budget: ResourceBudget,
    output: PathBuf,
    presentation: &RuntimePresentation,
) -> Result<RunSummary, RuntimeError> {
    let count = study.replicate_policy().count();
    let task_count_per_replicate = study.phases().iter().map(|phase| phase.tasks().len()).sum();
    for replicate in 0..count {
        for phase in study.phases() {
            for task in phase.tasks() {
                presentation.publish(RuntimeEvent::TaskPlanned {
                    replicate,
                    phase: phase.name(),
                    identity: task.identity(),
                    label: task.label(),
                    kind: task.kind_name(),
                })?;
            }
        }
    }
    presentation.publish(RuntimeEvent::ExecutionStarted {
        output_directory: &output,
        replicate_count: count,
        task_count_per_replicate,
    })?;

    let result = (|| {
        let mut scopes = Vec::new();
        for index in 0..count {
            scopes.push((index, create_replicate(&output, index)?));
        }
        match study.replicate_policy().scheduling() {
            ReplicateScheduling::Sequential => run_replicates_sequential(
                &study,
                scopes,
                presentation,
                &compute_pool,
                &resource_budget,
            ),
            ReplicateScheduling::Parallel => run_replicates_parallel(
                &study,
                scopes,
                presentation,
                &compute_pool,
                &resource_budget,
            ),
        }
    })();

    let result = if presentation.cancellation_requested()? {
        Err(RuntimeError::ExecutionCancelled)
    } else {
        result
    };
    match result {
        Ok(replicates) => {
            presentation.publish(RuntimeEvent::ExecutionCompleted {
                output_directory: &output,
            })?;
            Ok(RunSummary {
                output_directory: output,
                replicates: replicates.into_boxed_slice(),
            })
        }
        Err(error) => {
            if matches!(error, RuntimeError::ExecutionCancelled) {
                presentation.publish(RuntimeEvent::ExecutionCancelled)?;
            } else {
                let reason = error.to_string();
                presentation.publish(RuntimeEvent::ExecutionFailed { reason: &reason })?;
            }
            Err(error)
        }
    }
}

fn run_replicates_sequential(
    study: &Study,
    scopes: Vec<(u64, PathBuf)>,
    presentation: &RuntimePresentation,
    compute_pool: &Arc<rayon::ThreadPool>,
    resource_budget: &ResourceBudget,
) -> Result<Vec<ReplicateRunSummary>, RuntimeError> {
    let mut summaries = Vec::with_capacity(scopes.len());
    let mut first_error = None;
    for (index, scope) in scopes {
        let cancellation = AtomicBool::new(false);
        match run_replicate(
            study,
            index,
            scope,
            presentation,
            &cancellation,
            compute_pool,
            resource_budget,
        ) {
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
    presentation: &RuntimePresentation,
    compute_pool: &Arc<rayon::ThreadPool>,
    resource_budget: &ResourceBudget,
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
        let presentation = presentation.clone();
        let outcomes = outcomes.clone();
        let worker_cancellation = Arc::clone(&cancellation);
        let compute_pool = Arc::clone(compute_pool);
        let resource_budget = resource_budget.clone();
        let worker = match thread::Builder::new()
            .name(format!("workflow-replicate-{index}"))
            .spawn(move || {
                let outcome = catch_unwind(AssertUnwindSafe(|| {
                    run_replicate(
                        &study,
                        index,
                        scope,
                        &presentation,
                        &worker_cancellation,
                        &compute_pool,
                        &resource_budget,
                    )
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
    presentation: &RuntimePresentation,
    scheduler_cancellation: &AtomicBool,
    compute_pool: &Arc<rayon::ThreadPool>,
    resource_budget: &ResourceBudget,
) -> Result<ReplicateRunSummary, RuntimeError> {
    presentation.publish(RuntimeEvent::ReplicateStarted { index })?;
    let result = run_replicate_inner(
        study,
        index,
        scope,
        presentation,
        scheduler_cancellation,
        compute_pool,
        resource_budget,
    );
    match &result {
        Ok(_) => presentation.publish(RuntimeEvent::ReplicateCompleted { index })?,
        Err(RuntimeError::ExecutionCancelled) => {
            presentation.publish(RuntimeEvent::ReplicateCancelled { index })?;
        }
        Err(error) => {
            let reason = error.to_string();
            presentation.publish(RuntimeEvent::ReplicateFailed {
                index,
                reason: &reason,
            })?;
        }
    }
    result
}

fn run_replicate_inner(
    study: &Study,
    index: u64,
    scope: PathBuf,
    presentation: &RuntimePresentation,
    scheduler_cancellation: &AtomicBool,
    compute_pool: &Arc<rayon::ThreadPool>,
    resource_budget: &ResourceBudget,
) -> Result<ReplicateRunSummary, RuntimeError> {
    let positions = topological_positions(study.phases());
    let mut phases = Vec::with_capacity(positions.len());
    for position in positions {
        if scheduler_cancellation.load(Ordering::Acquire) {
            return Err(RuntimeError::ExecutionCancelled);
        }
        let phase = &study.phases()[position];
        let context = PhaseRuntime {
            study,
            replicate_directory: &scope,
            completed_phases: &phases,
            replicate: index,
            presentation,
            scheduler_cancellation,
            compute_pool,
            resource_budget,
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
    completed_phases: &'a [PhaseRunSummary],
    replicate: u64,
    presentation: &'a RuntimePresentation,
    scheduler_cancellation: &'a AtomicBool,
    compute_pool: &'a Arc<rayon::ThreadPool>,
    resource_budget: &'a ResourceBudget,
}

struct TaskRuntime {
    persistence_plan: PersistencePlan,
    config_snapshot: ConfigSnapshot,
    project_root: PathBuf,
    replicate_directory: PathBuf,
    dependencies_json: Box<[u8]>,
    processed_directory: Option<PathBuf>,
    configuration: usize,
    replicate: u64,
    master_seed: Option<u64>,
    threads: usize,
    compute_pool: Arc<rayon::ThreadPool>,
    presentation: RuntimePresentation,
}

fn run_phase(
    phase: &StudyPhase,
    context: &PhaseRuntime<'_>,
) -> Result<PhaseRunSummary, RuntimeError> {
    context.presentation.publish(RuntimeEvent::PhaseStarted {
        replicate: context.replicate,
        name: phase.name(),
        task_count: phase.tasks().len(),
    })?;
    let result = run_phase_inner(phase, context);
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
    let phase_started = Instant::now();
    let mut pending = phase.tasks().iter().cloned().collect::<VecDeque<_>>();
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
            let task = pending.front().expect("checked nonempty task queue");
            let requirement = task_resource_requirement(task);
            let Some(resource_lease) = context.resource_budget.try_acquire(requirement) else {
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
    phase: &StudyPhase,
    context: &PhaseRuntime<'_>,
    resource_lease: ResourceLease,
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
        threads: task_effective_threads(&worker_task, context.study.threads()),
        compute_pool: Arc::clone(context.compute_pool),
        presentation: context.presentation.clone(),
    };
    let thread_name = format!("workflow-task-{:06}", worker_task.output_ordinal());
    let started = Instant::now();
    let worker = match thread::Builder::new().name(thread_name).spawn(move || {
        let _resource_lease = resource_lease;
        let identity = worker_task.identity().to_owned();
        let task_kind = worker_task.kind();
        let compute_pool = Arc::clone(&runtime.compute_pool);
        let result = catch_unwind(AssertUnwindSafe(|| {
            let execute = || run_task(worker_task, runtime, worker_cancellation, output_directory);
            match task_kind {
                TaskKind::ExecutionUnit => compute_pool.install(execute),
                TaskKind::Program => execute(),
            }
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
            context.presentation.publish(RuntimeEvent::TaskFailed {
                replicate: context.replicate,
                identity: task.identity(),
                reason: &reason,
            })?;
            return Err(error);
        }
    };
    context.presentation.publish(RuntimeEvent::TaskStarted {
        replicate: context.replicate,
        phase: phase.name(),
        identity: task.identity(),
        label: task.label(),
        kind: task.kind_name(),
        subject: task.subject(),
    })?;
    Ok(ActiveTask {
        task,
        cancellation,
        started,
        worker,
    })
}

fn task_resource_requirement(task: &StudyTask) -> ResourceRequirement {
    match task.kind() {
        TaskKind::ExecutionUnit => ResourceRequirement::InProcess,
        TaskKind::Program => ResourceRequirement::External {
            threads: task.program_threads(),
        },
    }
}

fn task_effective_threads(task: &StudyTask, study_threads: usize) -> usize {
    match task.kind() {
        TaskKind::ExecutionUnit => study_threads,
        TaskKind::Program => task.program_threads(),
    }
}

fn run_task(
    task: StudyTask,
    runtime: TaskRuntime,
    cancellation: Arc<AtomicBool>,
    output_directory: PathBuf,
) -> Result<TaskRunSummary, RuntimeError> {
    let processed_directory = runtime.processed_directory.clone();
    let program_seed = task.program_seed_purpose().map(|purpose| {
        let master_seed = runtime
            .master_seed
            .expect("Config rejects a program seed request without a master seed");
        let seed = derive_program_seed(
            master_seed,
            runtime.replicate,
            task.identity(),
            task.kind_name(),
            purpose,
        );
        ProgramSeed::new(
            seed,
            serde_json::json!({
                "algorithm": SEED_DERIVATION_ALGORITHM,
                "master_seed": master_seed,
                "requests": [{
                    "scope": "task",
                    "purpose": purpose,
                    "seed": seed
                }]
            }),
        )
    });
    let initialization_context = task.execution_unit().map(|execution_unit_key| {
        let dependencies = serde_json::from_slice(&runtime.dependencies_json)
            .expect("Runtime's dependency snapshot is valid JSON");
        InitializationContext::with_dependencies(
            runtime.master_seed,
            runtime.replicate,
            task.identity(),
            execution_unit_key,
            dependencies,
        )
    });
    let provenance = task.execution_unit_provenance().map(|provenance| {
        let mut parameters = runtime.config_snapshot.parameters().clone();
        parameters
            .as_object_mut()
            .expect("parameters.json root is an object")
            .insert(
                provenance.execution_unit().to_owned(),
                provenance.constants().clone(),
            );
        MemberRecordingProvenance::new(
            task.identity(),
            provenance.execution_unit(),
            provenance.state(),
            provenance.parameter_ordinal(),
            provenance.parameter_source(),
            provenance.constants().clone(),
            runtime.threads,
        )
        .with_parameters(parameters)
    });
    let environment = RuntimeTaskEnvironment::new(
        runtime.config_snapshot,
        runtime.project_root,
        runtime.replicate_directory,
        runtime.dependencies_json,
        runtime.processed_directory,
    );
    let mut host = RuntimeTaskHost::new(
        runtime.persistence_plan,
        cancellation,
        output_directory,
        RuntimeTaskLaunch::new(
            provenance,
            initialization_context,
            program_seed,
            runtime.threads,
            runtime
                .presentation
                .task(runtime.replicate, task.identity()),
            environment,
        ),
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
    let kind = match task.kind() {
        TaskKind::ExecutionUnit => TaskRunKind::ExecutionUnit {
            execution_unit: task
                .execution_unit()
                .expect("execution-unit task retains its registration key")
                .into(),
            members: host.member_summaries(),
        },
        TaskKind::Program if task.is_npy() => TaskRunKind::Npy {
            launcher: task
                .program_path()
                .expect("NPY task retains its resolved Python launcher")
                .to_path_buf(),
            processed_directory: processed_directory
                .expect("an NPY task retains its standard processed directory"),
        },
        TaskKind::Program => TaskRunKind::Program {
            executable: task
                .program_path()
                .expect("program task retains its resolved invocation")
                .to_path_buf(),
            python_script: task.python_script().map(Path::to_path_buf),
        },
    };
    Ok(TaskRunSummary {
        identity: task.identity().into(),
        kind,
        output_directory: host.output_directory().to_path_buf(),
        configuration: runtime.configuration,
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

fn dependency_snapshot(
    phase: &StudyPhase,
    study_phases: &[StudyPhase],
    completed: &[PhaseRunSummary],
    configuration: usize,
    transitive: bool,
) -> Box<[u8]> {
    let dependencies = if transitive {
        transitive_dependencies(phase, study_phases)
    } else {
        phase.dependencies().collect()
    };
    let values = completed
        .iter()
        .filter(|summary| dependencies.contains(summary.name()))
        .map(|summary| {
            serde_json::json!({
                "phase": summary.name(),
                "tasks": summary.tasks().iter().filter(|task| {
                    transitive
                        || task.configuration() == configuration
                        || matches!(task.kind(), TaskRunKind::Npy { .. })
                }).map(|task| {
                    serde_json::json!({
                        "identity": task.identity(),
                        "workload": dependency_workload(task.kind()),
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

fn transitive_dependencies<'a>(
    phase: &'a StudyPhase,
    phases: &'a [StudyPhase],
) -> std::collections::HashSet<&'a str> {
    fn visit<'a>(
        phase: &'a StudyPhase,
        by_name: &HashMap<&'a str, &'a StudyPhase>,
        found: &mut std::collections::HashSet<&'a str>,
    ) {
        for dependency in phase.dependencies() {
            if found.insert(dependency) {
                visit(by_name[dependency], by_name, found);
            }
        }
    }

    let by_name = phases
        .iter()
        .map(|phase| (phase.name(), phase))
        .collect::<HashMap<_, _>>();
    let mut found = std::collections::HashSet::new();
    visit(phase, &by_name, &mut found);
    found
}

fn dependency_workload(kind: &TaskRunKind) -> serde_json::Value {
    match kind {
        TaskRunKind::ExecutionUnit {
            execution_unit,
            members,
        } => serde_json::json!({
            "kind": "execution_unit",
            "execution_unit": execution_unit,
            "members": members.iter().map(|member| {
                serde_json::json!({
                    "identity": member.identity(),
                    "final_iteration": member.final_iteration(),
                    "output_directory": member.output_directory().to_str()
                        .expect("UTF-8 project roots produce UTF-8 output paths")
                })
            }).collect::<Vec<_>>(),
        }),
        TaskRunKind::Program {
            executable,
            python_script,
        } => serde_json::json!({
            "kind": if python_script.is_some() { "python" } else { "program" },
            "executable": executable.to_str()
                .expect("Config preflight requires UTF-8 program paths"),
            "python_script": python_script.as_deref().map(|path| path.to_str()
                .expect("Config preflight requires UTF-8 Python script paths")),
        }),
        TaskRunKind::Npy {
            launcher,
            processed_directory,
        } => serde_json::json!({
            "kind": "npy",
            "launcher": launcher.to_str()
                .expect("Config preflight requires UTF-8 Python launcher paths"),
            "processed_directory": processed_directory.to_str()
                .expect("UTF-8 project roots produce UTF-8 processed paths"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phase(name: &str, dependencies: &[&str]) -> StudyPhase {
        StudyPhase {
            name: name.into(),
            dependencies: dependencies.iter().map(|value| (*value).into()).collect(),
            tasks: Vec::new().into_boxed_slice(),
            max_concurrency: 1,
            start_interval: Duration::ZERO,
            timeout: None,
            failure_policy: FailurePolicy::FailFast,
        }
    }

    #[test]
    fn npy_dependency_walk_reaches_every_ancestor_once() {
        let phases = [
            phase("prepare", &[]),
            phase("simulate", &["prepare"]),
            phase("export", &["prepare", "simulate"]),
            phase("$npy", &["export"]),
        ];

        let dependencies = transitive_dependencies(&phases[3], &phases);

        assert_eq!(dependencies.len(), 3);
        assert!(dependencies.contains("prepare"));
        assert!(dependencies.contains("simulate"));
        assert!(dependencies.contains("export"));
    }

    #[test]
    fn npy_aggregates_configurations_and_remains_visible_downstream() {
        let phases = [
            phase("simulate", &[]),
            phase("$npy", &["simulate"]),
            phase("plot", &["$npy"]),
        ];
        let completed_simulation = PhaseRunSummary {
            name: "simulate".into(),
            tasks: [0, 1]
                .into_iter()
                .map(|configuration| TaskRunSummary {
                    identity: format!("simulate-{configuration}").into(),
                    kind: TaskRunKind::Program {
                        executable: PathBuf::from("/bin/true"),
                        python_script: None,
                    },
                    output_directory: PathBuf::from(format!("task-{configuration}")),
                    configuration,
                })
                .collect(),
        };

        let aggregate = dependency_snapshot(
            &phases[1],
            &phases,
            std::slice::from_ref(&completed_simulation),
            0,
            true,
        );
        let aggregate: serde_json::Value = serde_json::from_slice(&aggregate).unwrap();
        assert_eq!(aggregate[0]["tasks"].as_array().unwrap().len(), 2);

        let completed_npy = PhaseRunSummary {
            name: "$npy".into(),
            tasks: [TaskRunSummary {
                identity: "npy".into(),
                kind: TaskRunKind::Npy {
                    launcher: PathBuf::from("/usr/bin/python3"),
                    processed_directory: PathBuf::from("processed/replicate-000000"),
                },
                output_directory: PathBuf::from("task-npy"),
                configuration: 0,
            }]
            .into(),
        };
        let downstream = dependency_snapshot(
            &phases[2],
            &phases,
            std::slice::from_ref(&completed_npy),
            1,
            false,
        );
        let downstream: serde_json::Value = serde_json::from_slice(&downstream).unwrap();
        assert_eq!(
            downstream[0]["tasks"][0]["workload"]["processed_directory"],
            "processed/replicate-000000"
        );
    }
}
