//! Focused contract tests for `storage/encoder.rs`.
//!
//! The encoder consumes crate-private SystemState serialization borrows, while
//! the storage facade is still staged outside `lib.rs`. The review command
//! therefore compiles this file inside a small same-crate source harness. Once
//! `storage.rs` is exported, the unified storage target will include these
//! tests through the ordinary library boundary.

use std::error::Error as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::{Serialize, Serializer};
use serde_json::{Value, json};

use crate::storage::encoder::JsonEncoder;
use crate::storage::error::StorageError;
use crate::system_state::{StateError, StateSpec, TimePoint};

/// Parses a compact in-memory template through the real validation boundary.
fn spec(fields: &[&str]) -> StateSpec {
    let declarations = fields
        .iter()
        .map(|name| json!({"name": name, "description": format!("{name} payload")}))
        .collect::<Vec<_>>();
    StateSpec::parse(
        PathBuf::from("encoder-state.json"),
        &serde_json::to_vec(&json!({"fields": declarations})).unwrap(),
    )
    .expect("the generated test template must be valid")
}

/// Payload whose Clone implementation makes accidental encoder cloning visible.
#[derive(Debug)]
struct CloneTracked {
    values: Vec<u64>,
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

impl Serialize for CloneTracked {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.values.serialize(serializer)
    }
}

/// Payload that deliberately exercises per-field serializer error context.
#[derive(Clone)]
struct RejectEncoding;

impl Serialize for RejectEncoding {
    fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        Err(serde::ser::Error::custom("deliberate payload failure"))
    }
}

#[test]
fn construction_normalizes_stream_and_canonicalizes_arbitrary_field_order() {
    let state_spec = spec(&["population", "space", "activity"]);
    let encoder = JsonEncoder::new("  signal  ", &state_spec, ["activity", "population"])
        .expect("valid selected fields must construct");

    assert_eq!(encoder.stream(), "signal");
    assert!(encoder.spec().shares_layout(&state_spec));
    assert_eq!(
        encoder.fields().collect::<Vec<_>>(),
        vec!["population", "activity"]
    );
}

#[test]
fn construction_rejects_empty_unknown_and_duplicate_configuration() {
    let state_spec = spec(&["population", "space"]);

    for result in [
        JsonEncoder::new(" ", &state_spec, ["population"]),
        JsonEncoder::new("signal", &state_spec, [""]),
        JsonEncoder::new("signal", &state_spec, ["unknown"]),
        JsonEncoder::new("signal", &state_spec, ["space", "space"]),
    ] {
        assert!(matches!(result, Err(StorageError::InvalidConfig { .. })));
    }
}

#[test]
fn encoding_emits_exact_compact_shape_in_canonical_order() {
    let state_spec = spec(&["population", "space", "activity"]);
    let mut state = state_spec.empty(TimePoint::from_physical(12, 0.25).unwrap());
    assert!(
        state
            .set("population", vec![1_u64, 2, 3])
            .unwrap()
            .is_none()
    );
    assert!(state.set("activity", true).unwrap().is_none());
    let encoder = JsonEncoder::new("signal", &state_spec, ["activity", "population"]).unwrap();

    let encoded = encoder
        .encode(&state)
        .expect("populated fields must encode");

    assert_eq!(encoded.time(), state.time());
    assert_eq!(
        encoded.bytes(),
        br#"{"index":12,"physical":0.25,"values":{"population":[1,2,3],"activity":true}}
"#
    );
    assert_eq!(encoded.len(), encoded.bytes().len());

    let raw: Value = serde_json::from_slice(&encoded.bytes()[..encoded.len() - 1])
        .expect("encoded body must satisfy the raw record contract");
    assert_eq!(raw["index"], 12);
    assert_eq!(raw["physical"], 0.25);
    assert_eq!(raw["values"]["population"], json!([1, 2, 3]));
    assert_eq!(raw["values"]["activity"], Value::Bool(true));
}

