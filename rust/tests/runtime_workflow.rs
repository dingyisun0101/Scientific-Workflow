//! Integrated coverage for configuration-backed phase scheduling and display.

use std::io::Write;
use std::num::NonZeroU64;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use scientific_workflow::prelude::basics::*;
use scientific_workflow::prelude::runtime::*;

static RUNTIME_TEST: Mutex<()> = Mutex::new(());
static EXECUTION_RECORD_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

fn execution_record_path() -> PathBuf {
    let sequence = EXECUTION_RECORD_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "scientific-workflow-runtime-record-{}-{sequence}.json",
        std::process::id()
    ))
}

fn fixture_project(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/configuration")
        .join(name)
}

fn project() -> ScientificProject {
    ScientificProject::load(fixture_project("cartesian_project")).unwrap()
}

fn config(project: &ScientificProject, ordinal: usize) -> TaskConfig {
    project.task_configs().nth(ordinal).unwrap()
}

fn activity_phase<W>(project: &ScientificProject, id: u64, kind: &str, workload: W) -> Phase
where
    W: FnOnce(&TaskContext) -> TaskResult + Send + 'static,
{
    Phase::builder(id, format!("phase-{id}"))
        .activity_workload(config(project, 0), kind, workload)
        .build()
        .unwrap()
}

#[test]
fn plan_validation_rejects_invalid_hierarchies_and_dependencies() {
    let _guard = RUNTIME_TEST.lock().unwrap();
    let project = project();
    assert!(matches!(
        WorkflowRuntime::builder(execution_record_path())
            .hidden()
            .build(),
        Err(RuntimeError::EmptyPhaseSet)
    ));
    assert!(matches!(
        Phase::builder(1, "empty").build(),
        Err(RuntimeError::EmptyPhase { phase: 1 })
    ));
    assert!(!activity_phase(&project, 1, "default", |_| Ok(())).requires_confirmation());
    let default_timing = activity_phase(&project, 2, "default-timing", |_| Ok(()));
    assert_eq!(default_timing.delay_per_task(), None);
    assert_eq!(default_timing.task_timeout(), None);
    assert_eq!(default_timing.deadline_after(), None);
    for (setting, builder) in [
        (
            "delay_per_task",
            Phase::builder(3, "invalid delay")
                .activity_workload(config(&project, 0), "delay", |_| Ok(()))
                .delay_per_task(Duration::ZERO),
        ),
        (
            "task_timeout",
            Phase::builder(4, "invalid timeout")
                .activity_workload(config(&project, 0), "timeout", |_| Ok(()))
                .task_timeout(Duration::ZERO),
        ),
        (
            "deadline_after",
            Phase::builder(5, "invalid deadline")
                .activity_workload(config(&project, 0), "deadline", |_| Ok(()))
                .deadline_after(Duration::ZERO),
        ),
    ] {
        assert!(matches!(
            builder.build(),
            Err(RuntimeError::InvalidPhaseTiming { setting: actual, .. }) if actual == setting
        ));
    }
    let duplicate_a = activity_phase(&project, 1, "one", |_| Ok(()));
    let duplicate_b = activity_phase(&project, 1, "two", |_| Ok(()));
    assert!(matches!(
        WorkflowRuntime::builder(execution_record_path())
            .phases([duplicate_a, duplicate_b])
            .hidden()
            .build(),
        Err(RuntimeError::DuplicatePhaseId { phase: 1 })
    ));

    let unknown = Phase::builder(2, "unknown")
        .depends_on(99)
        .activity_workload(config(&project, 0), "unknown", |_| Ok(()))
        .build()
        .unwrap();
    assert!(matches!(
        WorkflowRuntime::builder(execution_record_path())
            .phase(unknown)
            .hidden()
            .build(),
        Err(RuntimeError::UnknownPhaseDependency {
            phase: 2,
            dependency: 99
        })
    ));

    let first = Phase::builder(1, "first")
        .depends_on(2)
        .activity_workload(config(&project, 0), "first", |_| Ok(()))
        .build()
        .unwrap();
    let second = Phase::builder(2, "second")
        .depends_on(1)
        .activity_workload(config(&project, 0), "second", |_| Ok(()))
        .build()
        .unwrap();
    assert!(matches!(
        WorkflowRuntime::builder(execution_record_path())
            .phases([first, second])
            .hidden()
            .build(),
        Err(RuntimeError::PhaseDependencyCycle { .. })
    ));
}

