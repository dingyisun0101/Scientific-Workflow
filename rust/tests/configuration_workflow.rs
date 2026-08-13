//! Logged integration coverage for standard project configuration.
//!
//! Run with:
//!
//! ```text
//! cargo test --test configuration_workflow -- --nocapture
//! ```

use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use scientific_workflow::prelude::basics::*;
use serde::Serialize;
use serde::ser::Error as _;
use serde_json::Value;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct UnencodableSelector;

impl Serialize for UnencodableSelector {
    fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        Err(S::Error::custom("intentional selector encoding failure"))
    }
}

struct TempWorkspace {
    root: PathBuf,
}

impl TempWorkspace {
    fn new() -> Self {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "scientific-workflow-configuration-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        Self { root }
    }

    fn project(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.root) {
            eprintln!("[cleanup] {}: {error}", self.root.display());
        }
    }
}

fn fixture_project(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/configuration")
        .join(name)
}

fn write_project(root: &Path, fixed: &[u8], sweep: &[u8], paths: &[u8]) {
    let configuration = root.join("config");
    fs::create_dir_all(&configuration).unwrap();
    fs::write(configuration.join("fixed.json"), fixed).unwrap();
    fs::write(configuration.join("sweep.json"), sweep).unwrap();
    fs::write(configuration.join("paths.json"), paths).unwrap();
}

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn model_owned_schema_removes_the_project_state_file() {
    let workspace = TempWorkspace::new();
    let root = workspace.project("model-owned-schema");
    write_project(
        &root,
        br#"{"iterations":10}"#,
        br#"{"mode":"cartesian","axes":[]}"#,
        br#"{"recordings":"recordings"}"#,
    );
    let schema_path = fixture_project("cartesian_project").join("config/state.json");
    let schema = SystemStateSchema::load_json_template(&schema_path).unwrap();

    assert!(matches!(
        ScientificProject::load(&root),
        Err(ScientificProjectError::State(
            StateError::TemplateRead { .. }
        ))
    ));

    let project = ScientificProject::load_with_state_schema(&root, schema).unwrap();
    assert_eq!(project.task_count(), 1);
    assert_eq!(project.state_schema().template_path(), schema_path);
    assert!(project.state_schema().contains_field("population"));
    assert_eq!(
        project.resolve_path("recordings").unwrap(),
        root.join("recordings")
    );
    assert!(!root.join("config/state.json").exists());
}

