//! Target project-specification and resolved-input configuration boundary.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use super::advanced::*;
use scientific_workflow::state::advanced::{StateSchemaAccess, SystemStateSchema};
use serde::Deserialize;

static NEXT_PROJECT: AtomicUsize = AtomicUsize::new(0);

struct TestProject(PathBuf);

impl TestProject {
    fn new(study: &str, inputs: &[(&str, &str)]) -> Self {
        let sequence = NEXT_PROJECT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "scientific-workflow-config-{sequence}-{}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("config/inputs")).unwrap();
        fs::write(root.join("study.json"), study).unwrap();
        fs::write(
            root.join("config/state.json"),
            r#"{
              "fields": [
                {"name": "population"},
                {"name": "energy", "description": "Current energy"}
              ]
            }"#,
        )
        .unwrap();
        for (name, source) in inputs {
            fs::write(root.join("config/inputs").join(name), source).unwrap();
        }
        Self(root)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestProject {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn manifest() -> &'static str {
    r#"{
      "replicates": {
        "count": 3,
        "scheduling": "parallel",
        "failure_policy": "finish_all"
      },
      "persistence": {
        "chunk_target_bytes": 8192,
        "queue_capacity_bytes": 16384
      },
      "phases": {
        "simulate": {
          "tasks": [{
            "model": "model",
            "input": "inputs/run.json",
            "timeout_ms": 250
          }],
          "max_concurrency": 2,
          "start_interval_ms": 10
        },
        "analyze": {
          "after": ["simulate"],
          "tasks": [{
            "model": "analysis",
            "input": "inputs/analysis.json"
          }],
          "timeout_ms": 500,
          "failure_policy": "finish_all"
        }
      }
    }"#
}

#[test]
fn one_project_root_compiles_every_document_into_a_resolved_specification() {
    let project = TestProject::new(
        manifest(),
        &[
            (
                "run.json",
                r#"{
                  "shape": [64, 64],
                  "temperature": {"$sweep": [280.0, 300.0]},
                  "solver": {"method": {"$sweep": ["rk4", "euler"]}}
                }"#,
            ),
            ("analysis.json", r#"{"minimum": 0.25}"#),
        ],
    );

    let specification = ProjectSpecification::load(project.path()).unwrap();
    assert!(specification.project_root().is_absolute());
    assert_eq!(specification.phases().len(), 2);

    let replicates = specification.manifest().replicate_policy();
    assert_eq!(replicates.count(), 3);
    assert_eq!(replicates.scheduling(), ReplicateScheduling::Parallel);
    assert_eq!(replicates.failure_policy(), FailurePolicy::FinishAll);
    let persistence = specification.manifest().persistence();
    assert_eq!(persistence.chunk_target_bytes().get(), 8192);
    assert_eq!(persistence.queue_capacity_bytes().get(), 16384);

    let simulation = &specification.phases()[0];
    assert_eq!(simulation.name(), "simulate");
    assert_eq!(simulation.tasks().len(), 4);
    assert_eq!(simulation.max_concurrency(), 2);
    assert_eq!(simulation.start_interval(), Duration::from_millis(10));
    assert_eq!(simulation.failure_policy(), FailurePolicy::FailFast);

    #[derive(Debug, Deserialize, PartialEq)]
    struct Solver {
        method: String,
    }
    #[derive(Debug, Deserialize, PartialEq)]
    struct Constants {
        shape: Vec<usize>,
        temperature: f64,
        solver: Solver,
    }

    let decoded = simulation
        .tasks()
        .iter()
        .map(ResolvedTaskInput::decode::<Constants>)
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        decoded,
        [
            Constants {
                shape: vec![64, 64],
                temperature: 280.0,
                solver: Solver {
                    method: "rk4".to_owned()
                }
            },
            Constants {
                shape: vec![64, 64],
                temperature: 280.0,
                solver: Solver {
                    method: "euler".to_owned()
                }
            },
            Constants {
                shape: vec![64, 64],
                temperature: 300.0,
                solver: Solver {
                    method: "rk4".to_owned()
                }
            },
            Constants {
                shape: vec![64, 64],
                temperature: 300.0,
                solver: Solver {
                    method: "euler".to_owned()
                }
            },
        ]
    );
    assert_eq!(simulation.tasks()[3].model(), "model");
    assert_eq!(simulation.tasks()[3].ordinal(), 3);
    assert_eq!(
        simulation.tasks()[3].timeout(),
        Some(Duration::from_millis(250))
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(simulation.tasks()[3].resolved_json()).unwrap(),
        serde_json::json!({
            "shape": [64, 64],
            "temperature": 300.0,
            "solver": {"method": "euler"}
        })
    );

    let analysis = &specification.phases()[1];
    assert_eq!(analysis.dependencies().collect::<Vec<_>>(), ["simulate"]);
    assert_eq!(analysis.timeout(), Some(Duration::from_millis(500)));
    assert_eq!(analysis.failure_policy(), FailurePolicy::FinishAll);
}

#[test]
fn state_schema_is_parsed_once_by_config_then_semantically_validated_by_state() {
    let project = TestProject::new(
        r#"{"phases":{"only":{"tasks":[{"model":"model","input":"inputs/run.json"}]}}}"#,
        &[("run.json", "{}")],
    );
    let specification = ProjectSpecification::load(project.path()).unwrap();
    let document = specification.state_schema();
    let schema =
        SystemStateSchema::from_json_template_value(document.path(), document.json_value())
            .unwrap();

    assert_eq!(schema.field_schemas().len(), 2);
    assert_eq!(schema.template_path(), document.path());
}