#[test]
fn execution_plan_export_is_complete_deterministic_and_never_executes() {
    let _guard = RUNTIME_TEST.lock().unwrap();
    let project = project();
    let executed = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&executed);
    let first = Phase::builder(1, "prepare")
        .activity_workload(config(&project, 0), "prepare", move |_| {
            observed.fetch_add(1, Ordering::Relaxed);
            Ok(())
        })
        .max_active_tasks(2)
        .prepared_task_queue_capacity(3)
        .delay_per_task(Duration::from_millis(5))
        .build()
        .unwrap();
    let second = Phase::builder(2, "simulate")
        .depends_on(1)
        .reused_activity(config(&project, 1), "simulate")
        .build()
        .unwrap();
    let runtime = WorkflowRuntime::builder(execution_record_path())
        .phases([first, second])
        .hidden()
        .build()
        .unwrap();
    let path = std::env::temp_dir().join(format!(
        "scientific-workflow-plan-{}.json",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    runtime.write_execution_plan_json(&path).unwrap();
    let first_bytes = std::fs::read(&path).unwrap();
    runtime.write_execution_plan_json(&path).unwrap();
    assert_eq!(first_bytes, std::fs::read(&path).unwrap());
    assert_eq!(executed.load(Ordering::Relaxed), 0);
    let plan: serde_json::Value = serde_json::from_slice(&first_bytes).unwrap();
    assert_eq!(plan["format"], "scientific-workflow.execution-plan.v1");
    assert_eq!(plan["phases"][0]["max_active_tasks"], 2);
    assert_eq!(plan["phases"][0]["prepared_task_queue_capacity"], 3);
    assert_eq!(plan["phases"][0]["tasks"][0]["status"], "pending");
    assert_eq!(plan["phases"][1]["tasks"][0]["status"], "reused");
    assert!(plan["phases"][0]["tasks"][0]["configuration"]["parameters"].is_object());
    std::fs::write(&path, b"{}\n").unwrap();
    assert!(matches!(
        runtime.write_execution_plan_json(&path),
        Err(RuntimeError::ExecutionPlanConflict { .. })
    ));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn execution_record_is_always_written_for_success_and_failure() {
    let _guard = RUNTIME_TEST.lock().unwrap();
    let project = project();

    let success_path = execution_record_path();
    let success = WorkflowRuntime::builder(&success_path)
        .phase(activity_phase(&project, 1, "success", |_| Ok(())))
        .hidden()
        .build()
        .unwrap()
        .run_phases([1])
        .unwrap();
    let success_record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&success_path).unwrap()).unwrap();
    assert_eq!(
        success_record["format"],
        "scientific-workflow.execution-record.v1"
    );
    assert_eq!(success_record["status"], "completed");
    assert_eq!(success_record["phase_count"], 1);
    assert_eq!(success_record["task_count"], 1);
    assert_eq!(success_record["phases"][0]["status"], "completed");
    assert_eq!(
        success_record["phases"][0]["tasks"][0]["status"],
        "completed"
    );
    assert!(success_record["duration_ns"].is_u64());
    assert!(success_record["phases"][0]["tasks"][0]["duration_ns"].is_u64());
    assert_eq!(
        serde_json::to_value(success.execution_record()).unwrap(),
        success_record
    );

    let failure_path = execution_record_path();
    let failure = WorkflowRuntime::builder(&failure_path)
        .phase(activity_phase(&project, 2, "failure", |_| {
            Err(std::io::Error::other("expected failure").into())
        }))
        .hidden()
        .build()
        .unwrap()
        .run_phases([2]);
    assert!(failure.is_err());
    let failure_record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&failure_path).unwrap()).unwrap();
    assert_eq!(failure_record["status"], "failed");
    assert_eq!(failure_record["phases"][0]["status"], "failed");
    assert_eq!(failure_record["phases"][0]["tasks"][0]["status"], "failed");
}

