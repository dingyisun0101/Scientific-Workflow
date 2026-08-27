//! Target project-specification and resolved-parameter configuration boundary.

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
    fn new(study: &str, parameter_sections: &[(&str, &str)]) -> Self {
        let sequence = NEXT_PROJECT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "scientific-workflow-config-{sequence}-{}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("config")).unwrap();
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
        let parameters = parameter_sections
            .iter()
            .map(|(name, source)| format!("{name:?}:{source}"))
            .collect::<Vec<_>>()
            .join(",");
        fs::write(
            root.join("config/parameters.json"),
            format!("{{{parameters}}}"),
        )
        .unwrap();
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
        "chunk_target_mb": 2,
        "queue_capacity_mb": 3
      },
      "phases": {
        "simulate": {
          "tasks": [{
            "model": "model",
            "timeout_ms": 250
          }],
          "max_concurrency": 2,
          "start_interval_ms": 10000
        },
        "analyze": {
          "after": ["simulate"],
          "tasks": [{
            "model": "analysis"
          }],
          "timeout_ms": 500,
          "failure_policy": "finish_all"
        }
      }
    }"#
}

fn model_task(task: &ResolvedTask) -> &ResolvedModelParameters {
    match task {
        ResolvedTask::Model(parameters) => parameters,
        ResolvedTask::Program(_) => panic!("expected a model task"),
    }
}

fn program_task(task: &ResolvedTask) -> &ResolvedProgramTask {
    match task {
        ResolvedTask::Model(_) => panic!("expected a program task"),
        ResolvedTask::Program(program) => program,
    }
}