#[test]
fn time_only_encoding_omits_absent_physical_time_and_has_an_empty_values_object() {
    let state_spec = spec(&["population"]);
    let state = state_spec.empty(TimePoint::new(7));
    let encoder = JsonEncoder::new("events", &state_spec, std::iter::empty::<&str>()).unwrap();

    let encoded = encoder.encode(&state).expect("empty selection must encode");

    assert_eq!(encoded.bytes(), b"{\"index\":7,\"values\":{}}\n");
    assert!(
        !encoded
            .bytes()
            .windows(8)
            .any(|window| window == b"physical")
    );
}

#[test]
fn state_access_errors_retain_stream_time_field_and_typed_source() {
    let state_spec = spec(&["population"]);
    let state = state_spec.empty(TimePoint::new(19));
    let encoder = JsonEncoder::new("signal", &state_spec, ["population"]).unwrap();

    let error = encoder
        .encode(&state)
        .expect_err("an empty selected slot must fail preflight");

    assert!(matches!(
        &error,
        StorageError::StateAccess {
            stream,
            index: 19,
            field,
            source: StateError::MissingValue { field: missing },
        } if stream == "signal" && field == "population" && missing == "population"
    ));
    assert!(error.source().unwrap().is::<StateError>());
}

#[test]
fn compatible_but_independently_allocated_state_layouts_can_be_encoded() {
    let configured_spec = spec(&["population", "space"]);
    let independent_spec = spec(&["population", "space"]);
    assert!(!configured_spec.shares_layout(&independent_spec));
    let mut state = independent_spec.empty(TimePoint::new(3));
    assert!(state.set("space", vec![8_u8, 13]).unwrap().is_none());
    let encoder = JsonEncoder::new("space", &configured_spec, ["space"]).unwrap();

    let encoded = encoder
        .encode(&state)
        .expect("field-compatible independent state must encode");

    assert_eq!(
        encoded.bytes(),
        b"{\"index\":3,\"values\":{\"space\":[8,13]}}\n"
    );
}

#[test]
fn payload_serializer_failure_identifies_the_active_field_and_returns_no_record() {
    let state_spec = spec(&["population", "activity"]);
    let mut state = state_spec.empty(TimePoint::new(23));
    assert!(state.set("population", vec![1_u8]).unwrap().is_none());
    assert!(state.set("activity", RejectEncoding).unwrap().is_none());
    let encoder = JsonEncoder::new("signal", &state_spec, ["population", "activity"]).unwrap();

    let error = encoder
        .encode(&state)
        .expect_err("payload rejection must abort the whole record");

    assert!(matches!(
        &error,
        StorageError::EncodeField {
            stream,
            index: 23,
            field,
            ..
        } if stream == "signal" && field == "activity"
    ));
    assert_eq!(
        error.source().unwrap().to_string(),
        "deliberate payload failure"
    );
    assert!(state.has("population").unwrap());
    assert!(state.has("activity").unwrap());
}

#[test]
fn encoding_borrows_without_cloning_and_releases_the_state_for_mutation() {
    let state_spec = spec(&["population"]);
    let clones = Arc::new(AtomicUsize::new(0));
    let payload = CloneTracked {
        values: vec![2, 3, 5, 7],
        clones: Arc::clone(&clones),
    };
    let original_allocation = payload.values.as_ptr();
    let mut state = state_spec.empty(TimePoint::new(31));
    assert!(state.set("population", payload).unwrap().is_none());
    let encoder = JsonEncoder::new("signal", &state_spec, ["population"]).unwrap();

    let encoded = encoder
        .encode(&state)
        .expect("borrowed payload must encode");

    assert_eq!(clones.load(Ordering::Relaxed), 0);
    assert_eq!(
        state
            .get::<CloneTracked>("population")
            .unwrap()
            .values
            .as_ptr(),
        original_allocation
    );
    state
        .get_mut::<CloneTracked>("population")
        .unwrap()
        .values
        .push(11);
    assert_eq!(
        state.get::<CloneTracked>("population").unwrap().values,
        vec![2, 3, 5, 7, 11]
    );
    assert_eq!(
        encoded.bytes(),
        b"{\"index\":31,\"values\":{\"population\":[2,3,5,7]}}\n"
    );
}