#[test]
fn optional_phase_timing_staggers_starts_and_expires_cooperatively() {
    let _guard = RUNTIME_TEST.lock().unwrap();
    let project = project();

    let starts = Arc::new(Mutex::new(Vec::new()));
    let observed_starts = Arc::clone(&starts);
    let delayed = Phase::builder(20, "delayed starts")
        .activity_workloads_from_project(&project, "delayed", move |task| {
            let ordinal = task.task_ordinal();
            let starts = Arc::clone(&observed_starts);
            move |_| {
                starts.lock().unwrap().push((ordinal, Instant::now()));
                Ok(())
            }
        })
        .max_active_tasks(3)
        .prepared_task_queue_capacity(3)
        .delay_per_task(Duration::from_millis(20))
        .build()
        .unwrap();
    assert_eq!(delayed.delay_per_task(), Some(Duration::from_millis(20)));
    WorkflowRuntime::builder(execution_record_path())
        .phase(delayed)
        .hidden()
        .build()
        .unwrap()
        .run_phases([20])
        .unwrap();
    let starts = starts.lock().unwrap();
    assert_eq!(
        starts
            .iter()
            .map(|(ordinal, _)| *ordinal)
            .collect::<Vec<_>>(),
        [0, 1, 2, 3, 4, 5]
    );
    assert!(
        starts
            .windows(2)
            .all(|pair| pair[1].1.duration_since(pair[0].1) >= Duration::from_millis(15))
    );
    drop(starts);

    let timeout_observed = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&timeout_observed);
    let timed = Phase::builder(21, "task timeout")
        .activity_workload(config(&project, 0), "timed", move |context| {
            while !context.is_cancelled() {
                thread::yield_now();
            }
            observed.store(true, Ordering::Release);
            Err(std::io::Error::other("cooperative timeout").into())
        })
        .task_timeout(Duration::from_millis(20))
        .build()
        .unwrap();
    assert_eq!(timed.task_timeout(), Some(Duration::from_millis(20)));
    let timeout = WorkflowRuntime::builder(execution_record_path())
        .phase(timed)
        .hidden()
        .build()
        .unwrap()
        .run_phases([21])
        .unwrap_err();
    assert!(matches!(
        timeout.execution_cause(),
        Some(RuntimeError::TaskTimedOut { timeout, .. })
            if *timeout == Duration::from_millis(20)
    ));
    assert!(timeout_observed.load(Ordering::Acquire));

    let deadline_observed = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&deadline_observed);
    let deadline = Phase::builder(22, "phase deadline")
        .activity_workload(config(&project, 0), "deadline", move |context| {
            while !context.is_cancelled() {
                thread::yield_now();
            }
            observed.store(true, Ordering::Release);
            Err(std::io::Error::other("cooperative deadline").into())
        })
        .deadline_after(Duration::from_millis(20))
        .build()
        .unwrap();
    assert_eq!(deadline.deadline_after(), Some(Duration::from_millis(20)));
    let deadline = WorkflowRuntime::builder(execution_record_path())
        .phase(deadline)
        .hidden()
        .build()
        .unwrap()
        .run_phases([22])
        .unwrap_err();
    assert!(matches!(
        deadline.execution_cause(),
        Some(RuntimeError::PhaseDeadlineExceeded {
            phase: 22,
            deadline_after,
        }) if *deadline_after == Duration::from_millis(20)
    ));
    assert!(deadline_observed.load(Ordering::Acquire));
}

#[test]
fn exact_and_dependency_inclusive_selection_are_deterministic() {
    let _guard = RUNTIME_TEST.lock().unwrap();
    let project = project();
    let build = |order: Arc<Mutex<Vec<u64>>>| {
        [1_u64, 2, 4].map(|id| {
            let observed = Arc::clone(&order);
            let mut builder = Phase::builder(id, format!("phase-{id}")).activity_workload(
                config(&project, 0),
                format!("task-{id}"),
                move |_| {
                    observed.lock().unwrap().push(id);
                    Ok(())
                },
            );
            if id == 2 {
                builder = builder.depends_on(1);
            } else if id == 4 {
                builder = builder.depends_on(2);
            }
            builder.build().unwrap()
        })
    };
    assert!(matches!(
        WorkflowRuntime::builder(execution_record_path())
            .phases(build(Arc::new(Mutex::new(Vec::new()))))
            .hidden()
            .build()
            .unwrap()
            .run_phases_exact([4]),
        Err(RuntimeError::UnsatisfiedPhaseDependency {
            phase: 4,
            dependency: 2
        })
    ));

    let order = Arc::new(Mutex::new(Vec::new()));
    WorkflowRuntime::builder(execution_record_path())
        .phases(build(Arc::clone(&order)))
        .hidden()
        .build()
        .unwrap()
        .run_phases_with_dependencies([4])
        .unwrap();
    assert_eq!(*order.lock().unwrap(), [1, 2, 4]);

    let verified = activity_phase(&project, 2, "verified", |_| Ok(()));
    let selected = Phase::builder(4, "selected")
        .depends_on(2)
        .activity_workload(config(&project, 0), "selected", |_| Ok(()))
        .build()
        .unwrap();
    assert!(
        WorkflowRuntime::builder(execution_record_path())
            .phases([verified, selected])
            .satisfied_phase_verifier(|id| id == PhaseId::new(2))
            .hidden()
            .build()
            .unwrap()
            .run_phases_exact([4])
            .unwrap()
            .is_success()
    );
}

