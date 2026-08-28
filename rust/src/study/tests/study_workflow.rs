use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::advanced::{Study, StudyError};
use scientific_workflow::prelude::basic::*;
use scientific_workflow::runtime::advanced::TaskRunKind;
use scientific_workflow::runtime::advanced::execute;
use serde::Deserialize;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CounterConstants {
    initial: u64,
    steps: u64,
}

struct CounterModel {
    state: SystemState,
    steps: u64,
}

#[derive(Deserialize)]
struct EnergyConstants {
    initial: f64,
}

struct EnergyModel {
    state: SystemState,
}

#[scientific_workflow::model("energy")]
impl ScientificModel for EnergyModel {
    type Constants = EnergyConstants;

    fn initialize(constants: Self::Constants, schema: &SystemStateSchema) -> TaskResult<Self> {
        let mut state = schema.create_empty_state(StateTime::from_iteration(0));
        state.initialize_payload("energy", constants.initial)?;
        Ok(Self { state })
    }

    fn state(&self) -> &SystemState {
        &self.state
    }

    fn is_complete(&self) -> bool {
        self.state.time().iteration() == 1
    }

    fn step(&mut self) -> TaskResult {
        *self.state.payload_mut::<f64>("energy")? *= 0.5;
        self.state.advance_time(None)?;
        Ok(())
    }
}

#[scientific_workflow::model("counter")]
impl ScientificModel for CounterModel {
    type Constants = CounterConstants;

    fn initialize(constants: Self::Constants, schema: &SystemStateSchema) -> TaskResult<Self> {
        let mut state = schema.create_empty_state(StateTime::from_iteration(0));
        state.initialize_payload("count", constants.initial)?;
        Ok(Self {
            state,
            steps: constants.steps,
        })
    }

    fn state(&self) -> &SystemState {
        &self.state
    }

    fn is_complete(&self) -> bool {
        self.state.time().iteration() == self.steps
    }

    fn step(&mut self) -> TaskResult {
        *self.state.payload_mut::<u64>("count")? += 1;
        self.state.advance_time(None)?;
        Ok(())
    }

    fn target_iteration(&self) -> Option<u64> {
        Some(self.steps)
    }
}

struct Project(PathBuf);

