//! Tests for loading an actual JSON template through `StateSpec`.
//!
//! The production module is included directly while the crate facade remains
//! intentionally unwired. Minimal `SystemState` and `TimePoint` definitions
//! satisfy `spec.rs`'s construction boundary; state behavior is tested
//! separately when `state.rs` is implemented.

use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "../../src/system_state/error.rs"]
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
      "type": "vec.u64"
    },
    {
      "name": "space",
      "type": "dses.square_lattice.u64.v1"
    },
    {
      "name": "activity",
      "type": "dses.activity.v1"
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
    assert_eq!(fields[0].type_tag(), "vec.u64");
    assert_eq!(fields[1].index(), 1);
    assert_eq!(fields[1].name(), "space");
    assert_eq!(fields[1].type_tag(), "dses.square_lattice.u64.v1");
    assert_eq!(fields[2].index(), 2);
    assert_eq!(fields[2].name(), "activity");
    assert_eq!(fields[2].type_tag(), "dses.activity.v1");

    let state = specification.empty(TimePoint);

    // Pointer equality proves that StateSpec::empty cheaply cloned the Arc
    // handle instead of duplicating the field metadata allocation.
    assert!(std::ptr::eq(state.spec().fields(), specification.fields()));

    let space = specification
        .get("space")
        .expect("declared field must be found by name");
    assert_eq!(space.index(), 1);
    assert_eq!(space.type_tag(), "dses.square_lattice.u64.v1");
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
    // and stable type tag in deterministic declaration order.
    assert_eq!(restored.fields(), specification.fields());
    assert_eq!(restored.len(), specification.len());
    assert_eq!(restored.source(), round_trip_path);

    fs::remove_dir_all(&directory).expect("temporary test directory must be removed");
}