#[test]
fn fn_once_scheduler_bounds_work_and_supports_reuse() {
    let _guard = RUNTIME_TEST.lock().unwrap();
    let project = project();
    let active = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let completed = Arc::new(AtomicUsize::new(0));
    let active_factory = Arc::clone(&active);
    let maximum_factory = Arc::clone(&maximum);
    let completed_factory = Arc::clone(&completed);
    let simulation = Phase::builder(2, "simulation")
        .progress_workloads_from_project(&project, "simulation", move |task_config| {
            struct NonCloneResource(u64);
            let resource = NonCloneResource(task_config.task_ordinal());
            let active = Arc::clone(&active_factory);
            let maximum = Arc::clone(&maximum_factory);
            let completed = Arc::clone(&completed_factory);
            move |context: &TaskContext| {
                assert_eq!(context.configuration().task_ordinal(), resource.0);
                assert!(context.value("/temperature").is_some());
                context.set_target_iteration(2)?;
                let current = active.fetch_add(1, Ordering::AcqRel) + 1;
                maximum.fetch_max(current, Ordering::AcqRel);
                thread::sleep(Duration::from_millis(5));
                context.set_iteration(2)?;
                assert!(!context.should_continue(2)?);
                active.fetch_sub(1, Ordering::AcqRel);
                completed.fetch_add(1, Ordering::AcqRel);
                Ok(())
            }
        })
        .reused_activity(config(&project, 0), "cache")
        .max_active_tasks(2)
        .prepared_task_queue_capacity(1)
        .build()
        .unwrap();
    assert_eq!(simulation.tasks().len(), 7);
    let summary = WorkflowRuntime::builder(execution_record_path())
        .phase(simulation)
        .hidden()
        .build()
        .unwrap()
        .run_phases([2])
        .unwrap();
    assert!(summary.is_success());
    assert_eq!(completed.load(Ordering::Acquire), 6);
    assert!(maximum.load(Ordering::Acquire) <= 2);
    assert_eq!(summary.total_tasks(), 7);
    assert_eq!(summary.phases()[0].progress().reused(), 1);
}