impl Project {
    fn new(study: &str, parameters: &str) -> Self {
        let mut study: serde_json::Value = serde_json::from_str(study).unwrap();
        let root = study.as_object_mut().unwrap();
        root.entry("paths").or_insert_with(
            || serde_json::json!({"states":{"default":"wf_configs/states/default.json"}}),
        );
        if let Some(phases) = root
            .get_mut("phases")
            .and_then(serde_json::Value::as_object_mut)
        {
            for phase in phases.values_mut() {
                if let Some(tasks) = phase
                    .get_mut("tasks")
                    .and_then(serde_json::Value::as_array_mut)
                {
                    for task in tasks {
                        if task.get("model").is_some() && task.get("state").is_none() {
                            task.as_object_mut()
                                .unwrap()
                                .insert("state".to_owned(), "default".into());
                        }
                    }
                }
            }
        }
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "scientific-workflow-study-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("wf_configs/states")).unwrap();
        fs::write(
            root.join("wf_configs/study.json"),
            serde_json::to_vec_pretty(&study).unwrap(),
        )
        .unwrap();
        fs::write(
            root.join("wf_configs/states/default.json"),
            r#"{"fields":[{"name":"count"}]}"#,
        )
        .unwrap();
        fs::write(
            root.join("wf_configs/parameters.json"),
            format!(r#"{{"counter":{parameters}}}"#),
        )
        .unwrap();
        Self(root)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

const STUDY: &str = r#"
{
  "persistence": {
    "chunk_target_mb": 1,
    "queue_capacity_mb": 2
  },
  "phases": {
    "measure": {
      "after": ["simulate"],
      "tasks": [{"model":"counter"}]
    },
    "simulate": {
      "tasks": [{
        "model":"counter"
      }],
      "max_concurrency": 2
    }
  }
}
"#;

#[test]
fn study_and_its_error_are_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}

    assert_send_sync::<Study>();
    assert_send_sync::<StudyError>();
}

#[test]
fn study_binds_registered_models_and_infers_plan_facts_without_output() {
    let project = Project::new(STUDY, r#"{"initial":5,"steps":2}"#);
    let study = Study::load(project.path()).unwrap();

    assert_eq!(
        study.project_root(),
        fs::canonicalize(project.path()).unwrap()
    );
    assert_eq!(study.output_root(), study.project_root().join("output"));
    assert!(!study.output_root().exists());
    assert_eq!(study.persistence_plan().chunk_target().get(), 1_000_000);
    assert_eq!(study.persistence_plan().queue_capacity().get(), 2_000_000);
    assert_eq!(study.phases().len(), 2);
    assert_eq!(study.phases()[0].name(), "measure");
    assert_eq!(
        study.phases()[0].dependencies().collect::<Vec<_>>(),
        ["simulate"]
    );
    assert_eq!(study.phases()[1].tasks()[0].model(), Some("counter"));
    assert_eq!(
        study.phases()[0].tasks()[0].identity(),
        "measure/000000/counter-000000"
    );
    assert_eq!(
        study.phases()[1].tasks()[0].identity(),
        "simulate/000001/counter-000000"
    );
}

#[test]
fn runtime_executes_dependencies_and_records_each_inferred_task() {
    let project = Project::new(STUDY, r#"{"initial":5,"steps":2}"#);
    let summary = execute(Study::load(project.path()).unwrap()).unwrap();

    assert_eq!(summary.replicates().len(), 1);
    let replicate = &summary.replicates()[0];
    assert_eq!(
        replicate
            .phases()
            .iter()
            .map(|phase| phase.name())
            .collect::<Vec<_>>(),
        ["simulate", "measure"]
    );
    for phase in replicate.phases() {
        assert_eq!(phase.tasks()[0].final_iteration(), Some(2));
        let metadata_path = phase.tasks()[0].output_directory().join("metadata.json");
        assert!(metadata_path.is_file());
        let metadata: serde_json::Value =
            serde_json::from_slice(&fs::read(metadata_path).unwrap()).unwrap();
        assert_eq!(metadata["user_metadata"]["model_constants"]["initial"], 5);
        assert_eq!(metadata["user_metadata"]["workflow"]["model"], "counter");
        assert_eq!(metadata["user_metadata"]["workflow"]["state"], "default");
        assert_eq!(
            metadata["user_metadata"]["workflow"]["parameter_ordinal"],
            0
        );
        let parameter_source = metadata["user_metadata"]["workflow"]["parameter_source"]
            .as_str()
            .unwrap();
        assert!(parameter_source.ends_with("wf_configs/parameters.json"));
        assert!(metadata["user_metadata"]["workflow"]["input_ordinal"].is_null());
        assert!(metadata["user_metadata"]["workflow"]["input_source"].is_null());
        assert_eq!(
            metadata["user_metadata"]["workflow"]["persistence"],
            serde_json::json!({
                "backend": "local",
                "chunk_target_bytes": 1_000_000,
                "queue_capacity_bytes": 2_000_000
            })
        );
        assert_eq!(
            metadata["streams"][0]["storage"]["layout"]["target_bytes"],
            1_000_000
        );
    }
}

#[test]
fn one_study_binds_each_model_task_to_its_selected_named_state() {
    let project = Project::new(
        r#"{
          "paths":{"states":{
            "counter-state":"wf_configs/states/default.json",
            "energy-state":"wf_configs/states/energy.json"
          }},
          "phases":{"simulate":{"tasks":[
            {"model":"counter","state":"counter-state"},
            {"model":"energy","state":"energy-state"}
          ]}}
        }"#,
        r#"{"initial":2,"steps":1}"#,
    );
    fs::write(
        project.path().join("wf_configs/states/energy.json"),
        r#"{"fields":[{"name":"energy"}]}"#,
    )
    .unwrap();
    fs::write(
        project.path().join("wf_configs/parameters.json"),
        r#"{
          "counter":{"initial":2,"steps":1},
          "energy":{"initial":8.0}
        }"#,
    )
    .unwrap();

    let summary = execute(Study::load(project.path()).unwrap()).unwrap();
    let tasks = summary.replicates()[0].phases()[0].tasks();
    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks[0].model(), Some("counter"));
    assert_eq!(tasks[0].final_iteration(), Some(1));
    assert_eq!(tasks[1].model(), Some("energy"));
    assert_eq!(tasks[1].final_iteration(), Some(1));
}

