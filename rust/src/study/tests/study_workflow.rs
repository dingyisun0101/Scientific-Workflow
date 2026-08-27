use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::advanced::{Study, StudyError};
use scientific_workflow::prelude::basic::*;
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
    fn new(study: &str, input: &str) -> Self {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "scientific-workflow-study-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("config/inputs")).unwrap();
        fs::write(root.join("study.json"), study).unwrap();
        fs::write(
            root.join("config/state.json"),
            r#"{"fields":[{"name":"count"}]}"#,
        )
        .unwrap();
        fs::write(root.join("config/inputs/counter.json"), input).unwrap();
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
    "chunk_target_bytes": 4096,
    "queue_capacity_bytes": 8192
  },
  "phases": {
    "measure": {
      "after": ["simulate"],
      "tasks": [{"model":"counter","input":"inputs/counter.json"}]
    },
    "simulate": {
      "tasks": [{
        "model":"counter",
        "input":"inputs/counter.json"
      }],
      "max_concurrency": 2
    }
  }
}
"#;

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
    assert_eq!(study.persistence_plan().chunk_target().get(), 4096);
    assert_eq!(study.persistence_plan().queue_capacity().get(), 8192);
    assert_eq!(study.phases().len(), 2);
    assert_eq!(study.phases()[0].name(), "measure");
    assert_eq!(
        study.phases()[0].dependencies().collect::<Vec<_>>(),
        ["simulate"]
    );
    assert_eq!(study.phases()[1].tasks()[0].model(), "counter");
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
        assert_eq!(phase.tasks()[0].final_iteration(), 2);
        let metadata_path = phase.tasks()[0].recording_directory().join("metadata.json");
        assert!(metadata_path.is_file());
        let metadata: serde_json::Value =
            serde_json::from_slice(&fs::read(metadata_path).unwrap()).unwrap();
        assert_eq!(metadata["user_metadata"]["model_constants"]["initial"], 5);
        assert_eq!(metadata["user_metadata"]["workflow"]["model"], "counter");
        assert_eq!(
            metadata["user_metadata"]["workflow"]["persistence"],
            serde_json::json!({
                "backend": "local",
                "chunk_target_bytes": 4096,
                "queue_capacity_bytes": 8192
            })
        );
        assert_eq!(
            metadata["streams"][0]["storage"]["layout"]["target_bytes"],
            4096
        );
    }
}

#[test]
fn crate_level_run_is_the_complete_ordinary_entry_point() {
    let project = Project::new(
        r#"{"phases":{"only":{"tasks":[{"model":"counter","input":"inputs/counter.json"}]}}}"#,
        r#"{"initial":1,"steps":1}"#,
    );
    let study = Study::load(project.path()).unwrap();
    assert_eq!(
        study.persistence_plan().chunk_target().get(),
        64 * 1024 * 1024
    );
    assert_eq!(
        study.persistence_plan().queue_capacity().get(),
        64 * 1024 * 1024
    );
    scientific_workflow::run(project.path()).unwrap();
    assert!(project.path().join("output").is_dir());
}

#[test]
fn preflight_rejects_invalid_binding_without_output() {
    let missing = Project::new(
        r#"{"phases":{"only":{"tasks":[{"model":"absent","input":"inputs/counter.json"}]}}}"#,
        r#"{"initial":1,"steps":1}"#,
    );
    assert!(matches!(
        Study::load(missing.path()),
        Err(StudyError::UnknownModel { model, .. }) if model == "absent"
    ));
    assert!(!missing.path().join("output").exists());

    let bad_constants = Project::new(
        r#"{"phases":{"only":{"tasks":[{"model":"counter","input":"inputs/counter.json"}]}}}"#,
        r#"{"initial":"wrong","steps":1}"#,
    );
    assert!(matches!(
        Study::load(bad_constants.path()),
        Err(StudyError::ModelPreflight { model, .. }) if model == "counter"
    ));
    assert!(!bad_constants.path().join("output").exists());
}
