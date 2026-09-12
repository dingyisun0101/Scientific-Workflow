//! Runtime scheduling for a completed immutable Study.

mod phase;
mod task;

#[cfg(test)]
pub(super) use self::phase::task_exceeded_timeout;
use self::phase::{PhaseRuntime, run_phase};

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
#[cfg(test)]
use std::time::Duration;

use super::error::RuntimeError;
use super::event::RuntimeEvent;
use super::output::{create_execution, create_replicate};
use super::presentation::{PresentationFailure, RuntimeObserver, RuntimePresentation};
use super::resources::ResourceCoordinator;
use super::reuse::{self, ReusedPhases};
#[cfg(test)]
use super::summary::TaskRunSummary;
use super::summary::{PhaseRunSummary, ReplicateRunSummary, RunSummary, TaskRunKind};
use crate::config::{FailurePolicy, ReplicateScheduling};
use crate::study::{Study, StudyPhase};

#[cfg(test)]
pub(crate) fn execute_with_observer<O, F>(
    study: Study,
    create_observer: F,
) -> Result<RunSummary, RuntimeError>
where
    O: RuntimeObserver,
    F: FnOnce() -> Result<O, PresentationFailure>,
{
    execute_with_observer_options(study, create_observer, false)
}

pub(crate) fn execute_with_observer_options<O, F>(
    study: Study,
    create_observer: F,
    clean: bool,
) -> Result<RunSummary, RuntimeError>
where
    O: RuntimeObserver,
    F: FnOnce() -> Result<O, PresentationFailure>,
{
    let reused = reuse::prepare(&study)?;
    super::program::check_prerequisites(&study)?;
    let _lease =
        super::output::OutputLease::acquire(study.project_root(), clean).map_err(|source| {
            RuntimeError::OutputScope {
                path: study.output_root().to_path_buf(),
                source,
            }
        })?;
    if clean {
        let config_root = study.project_root().join("wf_configs");
        let mut protected = vec![config_root.as_path()];
        protected.extend(study.reuse_from());
        for phase in study.phases() {
            for task in phase.tasks() {
                protected.extend(task.program_path());
                protected.extend(task.python_script());
            }
        }
        for phase in reused.values() {
            for task in phase.tasks() {
                protected.push(task.output_directory());
                match task.kind() {
                    TaskRunKind::ExecutionUnit { members, .. } => {
                        protected.extend(members.iter().map(|member| member.output_directory()));
                    }
                    TaskRunKind::Npy {
                        processed_directory,
                        ..
                    } => protected.push(processed_directory),
                    _ => {}
                }
            }
        }
        super::output::clean_output(study.output_root(), &protected).map_err(|source| {
            RuntimeError::OutputScope {
                path: study.output_root().to_path_buf(),
                source,
            }
        })?;
    }
    let resources = ResourceCoordinator::new(study.threads(), study.compute_mode());
    let output = create_execution(study.output_root())?;
    let observer = create_observer().map_err(RuntimeError::presentation_boxed)?;
    let presentation = RuntimePresentation::new(observer);
    let outcome = execute_with_presentation(study, resources, output, &presentation, &reused);
    let finish = presentation.finish();
    finish?;
    outcome
}