#[test]
fn failure_policy_controls_active_work_and_stops_admission() {
    let _guard = RUNTIME_TEST.lock().unwrap();
    let project = project();
    let run = |policy| {
        let active = Arc::new(AtomicUsize::new(0));
        let cancelled = Arc::new(AtomicBool::new(false));
        let finished_active = Arc::new(AtomicBool::new(false));
        let active_factory = Arc::clone(&active);
        let cancelled_factory = Arc::clone(&cancelled);
        let finished_factory = Arc::clone(&finished_active);
        let phase = Phase::builder(3, "failure policy")
            .activity_workloads_from_project(&project, "policy", move |task_config| {
                let ordinal = task_config.task_ordinal();
                let active = Arc::clone(&active_factory);
                let cancelled = Arc::clone(&cancelled_factory);
                let finished = Arc::clone(&finished_factory);
                move |context| {
                    active.fetch_add(1, Ordering::AcqRel);
                    if ordinal == 0 {
                        while active.load(Ordering::Acquire) < 2 {
                            thread::yield_now();
                        }
                        Err(std::io::Error::other("first failure").into())
                    } else {
                        for _ in 0..50_000 {
                            if context.is_cancelled() {
                                cancelled.store(true, Ordering::Release);
                                return Err(std::io::Error::other("cancelled").into());
                            }
                            thread::yield_now();
                        }
                        finished.store(true, Ordering::Release);
                        Ok(())
                    }
                }
            })
            .failure_policy(policy)
            .max_active_tasks(2)
            .prepared_task_queue_capacity(1)
            .build()
            .unwrap();
        let result = WorkflowRuntime::builder(execution_record_path())
            .phase(phase)
            .hidden()
            .build()
            .unwrap()
            .run_phases([3]);
        (result, active, cancelled, finished_active)
    };

    let (result, active, cancelled, _) = run(PhaseFailurePolicy::FailFast);
    let error = result.unwrap_err();
    assert!(matches!(
        error.execution_cause(),
        Some(RuntimeError::TaskWorkload { .. })
    ));
    let progress = error.runtime_summary().unwrap().phases()[0].progress();
    assert_eq!(progress.failed(), 1);
    assert!(progress.cancelled() >= 1);
    assert!(progress.skipped() >= 1);
    assert!(active.load(Ordering::Acquire) <= 2);
    assert!(cancelled.load(Ordering::Acquire));

    let (result, active, cancelled, finished) = run(PhaseFailurePolicy::FinishActive);
    let error = result.unwrap_err();
    assert!(matches!(
        error.execution_cause(),
        Some(RuntimeError::TaskWorkload { .. })
    ));
    let progress = error.runtime_summary().unwrap().phases()[0].progress();
    assert_eq!(progress.failed(), 1);
    assert_eq!(progress.cancelled(), 0);
    assert!(progress.skipped() >= 1);
    assert!(active.load(Ordering::Acquire) <= 2);
    assert!(!cancelled.load(Ordering::Acquire));
    assert!(finished.load(Ordering::Acquire));
}

#[test]
fn one_active_runtime_is_enforced_and_released() {
    let _guard = RUNTIME_TEST.lock().unwrap();
    let project = project();
    let barrier = Arc::new(Barrier::new(2));
    let (release_sender, release_receiver) = mpsc::channel();
    let worker_barrier = Arc::clone(&barrier);
    let blocking = activity_phase(&project, 1, "blocking", move |_| {
        worker_barrier.wait();
        release_receiver.recv().unwrap();
        Ok(())
    });
    let runner = thread::spawn(move || {
        WorkflowRuntime::builder(execution_record_path())
            .phase(blocking)
            .hidden()
            .build()
            .unwrap()
            .run_phases([1])
    });
    barrier.wait();
    assert!(matches!(
        WorkflowRuntime::builder(execution_record_path())
            .phase(activity_phase(&project, 9, "second", |_| Ok(())))
            .hidden()
            .build()
            .unwrap()
            .run_phases([9]),
        Err(RuntimeError::TerminalAlreadyOwned)
    ));
    release_sender.send(()).unwrap();
    assert!(runner.join().unwrap().unwrap().is_success());

    let failing = activity_phase(&project, 2, "failure", |_| {
        Err(std::io::Error::other("intentional failure").into())
    });
    assert!(matches!(
        WorkflowRuntime::builder(execution_record_path())
            .phase(failing)
            .hidden()
            .build()
            .unwrap()
            .run_phases([2]),
        Err(RuntimeError::PhaseExecutionFailed { .. })
    ));
    assert!(
        WorkflowRuntime::builder(execution_record_path())
            .phase(activity_phase(&project, 3, "after", |_| Ok(())))
            .hidden()
            .build()
            .unwrap()
            .run_phases([3])
            .unwrap()
            .is_success()
    );
}

