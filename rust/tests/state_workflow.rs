//! Logged integration workflow for simulation-owned system state.
//!
//! This integration test imports the package exactly as a downstream Rust
//! crate would and reports stable semantic results under `--nocapture`.
//!
//! The test connects every current production layer:
//!
//! - the public `state` module root and ordinary prelude;
//! - JSON template loading and semantic round-trip serialization;
//! - fixed field metadata and shared specification ownership;
//! - time-point and state construction;
//! - real `physics_in_parallel` tensor insertion, borrowing, mutation,
//!   cloning, and owned extraction;
//! - immutable and mutable heterogeneous tuple borrowing;
//! - generated tuple arities two through eight and duplicate rejection;
//! - retained type contracts after extraction, clearing, and empty derivation;
//! - ownership-preserving replacement and rejection;
//! - checked mutable simulation time;
//! - public errors for unknown, missing, and mismatched fields.
//!
//! Payload persistence is intentionally outside this contract. The JSON
//! fixture defines only the in-memory field layout; the persistence module
//! will borrow each tensor's existing Serialize implementation.

use std::error::Error as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use physics_in_parallel::prelude::advanced::Dense;
use physics_in_parallel::prelude::basic::Tensor;
use scientific_workflow::state::StateFieldSchema;
use scientific_workflow::state::{
    StateError, StateSchemaProvider, StateTime, SystemState, SystemStateSchema,
};
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
const COUPLED_TEMPLATE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/coupled_state.json"
);

fn temporary_test_directory(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock must be after the Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "scientific-workflow-state-{label}-{}-{nonce}",
        std::process::id()
    ))
}

#[test]
fn state_module_and_prelude_share_the_ordinary_types() {
    let module_time = scientific_workflow::state::StateTime::from_iteration(1);
    let prelude_time: scientific_workflow::prelude::StateTime = module_time;
    assert_eq!(prelude_time.iteration(), 1);

    fn inspect_schema(schema: &scientific_workflow::prelude::SystemStateSchema) -> usize {
        schema.len()
    }

    let schema = SystemStateSchema::load_json_template(Path::new(STATE_TEMPLATE)).unwrap();
    assert_eq!(inspect_schema(&schema), 3);
}

#[test]
fn static_provider_resolves_without_a_project_file() {
    let provider = StateSchemaProvider::new(
        "test.direct-state.v1",
        br#"{"fields":[{"name":"population"},{"name":"space"}]}"#,
    );
    let schema = provider.resolve().unwrap();

    assert_eq!(
        schema.template_path(),
        Path::new("<state-schema-provider:test.direct-state.v1>")
    );
    assert_eq!(
        schema
            .field_schemas()
            .iter()
            .map(StateFieldSchema::name)
            .collect::<Vec<_>>(),
        ["population", "space"]
    );
}

