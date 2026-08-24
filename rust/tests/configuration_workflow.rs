use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use scientific_workflow::configuration::{
    ConfigurationError, ProjectPaths, ReplicateExecutionMode, ReplicateFailurePolicy,
    ResolvedConfiguration, StudyConfiguration, StudySettings,
};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

fn study_directory(name: &str, parameters: &str) -> PathBuf {
    let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "scientific-workflow-{name}-{}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(root.join("config")).unwrap();
    fs::write(root.join("config/parameters.json"), parameters).unwrap();
    root
}

#[test]
fn phase_expansion_combines_global_group_and_local_scopes() {
    let root = study_directory(
        "scoped-cartesian",
        r#"{
          "global":{
            "temperature":{"$sweep":[280.0,300.0]},
            "solver":{"step":0.01}
          },
          "phase_group":{"models":{
            "shared":{
              "seed":{"$sweep":[7,11]},
              "solver":{"tolerance":0.000001}
            },
            "phase":{
              "glv":{"solver":{"method":"adaptive"}},
              "lattice":{
                "size":{"$sweep":[4,8]},
                "solver":{"method":"rk4"}
              }
            }
          }}
        }"#,
    );
    let study = StudyConfiguration::load(&root).unwrap();
    let glv = study.phase("models", "glv").unwrap();
    let lattice = study.phase("models", "lattice").unwrap();

    assert_eq!(glv.combination_count(), 4);
    assert_eq!(lattice.combination_count(), 8);
    assert_eq!(
        lattice.fixed_keys().collect::<Vec<_>>(),
        ["/solver/step", "/solver/tolerance", "/solver/method"]
    );
    assert_eq!(lattice.sweep_keys().len(), 3);
    assert_eq!(
        lattice.sweep_keys().collect::<Vec<_>>(),
        ["/temperature", "/seed", "/size"]
    );
    let resolved = lattice
        .combinations()
        .map(|configuration| {
            let values: (f64, u64, usize) = configuration
                .decode_values(("/temperature", "/seed", "/size"))
                .unwrap();
            (
                configuration.global_ordinal(),
                configuration.group_ordinal(),
                configuration.phase_ordinal(),
                values,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(resolved[0], (0, 0, 0, (280.0, 7, 4)));
    assert_eq!(resolved[1], (0, 0, 1, (280.0, 7, 8)));
    assert_eq!(resolved[7], (1, 1, 1, (300.0, 11, 8)));

    let first = lattice.combination(0).unwrap();
    assert_eq!(
        first.require_value("/solver").unwrap(),
        &serde_json::json!({
            "step": 0.01,
            "tolerance": 0.000001,
            "method": "rk4"
        })
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&first.to_json()).unwrap()["solver"]["method"],
        "rk4"
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn ordinary_arrays_are_literal_and_cases_remain_correlated() {
    let root = study_directory(
        "literal-and-cases",
        r#"{
          "global":{"shape":[64],"fields":["abundance","space"]},
          "phase_group":{"analysis":{
            "shared":{},
            "phase":{"convert":{
              "$cases":[
                {"temperature":280.0,"seed":1},
                {"temperature":300.0,"seed":9}
              ]
            }}
          }}
        }"#,
    );
    let combinations = StudyConfiguration::load(&root)
        .unwrap()
        .phase("analysis", "convert")
        .unwrap()
        .combinations()
        .collect::<Vec<ResolvedConfiguration>>();

    assert_eq!(combinations.len(), 2);
    assert_eq!(
        combinations[0]
            .decode_value::<Vec<usize>>("/shape")
            .unwrap(),
        [64]
    );
    assert_eq!(combinations[0].decode_value::<u64>("/seed").unwrap(), 1);
    assert_eq!(combinations[1].decode_value::<u64>("/seed").unwrap(), 9);

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_scope_overlap_and_mixed_case_grammars() {
    let overlap = study_directory(
        "overlap",
        r#"{
          "global":{"seed":1},
          "phase_group":{"g":{"shared":{"seed":2},"phase":{"p":{}}}}
        }"#,
    );
    assert!(matches!(
        StudyConfiguration::load(&overlap),
        Err(ConfigurationError::InvalidConfigurationDocument { .. })
    ));
    fs::remove_dir_all(overlap).unwrap();

    let mixed = study_directory(
        "mixed",
        r#"{
          "global":{},
          "phase_group":{"g":{"shared":{},"phase":{"p":{
            "seed":{"$sweep":[1,2]},"$cases":[{"size":4},{"size":8}]
          }}}}
        }"#,
    );
    assert!(matches!(
        StudyConfiguration::load(&mixed),
        Err(ConfigurationError::InvalidConfigurationDocument { .. })
    ));
    fs::remove_dir_all(mixed).unwrap();
}

#[test]
fn indexed_access_and_errors_retain_the_combination_ordinal() {
    let root = study_directory(
        "indexed",
        r#"{"global":{"seed":7},"phase_group":{"g":{"shared":{},"phase":{"p":{}}}}}"#,
    );
    let space = StudyConfiguration::load(&root)
        .unwrap()
        .phase("g", "p")
        .unwrap();
    let configuration = space.combination(0).unwrap();

    assert!(matches!(
        configuration.decode_value::<String>("/seed"),
        Err(ConfigurationError::DecodeConfigurationValue { ordinal: 0, .. })
    ));
    assert!(matches!(
        space.combination(1),
        Err(ConfigurationError::CombinationOrdinalOutOfBounds {
            ordinal: 1,
            combination_count: 1
        })
    ));
    assert!(matches!(
        StudyConfiguration::load(&root)
            .unwrap()
            .phase("g", "missing"),
        Err(ConfigurationError::UnknownPhaseConfiguration { .. })
    ));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn project_paths_are_strict_ordered_and_resolve_lexically() {
    let root = study_directory(
        "paths",
        r#"{"global":{},"phase_group":{"g":{"shared":{},"phase":{"p":{}}}}}"#,
    );
    let configuration = root.join("config");
    fs::write(
        configuration.join("paths.json"),
        r#"{"recordings":"results","input":"data/input.json"}"#,
    )
    .unwrap();

    let paths = ProjectPaths::load(&root).unwrap();
    assert_eq!(paths.keys().collect::<Vec<_>>(), ["recordings", "input"]);
    assert_eq!(
        paths.resolve_path("recordings").unwrap(),
        root.join("results")
    );
    assert_eq!(paths.to_json_value()["input"], "data/input.json");

    fs::write(
        configuration.join("paths.json"),
        r#"{"recordings":"first","recordings":"second"}"#,
    )
    .unwrap();
    assert!(matches!(
        ProjectPaths::load(&root),
        Err(ConfigurationError::DuplicateConfigurationKey { key, .. }) if key == "recordings"
    ));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn study_settings_strictly_define_replicate_execution() {
    let root = study_directory(
        "replicate-settings",
        r#"{"global":{},"phase_group":{"g":{"shared":{},"phase":{"p":{}}}}}"#,
    );
    let source = br#"{
      "replicate_settings": {
        "replicates": 4,
        "execution": "parallel",
        "failure_policy": "finish_all",
        "seed": 1101
      }
    }"#;
    fs::write(root.join("study.json"), source).unwrap();

    let study = StudySettings::load(&root).unwrap();
    let settings = study.replicate_settings();
    assert_eq!(study.study_root(), root);
    assert_eq!(study.source_json(), source);
    assert_eq!(settings.replicates(), 4);
    assert_eq!(settings.execution(), ReplicateExecutionMode::Parallel);
    assert_eq!(settings.failure_policy(), ReplicateFailurePolicy::FinishAll);
    assert_eq!(settings.seed(), 1101);

    fs::write(
        root.join("study.json"),
        r#"{"replicate_settings":{"replicates":0,"execution":"sequential","failure_policy":"fail_fast","seed":1}}"#,
    )
    .unwrap();
    assert!(matches!(
        StudySettings::load(&root),
        Err(ConfigurationError::InvalidConfigurationDocument { .. })
    ));

    fs::write(
        root.join("study.json"),
        r#"{"replicate_settings":{"replicates":1,"execution":"sequential","failure_policy":"fail_fast","seed":1,"unknown":true}}"#,
    )
    .unwrap();
    assert!(matches!(
        StudySettings::load(&root),
        Err(ConfigurationError::InvalidConfigurationDocument { .. })
    ));

    fs::remove_dir_all(root).unwrap();
}