#[test]
fn failure_blocks_dependents_and_task_owned_io_is_preserved() {
    let _guard = RUNTIME_TEST.lock().unwrap();
    let project = project();
    let dependent_ran = Arc::new(AtomicUsize::new(0));
    let dependent_counter = Arc::clone(&dependent_ran);
    let first = activity_phase(&project, 1, "failure", |_| {
        Err(std::io::Error::other("stop phase").into())
    });
    let second = Phase::builder(2, "dependent")
        .depends_on(1)
        .activity_workload(config(&project, 0), "dependent", move |_| {
            dependent_counter.fetch_add(1, Ordering::AcqRel);
            Ok(())
        })
        .build()
        .unwrap();
    assert!(
        WorkflowRuntime::builder(execution_record_path())
            .phases([first, second])
            .hidden()
            .build()
            .unwrap()
            .run_phases([1, 2])
            .is_err()
    );
    assert_eq!(dependent_ran.load(Ordering::Acquire), 0);

    let path = std::env::temp_dir().join(format!(
        "scientific-workflow-task-owned-io-{}",
        std::process::id()
    ));
    let owned_path = path.clone();
    let io_phase = activity_phase(&project, 4, "io", move |_| {
        std::fs::write(&owned_path, b"task-owned")?;
        Ok(())
    });
    WorkflowRuntime::builder(execution_record_path())
        .phase(io_phase)
        .hidden()
        .build()
        .unwrap()
        .run_phases([4])
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"task-owned");
    std::fs::remove_file(path).unwrap();
}

#[test]
fn structured_selectors_use_complete_configuration_identity() {
    let _guard = RUNTIME_TEST.lock().unwrap();
    let project = project();
    let phase = Phase::builder(2, "simulation")
        .activity_tasks_from_project(&project, "simulation", |context| {
            assert!(matches!(
                context.set_iteration(1),
                Err(RuntimeError::ManagedTaskKindMismatch { .. })
            ));
            Ok(())
        })
        .display_tasks_by("simulation", ["/temperature", "/seed"])
        .build()
        .unwrap();
    let runtime = WorkflowRuntime::builder(execution_record_path())
        .phase(phase)
        .hidden()
        .build()
        .unwrap();
    let selected = runtime
        .unique_task_matching(
            &TaskSelector::new()
                .parameter("/temperature", 280.0)
                .parameter("/seed", 7),
        )
        .unwrap();
    assert_eq!(selected.configuration_ordinal(), 0);
    assert!(runtime.run_phases([2]).unwrap().is_success());
}

#[test]
fn message_bursts_remain_bounded_and_responsive() {
    let _guard = RUNTIME_TEST.lock().unwrap();
    let project = project();
    let completed = Arc::new(AtomicUsize::new(0));
    let factory_completed = Arc::clone(&completed);
    let phase = Phase::builder(7, "messages")
        .activity_workloads_from_project(&project, "many", move |task_config| {
            let ordinal = task_config.task_ordinal();
            let completed = Arc::clone(&factory_completed);
            move |context| {
                if ordinal == 0 {
                    for message in 0..512 {
                        context.report(format!("burst-{message}"))?;
                    }
                }
                completed.fetch_add(1, Ordering::AcqRel);
                Ok(())
            }
        })
        .max_active_tasks(2)
        .prepared_task_queue_capacity(1)
        .build()
        .unwrap();
    let summary = WorkflowRuntime::builder(execution_record_path())
        .phase(phase)
        .hidden()
        .build()
        .unwrap()
        .run_phases([7])
        .unwrap();
    assert!(summary.is_success());
    assert_eq!(completed.load(Ordering::Acquire), 6);
}

