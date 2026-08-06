//! Logged integration workflow for simulation-owned system state.
//!
//! This integration test imports the package exactly as a downstream Rust
//! crate would and reports stable semantic results under `--nocapture`.
//!
//! The test connects every current production layer:
//!
//! - the crate root and public `system_state` facade;
//! - JSON template loading and semantic round-trip serialization;
//! - fixed field metadata and shared specification ownership;
//! - time-point and state construction;
//! - real `physics_in_parallel` tensor insertion, borrowing, mutation,
//!   cloning, and owned extraction;
//! - ownership-preserving replacement and rejection;
//! - checked mutable simulation time;
//! - public errors for unknown, missing, and mismatched fields.
//!
//! Payload persistence is intentionally outside this contract. The JSON
//! fixture defines only the in-memory field layout; the future storage module
//! will borrow each tensor's existing Serialize implementation.

use std::error::Error as _;
use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use physics_in_parallel::math::{Dense, Tensor};
use scientific_workflow::system_state::{FieldSpec, StateError, StateSpec, SystemState, TimePoint};
use serde::Serialize;

/// Serializable payload whose Clone implementation exposes expensive copying.
#[derive(Debug, Serialize)]
struct CloneTracked {
    values: Vec<u64>,
    #[serde(skip)]
    clones: Arc<AtomicUsize>,
}

