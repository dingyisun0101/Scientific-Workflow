use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use scientific_workflow::configuration::ConfigurationSpace;
use scientific_workflow::prelude::study::*;

static STUDY_LOCK: Mutex<()> = Mutex::new(());

fn record_path(label: &str) -> std::path::PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!(
        "scientific-workflow-study-{}-{label}-{}.json",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn configuration_space() -> (std::path::PathBuf, ConfigurationSpace) {
    let directory = record_path("configuration").with_extension("");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("fixed.json"), r#"{"iterations":3}"#).unwrap();
    fs::write(
        directory.join("sweep.json"),
        r#"{"mode":"cartesian","axes":{"seed":{"values":[7,11,13]}}}"#,
    )
    .unwrap();
    let space = ConfigurationSpace::load(&directory).unwrap();
    (directory, space)
}

#[test]
fn configurations_are_mapped_to_tasks_by_the_application() {
    let _guard = STUDY_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let (_directory, configurations) = configuration_space();
    let completed = Arc::new(AtomicU64::new(0));
    let tasks = configurations
        .combinations()
        .map(|configuration| {
            let ordinal = configuration.ordinal();
            let seed = configuration.decode_value::<u64>("/seed").unwrap();
            let completed = Arc::clone(&completed);
            Task::progress(
                format!("simulation-{ordinal}"),
                format!("simulation seed={seed}"),
                move |context| {
                    context.set_target_iteration(3)?;
                    for iteration in 1..=3 {
                        context.set_iteration(iteration)?;
                        if context.is_cancelled() {
                            return Ok(());
                        }
                    }
                    completed.fetch_add(1, Ordering::Relaxed);
                    Ok(())
                },
            )
            .category("simulation")
            .metadata("configuration_ordinal", ordinal)
            .metadata("seed", seed)
        })
        .collect::<Vec<_>>();

    let phase = Phase::builder(1, "simulations")
        .tasks(tasks)
        .max_active_tasks(2)
        .prepared_task_queue_capacity(2)
        .delay_per_task(Duration::from_millis(1))
        .build()
        .unwrap();
    let study = Study::builder(record_path("mapping"))
        .phase(phase)
        .hidden()
        .build()
        .unwrap();

    assert_eq!(
        study
            .unique_task_matching(
                &TaskSelector::new()
                    .category("simulation")
                    .metadata("seed", 11),
            )
            .unwrap()
            .metadata_value("configuration_ordinal"),
        Some(&serde_json::json!(1))
    );
    let summary = study.run_phases([1]).unwrap();
    assert!(summary.is_success());
    assert_eq!(summary.total_tasks(), 3);
    assert_eq!(completed.load(Ordering::Relaxed), 3);
}

#[test]
fn prelude_study_types_are_reexports_of_the_canonical_module() {
    let task = scientific_workflow::prelude::study::Task::completed("same", "same");
    let _: scientific_workflow::study::Task = task;
}

#[test]
fn phase_owns_scheduling_and_dependencies() {
    let _guard = STUDY_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let order = Arc::new(AtomicU64::new(0));
    let first_order = Arc::clone(&order);
    let first = Phase::builder(1, "prepare")
        .task(Task::one_shot("prepare", "prepare", move |_| {
            assert_eq!(first_order.fetch_add(1, Ordering::SeqCst), 0);
            Ok(())
        }))
        .build()
        .unwrap();
    let second_order = Arc::clone(&order);
    let second = Phase::builder(2, "analyze")
        .depends_on(1)
        .task(Task::one_shot("analyze", "analyze", move |_| {
            assert_eq!(second_order.fetch_add(1, Ordering::SeqCst), 1);
            Ok(())
        }))
        .build()
        .unwrap();

    let summary = Study::builder(record_path("dependencies"))
        .phases([first, second])
        .hidden()
        .build()
        .unwrap()
        .run_phases_with_dependencies([2])
        .unwrap();
    assert!(summary.is_success());
    assert_eq!(summary.phases().len(), 2);
}

#[test]
fn one_shot_and_progress_are_modes_of_the_same_task_type() {
    let _guard = STUDY_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let one_shot = Task::one_shot("prepare", "prepare", |_| Ok(()));
    let progress = Task::progress("simulate", "simulate", |context| {
        context.set_target_iteration(1)?;
        context.set_iteration(1)?;
        Ok(())
    });
    assert_eq!(one_shot.mode(), TaskMode::OneShot);
    assert_eq!(progress.mode(), TaskMode::Progress);

    let phase = Phase::builder(1, "mixed")
        .tasks([one_shot, progress])
        .max_active_tasks(2)
        .build()
        .unwrap();
    let summary = Study::builder(record_path("modes"))
        .phase(phase)
        .hidden()
        .build()
        .unwrap()
        .run_phases([1])
        .unwrap();
    assert_eq!(summary.total_tasks(), 2);
}

#[test]
fn invalid_studies_and_phases_are_rejected() {
    assert!(matches!(
        Study::builder(record_path("empty")).build(),
        Err(StudyError::EmptyPhaseSet)
    ));
    assert!(matches!(
        Phase::builder(1, "empty").build(),
        Err(StudyError::EmptyPhase { phase: 1 })
    ));

    let first = Phase::builder(1, "first")
        .depends_on(2)
        .task(Task::one_shot("first", "first", |_| Ok(())))
        .build()
        .unwrap();
    let second = Phase::builder(2, "second")
        .depends_on(1)
        .task(Task::one_shot("second", "second", |_| Ok(())))
        .build()
        .unwrap();
    assert!(matches!(
        Study::builder(record_path("cycle"))
            .phases([first, second])
            .build(),
        Err(StudyError::PhaseDependencyCycle { .. })
    ));
}

#[test]
fn study_writes_a_durable_record() {
    let _guard = STUDY_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let path = record_path("record");
    let phase = Phase::builder(9, "recorded")
        .task(
            Task::one_shot("write", "write result", |_| Ok(()))
                .category("output")
                .metadata("format", "json"),
        )
        .build()
        .unwrap();
    let summary = Study::builder(&path)
        .phase(phase)
        .hidden()
        .build()
        .unwrap()
        .run_phases([9])
        .unwrap();
    assert!(summary.is_success());
    let record: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(record["phases"][0]["tasks"][0]["category"], "output");
    assert_eq!(record["phases"][0]["tasks"][0]["mode"], "one-shot");
    assert_eq!(
        record["phases"][0]["tasks"][0]["metadata"]["format"],
        "json"
    );
}

#[test]
fn configured_tasks_preserve_complete_plan_and_record_provenance() {
    let _guard = STUDY_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let root = record_path("provenance-root").with_extension("");
    let configuration_directory = root.join("config");
    fs::create_dir_all(&configuration_directory).unwrap();
    fs::write(
        configuration_directory.join("fixed.json"),
        r#"{"solver":{"step":0.25}}"#,
    )
    .unwrap();
    fs::write(
        configuration_directory.join("sweep.json"),
        r#"{"mode":"cartesian","axes":{"seed":{"values":[17]}}}"#,
    )
    .unwrap();
    fs::write(
        configuration_directory.join("paths.json"),
        r#"{"recordings":"results"}"#,
    )
    .unwrap();
    let configurations = ConfigurationSpace::load(&configuration_directory).unwrap();
    let configuration = configurations.combination(0).unwrap();
    let paths = scientific_workflow::configuration::ProjectPaths::load(&root).unwrap();
    let record_path = root.join("study-record.json");
    let phase = Phase::builder(1, "configured")
        .task(
            Task::one_shot_for_configuration("simulation", &configuration, |_| Ok(()))
                .with_project_paths(&paths),
        )
        .build()
        .unwrap();
    let study = Study::builder(&record_path)
        .phase(phase)
        .hidden()
        .build()
        .unwrap();

    let plan: serde_json::Value =
        serde_json::from_slice(&study.plan().to_pretty_json().unwrap()).unwrap();
    let metadata = &plan["phases"][0]["tasks"][0]["metadata"];
    assert_eq!(metadata["configuration_ordinal"], 0);
    assert_eq!(metadata["configuration"]["solver"]["step"], 0.25);
    assert_eq!(metadata["configuration"]["seed"], 17);
    assert_eq!(metadata["project_paths"]["recordings"], "results");

    study.run_phases([1]).unwrap();
    let record: serde_json::Value =
        serde_json::from_slice(&fs::read(&record_path).unwrap()).unwrap();
    assert_eq!(record["phases"][0]["tasks"][0]["metadata"], *metadata);
    fs::remove_dir_all(root).unwrap();
}
