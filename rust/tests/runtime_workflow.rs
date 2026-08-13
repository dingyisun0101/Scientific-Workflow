//! Integrated coverage for phase scheduling and runtime-owned display.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use scientific_workflow::prelude::*;

static RUNTIME_TEST: Mutex<()> = Mutex::new(());

fn fixture_project(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/configuration")
        .join(name)
}

fn activity(id: &str) -> Task {
    Task::activity(id, "activity").workload(|_| Ok(()))
}

fn phase(id: u64, task: Task) -> Phase {
    Phase::builder(id, format!("phase-{id}"))
        .task(task)
        .build()
        .unwrap()
}

#[test]
fn plan_validation_rejects_invalid_hierarchies_and_dependencies() {
    let _guard = RUNTIME_TEST.lock().unwrap();
    assert!(matches!(
        WorkflowRuntime::builder().hidden().build(),
        Err(RuntimeError::EmptyPhaseSet)
    ));
    assert!(matches!(
        Phase::builder(1, "empty").build(),
        Err(RuntimeError::EmptyPhase { phase: 1 })
    ));
    assert!(matches!(
        WorkflowRuntime::builder()
            .phase(phase(1, Task::activity("missing", "activity")))
            .hidden()
            .build(),
        Err(RuntimeError::MissingTaskWorkload { .. })
    ));

    let duplicate = phase(1, activity("one"));
    assert!(matches!(
        WorkflowRuntime::builder()
            .phases([duplicate.clone(), duplicate])
            .hidden()
            .build(),
        Err(RuntimeError::DuplicatePhaseId { phase: 1 })
    ));

    let unknown = Phase::builder(2, "unknown")
        .depends_on(99)
        .task(activity("unknown"))
        .build()
        .unwrap();
    assert!(matches!(
        WorkflowRuntime::builder().phase(unknown).hidden().build(),
        Err(RuntimeError::UnknownPhaseDependency {
            phase: 2,
            dependency: 99
        })
    ));

    let first = Phase::builder(1, "first")
        .depends_on(2)
        .task(activity("first"))
        .build()
        .unwrap();
    let second = Phase::builder(2, "second")
        .depends_on(1)
        .task(activity("second"))
        .build()
        .unwrap();
    assert!(matches!(
        WorkflowRuntime::builder()
            .phases([first, second])
            .hidden()
            .build(),
        Err(RuntimeError::PhaseDependencyCycle { .. })
    ));
}

