//! Private scheduler, deadline, and failure-lifecycle coverage.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::runtime::{RuntimeError, TaskRunKind, TaskRunSummary, execute};
use crate::state::{StateTime, SystemState, SystemStateSchema};
use crate::study::Study;
use crate::task::{ExecutionUnit, InitializationContext, MemberCompletion, MemberView, UnitResult};

use super::execution::task_exceeded_timeout;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct Project(PathBuf);

impl Project {
    fn new(study: serde_json::Value, parameters: serde_json::Value) -> Self {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "scientific-workflow-runtime-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("wf_configs/states")).unwrap();
        fs::write(
            root.join("wf_configs/study.json"),
            serde_json::to_vec_pretty(&study).unwrap(),
        )
        .unwrap();
        fs::write(
            root.join("wf_configs/parameters.json"),
            serde_json::to_vec_pretty(&parameters).unwrap(),
        )
        .unwrap();
        fs::write(
            root.join("wf_configs/states/value.json"),
            r#"{"fields":[{"name":"value"}]}"#,
        )
        .unwrap();
        Self(root)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn execution_directory(&self) -> PathBuf {
        let mut entries = fs::read_dir(self.path().join("output"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        assert_eq!(entries.len(), 1);
        entries.pop().unwrap()
    }

    #[cfg(unix)]
    fn write_executable(&self, name: &str, contents: &str) {
        use std::os::unix::fs::PermissionsExt as _;

        let path = self.path().join(name);
        fs::write(&path, contents).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PanicConstants {}

struct PanicAfterBeginUnit {
    state: SystemState,
}

#[scientific_workflow::execution_unit("runtime-panic-after-begin")]
impl ExecutionUnit for PanicAfterBeginUnit {
    type Constants = PanicConstants;

    fn initialize(
        _constants: Self::Constants,
        schema: &SystemStateSchema,
        _context: &InitializationContext,
    ) -> UnitResult<Self> {
        let mut state = schema.create_empty_state(StateTime::from_iteration(0));
        state.initialize_payload("value", 0_u64)?;
        Ok(Self { state })
    }

    fn member_count(&self) -> usize {
        1
    }

    fn member(&self, index: usize) -> Option<MemberView<'_>> {
        (index == 0).then(|| MemberView::new("panic", &self.state, None, None))
    }

    fn step(&mut self) -> UnitResult {
        panic!("runtime panic sentinel")
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SlowConstants {
    sleep_ms: u64,
}

struct SlowUnit {
    state: SystemState,
    sleep: Duration,
}

#[scientific_workflow::execution_unit("runtime-slow")]
impl ExecutionUnit for SlowUnit {
    type Constants = SlowConstants;

    fn initialize(
        constants: Self::Constants,
        schema: &SystemStateSchema,
        _context: &InitializationContext,
    ) -> UnitResult<Self> {
        let mut state = schema.create_empty_state(StateTime::from_iteration(0));
        state.initialize_payload("value", 0_u64)?;
        Ok(Self {
            state,
            sleep: Duration::from_millis(constants.sleep_ms),
        })
    }

    fn member_count(&self) -> usize {
        1
    }

    fn member(&self, index: usize) -> Option<MemberView<'_>> {
        (index == 0).then(|| {
            MemberView::new(
                "slow",
                &self.state,
                (self.state.time().iteration() == 1).then_some(MemberCompletion::without_reason()),
                Some(1),
            )
        })
    }

    fn step(&mut self) -> UnitResult {
        std::thread::sleep(self.sleep);
        *self.state.payload_mut::<u64>("value")? += 1;
        self.state.advance_time(None)?;
        Ok(())
    }
}

struct RuntimeEnsemble {
    states: Vec<SystemState>,
}

#[scientific_workflow::execution_unit("runtime-ensemble")]
impl ExecutionUnit for RuntimeEnsemble {
    type Constants = PanicConstants;

    fn initialize(
        _constants: Self::Constants,
        schema: &SystemStateSchema,
        context: &InitializationContext,
    ) -> UnitResult<Self> {
        let _ = context.shared_seed("coordination")?;
        let _ = context.member_seed("first", "initialization")?;
        let _ = context.member_seed("second", "initialization")?;
        let mut states = Vec::with_capacity(2);
        for initial in [10_u64, 20] {
            let mut state = schema.create_empty_state(StateTime::from_iteration(0));
            state.initialize_payload("value", initial)?;
            states.push(state);
        }
        Ok(Self { states })
    }

    fn member_count(&self) -> usize {
        self.states.len()
    }

    fn member(&self, index: usize) -> Option<MemberView<'_>> {
        let state = self.states.get(index)?;
        Some(MemberView::new(
            ["first", "second"][index],
            state,
            (state.time().iteration() >= (index as u64 + 1))
                .then_some(MemberCompletion::without_reason()),
            Some(index as u64 + 1),
        ))
    }

    fn step(&mut self) -> UnitResult {
        for (index, state) in self.states.iter_mut().enumerate() {
            if state.time().iteration() < index as u64 + 1 {
                *state.payload_mut::<u64>("value")? += 1;
                state.advance_time(None)?;
            }
        }
        Ok(())
    }
}

fn execution_unit_study(execution_unit: &str, timeout_ms: Option<u64>) -> serde_json::Value {
    let mut task = serde_json::json!({"execution_unit": execution_unit, "state": "value"});
    if let Some(timeout_ms) = timeout_ms {
        task["timeout_ms"] = timeout_ms.into();
    }
    serde_json::json!({
        "paths": {"states": {"value": "wf_configs/states/value.json"}},
        "phases": {"run": {"tasks": [task]}}
    })
}

fn has_task_panic(error: &RuntimeError) -> bool {
    match error {
        RuntimeError::TaskPanicked { .. } => true,
        RuntimeError::Replicate { source, .. } => has_task_panic(source),
        _ => false,
    }
}

fn has_task_timeout(error: &RuntimeError) -> bool {
    match error {
        RuntimeError::TaskTimedOut { .. } => true,
        RuntimeError::Replicate { source, .. } => has_task_timeout(source),
        _ => false,
    }
}

fn has_phase_timeout(error: &RuntimeError) -> bool {
    match error {
        RuntimeError::PhaseTimedOut { .. } => true,
        RuntimeError::Replicate { source, .. } => has_phase_timeout(source),
        _ => false,
    }
}

#[test]
fn runtime_public_results_are_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}

    assert_send_sync::<RuntimeError>();
    assert_send_sync::<TaskRunSummary>();
}

#[test]
fn an_ensemble_task_persists_and_summarizes_each_member_independently() {
    let mut study = execution_unit_study("runtime-ensemble", None);
    study["seed"] = 42.into();
    let project = Project::new(study, serde_json::json!({"runtime-ensemble": {}}));

    let summary = execute(Study::load(project.path()).unwrap()).unwrap();
    let task = &summary.replicates()[0].phases()[0].tasks()[0];
    let TaskRunKind::ExecutionUnit { members, .. } = task.kind() else {
        panic!("expected execution-unit summary");
    };
    assert_eq!(
        members
            .iter()
            .map(|unit| (unit.identity(), unit.final_iteration()))
            .collect::<Vec<_>>(),
        [("first", 1), ("second", 2)]
    );
    for (index, unit) in members.iter().enumerate() {
        assert!(
            unit.output_directory()
                .ends_with(format!("members/member-{index:06}"))
        );
        let metadata: serde_json::Value = serde_json::from_slice(
            &fs::read(unit.output_directory().join("metadata.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(metadata["status"]["state"], "complete");
        assert_eq!(metadata["user_metadata"]["workflow"]["member_index"], index);
        assert_eq!(
            metadata["user_metadata"]["workflow"]["member_identity"],
            unit.identity()
        );
        let derivation = &metadata["user_metadata"]["workflow"]["seed_derivation"];
        assert_eq!(derivation["algorithm"], "scientific-workflow.seed.v1");
        assert_eq!(derivation["master_seed"], 42);
        assert_eq!(derivation["requests"].as_array().unwrap().len(), 2);
        assert!(
            derivation["requests"]
                .as_array()
                .unwrap()
                .iter()
                .any(|request| {
                    request["scope"] == "shared" && request["purpose"] == "coordination"
                })
        );
        assert!(
            derivation["requests"]
                .as_array()
                .unwrap()
                .iter()
                .any(|request| {
                    request["scope"] == "member"
                        && request["member_identity"] == unit.identity()
                        && request["purpose"] == "initialization"
                })
        );
    }
}

#[test]
fn completion_timestamp_controls_task_timeout_classification() {
    let started = Instant::now();
    let timeout = Duration::from_millis(20);

    assert!(!task_exceeded_timeout(
        started,
        started + Duration::from_millis(19),
        timeout
    ));
    assert!(task_exceeded_timeout(started, started + timeout, timeout));
}

#[test]
fn a_panicking_unit_is_reported_and_its_active_recording_is_failed() {
    let project = Project::new(
        execution_unit_study("runtime-panic-after-begin", None),
        serde_json::json!({"runtime-panic-after-begin": {}}),
    );

    let error = execute(Study::load(project.path()).unwrap()).unwrap_err();
    assert!(has_task_panic(&error));

    let metadata: serde_json::Value = serde_json::from_slice(
        &fs::read(
            project
                .execution_directory()
                .join("replicate-000000/task-000000/metadata.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(metadata["status"]["state"], "failed");
    assert!(
        metadata["status"]["message"]
            .as_str()
            .unwrap()
            .contains("runtime panic sentinel")
    );
}

#[test]
fn a_task_finishing_after_its_deadline_is_timed_out_and_not_completed() {
    let project = Project::new(
        execution_unit_study("runtime-slow", Some(20)),
        serde_json::json!({"runtime-slow": {"sleep_ms": 80}}),
    );

    let error = execute(Study::load(project.path()).unwrap()).unwrap_err();
    assert!(has_task_timeout(&error));

    let metadata: serde_json::Value = serde_json::from_slice(
        &fs::read(
            project
                .execution_directory()
                .join("replicate-000000/task-000000/metadata.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(metadata["status"]["state"], "failed");
}

#[test]
fn a_phase_finishing_after_its_deadline_is_timed_out_and_not_completed() {
    let mut study = execution_unit_study("runtime-slow", None);
    study["phases"]["run"]["timeout_ms"] = 20.into();
    let project = Project::new(study, serde_json::json!({"runtime-slow": {"sleep_ms": 80}}));

    let error = execute(Study::load(project.path()).unwrap()).unwrap_err();
    assert!(has_phase_timeout(&error));

    let metadata: serde_json::Value = serde_json::from_slice(
        &fs::read(
            project
                .execution_directory()
                .join("replicate-000000/task-000000/metadata.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(metadata["status"]["state"], "failed");
}

#[cfg(unix)]
fn replicate_program_project(failure_policy: &str, sibling_sleep: u64) -> Project {
    let project = Project::new(
        serde_json::json!({
            "paths": {"states": {}},
            "replicates": {
                "count": 3,
                "scheduling": "parallel",
                "failure_policy": failure_policy
            },
            "phases": {"run": {"tasks": [{"program": "replicate-worker.sh"}]}}
        }),
        serde_json::json!({}),
    );
    project.write_executable(
        "replicate-worker.sh",
        format!(
            "#!/bin/sh\ncase \"$WORKFLOW_REPLICATE_ROOT\" in\n  *replicate-000000) exit 9 ;;\n  *) sleep {sibling_sleep}; printf done > \"$WORKFLOW_TASK_OUTPUT/done\" ;;\nesac\n"
        )
        .as_str(),
    );
    project
}

#[cfg(unix)]
#[test]
fn parallel_replicate_fail_fast_cancels_siblings_on_first_observed_failure() {
    let project = replicate_program_project("fail_fast", 3);
    let started = Instant::now();

    let error = execute(Study::load(project.path()).unwrap()).unwrap_err();

    assert!(matches!(error, RuntimeError::Replicate { index: 0, .. }));
    assert!(started.elapsed() < Duration::from_secs(2));
    let execution = project.execution_directory();
    for index in 1..=2 {
        assert!(
            !execution
                .join(format!("replicate-{index:06}/task-000000/artifacts/done"))
                .exists()
        );
    }
}

#[cfg(unix)]
#[test]
fn parallel_replicate_finish_all_allows_successful_siblings_to_finish() {
    let project = replicate_program_project("finish_all", 0);

    let error = execute(Study::load(project.path()).unwrap()).unwrap_err();

    assert!(matches!(error, RuntimeError::Replicate { index: 0, .. }));
    let execution = project.execution_directory();
    for index in 1..=2 {
        assert!(
            execution
                .join(format!("replicate-{index:06}/task-000000/artifacts/done"))
                .is_file()
        );
    }
}

#[cfg(unix)]
fn phase_program_project(failure_policy: &str, sibling_sleep: u64) -> Project {
    let project = Project::new(
        serde_json::json!({
            "paths": {"states": {}},
            "phases": {"run": {
                "tasks": [
                    {"program": "phase-fail.sh"},
                    {"program": "phase-sibling.sh"}
                ],
                "max_concurrency": 2,
                "failure_policy": failure_policy
            }}
        }),
        serde_json::json!({}),
    );
    project.write_executable("phase-fail.sh", "#!/bin/sh\nexit 7\n");
    project.write_executable(
        "phase-sibling.sh",
        format!("#!/bin/sh\nsleep {sibling_sleep}\nprintf done > \"$WORKFLOW_TASK_OUTPUT/done\"\n")
            .as_str(),
    );
    project
}

#[cfg(unix)]
#[test]
fn phase_fail_fast_cancels_an_active_sibling() {
    let project = phase_program_project("fail_fast", 3);
    let started = Instant::now();

    execute(Study::load(project.path()).unwrap()).unwrap_err();

    assert!(started.elapsed() < Duration::from_secs(2));
    assert!(
        !project
            .execution_directory()
            .join("replicate-000000/task-000001/artifacts/done")
            .exists()
    );
}

#[cfg(unix)]
#[test]
fn phase_finish_all_allows_an_active_sibling_to_finish() {
    let project = phase_program_project("finish_all", 0);

    execute(Study::load(project.path()).unwrap()).unwrap_err();

    assert!(
        project
            .execution_directory()
            .join("replicate-000000/task-000001/artifacts/done")
            .is_file()
    );
}

#[cfg(unix)]
fn timing_program_project(scheduling: &str) -> Project {
    let project = Project::new(
        serde_json::json!({
            "paths": {"states": {}},
            "replicates": {"count": 2, "scheduling": scheduling},
            "phases": {"run": {"tasks": [{"program": "timed.sh"}]}}
        }),
        serde_json::json!({}),
    );
    project.write_executable(
        "timed.sh",
        r#"#!/bin/sh
replicate=$(basename "$WORKFLOW_REPLICATE_ROOT")
: > "$WORKFLOW_PROJECT_ROOT/${replicate}.start"
sleep 0.2
: > "$WORKFLOW_PROJECT_ROOT/${replicate}.end"
"#,
    );
    project
}

#[cfg(unix)]
fn timing_ranges(path: &Path, prefix: &str) -> Vec<(std::time::SystemTime, std::time::SystemTime)> {
    let mut ranges = std::collections::BTreeMap::<
        String,
        (Option<std::time::SystemTime>, Option<std::time::SystemTime>),
    >::new();
    for entry in fs::read_dir(path).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().into_string().unwrap();
        let (subject, position) = if let Some(subject) = name.strip_suffix(".start") {
            (subject, 0)
        } else if let Some(subject) = name.strip_suffix(".end") {
            (subject, 1)
        } else {
            continue;
        };
        if !subject.starts_with(prefix) {
            continue;
        }
        let modified = entry.metadata().unwrap().modified().unwrap();
        let range = ranges.entry(subject.to_owned()).or_default();
        if position == 0 {
            range.0 = Some(modified);
        } else {
            range.1 = Some(modified);
        }
    }
    ranges
        .into_values()
        .map(|(start, end)| (start.unwrap(), end.unwrap()))
        .collect()
}

#[cfg(unix)]
#[test]
fn sequential_and_parallel_replicate_policies_have_distinct_admission() {
    let sequential = timing_program_project("sequential");
    let sequential_summary = execute(Study::load(sequential.path()).unwrap()).unwrap();
    assert_eq!(
        sequential_summary
            .replicates()
            .iter()
            .map(|replicate| replicate.index())
            .collect::<Vec<_>>(),
        [0, 1]
    );
    let sequential_ranges = timing_ranges(sequential.path(), "replicate-");
    assert!(sequential_ranges[1].0 >= sequential_ranges[0].1);

    let parallel = timing_program_project("parallel");
    let parallel_summary = execute(Study::load(parallel.path()).unwrap()).unwrap();
    assert_eq!(
        parallel_summary
            .replicates()
            .iter()
            .map(|replicate| replicate.index())
            .collect::<Vec<_>>(),
        [0, 1]
    );
    let parallel_ranges = timing_ranges(parallel.path(), "replicate-");
    let latest_start = parallel_ranges.iter().map(|range| range.0).max().unwrap();
    let earliest_end = parallel_ranges.iter().map(|range| range.1).min().unwrap();
    assert!(latest_start < earliest_end);
}

#[cfg(unix)]
#[test]
fn phase_concurrency_respects_start_interval_and_summary_order() {
    let project = Project::new(
        serde_json::json!({
            "paths": {"states": {}},
            "phases": {"run": {
                "tasks": [
                    {"program": "phase-timed.sh"},
                    {"program": "phase-timed.sh"},
                    {"program": "phase-timed.sh"}
                ],
                "max_concurrency": 3,
                "start_interval_ms": 60
            }}
        }),
        serde_json::json!({}),
    );
    project.write_executable(
        "phase-timed.sh",
        r#"#!/bin/sh
task=$(basename "$(dirname "$WORKFLOW_TASK_OUTPUT")")
: > "$WORKFLOW_PROJECT_ROOT/${task}.start"
sleep 0.25
: > "$WORKFLOW_PROJECT_ROOT/${task}.end"
"#,
    );

    let summary = execute(Study::load(project.path()).unwrap()).unwrap();
    let tasks = summary.replicates()[0].phases()[0].tasks();
    assert_eq!(tasks.len(), 3);
    assert!(tasks[0].output_directory().ends_with("task-000000"));
    assert!(tasks[1].output_directory().ends_with("task-000001"));
    assert!(tasks[2].output_directory().ends_with("task-000002"));

    let ranges = timing_ranges(project.path(), "task-");
    assert!(ranges[1].0.duration_since(ranges[0].0).unwrap() >= Duration::from_millis(40));
    assert!(ranges[2].0.duration_since(ranges[1].0).unwrap() >= Duration::from_millis(40));
    assert!(ranges[2].0 < ranges[0].1);
}