impl Clone for CloneTracked {
    fn clone(&self) -> Self {
        self.clones.fetch_add(1, Ordering::Relaxed);
        Self {
            values: self.values.clone(),
            clones: Arc::clone(&self.clones),
        }
    }
}

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
        fields[0].description(),
        Some("Population count at each modeled location")
    );
    assert_eq!(fields[1].index(), 1);
    assert_eq!(fields[1].name(), "space");
    assert_eq!(
        fields[1].description(),
        Some("Spatial lattice values for the current state")
    );
    assert_eq!(fields[2].index(), 2);
    assert_eq!(fields[2].name(), "activity");
    assert_eq!(
        fields[2].description(),
        Some("Activity flag at each modeled location")
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
    println!(
        "[template] fields={} round_trip=true shared_layout=true",
        specification.len()
    );

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
    assert!(!restored.shares_layout(&specification));
    assert!(specification.clone().shares_layout(&specification));

    let missing_template = round_trip_directory.join("missing.json");
    let read_error = StateSpec::load(&missing_template).unwrap_err();
    assert!(matches!(read_error, StateError::TemplateRead { .. }));
    assert!(read_error.source().is_some());
    let malformed_template = round_trip_directory.join("malformed.json");
    fs::write(&malformed_template, b"{").unwrap();
    let parse_error = StateSpec::load(&malformed_template).unwrap_err();
    assert!(matches!(parse_error, StateError::TemplateParse { .. }));
    assert!(parse_error.source().is_some());
    let duplicate_template = round_trip_directory.join("duplicate.json");
    fs::write(
        &duplicate_template,
        br#"{"fields":[{"name":"x"},{"name":" x "}]}"#,
    )
    .unwrap();
    assert!(matches!(
        StateSpec::load(&duplicate_template),
        Err(StateError::DuplicateField { field }) if field == "x"
    ));

    assert!(TimePoint::from_physical(0, f64::NAN).is_none());
    assert!(TimePoint::from_physical(0, f64::INFINITY).is_none());

    // State construction retains both the exact integer index and optional
    // finite physical coordinate.
    let initial_time =
        TimePoint::from_physical(0, 0.25).expect("finite physical time must be accepted");
    let mut state: SystemState = specification.empty(initial_time);

    assert_eq!(state.time().index(), 0);
    assert_eq!(state.time().physical(), Some(0.25));
    assert_eq!(state.len(), 3);
    assert!(!state.is_empty());
    assert_eq!(state.loaded(), 0);
    assert!(state.is_blank());
    assert!(!state.has("population").expect("field must be declared"));

    // The owning simulation may replace or advance time without touching
    // payload layout. Checked advancement returns the new complete coordinate.
    let replacement_time = TimePoint::new(10);
    assert_eq!(state.set_time(replacement_time), initial_time);
    assert_eq!(state.set_time(initial_time), replacement_time);
    let advanced = state
        .advance(Some(0.25))
        .expect("finite physical time must advance");
    assert_eq!(advanced.index(), 1);
    assert_eq!(advanced.physical(), Some(0.5));
    let before_failed_advance = state.time();
    assert!(state.advance(Some(f64::INFINITY)).is_err());
    assert_eq!(state.time(), before_failed_advance);
    let mut overflow = specification.empty(TimePoint::new(u64::MAX));
    assert!(matches!(
        overflow.advance(None),
        Err(StateError::TimeIndexOverflow { index: u64::MAX })
    ));
    assert_eq!(overflow.time().index(), u64::MAX);
    let mut no_physical = specification.empty(TimePoint::new(3));
    assert!(matches!(
        no_physical.advance(Some(0.25)),
        Err(StateError::MissingPhysicalTime { index: 3 })
    ));

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

    let rejected = vec![1_u64, 2, 3];
    let rejected_pointer = rejected.as_ptr();
    let rejection = state
        .set("temperature", rejected)
        .expect_err("an undeclared field must reject and return its payload");
    assert!(matches!(
        rejection.error(),
        StateError::UnknownField { field } if field == "temperature"
    ));
    assert_eq!(rejection.payload().as_ptr(), rejected_pointer);
    assert!(format!("{rejection:?}").contains("SetError"));
    assert!(rejection.to_string().contains("temperature"));
    assert!(rejection.source().is_some());
    let (_, rejected) = rejection.into_parts();
    assert_eq!(rejected.as_ptr(), rejected_pointer);

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

    assert!(
        state
            .set("population", population)
            .expect("population tensor must move into its declared slot")
            .is_none()
    );
    assert!(
        state
            .set("space", space)
            .expect("space tensor must move into its declared slot")
            .is_none()
    );
    assert!(
        state
            .set("activity", activity)
            .expect("activity tensor must move into its declared slot")
            .is_none()
    );

    let rejection = state
        .set("population", String::from("wrong concrete type"))
        .expect_err("an occupied field must reject a different concrete type");
    assert!(matches!(
        rejection.error(),
        StateError::TypeMismatch {
            field,
            expected,
            actual,
        } if field == "population"
            && *expected == std::any::type_name::<String>()
            && *actual == std::any::type_name::<Tensor<u64, Dense>>()
    ));
    let (_, rejected) = rejection.into_parts();
    assert_eq!(rejected, "wrong concrete type");
    assert!(
        state
            .is::<Tensor<u64, Dense>>("population")
            .expect("rejected replacement must preserve the tensor")
    );

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

    // A failed owning downcast restores the original payload transactionally.
    assert!(matches!(
        state.take::<Tensor<u8, Dense>>("population"),
        Err(StateError::TypeMismatch { .. })
    ));
    assert_eq!(
        state
            .get::<Tensor<u64, Dense>>("population")
            .unwrap()
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

    // Ordinary vector payloads make allocation identity directly observable.
    let owned = vec![3_u64, 5, 8, 13];
    let owned_pointer = owned.as_ptr();
    assert!(state.set("population", owned).unwrap().is_none());
    let replacement = vec![21_u64, 34];
    let previous = state
        .set("population", replacement)
        .unwrap()
        .expect("same-type replacement must return the previous vector");
    assert_eq!(previous.as_ptr(), owned_pointer);
    let replacement_pointer = state.get::<Vec<u64>>("population").unwrap().as_ptr();
    let extracted = state.take::<Vec<u64>>("population").unwrap();
    assert_eq!(extracted.as_ptr(), replacement_pointer);

    let clones = Arc::new(AtomicUsize::new(0));
    assert!(
        state
            .set(
                "population",
                CloneTracked {
                    values: vec![1, 1, 2, 3, 5],
                    clones: Arc::clone(&clones),
                },
            )
            .unwrap()
            .is_none()
    );
    let mut cloned = state.clone();
    assert_eq!(clones.load(Ordering::Relaxed), 1);
    cloned
        .get_mut::<CloneTracked>("population")
        .unwrap()
        .values
        .push(8);
    assert_eq!(
        state
            .get::<CloneTracked>("population")
            .unwrap()
            .values
            .len(),
        5
    );
    assert_eq!(
        cloned
            .get::<CloneTracked>("population")
            .unwrap()
            .values
            .len(),
        6
    );
    assert!(state.clear("population").unwrap());
    assert!(!state.clear("population").unwrap());

    assert!(state.set("space", vec![1_u8]).unwrap().is_none());
    assert!(
        state
            .set("activity", String::from("active"))
            .unwrap()
            .is_none()
    );
    state.clear_all();
    assert!(state.is_blank());

    // Derived states share immutable schema storage but begin with no payloads.
    let later = state.empty(TimePoint::new(1));
    assert_eq!(later.time().index(), 1);
    assert_eq!(later.time().physical(), None);
    assert!(later.is_blank());
    assert_eq!(later.fields(), state.fields());
    assert!(std::ptr::eq(later.spec().fields(), state.spec().fields()));

    let debug = format!("{state:?}");
    assert!(debug.contains("SystemState"));
    assert!(!debug.contains("active"));

    println!(
        "[state] index={} physical={:?} loaded={} mutation_verified=true",
        advanced.index(),
        advanced.physical(),
        3
    );
    println!("[ownership] pointer_preserved=true rejected_payload_recovered=true");
    println!(
        "[validation] read_error=true parse_error=true duplicate_error=true time_transactional=true"
    );
    println!(
        "[clone] payload_clone_calls={} independent=true",
        clones.load(Ordering::Relaxed)
    );
    println!("[result] state_workflow=passed");

    fs::remove_dir_all(round_trip_directory)
        .expect("temporary round-trip directory must be removed");
}
