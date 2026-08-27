use std::fs;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

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

#[test]
fn prelude_study_types_are_reexports_of_the_canonical_module() {
    let task = scientific_workflow::prelude::study::Task::completed("same", "same");
    let _: scientific_workflow::study::Task = task;
    let completion = scientific_workflow::prelude::study::PhaseCompletion::Complete;
    let _: scientific_workflow::study::PhaseCompletion = completion;
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
fn complete_selected_phase_is_reused_without_entering_its_scheduler() {
    let _guard = STUDY_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let path = record_path("phase-reuse");
    let examinations = Arc::new(AtomicU64::new(0));
    let workloads = Arc::new(AtomicU64::new(0));
    let phase = Phase::builder(1, "prepared result")
        .task(Task::one_shot("prepare", "prepare", {
            let workloads = Arc::clone(&workloads);
            move |_| {
                workloads.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
        }))
        .examine_completion({
            let examinations = Arc::clone(&examinations);
            move || {
                examinations.fetch_add(1, Ordering::Relaxed);
                PhaseCompletion::Complete
            }
        })
        .build()
        .unwrap();

    let summary = Study::builder(&path)
        .phase(phase)
        .hidden()
        .build()
        .unwrap()
        .run()
        .unwrap();

    assert!(summary.is_success());
    assert!(summary.phases()[0].was_reused());
    assert_eq!(summary.total_tasks(), 1);
    assert_eq!(examinations.load(Ordering::Relaxed), 1);
    assert_eq!(workloads.load(Ordering::Relaxed), 0);
    let record: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(record["phases"][0]["status"], "completed");
    assert_eq!(record["phases"][0]["disposition"], "reused");
    assert_eq!(record["phases"][0]["tasks"][0]["status"], "reused");
}

#[test]
fn complete_omitted_phase_satisfies_exact_dependency_selection() {
    let _guard = STUDY_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let order = Arc::new(AtomicU64::new(0));
    let dependency = Phase::builder(1, "prepared")
        .task(Task::one_shot("prepare", "prepare", |_| {
            panic!("a completed omitted dependency must not run")
        }))
        .examine_completion(|| PhaseCompletion::Complete)
        .build()
        .unwrap();
    let selected = Phase::builder(2, "analyze")
        .depends_on(1)
        .task(Task::one_shot("analyze", "analyze", {
            let order = Arc::clone(&order);
            move |_| {
                order.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
        }))
        .build()
        .unwrap();

    let summary = Study::builder(record_path("exact-completion"))
        .phases([dependency, selected])
        .hidden()
        .build()
        .unwrap()
        .run_phases([2])
        .unwrap();

    assert_eq!(summary.phases().len(), 1);
    assert_eq!(summary.phases()[0].id(), PhaseId::new(2));
    assert!(!summary.phases()[0].was_reused());
    assert_eq!(order.load(Ordering::Relaxed), 1);
}

#[test]
fn complete_selected_phase_needs_no_dependency_examination() {
    let _guard = STUDY_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let dependency_examinations = Arc::new(AtomicU64::new(0));
    let dependency = Phase::builder(1, "dependency")
        .task(Task::one_shot("dependency", "dependency", |_| Ok(())))
        .examine_completion({
            let examinations = Arc::clone(&dependency_examinations);
            move || {
                examinations.fetch_add(1, Ordering::Relaxed);
                PhaseCompletion::Invalid("must remain irrelevant".to_owned())
            }
        })
        .build()
        .unwrap();
    let selected = Phase::builder(2, "already analyzed")
        .depends_on(1)
        .task(Task::one_shot("analyze", "analyze", |_| {
            panic!("a complete selected phase must not run")
        }))
        .examine_completion(|| PhaseCompletion::Complete)
        .build()
        .unwrap();

    let summary = Study::builder(record_path("complete-selected"))
        .phases([dependency, selected])
        .hidden()
        .build()
        .unwrap()
        .run_phases([2])
        .unwrap();

    assert!(summary.phases()[0].was_reused());
    assert_eq!(dependency_examinations.load(Ordering::Relaxed), 0);
}

#[test]
fn dependency_closure_omits_complete_dependency() {
    let _guard = STUDY_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let dependency = Phase::builder(1, "complete dependency")
        .task(Task::one_shot("dependency", "dependency", |_| {
            panic!("a complete dependency must not enter the closure")
        }))
        .examine_completion(|| PhaseCompletion::Complete)
        .build()
        .unwrap();
    let selected = Phase::builder(2, "selected")
        .depends_on(1)
        .task(Task::one_shot("selected", "selected", |_| Ok(())))
        .build()
        .unwrap();

    let summary = Study::builder(record_path("complete-closure"))
        .phases([dependency, selected])
        .hidden()
        .build()
        .unwrap()
        .run_phases_with_dependencies([2])
        .unwrap();

    assert_eq!(summary.phases().len(), 1);
    assert_eq!(summary.phases()[0].id(), PhaseId::new(2));
}

#[test]
fn dependency_closure_adds_incomplete_phase_and_leaves_resume_to_its_workload() {
    let _guard = STUDY_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let order = Arc::new(AtomicU64::new(0));
    let dependency = Phase::builder(1, "partially prepared")
        .task(Task::one_shot("prepare", "prepare", {
            let order = Arc::clone(&order);
            move |_| {
                assert_eq!(order.fetch_add(1, Ordering::SeqCst), 0);
                Ok(())
            }
        }))
        .examine_completion(|| PhaseCompletion::incomplete("checkpoint available"))
        .build()
        .unwrap();
    let selected = Phase::builder(2, "analyze")
        .depends_on(1)
        .task(Task::one_shot("analyze", "analyze", {
            let order = Arc::clone(&order);
            move |_| {
                assert_eq!(order.fetch_add(1, Ordering::SeqCst), 1);
                Ok(())
            }
        }))
        .build()
        .unwrap();

    let summary = Study::builder(record_path("incomplete-closure"))
        .phases([dependency, selected])
        .hidden()
        .build()
        .unwrap()
        .run_phases_with_dependencies([2])
        .unwrap();

    assert_eq!(summary.phases().len(), 2);
    assert!(summary.phases().iter().all(|phase| !phase.was_reused()));
    assert_eq!(order.load(Ordering::Relaxed), 2);
}

#[test]
fn invalid_completion_fails_before_any_workload_or_record_write() {
    let _guard = STUDY_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let path = record_path("invalid-completion");
    let workloads = Arc::new(AtomicU64::new(0));
    let phase = Phase::builder(7, "invalid")
        .task(Task::one_shot("work", "work", {
            let workloads = Arc::clone(&workloads);
            move |_| {
                workloads.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
        }))
        .examine_completion(|| PhaseCompletion::invalid("configuration fingerprint differs"))
        .build()
        .unwrap();

    let error = Study::builder(&path)
        .phase(phase)
        .hidden()
        .build()
        .unwrap()
        .run()
        .unwrap_err();

    assert!(matches!(
        error,
        StudyError::InvalidPhaseCompletion { phase: 7, .. }
    ));
    assert_eq!(workloads.load(Ordering::Relaxed), 0);
    assert!(!path.exists());
}

#[test]
fn disabling_examination_invokes_a_phase_reported_complete() {
    let _guard = STUDY_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let examinations = Arc::new(AtomicU64::new(0));
    let workloads = Arc::new(AtomicU64::new(0));
    let phase = Phase::builder(1, "forced")
        .task(Task::one_shot("work", "work", {
            let workloads = Arc::clone(&workloads);
            move |_| {
                workloads.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
        }))
        .examine_completion({
            let examinations = Arc::clone(&examinations);
            move || {
                examinations.fetch_add(1, Ordering::Relaxed);
                PhaseCompletion::Complete
            }
        })
        .build()
        .unwrap();

    let summary = Study::builder(record_path("disabled-completion"))
        .phase(phase)
        .without_completion_examination()
        .hidden()
        .build()
        .unwrap()
        .run()
        .unwrap();

    assert!(!summary.phases()[0].was_reused());
    assert_eq!(examinations.load(Ordering::Relaxed), 0);
    assert_eq!(workloads.load(Ordering::Relaxed), 1);
}

#[test]
fn disabling_examination_prevents_omitted_dependency_satisfaction() {
    let dependency = Phase::builder(1, "dependency")
        .task(Task::one_shot("dependency", "dependency", |_| Ok(())))
        .examine_completion(|| PhaseCompletion::Complete)
        .build()
        .unwrap();
    let selected = Phase::builder(2, "selected")
        .depends_on(1)
        .task(Task::one_shot("selected", "selected", |_| Ok(())))
        .build()
        .unwrap();

    assert!(matches!(
        Study::builder(record_path("disabled-dependency"))
            .phases([dependency, selected])
            .without_completion_examination()
            .hidden()
            .build()
            .unwrap()
            .run_phases([2]),
        Err(StudyError::UnsatisfiedPhaseDependency {
            phase: 2,
            dependency: 1
        })
    ));
}

#[test]
fn completion_examiner_panic_is_contained_before_execution() {
    let _guard = STUDY_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let phase = Phase::builder(4, "panic")
        .task(Task::one_shot("work", "work", |_| Ok(())))
        .examine_completion(|| panic!("examiner panic"))
        .build()
        .unwrap();

    assert!(matches!(
        Study::builder(record_path("completion-panic"))
            .phase(phase)
            .hidden()
            .build()
            .unwrap()
            .run(),
        Err(StudyError::PhaseCompletionExaminationPanicked { phase: 4 })
    ));
}

#[test]
fn static_plan_declares_examination_without_calling_the_examiner() {
    let examinations = Arc::new(AtomicU64::new(0));
    let phase = Phase::builder(1, "planned")
        .task(Task::one_shot("work", "work", |_| Ok(())))
        .examine_completion({
            let examinations = Arc::clone(&examinations);
            move || {
                examinations.fetch_add(1, Ordering::Relaxed);
                PhaseCompletion::Complete
            }
        })
        .build()
        .unwrap();
    let study = Study::builder(record_path("completion-plan"))
        .phase(phase)
        .hidden()
        .build()
        .unwrap();

    let plan: serde_json::Value =
        serde_json::from_slice(&study.plan().to_pretty_json().unwrap()).unwrap();
    assert_eq!(plan["format"], "scientific-workflow.study-plan.v2");
    assert_eq!(plan["phases"][0]["examines_completion"], true);
    assert_eq!(examinations.load(Ordering::Relaxed), 0);
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
    assert_eq!(record["format"], "scientific-workflow.study-record.v2");
    assert_eq!(record["phases"][0]["disposition"], "executed");
    assert_eq!(record["phases"][0]["tasks"][0]["category"], "output");
    assert_eq!(record["phases"][0]["tasks"][0]["mode"], "one-shot");
    assert_eq!(
        record["phases"][0]["tasks"][0]["metadata"]["format"],
        "json"
    );
}
