//! Public end-to-end contract test for the SystemState module.
//!
//! This integration test imports the package exactly as a downstream Rust
//! crate would. It does not include private source files, use test-only module
//! stubs, or access the erased payload representation.
//!
//! The test connects every current production layer:
//!
//! - the crate root and public `system_state` facade;
//! - JSON template loading and semantic round-trip serialization;
//! - fixed field metadata and shared specification ownership;
//! - time-point and state construction;
//! - real `physics_in_parallel` tensor insertion, borrowing, mutation,
//!   cloning, and owned extraction;
//! - public errors for unknown, missing, and mismatched fields.
//!
//! Payload persistence is intentionally outside this contract. The JSON
//! fixture defines the state schema; a later SSTS codec will encode tensor
//! payloads.

use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

use physics_in_parallel::math::{Dense, Tensor};
use scientific_workflow::system_state::{FieldSpec, StateError, StateSpec, SystemState, TimePoint};

/// Canonical state template resolved independently of the process working
/// directory.
const STATE_TEMPLATE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/state.json");

#[test]
fn tensor_state_round_trip_integrates_public_modules() {
    // Load the actual canonical template through the public crate API.
    let specification =
        StateSpec::load(STATE_TEMPLATE).expect("canonical state template must load");

    assert_eq!(specification.source().to_string_lossy(), STATE_TEMPLATE);
    assert_eq!(specification.len(), 3);
    assert!(!specification.is_empty());

    let fields: &[FieldSpec] = specification.fields();
    assert_eq!(fields[0].index(), 0);
    assert_eq!(fields[0].name(), "population");
    assert_eq!(
        fields[0].type_tag(),
        "physics_in_parallel.tensor.dense.u64.v1"
    );
    assert_eq!(fields[1].index(), 1);
    assert_eq!(fields[1].name(), "space");
    assert_eq!(
        fields[1].type_tag(),
        "physics_in_parallel.tensor.dense.u64.v1"
    );
    assert_eq!(fields[2].index(), 2);
    assert_eq!(fields[2].name(), "activity");
    assert_eq!(
        fields[2].type_tag(),
        "physics_in_parallel.tensor.dense.u8.v1"
    );
    assert!(specification.contains("population"));
    assert_eq!(
        specification.get("space").expect("space field must exist"),
        &fields[1]
    );

    // Compare parsed JSON values so formatting differences cannot disguise a
    // semantic template change.
    let original_json: serde_json::Value = serde_json::from_slice(
        &fs::read(STATE_TEMPLATE).expect("canonical state template must be readable"),
    )
    .expect("canonical state template must contain valid JSON");
    let serialized = specification
        .to_json()
        .expect("validated specification must serialize");
    let serialized_json: serde_json::Value =
        serde_json::from_str(&serialized).expect("serialized template must be valid JSON");
    assert_eq!(serialized_json, original_json);

    // Reload the generated JSON from a distinct path to verify the complete
    // filesystem round trip and deterministic field reconstruction.
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock must be after the Unix epoch")
        .as_nanos();
    let round_trip_directory = std::env::temp_dir().join(format!(
        "scientific-workflow-public-system-state-{}-{nonce}",
        std::process::id()
    ));
    let round_trip_path = round_trip_directory.join("state.json");

    fs::create_dir_all(&round_trip_directory)
        .expect("temporary round-trip directory must be created");
    fs::write(&round_trip_path, serialized).expect("round-trip template must be written");

    let restored =
        StateSpec::load(&round_trip_path).expect("round-trip template must load successfully");
    assert_eq!(restored.source(), round_trip_path);
    assert_eq!(restored.fields(), specification.fields());

    // State construction retains both the exact integer index and optional
    // finite physical coordinate.
    let initial_time =
        TimePoint::from_physical(0, 0.25).expect("finite physical time must be accepted");
    let mut state: SystemState = specification.empty(initial_time);

    assert_eq!(state.time().index(), 0);
    assert_eq!(state.time().physical(), Some(0.25));
    assert_eq!(state.len(), 3);
    assert_eq!(state.loaded(), 0);
    assert!(state.is_blank());
    assert!(!state.has("population").expect("field must be declared"));

    // Empty and unknown fields produce distinct public errors before any
    // tensor is inserted.
    assert!(matches!(
        state.get::<Tensor<u64, Dense>>("population"),
        Err(StateError::MissingValue { ref field }) if field == "population"
    ));
    assert!(matches!(
        state.get::<Tensor<u64, Dense>>("temperature"),
        Err(StateError::UnknownField { ref field }) if field == "temperature"
    ));

    // Construct realistic rank-one and rank-two tensors. Moving these values
    // into SystemState transfers their owned backing allocations.
    let mut population = Tensor::<u64, Dense>::zeros(&[3]);
    population.set(&[0], 10);
    population.set(&[1], 20);
    population.set(&[2], 30);

    let mut space = Tensor::<u64, Dense>::zeros(&[2, 2]);
    space.set(&[0, 0], 1);
    space.set(&[0, 1], 2);
    space.set(&[1, 0], 3);
    space.set(&[1, 1], 4);

    let mut activity = Tensor::<u8, Dense>::zeros(&[3]);
    activity.set(&[0], 1);
    activity.set(&[1], 0);
    activity.set(&[2], 1);

    state
        .set("population", population)
        .expect("population tensor must move into its declared slot");
    state
        .set("space", space)
        .expect("space tensor must move into its declared slot");
    state
        .set("activity", activity)
        .expect("activity tensor must move into its declared slot");

    assert_eq!(state.loaded(), 3);
    assert!(!state.is_blank());
    assert!(
        state
            .is::<Tensor<u64, Dense>>("population")
            .expect("field must be declared")
    );
    assert_eq!(
        state
            .get::<Tensor<u64, Dense>>("space")
            .expect("space tensor type must match")
            .shape(),
        &[2, 2]
    );

    // Mutate the original tensor allocation through a typed mutable borrow.
    state
        .get_mut::<Tensor<u64, Dense>>("population")
        .expect("population tensor type must match")
        .set(&[1], 21);
    assert_eq!(
        state
            .get::<Tensor<u64, Dense>>("population")
            .expect("population tensor must remain available")
            .get(&[1]),
        21
    );

    // A failed typed borrow must report both exact Rust types and leave the
    // activity tensor unchanged.
    let mismatch = state
        .get::<Tensor<u64, Dense>>("activity")
        .expect_err("activity stores a u8 tensor, not a u64 tensor");
    assert!(matches!(
        mismatch,
        StateError::TypeMismatch {
            ref field,
            expected,
            actual,
        } if field == "activity"
            && expected == std::any::type_name::<Tensor<u64, Dense>>()
            && actual == std::any::type_name::<Tensor<u8, Dense>>()
    ));
    assert_eq!(
        state
            .get::<Tensor<u8, Dense>>("activity")
            .expect("failed borrow must preserve activity tensor")
            .get(&[2]),
        1
    );

    // Explicit SystemState cloning must deeply clone tensor storage so the
    // branch can be mutated independently.
    let mut branch = state.clone();
    branch
        .get_mut::<Tensor<u64, Dense>>("space")
        .expect("cloned space tensor type must match")
        .set(&[0, 0], 99);
    assert_eq!(
        branch
            .get::<Tensor<u64, Dense>>("space")
            .expect("cloned space tensor must remain available")
            .get(&[0, 0]),
        99
    );
    assert_eq!(
        state
            .get::<Tensor<u64, Dense>>("space")
            .expect("original space tensor must remain available")
            .get(&[0, 0]),
        1
    );

    // Successful takes return the original concrete tensors and empty their
    // slots without invoking the payload Clone implementation.
    let population = state
        .take::<Tensor<u64, Dense>>("population")
        .expect("population tensor must move out of the state");
    let space = state
        .take::<Tensor<u64, Dense>>("space")
        .expect("space tensor must move out of the state");
    let activity = state
        .take::<Tensor<u8, Dense>>("activity")
        .expect("activity tensor must move out of the state");

    assert_eq!(population.shape(), &[3]);
    assert_eq!(population.get(&[0]), 10);
    assert_eq!(population.get(&[1]), 21);
    assert_eq!(population.get(&[2]), 30);
    assert_eq!(space.shape(), &[2, 2]);
    assert_eq!(space.get(&[1, 1]), 4);
    assert_eq!(activity.shape(), &[3]);
    assert_eq!(activity.get(&[0]), 1);
    assert_eq!(activity.get(&[1]), 0);
    assert_eq!(activity.get(&[2]), 1);
    assert_eq!(state.loaded(), 0);
    assert!(state.is_blank());

    // Derived states share immutable schema storage but begin with no payloads.
    let later = state.empty(TimePoint::new(1));
    assert_eq!(later.time().index(), 1);
    assert_eq!(later.time().physical(), None);
    assert!(later.is_blank());
    assert_eq!(later.fields(), state.fields());
    assert!(std::ptr::eq(later.spec().fields(), state.spec().fields()));

    fs::remove_dir_all(round_trip_directory)
        .expect("temporary round-trip directory must be removed");
}
