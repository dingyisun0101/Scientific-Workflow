//! Target project-specification and resolved-parameter configuration boundary.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use super::*;
use scientific_workflow::state::schema_from_json_value;
use serde::Deserialize;

static NEXT_PROJECT: AtomicUsize = AtomicUsize::new(0);

struct TestProject(PathBuf);

impl TestProject {
    fn new(study: &str, parameter_sections: &[(&str, &str)]) -> Self {
        let mut study: serde_json::Value = serde_json::from_str(study).unwrap();
        let root = study.as_object_mut().unwrap();
        root.entry("threads").or_insert(2.into());
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
                        if task.get("execution_unit").is_some() && task.get("state").is_none() {
                            task.as_object_mut()
                                .unwrap()
                                .insert("state".to_owned(), "default".into());
                        }
                    }
                }
            }
        }
        Self::write(
            &serde_json::to_string_pretty(&study).unwrap(),
            parameter_sections,
        )
    }

    fn new_raw(study: &str, parameter_sections: &[(&str, &str)]) -> Self {
        Self::write(study, parameter_sections)
    }

    fn write(study: &str, parameter_sections: &[(&str, &str)]) -> Self {
        let sequence = NEXT_PROJECT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "scientific-workflow-config-{sequence}-{}",
            std::process::id()
        ));
        fs::create_dir_all(root.join("wf_configs/states")).unwrap();
        fs::write(root.join("wf_configs/study.json"), study).unwrap();
        fs::write(
            root.join("wf_configs/states/default.json"),
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
            root.join("wf_configs/parameters.json"),
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
      "seed": 8675309,
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
            "execution_unit": "unit",
            "timeout_ms": 250
          }],
          "max_concurrency": 2,
          "start_interval_ms": 10000
        },
        "analyze": {
          "after": ["simulate"],
          "tasks": [{
            "execution_unit": "analysis"
          }],
          "timeout_ms": 500,
          "failure_policy": "finish_all"
        }
      }
    }"#
}

fn execution_unit_task(task: &ResolvedTask) -> &ResolvedExecutionUnitParameters {
    match task {
        ResolvedTask::ExecutionUnit { parameters, .. } => parameters,
        ResolvedTask::Program(_) => panic!("expected a unit task"),
    }
}

fn execution_unit_state(task: &ResolvedTask) -> &str {
    match task {
        ResolvedTask::ExecutionUnit { state, .. } => state
            .as_deref()
            .expect("test helper expects an explicit project state"),
        ResolvedTask::Program(_) => panic!("expected a unit task"),
    }
}

fn program_task(task: &ResolvedTask) -> &ResolvedProgramTask {
    match task {
        ResolvedTask::ExecutionUnit { .. } => panic!("expected a program task"),
        ResolvedTask::Program(program) => program,
    }
}