#[test]
fn schema_validation_is_strict_and_normalizes_metadata() {
    let directory = temporary_test_directory("schema-validation");
    fs::create_dir_all(&directory).expect("temporary schema directory must be created");

    let empty_name = directory.join("empty-name.json");
    fs::write(&empty_name, br#"{"fields":[{"name":"   "}]}"#).unwrap();
    assert!(matches!(
        SystemStateSchema::load_json_template(&empty_name),
        Err(StateError::EmptyFieldName { index: 0 })
    ));

    let unknown_property = directory.join("unknown-property.json");
    fs::write(
        &unknown_property,
        br#"{"fields":[{"name":"population","unit":"people"}]}"#,
    )
    .unwrap();
    assert!(matches!(
        SystemStateSchema::load_json_template(&unknown_property),
        Err(StateError::TemplateParse { .. })
    ));

    let normalized = directory.join("normalized.json");
    fs::write(
        &normalized,
        br#"{"fields":[{"name":" population ","description":"   "},{"name":" activity ","description":" enabled "}]}"#,
    )
    .unwrap();
    let schema = SystemStateSchema::load_json_template(&normalized).unwrap();
    assert_eq!(schema.field_schemas()[0].name(), "population");
    assert_eq!(schema.field_schemas()[0].description(), None);
    assert_eq!(schema.field_schemas()[1].name(), "activity");
    assert_eq!(schema.field_schemas()[1].description(), Some("enabled"));

    fs::remove_dir_all(directory).expect("temporary schema directory must be removed");
}

#[test]
fn failed_initialization_preserves_the_incoming_payload() {
    let schema = SystemStateSchema::load_json_template(Path::new(STATE_TEMPLATE)).unwrap();
    let mut state = schema.create_empty_state(StateTime::from_iteration(0));
    state
        .initialize_payload("population", vec![1_u64, 2, 3])
        .unwrap();

    let incoming = vec![5_u64, 8, 13];
    let incoming_pointer = incoming.as_ptr();
    let rejection = state
        .initialize_payload("population", incoming)
        .expect_err("a second initialization must return its incoming owner");
    assert!(matches!(
        rejection.error(),
        StateError::PayloadAlreadyInitialized { field } if field == "population"
    ));
    assert_eq!(rejection.payload().as_ptr(), incoming_pointer);
    let (_, recovered) = rejection.into_parts();
    assert_eq!(recovered.as_ptr(), incoming_pointer);
    assert_eq!(recovered, vec![5, 8, 13]);
}

#[test]
fn tensor_state_round_trip_integrates_public_modules() {
    // Load the actual canonical template through the public crate API.
    let specification = SystemStateSchema::load_json_template(Path::new(STATE_TEMPLATE))
        .expect("canonical state template must load");

    assert_eq!(
        specification.template_path().to_string_lossy(),
        STATE_TEMPLATE
    );
    assert_eq!(specification.len(), 3);
    assert!(!specification.is_empty());

    let fields: &[StateFieldSchema] = specification.field_schemas();
    assert_eq!(fields[0].position(), 0);
    assert_eq!(fields[0].name(), "population");
    assert_eq!(
        fields[0].description(),
        Some("Population count at each modeled location")
    );
    assert_eq!(fields[1].position(), 1);
    assert_eq!(fields[1].name(), "space");
    assert_eq!(
        fields[1].description(),
        Some("Spatial lattice values for the current state")
    );
    assert_eq!(fields[2].position(), 2);
    assert_eq!(fields[2].name(), "activity");
    assert_eq!(
        fields[2].description(),
        Some("Activity flag at each modeled location")
    );
    assert!(specification.contains_field("population"));
    assert_eq!(
        specification
            .field_schema("space")
            .expect("space field must exist"),
        &fields[1]
    );

    // Compare parsed JSON values so formatting differences cannot disguise a
    // semantic template change.
    let original_json: serde_json::Value = serde_json::from_slice(
        &fs::read(STATE_TEMPLATE).expect("canonical state template must be readable"),
    )
    .expect("canonical state template must contain valid JSON");
    let serialized = specification
        .to_json_template()
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
    let round_trip_directory = temporary_test_directory("round-trip");
    let round_trip_path = round_trip_directory.join("state.json");

    fs::create_dir_all(&round_trip_directory)
        .expect("temporary round-trip directory must be created");
    fs::write(&round_trip_path, serialized).expect("round-trip template must be written");

    let restored = SystemStateSchema::load_json_template(&round_trip_path)
        .expect("round-trip template must load successfully");
    assert_eq!(restored.template_path(), round_trip_path);
    assert_eq!(restored.field_schemas(), specification.field_schemas());

    let missing_template = round_trip_directory.join("missing.json");
    let read_error = SystemStateSchema::load_json_template(&missing_template).unwrap_err();
    assert!(matches!(read_error, StateError::TemplateRead { .. }));
    assert!(read_error.source().is_some());
    let malformed_template = round_trip_directory.join("malformed.json");
    fs::write(&malformed_template, b"{").unwrap();
    let parse_error = SystemStateSchema::load_json_template(&malformed_template).unwrap_err();
    assert!(matches!(parse_error, StateError::TemplateParse { .. }));
    assert!(parse_error.source().is_some());
    let duplicate_template = round_trip_directory.join("duplicate.json");
    fs::write(
        &duplicate_template,
        br#"{"fields":[{"name":"x"},{"name":" x "}]}"#,
    )
    .unwrap();
    assert!(matches!(
        SystemStateSchema::load_json_template(&duplicate_template),
        Err(StateError::DuplicateField { field }) if field == "x"
    ));

    assert!(StateTime::from_iteration_and_physical_time(0, f64::NAN).is_none());
    assert!(StateTime::from_iteration_and_physical_time(0, f64::INFINITY).is_none());

    // State construction retains both the exact integer index and optional
    // finite physical coordinate.
    let initial_time = StateTime::from_iteration_and_physical_time(0, 0.25)
        .expect("finite physical time must be accepted");
    let mut state: SystemState = specification.create_empty_state(initial_time);

    assert_eq!(state.time().iteration(), 0);
    assert_eq!(state.time().physical_time(), Some(0.25));
    assert_eq!(state.schema().len(), 3);
    assert_eq!(state.populated_field_count(), 0);
    assert!(
        !state
            .contains_payload("population")
            .expect("field must be declared")
    );

    // The owning simulation may replace or advance time without touching
    // payload layout. Checked advancement returns the new complete coordinate.
    let replacement_time = StateTime::from_iteration(10);
    assert_eq!(state.replace_time(replacement_time), initial_time);
    assert_eq!(state.replace_time(initial_time), replacement_time);
    let preview = state
        .time()
        .checked_advance(Some(0.25))
        .expect("finite physical time must preflight");
    let advanced = state
        .advance_time(Some(0.25))
        .expect("finite physical time must advance");
    assert_eq!(advanced, preview);
    assert_eq!(advanced.iteration(), 1);
    assert_eq!(advanced.physical_time(), Some(0.5));
    let before_failed_advance = state.time();
    assert!(state.advance_time(Some(f64::INFINITY)).is_err());
    assert_eq!(state.time(), before_failed_advance);
    let mut overflow = specification.create_empty_state(StateTime::from_iteration(u64::MAX));
    assert!(matches!(
        overflow.advance_time(None),
        Err(StateError::IterationOverflow {
            iteration: u64::MAX
        })
    ));
    assert_eq!(overflow.time().iteration(), u64::MAX);
    let mut no_physical = specification.create_empty_state(StateTime::from_iteration(3));
    assert!(matches!(
        no_physical.advance_time(Some(0.25)),
        Err(StateError::MissingPhysicalTime { iteration: 3 })
    ));

    // Empty and unknown fields produce distinct public errors before any
    // tensor is inserted.
    assert!(matches!(
        state.payload::<Tensor<u64, Dense>>("population"),
        Err(StateError::MissingPayload { ref field }) if field == "population"
    ));
    assert!(matches!(
        state.payload::<Tensor<u64, Dense>>("temperature"),
        Err(StateError::UnknownField { ref field }) if field == "temperature"
    ));

    let rejected = vec![1_u64, 2, 3];
    let rejected_pointer = rejected.as_ptr();
    let rejection = state
        .insert_payload("temperature", rejected)
        .expect_err("an undeclared field must reject and return its payload");
    assert!(matches!(
        rejection.error(),
        StateError::UnknownField { field } if field == "temperature"
    ));
    assert_eq!(rejection.payload().as_ptr(), rejected_pointer);
    assert!(format!("{rejection:?}").contains("PayloadInsertError"));
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

    state
        .initialize_payload("population", population)
        .expect("population tensor must initialize its declared slot");
    state
        .initialize_payload("space", space)
        .expect("space tensor must initialize its declared slot");
    state
        .initialize_payload("activity", activity)
        .expect("activity tensor must initialize its declared slot");

    let duplicate_initialization = state
        .initialize_payload("activity", Tensor::<u8, Dense>::zeros(&[1]))
        .expect_err("initialization must never replace an established payload");
    assert!(matches!(
        duplicate_initialization.error(),
        StateError::PayloadAlreadyInitialized { field } if field == "activity"
    ));

    let rejection = state
        .insert_payload("population", String::from("wrong concrete type"))
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
            .payload_has_type::<Tensor<u64, Dense>>("population")
            .expect("rejected replacement must preserve the tensor")
    );

    assert_eq!(state.populated_field_count(), 3);
    assert!(
        state
            .payload_has_type::<Tensor<u64, Dense>>("population")
            .expect("field must be declared")
    );
    assert_eq!(
        state
            .payload::<Tensor<u64, Dense>>("space")
            .expect("space tensor type must match")
            .shape(),
        &[2, 2]
    );

    // Mutate the original tensor allocation through a typed mutable borrow.
    state
        .payload_mut::<Tensor<u64, Dense>>("population")
        .expect("population tensor type must match")
        .set(&[1], 21);
    assert_eq!(
        state
            .payload::<Tensor<u64, Dense>>("population")
            .expect("population tensor must remain available")
            .get(&[1]),
        21
    );

    // One coordinated borrow resolves heterogeneous payloads once around a
    // coupled kernel. Reversed template order exercises safe slot sorting while
    // the returned tuple preserves caller order.
    {
        let (activity, population, space) =
            state
                .borrow_payloads_mut::<(Tensor<u8, Dense>, Tensor<u64, Dense>, Tensor<u64, Dense>)>(
                    ("activity", "population", "space"),
                )
                .expect("three distinct typed fields must be mutably borrowed together");
        activity.set(&[1], 1);
        population.set(&[2], 31);
        space.set(&[1, 0], 30);
    }
    let (population, activity) = state
        .borrow_payloads::<(Tensor<u64, Dense>, Tensor<u8, Dense>)>(("population", "activity"))
        .expect("two distinct typed fields must be immutably borrowed together");
    assert_eq!(population.get(&[2]), 31);
    assert_eq!(activity.get(&[1]), 1);

    let repeated = state
        .borrow_payloads_mut::<(Tensor<u64, Dense>, Tensor<u64, Dense>)>(("space", "space"))
        .expect_err("one coordinated borrow must reject a repeated field");
    assert!(matches!(
        repeated,
        StateError::RepeatedPayloadBorrow { ref field } if field == "space"
    ));
    let tuple_unknown = state
        .borrow_payloads::<(Tensor<u64, Dense>, Tensor<u64, Dense>)>(("population", "temperature"))
        .expect_err("tuple preflight must reject an undeclared field");
    assert!(matches!(
        tuple_unknown,
        StateError::UnknownField { ref field } if field == "temperature"
    ));
    let tuple_mismatch = state
        .borrow_payloads_mut::<(Tensor<u8, Dense>, Tensor<u64, Dense>)>(("population", "space"))
        .expect_err("tuple preflight must reject a retained type mismatch");
    assert!(matches!(
        tuple_mismatch,
        StateError::TypeMismatch { ref field, .. } if field == "population"
    ));
    assert_eq!(
        state
            .payload::<Tensor<u64, Dense>>("space")
            .unwrap()
            .get(&[1, 0]),
        30
    );

    // A failed owning request is rejected before the original payload moves.
    assert!(matches!(
        state.take_payload::<Tensor<u8, Dense>>("population"),
        Err(StateError::TypeMismatch { .. })
    ));
    assert_eq!(
        state
            .payload::<Tensor<u64, Dense>>("population")
            .unwrap()
            .get(&[1]),
        21
    );

    // A failed typed borrow must report both exact Rust types and leave the
    // activity tensor unchanged.
    let mismatch = state
        .payload::<Tensor<u64, Dense>>("activity")
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
            .payload::<Tensor<u8, Dense>>("activity")
            .expect("failed borrow must preserve activity tensor")
            .get(&[2]),
        1
    );

    // Explicit SystemState cloning must deeply clone tensor storage so the
    // branch can be mutated independently.
    let mut branch = state.clone();
    branch
        .payload_mut::<Tensor<u64, Dense>>("space")
        .expect("cloned space tensor type must match")
        .set(&[0, 0], 99);
    assert_eq!(
        branch
            .payload::<Tensor<u64, Dense>>("space")
            .expect("cloned space tensor must remain available")
            .get(&[0, 0]),
        99
    );
    assert_eq!(
        state
            .payload::<Tensor<u64, Dense>>("space")
            .expect("original space tensor must remain available")
            .get(&[0, 0]),
        1
    );

    // Successful takes return the original concrete tensors and empty their
    // slots without invoking the payload Clone implementation.
    let population = state
        .take_payload::<Tensor<u64, Dense>>("population")
        .expect("population tensor must move out of the state");
    let space = state
        .take_payload::<Tensor<u64, Dense>>("space")
        .expect("space tensor must move out of the state");
    let activity = state
        .take_payload::<Tensor<u8, Dense>>("activity")
        .expect("activity tensor must move out of the state");

    assert_eq!(population.shape(), &[3]);
    assert_eq!(population.get(&[0]), 10);
    assert_eq!(population.get(&[1]), 21);
    assert_eq!(population.get(&[2]), 31);
    assert_eq!(space.shape(), &[2, 2]);
    assert_eq!(space.get(&[1, 1]), 4);
    assert_eq!(space.get(&[1, 0]), 30);
    assert_eq!(activity.shape(), &[3]);
    assert_eq!(activity.get(&[0]), 1);
    assert_eq!(activity.get(&[1]), 1);
    assert_eq!(activity.get(&[2]), 1);
    assert_eq!(state.populated_field_count(), 0);

    // Extraction leaves the original slots empty but permanently typed. A
    // different payload type is rejected even though no value is present.
    let retype = state
        .insert_payload("population", vec![3_u64, 5, 8, 13])
        .expect_err("an emptied tensor field must retain its tensor type");
    assert!(matches!(
        retype.error(),
        StateError::TypeMismatch { field, actual, .. }
            if field == "population"
                && *actual == std::any::type_name::<Tensor<u64, Dense>>()
    ));
    let (_, recovered) = retype.into_parts();
    assert_eq!(recovered, vec![3, 5, 8, 13]);

    // A separately assembled state can bind the same JSON field to a vector.
    // Ordinary vector payloads make allocation identity directly observable.
    let mut allocation_state = specification.create_empty_state(StateTime::from_iteration(2));
    let owned = vec![3_u64, 5, 8, 13];
    let owned_pointer = owned.as_ptr();
    allocation_state
        .initialize_payload("population", owned)
        .unwrap();
    let replacement = vec![21_u64, 34];
    let previous = allocation_state
        .insert_payload("population", replacement)
        .unwrap()
        .expect("same-type replacement must return the previous vector");
    assert_eq!(previous.as_ptr(), owned_pointer);
    let replacement_pointer = allocation_state
        .payload::<Vec<u64>>("population")
        .unwrap()
        .as_ptr();
    let extracted = allocation_state
        .take_payload::<Vec<u64>>("population")
        .unwrap();
    assert_eq!(extracted.as_ptr(), replacement_pointer);

    let mut clone_state = specification.create_empty_state(StateTime::from_iteration(3));
    let clones = Arc::new(AtomicUsize::new(0));
    clone_state
        .initialize_payload(
            "population",
            CloneTracked {
                values: vec![1, 1, 2, 3, 5],
                clones: Arc::clone(&clones),
            },
        )
        .unwrap();
    let mut cloned = clone_state.clone();
    assert_eq!(clones.load(Ordering::Relaxed), 1);
    cloned
        .payload_mut::<CloneTracked>("population")
        .unwrap()
        .values
        .push(8);
    assert_eq!(
        clone_state
            .payload::<CloneTracked>("population")
            .unwrap()
            .values
            .len(),
        5
    );
    assert_eq!(
        cloned
            .payload::<CloneTracked>("population")
            .unwrap()
            .values
            .len(),
        6
    );
    assert!(clone_state.clear_payload("population").unwrap());
    assert!(!clone_state.clear_payload("population").unwrap());
    let cleared_retype = clone_state
        .insert_payload("population", String::from("wrong after clear"))
        .expect_err("clear must retain the field type definition");
    assert!(matches!(
        cleared_retype.error(),
        StateError::TypeMismatch { field, .. } if field == "population"
    ));

    // Derived states share immutable schema storage and type definitions while
    // beginning with no payloads.
    let mut later = state.clone_structure_without_payloads(StateTime::from_iteration(1));
    assert_eq!(later.time().iteration(), 1);
    assert_eq!(later.time().physical_time(), None);
    assert_eq!(later.populated_field_count(), 0);
    assert!(std::ptr::eq(
        later.schema().field_schemas(),
        state.schema().field_schemas()
    ));
    assert!(matches!(
        later.insert_payload("space", vec![1_u8]),
        Err(ref error)
            if matches!(error.error(), StateError::TypeMismatch { field, .. } if field == "space")
    ));
    let mut restored_space = Tensor::<u64, Dense>::zeros(&[1]);
    restored_space.set(&[0], 7);
    assert!(
        later
            .insert_payload("space", restored_space)
            .unwrap()
            .is_none()
    );
    let blank_tuple = later
        .borrow_payloads::<(Tensor<u64, Dense>, Tensor<u64, Dense>)>(("space", "population"))
        .expect_err("tuple preflight must reject a correctly typed empty field");
    assert!(matches!(
        blank_tuple,
        StateError::MissingPayload { ref field } if field == "population"
    ));

    let debug = format!("{state:?}");
    assert!(debug.contains("SystemState"));
    assert!(!debug.contains("active"));

    println!(
        "[state] iteration={} physical_time={:?} loaded={} mutation_verified=true",
        advanced.iteration(),
        advanced.physical_time(),
        3
    );
    println!("[ownership] pointer_preserved=true rejected_payload_recovered=true");
    println!(
        "[tuple] immutable=true mutable=true duplicate_rejected=true unknown_rejected=true preflight_atomic=true"
    );
    println!("[type-contract] take_retained=true clear_retained=true empty_inherited=true");
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