#[test]
fn crate_level_run_is_the_complete_ordinary_entry_point() {
    let project = Project::new(
        r#"{"phases":{"only":{"tasks":[{"model":"counter"}]}}}"#,
        r#"{"initial":1,"steps":1}"#,
    );
    let study = Study::load(project.path()).unwrap();
    assert_eq!(study.persistence_plan().chunk_target().get(), 64_000_000);
    assert_eq!(study.persistence_plan().queue_capacity().get(), 64_000_000);
    scientific_workflow::run(project.path()).unwrap();
    assert!(project.path().join("output").is_dir());
}

#[test]
fn preflight_rejects_invalid_binding_without_output() {
    let missing = Project::new(
        r#"{"phases":{"only":{"tasks":[{"model":"absent"}]}}}"#,
        r#"{"initial":1,"steps":1}"#,
    );
    fs::write(
        missing.path().join("wf_configs/parameters.json"),
        r#"{"absent":{"initial":1,"steps":1}}"#,
    )
    .unwrap();
    assert!(matches!(
        Study::load(missing.path()),
        Err(StudyError::UnknownModel { model, .. }) if model == "absent"
    ));
    assert!(!missing.path().join("output").exists());

    let bad_constants = Project::new(
        r#"{"phases":{"only":{"tasks":[{"model":"counter"}]}}}"#,
        r#"{"initial":"wrong","steps":1}"#,
    );
    assert!(matches!(
        Study::load(bad_constants.path()),
        Err(StudyError::ModelPreflight { model, .. }) if model == "counter"
    ));
    assert!(!bad_constants.path().join("output").exists());

    let bad_state = Project::new(
        r#"{
          "paths":{"states":{"broken":"wf_configs/states/broken.json"}},
          "phases":{"only":{"tasks":[{"model":"counter","state":"broken"}]}}
        }"#,
        r#"{"initial":1,"steps":1}"#,
    );
    fs::write(
        bad_state.path().join("wf_configs/states/broken.json"),
        r#"{"fields":[{"name":"count"},{"name":"count"}]}"#,
    )
    .unwrap();
    assert!(matches!(
        Study::load(bad_state.path()),
        Err(StudyError::State { state, path, source: StateError::DuplicateField { field } })
            if state == "broken"
                && path.ends_with("wf_configs/states/broken.json")
                && field == "count"
    ));
    assert!(!bad_state.path().join("output").exists());
}