#[test]
fn exact_and_dependency_inclusive_selection_are_deterministic() {
    let _guard = RUNTIME_TEST.lock().unwrap();
    let order = Arc::new(Mutex::new(Vec::new()));
    let make = |phase_id: u64, dependency: Option<u64>| {
        let observed = Arc::clone(&order);
        let mut builder = Phase::builder(phase_id, format!("phase-{phase_id}")).task(
            Task::activity(format!("task-{phase_id}"), "activity").workload(move |_| {
                observed.lock().unwrap().push(phase_id);
                Ok(())
            }),
        );
        if let Some(dependency) = dependency {
            builder = builder.depends_on(dependency);
        }
        builder.build().unwrap()
    };
    let phases = [make(1, None), make(2, Some(1)), make(4, Some(2))];
    let runtime = WorkflowRuntime::builder()
        .phases(phases.clone())
        .hidden()
        .build()
        .unwrap();
    assert!(matches!(
        runtime.run_phases_exact([4]),
        Err(RuntimeError::UnsatisfiedPhaseDependency {
            phase: 4,
            dependency: 2
        })
    ));

    WorkflowRuntime::builder()
        .phases(phases)
        .hidden()
        .build()
        .unwrap()
        .run_phases_with_dependencies([4])
        .unwrap();
    assert_eq!(*order.lock().unwrap(), [1, 2, 4]);

    let verified = phase(2, activity("verified-dependent"));
    let selected = Phase::builder(4, "selected")
        .depends_on(2)
        .task(activity("selected"))
        .build()
        .unwrap();
    assert!(
        WorkflowRuntime::builder()
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
fn phase_scheduler_bounds_active_work_and_supports_config_tasks() {
    let _guard = RUNTIME_TEST.lock().unwrap();
    let project = ScientificProject::load(fixture_project("cartesian_project")).unwrap();
    let active = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let completed = Arc::new(AtomicUsize::new(0));
    let active_factory = Arc::clone(&active);
    let maximum_factory = Arc::clone(&maximum);
    let completed_factory = Arc::clone(&completed);
    let simulation = Phase::builder(2, "simulation")
        .progress_workloads_from_project(&project, "simulation", move |config| {
            let active = Arc::clone(&active_factory);
            let maximum = Arc::clone(&maximum_factory);
            let completed = Arc::clone(&completed_factory);
            let ordinal = config.task_ordinal();
            move |context: &TaskContext| {
                assert_eq!(context.configuration().unwrap().task_ordinal(), ordinal);
                assert!(context.value("temperature").is_some());
                let progress = context.progress_handle().unwrap();
                progress.set_target_iteration(2)?;
                let current = active.fetch_add(1, Ordering::AcqRel) + 1;
                maximum.fetch_max(current, Ordering::AcqRel);
                thread::sleep(Duration::from_millis(5));
                progress.set_iteration(2)?;
                active.fetch_sub(1, Ordering::AcqRel);
                completed.fetch_add(1, Ordering::AcqRel);
                Ok(())
            }
        })
        .task(Task::activity("cached", "cache").reused())
        .max_concurrent_workloads(2)
        .queue_capacity(1)
        .build()
        .unwrap();
    assert_eq!(simulation.tasks().len(), 7);
    let summary = WorkflowRuntime::builder()
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
}

#[test]
fn one_active_runtime_is_enforced_and_released_after_every_terminal_path() {
    let _guard = RUNTIME_TEST.lock().unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let (release_sender, release_receiver) = mpsc::channel();
    let release_receiver = Arc::new(Mutex::new(release_receiver));
    let worker_barrier = Arc::clone(&barrier);
    let receiver = Arc::clone(&release_receiver);
    let blocking = phase(
        1,
        Task::activity("blocking", "activity").workload(move |_| {
            worker_barrier.wait();
            receiver.lock().unwrap().recv().unwrap();
            Ok(())
        }),
    );
    let runner = thread::spawn(move || {
        WorkflowRuntime::builder()
            .phase(blocking)
            .hidden()
            .build()
            .unwrap()
            .run_phases([1])
    });
    barrier.wait();
    assert!(matches!(
        WorkflowRuntime::builder()
            .phase(phase(9, activity("second")))
            .hidden()
            .build()
            .unwrap()
            .run_phases([9]),
        Err(RuntimeError::TerminalAlreadyOwned)
    ));
    release_sender.send(()).unwrap();
    assert!(runner.join().unwrap().unwrap().is_success());

    let failing = phase(
        2,
        Task::activity("failure", "activity")
            .workload(|_| Err(std::io::Error::other("intentional failure").into())),
    );
    assert!(matches!(
        WorkflowRuntime::builder()
            .phase(failing)
            .hidden()
            .build()
            .unwrap()
            .run_phases([2]),
        Err(RuntimeError::TaskWorkload { .. })
    ));
    assert!(
        WorkflowRuntime::builder()
            .phase(phase(3, activity("after-failure")))
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
    let dependent_ran = Arc::new(AtomicUsize::new(0));
    let dependent_counter = Arc::clone(&dependent_ran);
    let first = phase(
        1,
        Task::activity("failure", "activity")
            .workload(|_| Err(std::io::Error::other("stop phase").into())),
    );
    let second = Phase::builder(2, "dependent")
        .depends_on(1)
        .task(Task::activity("dependent", "activity").workload(move |_| {
            dependent_counter.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }))
        .build()
        .unwrap();
    assert!(
        WorkflowRuntime::builder()
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
    let io_task = Task::activity("io", "activity").workload(move |_| {
        std::fs::write(&owned_path, b"task-owned")?;
        Ok(())
    });
    WorkflowRuntime::builder()
        .phase(phase(4, io_task))
        .hidden()
        .build()
        .unwrap()
        .run_phases([4])
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"task-owned");
    std::fs::remove_file(path).unwrap();
}

#[test]
fn structured_selectors_remain_independent_of_generated_labels() {
    let _guard = RUNTIME_TEST.lock().unwrap();
    let phase = Phase::builder(2, "simulation")
        .task(
            Task::activity("low", "simulation")
                .with_parameter("mu", 0.25)
                .unwrap()
                .workload(|_| Ok(())),
        )
        .task(
            Task::activity("high", "simulation")
                .with_parameter("mu", 0.5)
                .unwrap()
                .workload(|_| Ok(())),
        )
        .display_tasks_by("simulation", ["mu"])
        .build()
        .unwrap();
    let runtime = WorkflowRuntime::builder()
        .phase(phase)
        .hidden()
        .build()
        .unwrap();
    let selected = runtime
        .unique_task_matching(&TaskSelector::new().parameter("mu", 0.25))
        .unwrap();
    assert_eq!(selected.id().as_str(), "low");
    assert!(runtime.run_phases([2]).unwrap().is_success());
}