#[test]
fn project_configuration_expands_round_trips_and_rejects_ambiguity() {
    assert_send_sync::<ParameterSpace>();
    assert_send_sync::<TaskParameters>();
    assert_send_sync::<TaskParametersIter>();
    assert_send_sync::<TaskConfig>();
    assert_send_sync::<TaskConfigIter>();
    assert_send_sync::<MatchingTaskConfigIter>();
    assert_send_sync::<ProjectPaths>();
    assert_send_sync::<ProjectConfig>();
    assert_send_sync::<ScientificProject>();
    assert_send_sync::<ExecutionScope>();

    let cartesian_root = fixture_project("cartesian_project");
    let scientific_project = ScientificProject::load(&cartesian_root).unwrap();
    assert_eq!(scientific_project.state_schema().len(), 2);
    assert!(
        scientific_project
            .state_schema()
            .contains_field("population")
    );
    assert_eq!(scientific_project.parameters().task_count(), 6);
    assert_eq!(scientific_project.task_count(), 6);
    assert_eq!(scientific_project.task_configs().count(), 6);
    assert_eq!(scientific_project.task_config(5).unwrap().task_ordinal(), 5);
    assert_eq!(
        scientific_project
            .task_configs_matching("temperature", 300.0)
            .unwrap()
            .count(),
        3
    );
    assert!(matches!(
        scientific_project.unique_task_config_matching("temperature", 300.0),
        Err(ConfigurationError::AmbiguousTaskConfiguration { key })
            if key == "temperature"
    ));
    assert_eq!(
        scientific_project.resolve_path("output_root").unwrap(),
        cartesian_root.join("results")
    );
    assert!(format!("{scientific_project:?}").contains("state_fields"));
    let project = ProjectConfig::load(&cartesian_root).unwrap();
    assert_eq!(project.task_count(), 6);
    assert_eq!(project.project_root(), cartesian_root);
    assert_eq!(
        project.configuration_directory(),
        cartesian_root.join("config")
    );
    let parameters = project.parameters();
    assert_eq!(
        parameters.configuration_directory(),
        cartesian_root.join("config")
    );
    assert_eq!(parameters.fixed_parameter_count(), 3);
    assert_eq!(parameters.sweep_parameter_count(), 2);
    assert_eq!(parameters.parameter_count(), 5);
    assert_eq!(parameters.task_count(), 6);
    assert!(parameters.contains_parameter("temperature"));
    assert!(!parameters.contains_parameter("missing"));
    assert_eq!(
        parameters.fixed_keys().collect::<Vec<_>>(),
        ["physical_time_increment", "lattice_shape", "solver"]
    );
    assert_eq!(
        parameters.sweep_keys().collect::<Vec<_>>(),
        ["temperature", "seed"]
    );
    assert_eq!(
        parameters.fixed_source_json(),
        fs::read(cartesian_root.join("config/fixed.json"))
            .unwrap()
            .as_slice()
    );
    assert_eq!(
        parameters.sweep_source_json(),
        fs::read(cartesian_root.join("config/sweep.json"))
            .unwrap()
            .as_slice()
    );
    println!(
        "[load] fixed={} swept={} parameters={} tasks={} paths={}",
        parameters.fixed_parameter_count(),
        parameters.sweep_parameter_count(),
        parameters.parameter_count(),
        parameters.task_count(),
        project.paths().len()
    );

    let combinations = parameters
        .tasks()
        .map(|task| {
            (
                task.task_ordinal(),
                task.decode_value::<f64>("temperature").unwrap(),
                task.decode_value::<u64>("seed").unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        combinations,
        [
            (0, 280.0, 7),
            (1, 280.0, 11),
            (2, 280.0, 13),
            (3, 300.0, 7),
            (4, 300.0, 11),
            (5, 300.0, 13),
        ]
    );

    let complete_configs = project
        .task_configs()
        .map(|task| {
            (
                task.task_ordinal(),
                task.decode_value::<f64>("temperature").unwrap(),
                task.decode_value::<u64>("seed").unwrap(),
                task.resolve_path("output_root").unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(complete_configs.len(), 6);
    assert_eq!(complete_configs[0].0, 0);
    assert_eq!(complete_configs[5].0, 5);
    assert_eq!(complete_configs[0].1, 280.0);
    assert_eq!(complete_configs[5].2, 13);
    assert!(
        complete_configs
            .iter()
            .all(|task| task.3 == cartesian_root.join("results"))
    );

    let matching = project.task_configs_matching("temperature", 280.0).unwrap();
    assert_eq!(matching.size_hint(), (0, Some(6)));
    let selected = matching
        .map(|task| task.decode_value::<u64>("seed").unwrap())
        .collect::<Vec<_>>();
    assert_eq!(selected, [7, 11, 13]);
    assert!(matches!(
        project.unique_task_config_matching("temperature", 280.0),
        Err(ConfigurationError::AmbiguousTaskConfiguration { key })
            if key == "temperature"
    ));
    assert!(matches!(
        project.unique_task_config_matching("temperature", 999.0),
        Err(ConfigurationError::NoMatchingTaskConfiguration { key })
            if key == "temperature"
    ));
    assert!(matches!(
        project.task_configs_matching("solver", "euler"),
        Err(ConfigurationError::UnknownSweepParameter { key }) if key == "solver"
    ));
    let selector_error = project
        .task_configs_matching("temperature", UnencodableSelector)
        .unwrap_err();
    assert!(matches!(
        selector_error,
        ConfigurationError::EncodeTaskSelection { ref key, .. }
            if key == "temperature"
    ));
    assert!(selector_error.source().unwrap().is::<serde_json::Error>());

    let complete = project.task_config(4).unwrap();
    let complete_clone = complete.clone();
    assert_eq!(complete.task_ordinal(), 4);
    assert_eq!(complete.parameters().task_ordinal(), 4);
    assert_eq!(complete.paths().len(), 3);
    assert_eq!(complete.require_value("temperature").unwrap(), 300.0);
    assert!(std::ptr::eq(
        complete.value("solver").unwrap(),
        complete_clone.value("solver").unwrap()
    ));
    assert!(std::ptr::eq(
        complete.paths().path("output_root").unwrap(),
        complete_clone.paths().path("output_root").unwrap()
    ));
    assert!(format!("{complete:?}").contains("task_ordinal"));
    let detached_tasks = project.clone().task_configs();
    assert_eq!(detached_tasks.size_hint(), (6, Some(6)));
    let mut copied_tasks = detached_tasks.clone();
    assert_eq!(copied_tasks.next().unwrap().task_ordinal(), 0);
    assert_eq!(detached_tasks.count(), 6);
    assert!(format!("{copied_tasks:?}").contains("parameters"));
    assert!(
        format!(
            "{:?}",
            project.task_configs_matching("temperature", 300.0).unwrap()
        )
        .contains("temperature")
    );
    println!(
        "[task-config] all={} selected={} shared_paths=true exact_match=true ambiguity_rejected=true",
        complete_configs.len(),
        selected.len()
    );
    println!(
        "[cartesian] tasks={} last_axis_fastest=true first=({}, {}) last=({}, {})",
        combinations.len(),
        combinations[0].1,
        combinations[0].2,
        combinations[5].1,
        combinations[5].2
    );

    let first = parameters.task(0).unwrap();
    let second = parameters.task(1).unwrap();
    let (dt, shape, solver, temperature, seed): (f64, Vec<u64>, Value, f64, u64) = first
        .decode_values((
            "physical_time_increment",
            "lattice_shape",
            "solver",
            "temperature",
            "seed",
        ))
        .unwrap();
    assert_eq!(dt, 0.125);
    assert_eq!(shape, [4, 8]);
    assert_eq!(solver["method"], "rk4");
    assert_eq!((temperature, seed), (280.0, 7));
    let twelve: (u64, u64, u64, u64, u64, u64, u64, u64, u64, u64, u64, u64) = first
        .decode_values((
            "seed", "seed", "seed", "seed", "seed", "seed", "seed", "seed", "seed", "seed", "seed",
            "seed",
        ))
        .unwrap();
    assert_eq!(twelve, (7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7));
    let copied = first.clone();
    assert_eq!(first.task_ordinal(), 0);
    assert_eq!(first.len(), 5);
    assert!(!first.is_empty());
    assert!(first.contains("solver"));
    assert!(!first.contains("unknown"));
    assert_eq!(
        first.keys().collect::<Vec<_>>(),
        [
            "physical_time_increment",
            "lattice_shape",
            "solver",
            "temperature",
            "seed"
        ]
    );
    assert_eq!(first.iter().count(), 5);
    assert!(std::ptr::eq(
        first.value("physical_time_increment").unwrap(),
        second.value("physical_time_increment").unwrap()
    ));
    assert!(std::ptr::eq(
        first.value("temperature").unwrap(),
        second.value("temperature").unwrap()
    ));
    assert!(std::ptr::eq(
        first.value("seed").unwrap(),
        copied.value("seed").unwrap()
    ));
    assert_eq!(
        first.require_value("lattice_shape").unwrap(),
        &serde_json::json!([4, 8])
    );
    assert_eq!(
        first.decode_value::<Vec<usize>>("lattice_shape").unwrap(),
        [4, 8]
    );
    let resolved_json = first.to_json().unwrap();
    let resolved: Value = serde_json::from_str(&resolved_json).unwrap();
    assert_eq!(resolved["physical_time_increment"], 0.125);
    assert_eq!(resolved["temperature"], 280.0);
    assert_eq!(resolved["seed"], 7);
    let key_positions = [
        resolved_json.find("physical_time_increment").unwrap(),
        resolved_json.find("lattice_shape").unwrap(),
        resolved_json.find("solver").unwrap(),
        resolved_json.find("temperature").unwrap(),
        resolved_json.find("seed").unwrap(),
    ];
    assert!(key_positions.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(format!("{parameters:?}").contains("task_count"));
    assert!(format!("{first:?}").contains("task_ordinal"));
    println!(
        "[ownership] fixed_shared=true selected_shared=true task_clone_shared=true merged_map_allocated=false"
    );

    let mut owning_iter = parameters.tasks();
    assert_eq!(owning_iter.size_hint(), (6, Some(6)));
    let mut copied_iter = owning_iter.clone();
    assert_eq!(owning_iter.next().unwrap().task_ordinal(), 0);
    assert_eq!(copied_iter.next().unwrap().task_ordinal(), 0);
    assert!(format!("{owning_iter:?}").contains("next"));
    let independent_iter = project.parameters().tasks();
    let project_clone = project.clone();
    drop(project);
    assert_eq!(independent_iter.count(), 6);
    let (separated_parameters, separated_paths) = project_clone.clone().into_parts();
    assert_eq!(separated_parameters.task_count(), 6);
    assert_eq!(separated_paths.len(), 3);

    let paths = project_clone.paths();
    assert_eq!(paths.project_root(), cartesian_root);
    assert_eq!(
        paths.source_path(),
        cartesian_root.join("config/paths.json")
    );
    assert_eq!(
        paths.source_json(),
        fs::read(cartesian_root.join("config/paths.json"))
            .unwrap()
            .as_slice()
    );
    assert!(!paths.is_empty());
    assert!(paths.contains("input_data"));
    assert_eq!(paths.path("input_data"), Some(Path::new("data/input.json")));
    assert_eq!(
        paths.require_path("output_root").unwrap(),
        Path::new("results")
    );
    assert_eq!(
        paths.resolve_path("input_data").unwrap(),
        cartesian_root.join("data/input.json")
    );
    assert_eq!(
        paths.keys().collect::<Vec<_>>(),
        ["input_data", "output_root", "cache"]
    );
    assert_eq!(paths.iter().count(), 3);
    assert!(format!("{paths:?}").contains("entries"));
    println!(
        "[paths] declared={} relative_resolution=true canonicalization=false existence_check=false",
        paths.len()
    );

    let workspace = TempWorkspace::new();
    let generated_scope =
        ExecutionScope::create_generated(workspace.project("recordings")).unwrap();
    assert!(generated_scope.directory().is_dir());
    assert!(generated_scope.created_at_utc().unwrap().ends_with('Z'));
    let task_recording = generated_scope.task_recording_directory(12);
    assert!(task_recording.ends_with("task-000012"));
    assert!(!task_recording.exists());
    let reopened = ExecutionScope::open_existing(generated_scope.directory()).unwrap();
    assert_eq!(reopened.directory(), generated_scope.directory());
    assert_eq!(reopened.created_at_utc(), None);
    let named_scope =
        ExecutionScope::create_named(workspace.project("recordings"), "reference-run").unwrap();
    assert!(named_scope.directory().ends_with("reference-run"));
    assert!(matches!(
        ExecutionScope::create_named(workspace.project("recordings"), "../unsafe"),
        Err(ExecutionScopeError::InvalidName { .. })
    ));
    println!(
        "[execution-scope] generated={} named={} task_path={} timestamp_managed=true",
        generated_scope.directory().display(),
        named_scope.directory().display(),
        task_recording.display()
    );
    let copied_root = workspace.project("copied");
    project_clone.write_source_config(&copied_root).unwrap();
    for name in ["fixed.json", "sweep.json", "paths.json"] {
        assert_eq!(
            fs::read(cartesian_root.join("config").join(name)).unwrap(),
            fs::read(copied_root.join("config").join(name)).unwrap()
        );
    }
    let copied_project = ProjectConfig::load(&copied_root).unwrap();
    assert_eq!(copied_project.parameters().task_count(), 6);
    let overwrite = project_clone
        .write_source_config(&copied_root)
        .expect_err("exact export must never replace an existing config directory");
    assert!(matches!(
        overwrite,
        ConfigurationError::WriteConfigurationFile { ref path, .. }
            if path == &copied_root.join("config")
    ));
    assert_eq!(
        overwrite
            .source()
            .and_then(|source| source.downcast_ref::<io::Error>())
            .map(io::Error::kind),
        Some(io::ErrorKind::AlreadyExists)
    );
    println!(
        "[round-trip] fixed_bytes=true sweep_bytes=true paths_bytes=true reload=true overwrite_rejected=true"
    );

    let bounds = separated_parameters.task(6).unwrap_err();
    assert!(matches!(
        bounds,
        ConfigurationError::TaskOrdinalOutOfBounds {
            ordinal: 6,
            task_count: 6
        }
    ));
    assert!(matches!(
        first.require_value("unknown"),
        Err(ConfigurationError::UnknownTaskParameter { task_ordinal: 0, key })
            if key == "unknown"
    ));
    let decode = first.decode_value::<String>("temperature").unwrap_err();
    assert!(matches!(
        decode,
        ConfigurationError::DecodeTaskParameter { task_ordinal: 0, ref key, .. }
            if key == "temperature"
    ));
    assert!(decode.source().unwrap().is::<serde_json::Error>());
    assert!(matches!(
        separated_paths.require_path("unknown"),
        Err(ConfigurationError::UnknownProjectPath { key }) if key == "unknown"
    ));
    println!("[lookup-errors] bounds=true missing=true type=true path=true");

    let cases_root = fixture_project("cases_project");
    let cases = ProjectConfig::load(&cases_root).unwrap();
    assert_eq!(cases.parameters().task_count(), 3);
    assert_eq!(
        cases.parameters().sweep_keys().collect::<Vec<_>>(),
        ["temperature", "physical_time_increment"]
    );
    let correlated = cases
        .parameters()
        .tasks()
        .map(|task| {
            (
                task.decode_value::<f64>("temperature").unwrap(),
                task.decode_value::<f64>("physical_time_increment").unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(correlated, [(275.0, 0.2), (290.0, 0.1), (310.0, 0.05)]);
    let unique = cases
        .unique_task_config_matching("temperature", 290.0)
        .unwrap();
    assert_eq!(unique.task_ordinal(), 1);
    assert_eq!(
        unique
            .decode_value::<f64>("physical_time_increment")
            .unwrap(),
        0.1
    );
    assert_eq!(
        cases
            .parameters()
            .task(1)
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        [
            "lattice_shape",
            "integrator",
            "temperature",
            "physical_time_increment"
        ]
    );
    println!(
        "[cases] tasks={} correlated=true key_order_normalized=true",
        correlated.len()
    );

    let fixed_only_root = workspace.project("fixed-only");
    write_project(
        &fixed_only_root,
        br#"{}"#,
        br#"{"mode":"cartesian","axes":[]}"#,
        br#"{}"#,
    );
    let fixed_only = ProjectConfig::load(&fixed_only_root).unwrap();
    assert_eq!(fixed_only.parameters().task_count(), 1);
    let empty_task = fixed_only.parameters().task(0).unwrap();
    assert!(empty_task.is_empty());
    assert_eq!(empty_task.to_json().unwrap(), "{}");
    assert!(fixed_only.paths().is_empty());

    let duplicate_root = workspace.project("duplicate");
    write_project(
        &duplicate_root,
        br#"{"solver":{"method":"rk4","method":"euler"}}"#,
        br#"{"mode":"cartesian","axes":[]}"#,
        br#"{}"#,
    );
    assert!(matches!(
        ProjectConfig::load(&duplicate_root),
        Err(ConfigurationError::DuplicateConfigurationKey { key, .. }) if key == "method"
    ));

    let overlap_root = workspace.project("overlap");
    write_project(
        &overlap_root,
        br#"{"temperature":300}"#,
        br#"{"mode":"cartesian","axes":[{"name":"temperature","values":[280,300]}]}"#,
        br#"{}"#,
    );
    assert!(matches!(
        ProjectConfig::load(&overlap_root),
        Err(ConfigurationError::FixedSweepKeyConflict { key, .. })
            if key == "temperature"
    ));

    let inconsistent_root = workspace.project("inconsistent");
    write_project(
        &inconsistent_root,
        br#"{}"#,
        br#"{"mode":"cases","cases":[{"a":1},{"b":2}]}"#,
        br#"{}"#,
    );
    assert!(matches!(
        ProjectConfig::load(&inconsistent_root),
        Err(ConfigurationError::InvalidConfigurationDocument { ref path, .. })
            if path == &inconsistent_root.join("config/sweep.json")
    ));

    let invalid_path_root = workspace.project("invalid-path");
    write_project(
        &invalid_path_root,
        br#"{}"#,
        br#"{"mode":"cartesian","axes":[]}"#,
        br#"{"output_root":42}"#,
    );
    assert!(matches!(
        ProjectConfig::load(&invalid_path_root),
        Err(ConfigurationError::InvalidConfigurationDocument { ref path, .. })
            if path == &invalid_path_root.join("config/paths.json")
    ));
    println!(
        "[validation] fixed_only=true nested_duplicate=true overlap=true inconsistent_cases=true invalid_path=true"
    );
    println!("[result] configuration_workflow=passed");
}