#[test]
fn correlated_cases_become_complete_typed_constant_values() {
    let project = TestProject::new(
        r#"{"phases":{"only":{"tasks":[{"model":"model","input":"inputs/run.json"}]}}}"#,
        &[(
            "run.json",
            r#"{
              "shape": [64],
              "$cases": [
                {"temperature": 280.0, "step": 0.02},
                {"temperature": 300.0, "step": 0.01}
              ]
            }"#,
        )],
    );
    let specification = ProjectSpecification::load(project.path()).unwrap();
    #[derive(Deserialize)]
    struct Constants {
        shape: Vec<usize>,
        temperature: f64,
        step: f64,
    }
    let values = specification.phases()[0]
        .tasks()
        .iter()
        .map(|input| input.decode::<Constants>().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(values.len(), 2);
    assert_eq!(values[0].shape, [64]);
    assert_eq!((values[0].temperature, values[0].step), (280.0, 0.02));
    assert_eq!((values[1].temperature, values[1].step), (300.0, 0.01));
}

#[test]
fn project_documents_are_strict_and_workflow_owned_objects_reject_unknown_fields() {
    let duplicate = TestProject::new(r#"{"phases":{"one":{"tasks":[],"tasks":[]}}}"#, &[]);
    assert!(matches!(
        ProjectSpecification::load(duplicate.path()),
        Err(ConfigError::DuplicateKey { key, .. }) if key == "tasks"
    ));

    let unknown = TestProject::new(
        r#"{"phases":{"one":{"tasks":[{"model":"x","input":"inputs/x.json","mystery":true}]}}}"#,
        &[("x.json", "{}")],
    );
    assert!(matches!(
        ProjectSpecification::load(unknown.path()),
        Err(ConfigError::InvalidDocument { pointer, .. }) if pointer == "/phases/one"
    ));

    let invalid_persistence = TestProject::new(
        r#"{"persistence":{"chunk_target_bytes":0},"phases":{"one":{"tasks":[{"model":"x","input":"inputs/x.json"}]}}}"#,
        &[("x.json", "{}")],
    );
    assert!(matches!(
        ProjectSpecification::load(invalid_persistence.path()),
        Err(ConfigError::InvalidDocument { pointer, .. }) if pointer == "/"
    ));

    let duplicate_input = TestProject::new(
        r#"{"phases":{"one":{"tasks":[{"model":"x","input":"inputs/x.json"}]}}}"#,
        &[("x.json", r#"{"value":1,"value":2}"#)],
    );
    assert!(matches!(
        ProjectSpecification::load(duplicate_input.path()),
        Err(ConfigError::DuplicateKey { key, .. }) if key == "value"
    ));
}

#[test]
fn task_inputs_cannot_escape_the_config_inputs_directory() {
    for input in ["../outside.json", "state.json", "/absolute.json"] {
        let study = serde_json::json!({
            "phases": {
                "one": {
                    "tasks": [{"model": "x", "input": input}]
                }
            }
        });
        let project = TestProject::new(&study.to_string(), &[]);
        assert!(matches!(
            ProjectSpecification::load(project.path()),
            Err(ConfigError::PathOutsideConfig { .. })
        ));
    }
}

#[test]
fn dependency_and_selection_grammar_fail_before_a_specification_is_published() {
    let missing = TestProject::new(
        r#"{"phases":{"one":{"after":["absent"],"tasks":[{"model":"x","input":"inputs/x.json"}]}}}"#,
        &[("x.json", "{}")],
    );
    assert!(matches!(
        ProjectSpecification::load(missing.path()),
        Err(ConfigError::UnknownDependency { .. })
    ));

    let cycle = TestProject::new(
        r#"{
          "phases": {
            "one": {"after":["two"],"tasks":[{"model":"x","input":"inputs/x.json"}]},
            "two": {"after":["one"],"tasks":[{"model":"x","input":"inputs/x.json"}]}
          }
        }"#,
        &[("x.json", "{}")],
    );
    assert!(matches!(
        ProjectSpecification::load(cycle.path()),
        Err(ConfigError::InvalidDocument { .. })
    ));

    let mixed = TestProject::new(
        r#"{"phases":{"one":{"tasks":[{"model":"x","input":"inputs/x.json"}]}}}"#,
        &[(
            "x.json",
            r#"{"choice":{"$sweep":[1,2]},"$cases":[{"value":1},{"value":2}]}"#,
        )],
    );
    assert!(matches!(
        ProjectSpecification::load(mixed.path()),
        Err(ConfigError::InvalidDocument { .. })
    ));
}

#[test]
fn typed_decode_errors_retain_model_source_and_combination() {
    let project = TestProject::new(
        r#"{"phases":{"one":{"tasks":[{"model":"model","input":"inputs/x.json"}]}}}"#,
        &[("x.json", r#"{"steps":"wrong"}"#)],
    );
    let specification = ProjectSpecification::load(project.path()).unwrap();
    #[derive(Deserialize)]
    struct Constants {
        #[allow(dead_code)]
        steps: u64,
    }
    assert!(matches!(
        specification.phases()[0].tasks()[0].decode::<Constants>(),
        Err(ConfigError::DecodeModelConstants {
            model,
            ordinal: 0,
            ..
        }) if model == "model"
    ));
}