#[test]
fn generated_tuple_arities_two_through_eight_are_available() {
    let specification = SystemStateSchema::load_json_template(Path::new(COUPLED_TEMPLATE))
        .expect("coupled state fixture must load");
    let mut state = specification.create_empty_state(StateTime::from_iteration(0));
    for (key, value) in [
        ("a", 1_u64),
        ("b", 2),
        ("c", 3),
        ("d", 4),
        ("e", 5),
        ("f", 6),
        ("g", 7),
        ("h", 8),
    ] {
        state.initialize_payload(key, value).unwrap();
    }

    let _ = state.borrow_payloads::<(u64, u64)>(("a", "b")).unwrap();
    let repeated = state
        .borrow_payloads::<(u64, u64)>(("a", "a"))
        .expect_err("immutable tuple borrowing must reject a repeated field");
    assert!(matches!(
        repeated,
        StateError::RepeatedPayloadBorrow { field } if field == "a"
    ));
    let _ = state
        .borrow_payloads::<(u64, u64, u64)>(("a", "b", "c"))
        .unwrap();
    let _ = state
        .borrow_payloads::<(u64, u64, u64, u64)>(("a", "b", "c", "d"))
        .unwrap();
    let _ = state
        .borrow_payloads::<(u64, u64, u64, u64, u64)>(("a", "b", "c", "d", "e"))
        .unwrap();
    let _ = state
        .borrow_payloads::<(u64, u64, u64, u64, u64, u64)>(("a", "b", "c", "d", "e", "f"))
        .unwrap();
    let _ = state
        .borrow_payloads::<(u64, u64, u64, u64, u64, u64, u64)>(("a", "b", "c", "d", "e", "f", "g"))
        .unwrap();

    let (h, g, f, e, d, c, b, a) = state
        .borrow_payloads_mut::<(u64, u64, u64, u64, u64, u64, u64, u64)>((
            "h", "g", "f", "e", "d", "c", "b", "a",
        ))
        .expect("arity-eight reverse-order mutable borrow must succeed");
    *a += 10;
    *b += 10;
    *c += 10;
    *d += 10;
    *e += 10;
    *f += 10;
    *g += 10;
    *h += 10;

    assert_eq!(*state.payload::<u64>("a").unwrap(), 11);
    assert_eq!(*state.payload::<u64>("h").unwrap(), 18);
    println!("[tuple-arities] min=2 max=8 reverse_order_mutation=true");
}