#[cfg(unix)]
#[test]
fn generic_program_task_receives_captured_config_and_dependency_outputs() {
    use std::os::unix::fs::PermissionsExt as _;

    let study_document = r#"
    {
      "phases": {
        "simulate": {
          "tasks": [{"model":"counter"}]
        },
        "plot": {
          "after": ["simulate"],
          "tasks": [{"program":"plot.py"}]
        }
      }
    }
    "#;
    let project = Project::new(study_document, r#"{"initial":2,"steps":1}"#);
    fs::write(
        project.path().join("wf_configs/parameters.json"),
        r#"{"counter":{"initial":2,"steps":1},"plot":{"title":"captured title","dpi":180}}"#,
    )
    .unwrap();
    let program = project.path().join("plot.py");
    fs::write(
        &program,
        r#"#!/usr/bin/env python3
import json
import os
from pathlib import Path

config = json.loads(Path(os.environ["WORKFLOW_CONFIG_PATH"]).read_text())
dependencies = json.loads(Path(os.environ["WORKFLOW_DEPENDENCIES_PATH"]).read_text())
output = Path(os.environ["WORKFLOW_TASK_OUTPUT"])
result = {
    "title": config["config"]["parameters.json"]["plot"]["title"],
    "phase": dependencies[0]["phase"],
    "dependency_kind": dependencies[0]["tasks"][0]["kind"],
    "dependency_exists": Path(dependencies[0]["tasks"][0]["output_directory"]).is_dir(),
}
(output / "plot-result.json").write_text(json.dumps(result))
"#,
    )
    .unwrap();
    let mut permissions = fs::metadata(&program).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&program, permissions).unwrap();

    let study = Study::load(project.path()).unwrap();
    fs::write(
        project.path().join("wf_configs/parameters.json"),
        r#"{"counter":{"initial":2,"steps":1},"plot":{"title":"changed after Study load","dpi":72}}"#,
    )
    .unwrap();

    let summary = execute(study).unwrap();
    let replicate = &summary.replicates()[0];
    assert_eq!(replicate.phases()[0].name(), "simulate");
    assert_eq!(replicate.phases()[1].name(), "plot");
    let program_summary = &replicate.phases()[1].tasks()[0];
    assert_eq!(program_summary.kind(), TaskRunKind::Program);
    assert_eq!(program_summary.model(), None);
    assert_eq!(program_summary.program(), Some(program.as_path()));
    assert_eq!(program_summary.program_kind(), Some("program"));
    assert_eq!(program_summary.python_script(), None);
    assert_eq!(program_summary.final_iteration(), None);

    let result: serde_json::Value = serde_json::from_slice(
        &fs::read(
            program_summary
                .output_directory()
                .join("artifacts/plot-result.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(result["title"], "captured title");
    assert_eq!(result["phase"], "simulate");
    assert_eq!(result["dependency_kind"], "model");
    assert_eq!(result["dependency_exists"], true);

    let metadata: serde_json::Value = serde_json::from_slice(
        &fs::read(program_summary.output_directory().join("program.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(metadata["status"], "complete");
    assert!(
        program_summary
            .output_directory()
            .join("stdout.log")
            .is_file()
    );
    assert!(
        program_summary
            .output_directory()
            .join("stderr.log")
            .is_file()
    );
}

#[cfg(unix)]
#[test]
fn nested_python_task_runs_without_a_rust_wrapper_or_executable_script() {
    use std::os::unix::fs::PermissionsExt as _;

    let study_document = r#"
    {
      "phases": {
        "analyze": {
          "tasks": [{
            "python": {
              "script": "analyze.py",
              "environment": {"manager": "system"}
            }
          }]
        }
      }
    }
    "#;
    let project = Project::new(study_document, r#"{"initial":0,"steps":0}"#);
    fs::write(
        project.path().join("wf_configs/parameters.json"),
        r#"{"counter":{"initial":0,"steps":0},"analysis":{"title":"direct Python task"}}"#,
    )
    .unwrap();
    let script = project.path().join("analyze.py");
    fs::write(
        &script,
        r#"import json
import os
from pathlib import Path

config = json.loads(Path(os.environ["WORKFLOW_CONFIG_PATH"]).read_text())
output = Path(os.environ["WORKFLOW_TASK_OUTPUT"])
(output / "analysis.json").write_text(json.dumps({
    "title": config["config"]["parameters.json"]["analysis"]["title"]
}))
"#,
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o644)).unwrap();

    let summary = execute(Study::load(project.path()).unwrap()).unwrap();
    let task = &summary.replicates()[0].phases()[0].tasks()[0];
    assert_eq!(task.identity(), "analyze/000000/python-analyze.py");
    assert_eq!(task.kind(), TaskRunKind::Program);
    assert_eq!(task.program_kind(), Some("python"));
    assert_eq!(task.python_script(), Some(script.as_path()));
    assert_eq!(task.final_iteration(), None);
    assert!(task.program().unwrap().is_absolute());
    let result: serde_json::Value = serde_json::from_slice(
        &fs::read(task.output_directory().join("artifacts/analysis.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(result["title"], "direct Python task");

    let metadata: serde_json::Value =
        serde_json::from_slice(&fs::read(task.output_directory().join("program.json")).unwrap())
            .unwrap();
    assert_eq!(metadata["kind"], "python");
    assert_eq!(metadata["python_environment_manager"], "system");
    assert_eq!(
        Path::new(metadata["python_script"].as_str().unwrap()),
        fs::canonicalize(script).unwrap()
    );
}
