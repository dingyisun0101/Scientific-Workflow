use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use scientific_workflow::configuration::{
    ConfigurationError, ConfigurationSpace, ProjectPaths, ResolvedConfiguration,
};

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

fn configuration_directory(name: &str, fixed: &str, sweep: &str) -> PathBuf {
    let sequence = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
    let directory = std::env::temp_dir().join(format!(
        "scientific-workflow-{name}-{}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("fixed.json"), fixed).unwrap();
    fs::write(directory.join("sweep.json"), sweep).unwrap();
    directory
}

#[test]
fn resolves_every_fixed_and_cartesian_sweep_combination() {
    let directory = configuration_directory(
        "cartesian",
        r#"{"solver":{"step":0.01},"seed":7}"#,
        r#"{"mode":"cartesian","axes":{"temperature":{"values":[280.0,300.0]},"size":{"values":[4,8]}}}"#,
    );
    let space = ConfigurationSpace::load(&directory).unwrap();

    assert_eq!(space.combination_count(), 4);
    let resolved = space
        .combinations()
        .map(|configuration| {
            let values: (f64, usize, u64, f64) = configuration
                .decode_values(("/temperature", "/size", "/seed", "/solver/step"))
                .unwrap();
            (configuration.ordinal(), values)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        resolved,
        vec![
            (0, (280.0, 4, 7, 0.01)),
            (1, (280.0, 8, 7, 0.01)),
            (2, (300.0, 4, 7, 0.01)),
            (3, (300.0, 8, 7, 0.01)),
        ]
    );

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn cartesian_axes_can_share_values_from_an_external_json_array() {
    let directory = configuration_directory(
        "external-values",
        r#"{"shared":true}"#,
        r#"{"mode":"cartesian","axes":{"sys_idx":{"values_from":"systems.json"}}}"#,
    );
    fs::write(directory.join("systems.json"), r#"[0,1]"#).unwrap();

    let space = ConfigurationSpace::load(&directory).unwrap();
    assert_eq!(space.combination_count(), 2);
    assert_eq!(
        space
            .combinations()
            .map(|configuration| configuration.decode_value::<u64>("/sys_idx").unwrap())
            .collect::<Vec<_>>(),
        [0, 1]
    );
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn resolves_explicit_cases_without_creating_tasks() {
    let directory = configuration_directory(
        "cases",
        r#"{"shared":true}"#,
        r#"{"mode":"cases","cases":[{"temperature":280.0,"seed":1},{"temperature":300.0,"seed":9}]}"#,
    );
    let combinations = ConfigurationSpace::load(&directory)
        .unwrap()
        .combinations()
        .collect::<Vec<ResolvedConfiguration>>();

    assert_eq!(combinations.len(), 2);
    assert_eq!(combinations[0].decode_value::<u64>("/seed").unwrap(), 1);
    assert_eq!(combinations[1].decode_value::<u64>("/seed").unwrap(), 9);

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn indexed_access_and_value_errors_retain_the_combination_ordinal() {
    let directory = configuration_directory(
        "indexed",
        r#"{"seed":7}"#,
        r#"{"mode":"cartesian","axes":{}}"#,
    );
    let space = ConfigurationSpace::load(&directory).unwrap();
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

    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn project_paths_are_strict_ordered_and_resolve_lexically() {
    let root = std::env::temp_dir().join(format!(
        "scientific-workflow-paths-{}-{}",
        std::process::id(),
        NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
    ));
    let configuration = root.join("config");
    fs::create_dir_all(&configuration).unwrap();
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
        Err(ConfigurationError::DuplicateConfigurationKey { key, .. })
            if key == "recordings"
    ));

    fs::write(configuration.join("paths.json"), r#"{" ":"results"}"#).unwrap();
    assert!(matches!(
        ProjectPaths::load(&root),
        Err(ConfigurationError::InvalidConfigurationDocument { .. })
    ));
    fs::remove_dir_all(root).unwrap();
}