#[test]
fn one_project_root_compiles_every_document_into_a_resolved_specification() {
    let project = TestProject::new(
        manifest(),
        &[
            (
                "model",
                r#"{
                  "shape": [64, 64],
                  "temperature": {"$sweep": [280.0, 300.0]},
                  "solver": {"method": {"$sweep": ["rk4", "euler"]}}
                }"#,
            ),
            ("analysis", r#"{"minimum": 0.25}"#),
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
    assert_eq!(persistence.chunk_target_bytes().get(), 2_000_000);
    assert_eq!(persistence.queue_capacity_bytes().get(), 3_000_000);

    let simulation = &specification.phases()[0];
    assert_eq!(simulation.name(), "simulate");
    assert_eq!(simulation.tasks().len(), 4);
    assert_eq!(simulation.max_concurrency(), 2);
    assert_eq!(simulation.start_interval(), Duration::from_secs(10));
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
        .map(model_task)
        .map(ResolvedModelParameters::decode::<Constants>)
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
    assert_eq!(model_task(&simulation.tasks()[3]).model(), "model");
    assert_eq!(model_task(&simulation.tasks()[3]).ordinal(), 3);
    assert_eq!(
        model_task(&simulation.tasks()[3]).timeout(),
        Some(Duration::from_millis(250))
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(
            model_task(&simulation.tasks()[3]).resolved_json()
        )
        .unwrap(),
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
        r#"{"phases":{"only":{"tasks":[{"model":"model"}]}}}"#,
        &[("model", "{}")],
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
        r#"{"phases":{"only":{"tasks":[{"model":"model"}]}}}"#,
        &[(
            "model",
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
        .map(model_task)
        .map(|parameters| parameters.decode::<Constants>().unwrap())
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
        r#"{"phases":{"one":{"tasks":[{"model":"x","mystery":true}]}}}"#,
        &[("x", "{}")],
    );
    assert!(matches!(
        ProjectSpecification::load(unknown.path()),
        Err(ConfigError::InvalidDocument { pointer, .. }) if pointer == "/phases/one"
    ));

    let invalid_persistence = TestProject::new(
        r#"{"persistence":{"chunk_target_mb":0},"phases":{"one":{"tasks":[{"model":"x"}]}}}"#,
        &[("x", "{}")],
    );
    assert!(matches!(
        ProjectSpecification::load(invalid_persistence.path()),
        Err(ConfigError::InvalidDocument { pointer, .. }) if pointer == "/"
    ));

    let legacy_chunk_bytes = TestProject::new(
        r#"{"persistence":{"chunk_target_bytes":1048576},"phases":{"one":{"tasks":[{"model":"x"}]}}}"#,
        &[("x", "{}")],
    );
    assert!(matches!(
        ProjectSpecification::load(legacy_chunk_bytes.path()),
        Err(ConfigError::InvalidDocument { .. })
    ));

    let legacy_queue_bytes = TestProject::new(
        r#"{"persistence":{"queue_capacity_bytes":1048576},"phases":{"one":{"tasks":[{"model":"x"}]}}}"#,
        &[("x", "{}")],
    );
    assert!(matches!(
        ProjectSpecification::load(legacy_queue_bytes.path()),
        Err(ConfigError::InvalidDocument { .. })
    ));

    let overflowing_chunk_mb = TestProject::new(
        r#"{"persistence":{"chunk_target_mb":18446744073709551615},"phases":{"one":{"tasks":[{"model":"x"}]}}}"#,
        &[("x", "{}")],
    );
    assert!(matches!(
        ProjectSpecification::load(overflowing_chunk_mb.path()),
        Err(ConfigError::InvalidDocument { pointer, .. })
            if pointer == "/persistence/chunk_target_mb"
    ));

    let duplicate_parameters = TestProject::new(
        r#"{"phases":{"one":{"tasks":[{"model":"x"}]}}}"#,
        &[("x", r#"{"value":1,"value":2}"#)],
    );
    assert!(matches!(
        ProjectSpecification::load(duplicate_parameters.path()),
        Err(ConfigError::DuplicateKey { key, .. }) if key == "value"
    ));
}

#[test]
fn legacy_model_input_paths_are_rejected() {
    let project = TestProject::new(
        r#"{"phases":{"one":{"tasks":[{"model":"x","input":"inputs/x.json"}]}}}"#,
        &[("x", "{}")],
    );
    assert!(matches!(
        ProjectSpecification::load(project.path()),
        Err(ConfigError::InvalidDocument { .. })
    ));
}

#[test]
fn every_model_key_requires_its_canonical_parameter_section() {
    let project = TestProject::new(
        r#"{"phases":{"one":{"tasks":[{"model":"missing"}]}}}"#,
        &[("another_model", "{}")],
    );
    assert!(matches!(
        ProjectSpecification::load(project.path()),
        Err(ConfigError::InvalidDocument { pointer, reason, .. })
            if pointer == "/missing" && reason.contains("no parameter section")
    ));
}

#[test]
fn dependency_and_selection_grammar_fail_before_a_specification_is_published() {
    let missing = TestProject::new(
        r#"{"phases":{"one":{"after":["absent"],"tasks":[{"model":"x"}]}}}"#,
        &[("x", "{}")],
    );
    assert!(matches!(
        ProjectSpecification::load(missing.path()),
        Err(ConfigError::UnknownDependency { .. })
    ));

    let cycle = TestProject::new(
        r#"{
          "phases": {
            "one": {"after":["two"],"tasks":[{"model":"x"}]},
            "two": {"after":["one"],"tasks":[{"model":"x"}]}
          }
        }"#,
        &[("x", "{}")],
    );
    assert!(matches!(
        ProjectSpecification::load(cycle.path()),
        Err(ConfigError::InvalidDocument { .. })
    ));

    let mixed = TestProject::new(
        r#"{"phases":{"one":{"tasks":[{"model":"x"}]}}}"#,
        &[(
            "x",
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
        r#"{"phases":{"one":{"tasks":[{"model":"model"}]}}}"#,
        &[("model", r#"{"steps":"wrong"}"#)],
    );
    let specification = ProjectSpecification::load(project.path()).unwrap();
    #[derive(Deserialize)]
    struct Constants {
        #[allow(dead_code)]
        steps: u64,
    }
    assert!(matches!(
        model_task(&specification.phases()[0].tasks()[0]).decode::<Constants>(),
        Err(ConfigError::DecodeModelConstants {
            model,
            ordinal: 0,
            ..
        }) if model == "model"
    ));
}

#[test]
fn central_config_captures_all_project_parameters_in_one_namespace() {
    let project = TestProject::new(
        r#"{"phases":{"only":{"tasks":[{"model":"model"}]}}}"#,
        &[
            ("model", r#"{"steps":1}"#),
            ("plot", r#"{"title":"Captured configuration","dpi":160}"#),
        ],
    );

    let specification = ProjectSpecification::load(project.path()).unwrap();
    let snapshot: serde_json::Value =
        serde_json::from_slice(specification.config().snapshot_json()).unwrap();
    assert_eq!(
        snapshot["study"]["phases"]["only"]["tasks"][0]["model"],
        "model"
    );
    assert_eq!(
        snapshot["config"]["state.json"]["fields"][0]["name"],
        "population"
    );
    assert_eq!(
        snapshot["config"]["parameters.json"]["plot"]["title"],
        "Captured configuration"
    );
}

#[test]
fn invalid_unreferenced_json_is_rejected_because_config_manages_all_documents() {
    let project = TestProject::new(
        r#"{"phases":{"only":{"tasks":[{"model":"model"}]}}}"#,
        &[("model", "{}")],
    );
    fs::write(
        project.path().join("config/unreferenced.json"),
        r#"{"duplicate":1,"duplicate":2}"#,
    )
    .unwrap();
    assert!(matches!(
        ProjectSpecification::load(project.path()),
        Err(ConfigError::DuplicateKey { key, .. }) if key == "duplicate"
    ));
}

#[cfg(unix)]
#[test]
fn program_resolution_rejects_a_regular_file_without_execute_permission() {
    use std::os::unix::fs::PermissionsExt as _;

    let project = TestProject::new(
        r#"{"phases":{"only":{"tasks":[{"program":"scripts/analyze"}]}}}"#,
        &[],
    );
    fs::create_dir_all(project.path().join("scripts")).unwrap();
    let program = project.path().join("scripts/analyze");
    fs::write(&program, "#!/bin/sh\n").unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o644)).unwrap();

    assert!(matches!(
        ProjectSpecification::load(project.path()),
        Err(ConfigError::InvalidProgram { path, reason })
            if path == Path::new("scripts/analyze") && reason.contains("executable")
    ));
}

#[cfg(unix)]
#[test]
fn nested_python_task_resolves_its_mamba_environment_during_loading() {
    use std::os::unix::fs::PermissionsExt as _;

    let project = TestProject::new(
        r#"{
          "phases": {
            "analyze": {
              "tasks": [{
                "python": {
                  "script": "scripts/analyze.py",
                  "environment": {
                    "manager": "mamba",
                    "name": "DSES",
                    "executable": "tools/mamba"
                  },
                  "args": ["--publication"]
                },
                "timeout_ms": 9000
              }]
            }
          }
        }"#,
        &[],
    );
    fs::create_dir_all(project.path().join("scripts")).unwrap();
    fs::create_dir_all(project.path().join("tools")).unwrap();
    let script = project.path().join("scripts/analyze.py");
    fs::write(&script, "print('analysis')\n").unwrap();
    let manager = project.path().join("tools/mamba");
    fs::write(&manager, "#!/bin/sh\n").unwrap();
    fs::set_permissions(&manager, fs::Permissions::from_mode(0o755)).unwrap();

    let specification = ProjectSpecification::load(project.path()).unwrap();
    let program = program_task(&specification.phases()[0].tasks()[0]);
    let script = fs::canonicalize(script).unwrap();
    assert_eq!(program.program(), fs::canonicalize(manager).unwrap());
    assert_eq!(program.kind_name(), "python");
    assert_eq!(program.subject(), "analyze.py");
    assert_eq!(program.python_script(), Some(script.as_path()));
    assert_eq!(program.python_environment_manager(), Some("mamba"));
    assert_eq!(program.timeout(), Some(Duration::from_millis(9000)));
    assert_eq!(
        program
            .args()
            .iter()
            .map(|argument| argument.to_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "run",
            "-n",
            "DSES",
            "python",
            script.to_str().unwrap(),
            "--publication"
        ]
    );
}
