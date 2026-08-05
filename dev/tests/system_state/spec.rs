//! Tests for loading an actual JSON template through `StateSpec`.
//!
//! The production module is included directly. Minimal `SystemState` and
//! `TimePoint` definitions isolate the specification construction boundary;
//! concrete state behavior is covered by `tests/system_state/state.rs`.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "../../src/system_state/error.rs"]
#[allow(dead_code)]
mod error;

mod state {
    use super::spec::StateSpec;

    /// Minimal state boundary required to verify `StateSpec::empty`.
    pub struct SystemState {
        spec: StateSpec,
    }

    /// Minimal placeholder required by `StateSpec::empty`.
    pub struct TimePoint;

    impl SystemState {
        /// Matches the crate-private constructor that `state.rs` will provide.
        pub(crate) fn new(spec: StateSpec, _time: TimePoint) -> Self {
            Self { spec }
        }

        /// Returns the specification received through the construction
        /// boundary.
        pub fn spec(&self) -> &StateSpec {
            &self.spec
        }
    }
}

#[path = "../../src/system_state/spec.rs"]
#[allow(dead_code)]
mod spec;

use spec::StateSpec;
use state::TimePoint;

/// A representative scientific state template with aggregate, spatial, and
/// status fields.
const EXAMPLE_TEMPLATE: &str = r#"
{
  "fields": [
    {
      "name": "population",
      "description": "Population count"
    },
    {
      "name": "space",
      "description": "Spatial lattice"
    },
    {
      "name": "activity"
    }
  ]
}
"#;

#[test]
fn loads_and_round_trips_an_actual_example_template() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock must be after the Unix epoch")
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "scientific-workflow-spec-test-{}-{nonce}",
        std::process::id()
    ));
    let path = directory.join("state.json");
    let round_trip_path = directory.join("state.round-trip.json");

    fs::create_dir_all(&directory).expect("temporary test directory must be created");
    fs::write(&path, EXAMPLE_TEMPLATE).expect("example template must be written");

    let specification = StateSpec::load(&path).expect("example template must load");

    assert_eq!(specification.source(), path);
    assert_eq!(specification.len(), 3);
    assert!(!specification.is_empty());
    assert!(specification.contains("population"));
    assert!(specification.contains("space"));
    assert!(specification.contains("activity"));
    assert!(!specification.contains("energy"));

    let fields = specification.fields();
    assert_eq!(fields.len(), 3);
    assert_eq!(fields[0].index(), 0);
    assert_eq!(fields[0].name(), "population");
    assert_eq!(fields[0].description(), Some("Population count"));
    assert_eq!(fields[1].index(), 1);
    assert_eq!(fields[1].name(), "space");
    assert_eq!(fields[1].description(), Some("Spatial lattice"));
    assert_eq!(fields[2].index(), 2);
    assert_eq!(fields[2].name(), "activity");
    assert_eq!(fields[2].description(), None);

    let state = specification.empty(TimePoint);

    // Pointer equality proves that StateSpec::empty cheaply cloned the Arc
    // handle instead of duplicating the field metadata allocation.
    assert!(std::ptr::eq(state.spec().fields(), specification.fields()));

    let space = specification
        .get("space")
        .expect("declared field must be found by name");
    assert_eq!(space.index(), 1);
    assert_eq!(space.description(), Some("Spatial lattice"));
    assert!(specification.get("energy").is_none());

    let serialized = specification
        .to_json()
        .expect("validated specification must serialize");
    let original_json: serde_json::Value =
        serde_json::from_str(EXAMPLE_TEMPLATE).expect("example template must be valid JSON");
    let serialized_json: serde_json::Value =
        serde_json::from_str(&serialized).expect("serialized template must be valid JSON");

    // Comparing parsed values verifies semantic JSON equivalence independently
    // of whitespace and pretty-print formatting.
    assert_eq!(serialized_json, original_json);

    fs::write(&round_trip_path, serialized).expect("serialized template must be written");
    let restored = StateSpec::load(&round_trip_path).expect("serialized template must reload");

    // FieldSpec equality covers every reconstructed index, normalized name,
    // and optional description in deterministic declaration order.
    assert_eq!(restored.fields(), specification.fields());
    assert_eq!(restored.len(), specification.len());
    assert_eq!(restored.source(), round_trip_path);

    fs::remove_dir_all(&directory).expect("temporary test directory must be removed");
}