fn execute_with_presentation(
    study: Study,
    resources: ResourceCoordinator,
    output: PathBuf,
    presentation: &RuntimePresentation,
    reused: &ReusedPhases,
) -> Result<RunSummary, RuntimeError> {
    let count = study.replicate_policy().count();
    let task_count_per_replicate = study
        .phase_order()
        .iter()
        .enumerate()
        .filter(|(index, _)| study.phase_is_active(*index))
        .map(|(_, &position)| {
            study.phases()[position]
                .tasks()
                .iter()
                .filter(|task| task.is_active())
                .count()
        })
        .sum();
    for replicate in 0..count {
        for (phase_index, &position) in study.phase_order().iter().enumerate() {
            if !study.phase_is_active(phase_index) {
                continue;
            }
            let phase = &study.phases()[position];
            for task in phase.tasks().iter().filter(|task| task.is_active()) {
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
        let disk = super::disk::DiskGuard::start(&output, study.disk_pause_at(), presentation)?;
        let mut scopes = Vec::new();
        for index in 0..count {
            scopes.push((index, create_replicate(&output, index)?));
        }
        let result = match study.replicate_policy().scheduling() {
            ReplicateScheduling::Sequential => {
                run_replicates_sequential(&study, scopes, presentation, &resources, reused)
            }
            ReplicateScheduling::Parallel => {
                run_replicates_parallel(&study, scopes, presentation, &resources, reused)
            }
        };
        disk.finish()?;
        result
    })();

    let result = if presentation.cancellation_requested()?
        && !matches!(&result, Err(RuntimeError::DiskMonitor { .. }))
    {
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
    resources: &ResourceCoordinator,
    reused: &ReusedPhases,
) -> Result<Vec<ReplicateRunSummary>, RuntimeError> {
    let mut summaries = Vec::with_capacity(scopes.len());
    let mut first_error = None;
    for (index, scope) in scopes {
        let cancellation = AtomicBool::new(false);
        match run_replicate(
            study,
            index,
            scope,
            &ReplicateContext {
                presentation,
                scheduler_cancellation: &cancellation,
                resources,
                reused,
            },
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
    resources: &ResourceCoordinator,
    reused: &ReusedPhases,
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
        let reused = ReusedPhases::clone(reused);
        let presentation = presentation.clone();
        let outcomes = outcomes.clone();
        let worker_cancellation = Arc::clone(&cancellation);
        let resources = resources.clone();
        let worker = match thread::Builder::new()
            .name(format!("workflow-replicate-{index}"))
            .spawn(move || {
                let outcome = catch_unwind(AssertUnwindSafe(|| {
                    run_replicate(
                        &study,
                        index,
                        scope,
                        &ReplicateContext {
                            presentation: &presentation,
                            scheduler_cancellation: &worker_cancellation,
                            resources: &resources,
                            reused: &reused,
                        },
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

struct ReplicateContext<'a> {
    presentation: &'a RuntimePresentation,
    scheduler_cancellation: &'a AtomicBool,
    resources: &'a ResourceCoordinator,
    reused: &'a ReusedPhases,
}

fn run_replicate(
    study: &Study,
    index: u64,
    scope: PathBuf,
    context: &ReplicateContext<'_>,
) -> Result<ReplicateRunSummary, RuntimeError> {
    let presentation = context.presentation;
    presentation.publish(RuntimeEvent::ReplicateStarted { index })?;
    let result = run_replicate_inner(study, index, scope, context);
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
    context: &ReplicateContext<'_>,
) -> Result<ReplicateRunSummary, RuntimeError> {
    let ReplicateContext {
        presentation,
        scheduler_cancellation,
        resources,
        reused,
    } = *context;
    let mut phases = Vec::with_capacity(study.phase_order().len());
    for (phase_index, &position) in study.phase_order().iter().enumerate() {
        if scheduler_cancellation.load(Ordering::Acquire) {
            return Err(RuntimeError::ExecutionCancelled);
        }
        let phase = &study.phases()[position];
        if !study.phase_is_active(phase_index) {
            if let Some(summary) = reused.get(&(index, phase_index)) {
                reuse::persist_phase(phase, summary, &scope)?;
                phases.push(summary.clone());
            }
            continue;
        }
        let context = PhaseRuntime {
            study,
            replicate_directory: &scope,
            completed_phases: &phases,
            replicate: index,
            presentation,
            scheduler_cancellation,
            resources,
        };
        phases.push(run_phase(phase, &context)?);
    }
    Ok(ReplicateRunSummary {
        index,
        output_directory: scope,
        phases: phases.into_boxed_slice(),
    })
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

pub(super) fn dependency_workload(kind: &TaskRunKind) -> serde_json::Value {
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
            reused: false,
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
            reused: false,
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
