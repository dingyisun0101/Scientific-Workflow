//! Integrated coverage for parameter-identified parallel progress reporting.
//!
//! Run with:
//!
//! ```text
//! cargo test --test reporting_workflow -- --nocapture
//! ```

use std::path::PathBuf;
use std::thread;

use scientific_workflow::prelude::*;

fn fixture_project(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/configuration")
        .join(name)
}

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn reporter_identifies_parallel_tasks_and_owns_their_lifecycle() {
    assert_send_sync::<ProgressReporter>();
    assert_send_sync::<TaskProgress>();
    assert_send_sync::<TaskIdentity>();
    assert_send_sync::<ProgressSummary>();

    let project = ScientificProject::load(fixture_project("cartesian_project")).unwrap();

    assert!(matches!(
        ProgressReporter::for_project(&project)
            .identify_tasks_by(["temperature"])
            .hidden()
            .start(),
        Err(ReportingError::NonUniqueTaskIdentity {
            first_ordinal: 0,
            second_ordinal: 1,
            ..
        })
    ));
    assert!(matches!(
        ProgressReporter::for_project(&project)
            .identify_tasks_by(["temperature", "temperature"])
            .hidden()
            .start(),
        Err(ReportingError::DuplicateIdentityParameter { key })
            if key == "temperature"
    ));
    assert!(matches!(
        ProgressReporter::for_project(&project)
            .identify_tasks_by(["missing"])
            .hidden()
            .start(),
        Err(ReportingError::UnknownIdentityParameter { key }) if key == "missing"
    ));
    println!(
        "[identity-validation] exact-combination=true duplicate-key=true unknown-key=true ambiguity=true"
    );

    let reporter = ProgressReporter::for_project(&project)
        .hidden()
        .start()
        .unwrap();
    assert!(format!("{reporter:?}").contains("identity_keys"));
    assert!(matches!(
        ProgressReporter::for_project(&project).hidden().start(),
        Err(ReportingError::TerminalAlreadyOwned)
    ));
    reporter.report("parallel task execution started").unwrap();

    thread::scope(|scope| {
        let mut workers = Vec::new();
        for task in project.task_configs() {
            let reporter = &reporter;
            workers.push(scope.spawn(move || {
                let progress = reporter.start_task(&task, 0, Some(4)).unwrap();
                assert_eq!(progress.status(), TaskStatus::Running);
                assert_eq!(progress.identity().len(), 2);
                assert!(!progress.identity().is_empty());
                assert!(progress.identity().value("temperature").is_some());
                assert_eq!(progress.identity().iter().count(), 2);
                assert!(progress.identity().label().contains("temperature="));
                assert_eq!(progress.current_iteration(), 0);
                assert_eq!(progress.target_iteration(), Some(4));
                assert!(format!("{progress:?}").contains("current_iteration"));
                progress.set_phase("evolving");
                for iteration in 1..=4 {
                    progress.set_iteration(iteration).unwrap();
                }
                progress.report("task validation passed").unwrap();
                progress.complete().unwrap();
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }
    });

    let live_summary = reporter.summary();
    assert_eq!(live_summary.total(), 6);
    assert_eq!(live_summary.pending(), 0);
    assert_eq!(live_summary.running(), 0);
    assert_eq!(live_summary.completed(), 6);
    assert_eq!(live_summary.failed(), 0);
    assert!(live_summary.is_success());
    let summary = reporter
        .complete("parallel reporting validation passed")
        .unwrap();
    assert_eq!(summary, live_summary);
    println!(
        "[parallel-progress] tasks={} completed={} atomic_iterations=true terminal_exclusive=true",
        summary.total(),
        summary.completed()
    );

    let reporter = ProgressReporter::for_configuration(project.configuration())
        .identify_tasks_by(["temperature", "seed"])
        .plain()
        .hidden()
        .start()
        .unwrap();
    let task = project.task_config(0).unwrap();
    let progress = reporter.start_task(&task, 2, Some(4)).unwrap();
    assert!(matches!(
        reporter.start_task(&task, 2, Some(4)),
        Err(ReportingError::TaskAlreadyStarted { .. })
    ));
    progress.set_iteration(3).unwrap();
    assert!(matches!(
        progress.set_iteration(2),
        Err(ReportingError::IterationRegressed {
            current: 3,
            attempted: 2,
            ..
        })
    ));
    assert!(matches!(
        progress.set_iteration(5),
        Err(ReportingError::IterationBeyondTarget {
            iteration: 5,
            target: 4,
            ..
        })
    ));
    drop(progress);
    let summary = reporter.fail("intentional lifecycle validation").unwrap();
    assert_eq!(summary.failed(), 1);
    assert_eq!(summary.pending(), 5);
    println!(
        "[failure-lifecycle] duplicate-start=true regression=true target-bound=true drop-fails=true"
    );

    let reporter = ProgressReporter::for_project(&project)
        .hidden()
        .start()
        .unwrap();
    let task = project.task_config(0).unwrap();
    assert!(matches!(
        reporter.start_task(&task, 5, Some(4)),
        Err(ReportingError::InitialIterationBeyondTarget {
            initial: 5,
            target: 4,
            ..
        })
    ));
    let progress = reporter.start_task(&task, 2, Some(4)).unwrap();
    assert!(matches!(
        progress.complete(),
        Err(ReportingError::TargetIterationNotReached {
            current: 2,
            target: 4,
            ..
        })
    ));
    reporter.fail("target validation complete").unwrap();

    let reporter = ProgressReporter::for_project(&project)
        .hidden()
        .start()
        .unwrap();
    let progress = reporter
        .start_task(&project.task_config(0).unwrap(), 0, None)
        .unwrap();
    progress.fail("intentional scientific failure");
    assert_eq!(
        reporter.fail("explicit failure complete").unwrap().failed(),
        1
    );

    let reporter = ProgressReporter::for_project(&project)
        .hidden()
        .start()
        .unwrap();
    assert!(matches!(
        reporter.complete("must reject pending tasks"),
        Err(ReportingError::IncompleteProgress { pending: 6, .. })
    ));
    ProgressReporter::report_error("intentional reporting boundary test");
    println!(
        "[completion-validation] initial-bound=true target-required=true explicit-fail=true incomplete-success-rejected=true"
    );

    let fixed_only_root = std::env::temp_dir().join(format!(
        "scientific-workflow-reporting-fixed-only-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(fixed_only_root.join("config")).unwrap();
    std::fs::write(fixed_only_root.join("config/fixed.json"), b"{}").unwrap();
    std::fs::write(
        fixed_only_root.join("config/sweep.json"),
        br#"{"mode":"cartesian","axes":[]}"#,
    )
    .unwrap();
    std::fs::write(fixed_only_root.join("config/paths.json"), b"{}").unwrap();
    let config = ProjectConfig::load(&fixed_only_root).unwrap();
    let reporter = ProgressReporter::for_configuration(&config)
        .terminal()
        .hidden()
        .start()
        .unwrap();
    let task = config.task_config(0).unwrap();
    let progress = reporter.start_task(&task, 8, None).unwrap();
    assert_eq!(progress.identity().label(), "task");
    assert_eq!(progress.target_iteration(), None);
    progress.complete().unwrap();
    assert!(
        reporter
            .complete("fixed-only task passed")
            .unwrap()
            .is_success()
    );
    std::fs::remove_dir_all(&fixed_only_root).unwrap();
    println!("[result] reporting_workflow=passed");
}