#[test]
fn in_memory_parse_matches_file_loading_without_sharing_layout_identity() {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock must be after the Unix epoch")
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "scientific-workflow-spec-parse-test-{}-{nonce}",
        std::process::id()
    ));
    let template_path = directory.join("state.json");
    let metadata_path = directory.join("metadata.json");

    fs::create_dir_all(&directory).expect("temporary test directory must be created");
    fs::write(&template_path, EXAMPLE_TEMPLATE).expect("example template must be written");

    let loaded = StateSpec::load(&template_path).expect("file template must load");
    let parsed = StateSpec::parse(metadata_path.clone(), EXAMPLE_TEMPLATE.as_bytes())
        .expect("embedded template must parse");

    assert_eq!(parsed.source(), metadata_path);
    assert_eq!(parsed.fields(), loaded.fields());
    assert_eq!(
        parsed.to_json().expect("parsed spec must serialize"),
        loaded.to_json().expect("loaded spec must serialize")
    );
    assert!(!parsed.shares_layout(&loaded));

    let parsed_clone = parsed.clone();
    assert!(parsed.shares_layout(&parsed_clone));

    fs::remove_dir_all(&directory).expect("temporary test directory must be removed");
}

#[test]
fn in_memory_parse_reports_the_supplied_metadata_path() {
    let metadata_path = PathBuf::from("output/metadata.json");
    let error = StateSpec::parse(metadata_path.clone(), br#"{"fields":[}"#)
        .expect_err("malformed embedded template must fail");

    assert!(matches!(
        error,
        error::StateError::TemplateParse { path, .. } if path == metadata_path
    ));
}

#[test]
fn semantic_validation_is_shared_by_load_and_parse() {
    let cases = [
        (br#"{"fields":[{"name":" "}]}"#.as_slice(), "empty name"),
        (
            br#"{"fields":[{"name":"x"},{"name":" x "}]}"#.as_slice(),
            "duplicate normalized name",
        ),
        (
            br#"{"fields":[{"name":"x","type":"legacy"}]}"#.as_slice(),
            "legacy type property",
        ),
        (
            br#"{"fields":[],"unknown":true}"#.as_slice(),
            "unknown property",
        ),
        (
            br#"{"fields":[{"name":"x","unknown":true}]}"#.as_slice(),
            "unknown field property",
        ),
    ];

    for (template, reason) in cases {
        assert!(
            StateSpec::parse(PathBuf::from("metadata.json"), template).is_err(),
            "{reason} must be rejected"
        );
    }
}

#[test]
fn descriptions_and_names_are_normalized_deterministically() {
    let template = br#"
    {
      "fields": [
        {"name":" population ","description":" Population count "},
        {"name":"missing"},
        {"name":"null","description":null},
        {"name":"empty","description":""},
        {"name":"whitespace","description":"   \n  "}
      ]
    }
    "#;

    let specification = StateSpec::parse(PathBuf::from("metadata.json"), template)
        .expect("all optional description forms must parse");
    let fields = specification.fields();

    assert_eq!(fields[0].name(), "population");
    assert_eq!(fields[0].description(), Some("Population count"));
    for field in &fields[1..] {
        assert_eq!(field.description(), None);
    }

    let normalized: serde_json::Value = serde_json::from_str(
        &specification
            .to_json()
            .expect("normalized specification must serialize"),
    )
    .expect("normalized output must be valid JSON");
    let expected = serde_json::json!({
        "fields": [
            {"name": "population", "description": "Population count"},
            {"name": "missing"},
            {"name": "null"},
            {"name": "empty"},
            {"name": "whitespace"}
        ]
    });
    assert_eq!(
        normalized, expected,
        "normalized round trip must be explicit"
    );
}

#[test]
fn empty_template_is_valid_and_round_trips() {
    let specification = StateSpec::parse(PathBuf::from("empty.json"), br#"{"fields":[]}"#)
        .expect("empty field arrays are valid");

    assert!(specification.is_empty());
    assert_eq!(specification.len(), 0);
    assert_eq!(specification.fields(), &[]);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(
            &specification.to_json().expect("empty spec must serialize")
        )
        .expect("serialized empty spec must be JSON"),
        serde_json::json!({"fields": []})
    );
}