#[test]
fn execution_unit_state_and_state_paths_may_be_omitted_for_later_provider_resolution() {
    let project = TestProject::new_raw(
        r#"{
          "threads": 2,
          "phases": {
            "simulate": {
              "tasks": [{"execution_unit":"unit"}]
            }
          }
        }"#,
        &[("unit", r#"{"steps":1}"#)],
    );

    let specification = ProjectSpecification::load(project.path()).unwrap();
    assert!(specification.state_schemas().is_empty());
    assert!(matches!(
        &specification.phases()[0].tasks()[0],
        ResolvedTask::ExecutionUnit { state: None, .. }
    ));
}

#[test]
fn one_project_root_compiles_every_document_into_a_resolved_specification() {
    let project = TestProject::new(
        manifest(),
        &[
            (
                "unit",
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
    assert_eq!(specification.manifest().master_seed(), Some(8_675_309));
    assert_eq!(specification.manifest().threads(), 2);

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
        .map(execution_unit_task)
        .map(ResolvedExecutionUnitParameters::decode::<Constants>)
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
    assert_eq!(
        execution_unit_task(&simulation.tasks()[3]).execution_unit(),
        "unit"
    );
    assert_eq!(execution_unit_task(&simulation.tasks()[3]).ordinal(), 3);
    assert_eq!(
        execution_unit_task(&simulation.tasks()[3]).timeout(),
        Some(Duration::from_millis(250))
    );
    assert_eq!(
        execution_unit_task(&simulation.tasks()[3]).resolved_value(),
        &serde_json::json!({
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
fn study_threads_are_required_and_positive() {
    let missing = TestProject::new_raw(
        r#"{"phases":{"only":{"tasks":[{"execution_unit":"unit"}]}}}"#,
        &[("unit", "{}")],
    );
    assert!(matches!(
        ProjectSpecification::load(missing.path()),
        Err(ConfigError::InvalidDocument { reason, .. })
            if reason.contains("missing field `threads`")
    ));

    let zero = TestProject::new_raw(
        r#"{"threads":0,"phases":{"only":{"tasks":[{"execution_unit":"unit"}]}}}"#,
        &[("unit", "{}")],
    );
    assert!(matches!(
        ProjectSpecification::load(zero.path()),
        Err(ConfigError::InvalidDocument { pointer, .. }) if pointer == "/threads"
    ));
}

#[test]
fn state_schema_is_parsed_once_by_config_then_semantically_validated_by_state() {
    let project = TestProject::new(
        r#"{"phases":{"only":{"tasks":[{"execution_unit":"unit"}]}}}"#,
        &[("unit", "{}")],
    );
    let specification = ProjectSpecification::load(project.path()).unwrap();
    let document = &specification.state_schemas()["default"];
    let schema = schema_from_json_value(document.path(), document.json_value()).unwrap();

    assert_eq!(schema.field_schemas().len(), 2);
    assert_eq!(schema.template_path(), document.path());
}

#[test]
fn named_state_paths_are_resolved_once_and_selected_explicitly_by_execution_unit_tasks() {
    let project = TestProject::new(
        r#"{
          "paths": {
            "states": {
              "population": "wf_configs/population.json",
              "energy": "wf_configs/states/energy.json"
            }
          },
          "phases": {
            "only": {
              "tasks": [
                {"execution_unit":"unit", "state":"population"},
                {"execution_unit":"analysis", "state":"energy"}
              ]
            }
          }
        }"#,
        &[("unit", "{}"), ("analysis", "{}")],
    );
    fs::write(
        project.path().join("wf_configs/population.json"),
        r#"{"fields":[{"name":"population"}]}"#,
    )
    .unwrap();
    fs::write(
        project.path().join("wf_configs/states/energy.json"),
        r#"{"fields":[{"name":"energy"}]}"#,
    )
    .unwrap();

    let specification = ProjectSpecification::load(project.path()).unwrap();
    assert_eq!(specification.state_schemas().len(), 2);
    assert!(
        specification.state_schemas()["population"]
            .path()
            .ends_with("wf_configs/population.json")
    );
    assert!(
        specification.state_schemas()["energy"]
            .path()
            .ends_with("wf_configs/states/energy.json")
    );
    assert_eq!(
        execution_unit_state(&specification.phases()[0].tasks()[0]),
        "population"
    );
    assert_eq!(
        execution_unit_state(&specification.phases()[0].tasks()[1]),
        "energy"
    );

    let unknown = TestProject::new(
        r#"{
          "paths":{"states":{"known":"wf_configs/states/default.json"}},
          "phases":{"only":{"tasks":[{"execution_unit":"unit","state":"missing"}]}}
        }"#,
        &[("unit", "{}")],
    );
    assert!(matches!(
        ProjectSpecification::load(unknown.path()),
        Err(ConfigError::UnknownState { phase, execution_unit, state })
            if phase == "only" && execution_unit == "unit" && state == "missing"
    ));

    let missing_selector = TestProject::new_raw(
        r#"{
          "threads": 2,
          "paths":{"states":{"known":"wf_configs/states/default.json"}},
          "phases":{"only":{"tasks":[{"execution_unit":"unit"}]}}
        }"#,
        &[("unit", "{}")],
    );
    let specification = ProjectSpecification::load(missing_selector.path()).unwrap();
    assert!(matches!(
        &specification.phases()[0].tasks()[0],
        ResolvedTask::ExecutionUnit { state: None, .. }
    ));
}

#[test]
fn correlated_cases_become_complete_typed_constant_values() {
    let project = TestProject::new(
        r#"{"phases":{"only":{"tasks":[{"execution_unit":"unit"}]}}}"#,
        &[(
            "unit",
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
        .map(execution_unit_task)
        .map(|parameters| parameters.decode::<Constants>().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(values.len(), 2);
    assert_eq!(values[0].shape, [64]);
    assert_eq!((values[0].temperature, values[0].step), (280.0, 0.02));
    assert_eq!((values[1].temperature, values[1].step), (300.0, 0.01));
}

#[test]
fn project_documents_are_strict_and_workflow_owned_objects_reject_unknown_fields() {
    let duplicate = TestProject::new_raw(
        r#"{"paths":{"states":{}},"phases":{"one":{"tasks":[],"tasks":[]}}}"#,
        &[],
    );
    assert!(matches!(
        ProjectSpecification::load(duplicate.path()),
        Err(ConfigError::DuplicateKey { key, .. }) if key == "tasks"
    ));

    let unknown = TestProject::new(
        r#"{"phases":{"one":{"tasks":[{"execution_unit":"x","mystery":true}]}}}"#,
        &[("x", "{}")],
    );
    assert!(matches!(
        ProjectSpecification::load(unknown.path()),
        Err(ConfigError::InvalidDocument { pointer, .. }) if pointer == "/phases/one"
    ));

    let removed_model_field = TestProject::new_raw(
        r#"{
          "threads": 2,
          "paths":{"states":{"default":"wf_configs/states/default.json"}},
          "phases":{"one":{"tasks":[{"model":"x","state":"default"}]}}
        }"#,
        &[("x", "{}")],
    );
    assert!(matches!(
        ProjectSpecification::load(removed_model_field.path()),
        Err(ConfigError::InvalidDocument { reason, .. })
            if reason.contains("unknown field `model`")
    ));

    let invalid_persistence = TestProject::new(
        r#"{"persistence":{"chunk_target_mb":0},"phases":{"one":{"tasks":[{"execution_unit":"x"}]}}}"#,
        &[("x", "{}")],
    );
    assert!(matches!(
        ProjectSpecification::load(invalid_persistence.path()),
        Err(ConfigError::InvalidDocument { pointer, .. }) if pointer == "/"
    ));

    let legacy_chunk_bytes = TestProject::new(
        r#"{"persistence":{"chunk_target_bytes":1048576},"phases":{"one":{"tasks":[{"execution_unit":"x"}]}}}"#,
        &[("x", "{}")],
    );
    assert!(matches!(
        ProjectSpecification::load(legacy_chunk_bytes.path()),
        Err(ConfigError::InvalidDocument { .. })
    ));

    let legacy_queue_bytes = TestProject::new(
        r#"{"persistence":{"queue_capacity_bytes":1048576},"phases":{"one":{"tasks":[{"execution_unit":"x"}]}}}"#,
        &[("x", "{}")],
    );
    assert!(matches!(
        ProjectSpecification::load(legacy_queue_bytes.path()),
        Err(ConfigError::InvalidDocument { .. })
    ));

    let overflowing_chunk_mb = TestProject::new(
        r#"{"persistence":{"chunk_target_mb":18446744073709551615},"phases":{"one":{"tasks":[{"execution_unit":"x"}]}}}"#,
        &[("x", "{}")],
    );
    assert!(matches!(
        ProjectSpecification::load(overflowing_chunk_mb.path()),
        Err(ConfigError::InvalidDocument { pointer, .. })
            if pointer == "/persistence/chunk_target_mb"
    ));

    let duplicate_parameters = TestProject::new(
        r#"{"phases":{"one":{"tasks":[{"execution_unit":"x"}]}}}"#,
        &[("x", r#"{"value":1,"value":2}"#)],
    );
    assert!(matches!(
        ProjectSpecification::load(duplicate_parameters.path()),
        Err(ConfigError::DuplicateKey { key, .. }) if key == "value"
    ));
}

#[test]
fn workflow_project_root_requires_reserved_wf_configs_documents() {
    let legacy_layout = TestProject::new(
        r#"{"phases":{"only":{"tasks":[{"execution_unit":"unit"}]}}}"#,
        &[("unit", "{}")],
    );
    fs::rename(
        legacy_layout.path().join("wf_configs"),
        legacy_layout.path().join("config"),
    )
    .unwrap();
    assert!(matches!(
        ProjectSpecification::load(legacy_layout.path()),
        Err(ConfigError::Read { path, .. }) if path.ends_with("wf_configs")
    ));

    let missing_study = TestProject::new(
        r#"{"phases":{"only":{"tasks":[{"execution_unit":"unit"}]}}}"#,
        &[("unit", "{}")],
    );
    fs::rename(
        missing_study.path().join("wf_configs/study.json"),
        missing_study.path().join("study.json"),
    )
    .unwrap();
    assert!(matches!(
        ProjectSpecification::load(missing_study.path()),
        Err(ConfigError::Read { path, .. }) if path.ends_with("wf_configs/study.json")
    ));

    let missing_parameters = TestProject::new(
        r#"{"phases":{"only":{"tasks":[{"execution_unit":"unit"}]}}}"#,
        &[("unit", "{}")],
    );
    fs::remove_file(missing_parameters.path().join("wf_configs/parameters.json")).unwrap();
    assert!(matches!(
        ProjectSpecification::load(missing_parameters.path()),
        Err(ConfigError::Read { path, .. }) if path.ends_with("wf_configs/parameters.json")
    ));
}

#[test]
fn state_documents_may_be_anywhere_beneath_wf_configs_but_not_outside_it() {
    let project = TestProject::new_raw(
        r#"{
          "threads": 2,
          "paths":{"states":{"outside":"state.json"}},
          "phases":{"only":{"tasks":[{"execution_unit":"unit","state":"outside"}]}}
        }"#,
        &[("unit", "{}")],
    );
    fs::write(
        project.path().join("state.json"),
        r#"{"fields":[{"name":"population"}]}"#,
    )
    .unwrap();
    assert!(matches!(
        ProjectSpecification::load(project.path()),
        Err(ConfigError::PathOutsideConfig { path, config_root })
            if path.ends_with("state.json") && config_root.ends_with("wf_configs")
    ));

    let absolute = TestProject::new(
        r#"{"phases":{"only":{"tasks":[{"execution_unit":"unit"}]}}}"#,
        &[("unit", "{}")],
    );
    let manifest_path = absolute.path().join("wf_configs/study.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["paths"]["states"]["default"] = absolute
        .path()
        .join("wf_configs/states/default.json")
        .to_string_lossy()
        .into_owned()
        .into();
    fs::write(manifest_path, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
    assert!(matches!(
        ProjectSpecification::load(absolute.path()),
        Err(ConfigError::InvalidDocument { pointer, reason, .. })
            if pointer == "/paths/states/default" && reason.contains("project root")
    ));
}

#[test]
fn removed_per_task_input_paths_are_rejected() {
    let project = TestProject::new(
        r#"{"phases":{"one":{"tasks":[{"execution_unit":"x","input":"inputs/x.json"}]}}}"#,
        &[("x", "{}")],
    );
    assert!(matches!(
        ProjectSpecification::load(project.path()),
        Err(ConfigError::InvalidDocument { .. })
    ));
}

#[test]
fn every_execution_unit_key_requires_its_canonical_parameter_section() {
    let project = TestProject::new(
        r#"{"phases":{"one":{"tasks":[{"execution_unit":"missing"}]}}}"#,
        &[("another_unit", "{}")],
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
        r#"{"phases":{"one":{"after":["absent"],"tasks":[{"execution_unit":"x"}]}}}"#,
        &[("x", "{}")],
    );
    assert!(matches!(
        ProjectSpecification::load(missing.path()),
        Err(ConfigError::UnknownDependency { .. })
    ));

    let cycle = TestProject::new(
        r#"{
          "phases": {
            "one": {"after":["two"],"tasks":[{"execution_unit":"x"}]},
            "two": {"after":["one"],"tasks":[{"execution_unit":"x"}]}
          }
        }"#,
        &[("x", "{}")],
    );
    assert!(matches!(
        ProjectSpecification::load(cycle.path()),
        Err(ConfigError::InvalidDocument { .. })
    ));

    let mixed = TestProject::new(
        r#"{"phases":{"one":{"tasks":[{"execution_unit":"x"}]}}}"#,
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
fn typed_decode_errors_retain_execution_unit_source_and_combination() {
    let project = TestProject::new(
        r#"{"phases":{"one":{"tasks":[{"execution_unit":"unit"}]}}}"#,
        &[("unit", r#"{"steps":"wrong"}"#)],
    );
    let specification = ProjectSpecification::load(project.path()).unwrap();
    #[derive(Deserialize)]
    struct Constants {
        #[allow(dead_code)]
        steps: u64,
    }
    assert!(matches!(
        execution_unit_task(&specification.phases()[0].tasks()[0]).decode::<Constants>(),
        Err(ConfigError::DecodeExecutionUnitConstants {
            execution_unit,
            ordinal: 0,
            ..
        }) if execution_unit == "unit"
    ));
}

#[test]
fn central_config_captures_all_project_parameters_in_one_namespace() {
    let project = TestProject::new(
        r#"{"phases":{"only":{"tasks":[{"execution_unit":"unit"}]}}}"#,
        &[
            ("unit", r#"{"steps":1}"#),
            ("plot", r#"{"title":"Captured configuration","dpi":160}"#),
        ],
    );

    let specification = ProjectSpecification::load(project.path()).unwrap();
    let snapshot: serde_json::Value =
        serde_json::from_slice(specification.config().snapshot().bytes()).unwrap();
    assert_eq!(
        snapshot["study"]["phases"]["only"]["tasks"][0]["execution_unit"],
        "unit"
    );
    assert_eq!(
        snapshot["config"]["states/default.json"]["fields"][0]["name"],
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
        r#"{"phases":{"only":{"tasks":[{"execution_unit":"unit"}]}}}"#,
        &[("unit", "{}")],
    );
    fs::write(
        project.path().join("wf_configs/unreferenced.json"),
        r#"{"duplicate":1,"duplicate":2}"#,
    )
    .unwrap();
    assert!(matches!(
        ProjectSpecification::load(project.path()),
        Err(ConfigError::DuplicateKey { key, .. }) if key == "duplicate"
    ));
}

#[test]
fn diagnostics_escape_authored_json_pointer_keys() {
    let phase = TestProject::new(r#"{"phases":{"bad/name~":{"tasks":[]}}}"#, &[]);
    assert!(matches!(
        ProjectSpecification::load(phase.path()),
        Err(ConfigError::InvalidDocument { pointer, .. })
            if pointer == "/phases/bad~1name~0/tasks"
    ));

    let parameters = TestProject::new(
        r#"{"phases":{"only":{"tasks":[{"execution_unit":"a/b~c"}]}}}"#,
        &[("different", "{}")],
    );
    assert!(matches!(
        ProjectSpecification::load(parameters.path()),
        Err(ConfigError::InvalidDocument { pointer, .. }) if pointer == "/a~1b~0c"
    ));
}

#[test]
fn empty_and_overlapping_expansion_markers_are_rejected() {
    for source in [
        r#"{"choice":{"$sweep":[]}}"#,
        r#"{"$cases":[]}"#,
        r#"{"$cases":[{"x":1},{"y":2}]}"#,
        r#"{"$unknown":[1,2]}"#,
    ] {
        let project = TestProject::new(
            r#"{"phases":{"only":{"tasks":[{"execution_unit":"unit"}]}}}"#,
            &[("unit", source)],
        );
        assert!(matches!(
            ProjectSpecification::load(project.path()),
            Err(ConfigError::InvalidDocument { .. })
        ));
    }
}

#[cfg(unix)]
#[test]
fn json_file_symlinks_preserve_authored_keys_and_enforce_containment() {
    use std::os::unix::fs::symlink;

    let contained = TestProject::new(
        r#"{
          "paths":{"states":{"alias":"wf_configs/alias.json"}},
          "phases":{"only":{"tasks":[{"execution_unit":"unit","state":"alias"}]}}
        }"#,
        &[("unit", "{}")],
    );
    symlink(
        "states/default.json",
        contained.path().join("wf_configs/alias.json"),
    )
    .unwrap();
    let specification = ProjectSpecification::load(contained.path()).unwrap();
    let snapshot: serde_json::Value =
        serde_json::from_slice(specification.config().snapshot().bytes()).unwrap();
    assert_eq!(
        snapshot["config"]["alias.json"],
        snapshot["config"]["states/default.json"]
    );
    assert!(
        specification.state_schemas()["alias"]
            .path()
            .ends_with("wf_configs/states/default.json")
    );

    let escaping = TestProject::new(
        r#"{"phases":{"only":{"tasks":[{"program":"missing"}]}}}"#,
        &[],
    );
    fs::write(escaping.path().join("outside.json"), "{}").unwrap();
    symlink(
        "../outside.json",
        escaping.path().join("wf_configs/escape.json"),
    )
    .unwrap();
    assert!(matches!(
        ProjectSpecification::load(escaping.path()),
        Err(ConfigError::PathOutsideConfig { .. })
    ));
}

#[cfg(unix)]
#[test]
fn non_utf8_config_document_paths_are_rejected_without_lossy_snapshot_keys() {
    use std::os::unix::ffi::OsStringExt as _;

    let project = TestProject::new(
        r#"{"phases":{"only":{"tasks":[{"program":"missing"}]}}}"#,
        &[],
    );
    let name = std::ffi::OsString::from_vec(b"invalid-\xff.json".to_vec());
    fs::write(project.path().join("wf_configs").join(name), "{}").unwrap();
    assert!(matches!(
        ProjectSpecification::load(project.path()),
        Err(ConfigError::NonUtf8Path { context, .. }) if context == "configuration document"
    ));
}

#[cfg(unix)]
#[test]
fn non_utf8_project_roots_are_rejected_before_json_provenance_is_built() {
    use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};

    let mut project = TestProject::new(
        r#"{"phases":{"only":{"tasks":[{"program":"missing"}]}}}"#,
        &[],
    );
    let mut renamed = project.path().as_os_str().as_bytes().to_vec();
    renamed.extend_from_slice(b"-\xff");
    let renamed = PathBuf::from(std::ffi::OsString::from_vec(renamed));
    fs::rename(project.path(), &renamed).unwrap();
    project.0 = renamed;

    assert!(matches!(
        ProjectSpecification::load(project.path()),
        Err(ConfigError::NonUtf8Path { context, .. }) if context == "project root"
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
fn program_seed_requests_require_the_master_seed_and_are_retained_semantically() {
    let project = TestProject::new_raw(
        r#"{
          "threads": 2,
          "seed": 42,
          "phases":{"only":{"tasks":[{
            "program":"/bin/true",
            "seed":{"purpose":"target-initial-conditions"}
          }]}}
        }"#,
        &[],
    );
    let specification = ProjectSpecification::load(project.path()).unwrap();
    assert_eq!(
        program_task(&specification.phases()[0].tasks()[0]).seed_purpose(),
        Some("target-initial-conditions")
    );

    let missing_master = TestProject::new_raw(
        r#"{
          "threads": 2,
          "phases":{"only":{"tasks":[{
            "program":"/bin/true",
            "seed":{"purpose":"target-initial-conditions"}
          }]}}
        }"#,
        &[],
    );
    assert!(matches!(
        ProjectSpecification::load(missing_master.path()),
        Err(ConfigError::InvalidDocument { pointer, reason, .. })
            if pointer == "/phases/only/tasks/0/seed"
                && reason.contains("top-level `seed`")
    ));

    let invalid_purpose = TestProject::new_raw(
        r#"{
          "threads": 2,
          "seed": 42,
          "phases":{"only":{"tasks":[{
            "program":"/bin/true",
            "seed":{"purpose":" "}
          }]}}
        }"#,
        &[],
    );
    assert!(matches!(
        ProjectSpecification::load(invalid_purpose.path()),
        Err(ConfigError::InvalidDocument { pointer, .. })
            if pointer == "/phases/only/tasks/0/seed/purpose"
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

#[cfg(unix)]
#[test]
fn every_supported_explicit_python_environment_is_lowered_during_loading() {
    use std::os::unix::fs::PermissionsExt as _;

    let project = TestProject::new(
        r#"{
          "phases":{"analyze":{"tasks":[
            {"python":{"script":"scripts/analyze.py","environment":{"manager":"venv","path":"env"}}},
            {"python":{"script":"scripts/analyze.py","environment":{"manager":"conda","name":"DSES","executable":"tools/conda"}}},
            {"python":{"script":"scripts/analyze.py","environment":{"manager":"uv","project":"uv-project","executable":"tools/uv"}}},
            {"python":{"script":"scripts/analyze.py","environment":{"manager":"poetry","project":"poetry-project","executable":"tools/poetry"}}}
          ]}}
        }"#,
        &[],
    );
    for directory in [
        "scripts",
        "env/bin",
        "tools",
        "uv-project",
        "poetry-project",
    ] {
        fs::create_dir_all(project.path().join(directory)).unwrap();
    }
    fs::write(project.path().join("scripts/analyze.py"), "print('ok')\n").unwrap();
    for executable in ["env/bin/python", "tools/conda", "tools/uv", "tools/poetry"] {
        let path = project.path().join(executable);
        fs::write(&path, "#!/bin/sh\n").unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    let specification = ProjectSpecification::load(project.path()).unwrap();
    let programs = specification.phases()[0]
        .tasks()
        .iter()
        .map(program_task)
        .collect::<Vec<_>>();
    assert_eq!(
        programs
            .iter()
            .map(|program| program.python_environment_manager().unwrap())
            .collect::<Vec<_>>(),
        ["venv", "conda", "uv", "poetry"]
    );
    assert_eq!(programs[1].args()[0..4], ["run", "-n", "DSES", "python"]);
    assert_eq!(programs[2].args()[0], "run");
    assert_eq!(programs[2].args()[1], "--project");
    assert_eq!(programs[3].args()[0], "--directory");
    assert_eq!(programs[3].args()[2], "run");
}