#[test]
fn cancellation_does_not_publish_task_owned_recording_failure() {
    let _guard = RUNTIME_TEST.lock().unwrap();
    let project = project();
    let root = std::env::temp_dir().join(format!(
        "scientific-workflow-runtime-cancel-{}",
        std::process::id()
    ));
    let recording = root.join("recording");
    std::fs::create_dir(&root).unwrap();
    let schema = SystemStateSchema::load_json_template(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/state.json"),
    )
    .unwrap();
    let (started_sender, started_receiver) = mpsc::channel();
    let task_recording = recording.clone();
    let phase = activity_phase(&project, 8, "recording", move |context| {
        let state = schema.create_empty_state(SimulationTime::from_iteration(0));
        let _writer = SystemStateWriter::builder(&task_recording, &state)
            .with_shared_stream_storage(StateStreamStorage::chunked(
                NonZeroU64::new(1_000_000).unwrap(),
                NonZeroU64::new(1_000_000).unwrap(),
            ))
            .add_state_stream(StateStreamConfig::new(
                "population",
                ["population"],
                SamplingInterval::iterations(1).unwrap(),
                None,
            ))
            .create_new_recording()?;
        started_sender.send(()).unwrap();
        while !context.is_cancelled() {
            thread::yield_now();
        }
        Err(std::io::Error::other("cancelled by runtime").into())
    });
    let runtime = WorkflowRuntime::builder(execution_record_path())
        .phase(phase)
        .hidden()
        .build()
        .unwrap();
    let cancellation = runtime.cancellation_token();
    let runner = thread::spawn(move || runtime.run_phases([8]));
    started_receiver.recv().unwrap();
    cancellation.cancel();
    assert!(matches!(
        runner.join().unwrap(),
        Err(RuntimeError::PhaseExecutionFailed { .. })
    ));
    let metadata: serde_json::Value =
        serde_json::from_slice(&std::fs::read(recording.join("metadata.json")).unwrap()).unwrap();
    assert_eq!(metadata["status"]["state"], "running");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn plain_and_hidden_public_output_modes_are_stable() {
    const CHILD_MODE: &str = "SCIENTIFIC_WORKFLOW_RUNTIME_OUTPUT_CHILD";
    if let Ok(mode) = std::env::var(CHILD_MODE) {
        let project = project();
        let confirm = mode.starts_with("confirm");
        let first = Phase::builder(2, "first")
            .activity_workload(config(&project, 0), "message", |context| {
                context.report("phase-local-message")?;
                Ok(())
            })
            .require_confirm(confirm)
            .build()
            .unwrap();
        let second = Phase::builder(4, "second")
            .depends_on(2)
            .activity_workload(config(&project, 0), "complete", |_| Ok(()))
            .require_confirm(confirm)
            .build()
            .unwrap();
        let builder = WorkflowRuntime::builder(execution_record_path()).phases([first, second]);
        let runtime = match mode.as_str() {
            "plain" | "confirm-yes" | "confirm-eof" => builder.plain().build().unwrap(),
            "terminal" => builder.terminal().build().unwrap(),
            _ => builder.hidden().build().unwrap(),
        };
        let result = runtime.run_phases([2, 4]);
        if mode == "confirm-eof" {
            let error = result.unwrap_err();
            assert!(matches!(
                error.execution_cause(),
                Some(RuntimeError::PhaseConfirmationEof { phase: 2 })
            ));
        } else {
            assert!(result.unwrap().is_success());
        }
        return;
    }

    let executable = std::env::current_exe().unwrap();
    let run_child = |mode: &str| {
        Command::new(&executable)
            .args([
                "--exact",
                "plain_and_hidden_public_output_modes_are_stable",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_MODE, mode)
            .output()
            .unwrap()
    };
    let plain = run_child("plain");
    assert!(plain.status.success());
    let stderr = String::from_utf8(plain.stderr).unwrap();
    assert!(stderr.contains("[phase-start] position=1/2 phase=2"));
    assert!(stderr.contains("require_confirm=false"));
    assert!(stderr.contains("[task] identity=2/message:0 status=completed"));
    assert!(stderr.contains("[phase-complete] phase=4"));
    assert!(stderr.contains("[runtime] status=completed phases=2 tasks=2"));
    assert!(!stderr.contains('\u{1b}'));

    let hidden = run_child("hidden");
    assert!(hidden.status.success());
    let stderr = String::from_utf8(hidden.stderr).unwrap();
    assert!(!stderr.contains("[phase-start]"));
    assert!(!stderr.contains("[task]"));
    assert!(!stderr.contains("[runtime]"));

    let mut confirmed = Command::new(&executable)
        .args([
            "--exact",
            "plain_and_hidden_public_output_modes_are_stable",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_MODE, "confirm-yes")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    confirmed
        .stdin
        .take()
        .unwrap()
        .write_all(b"no\nYeS\n")
        .unwrap();
    let confirmed = confirmed.wait_with_output().unwrap();
    assert!(confirmed.status.success());
    let stderr = String::from_utf8(confirmed.stderr).unwrap();
    assert_eq!(stderr.matches("type yes to continue").count(), 2);
    assert!(stderr.contains("confirmation not accepted"));
    assert!(stderr.contains("[phase-start] position=2/2 phase=4"));

    let eof = run_child("confirm-eof");
    assert!(eof.status.success());
    let stderr = String::from_utf8(eof.stderr).unwrap();
    assert!(stderr.contains("type yes to continue"));
    assert!(!stderr.contains("[phase-start] position=2/2 phase=4"));
}
