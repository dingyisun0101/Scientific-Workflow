//! Focused contract tests for `storage/decoder.rs` and its built-in decoders.
//!
//! These tests keep record traversal and filesystem behavior out of scope.
//! They verify direct conversion, exact key-to-type registry binding, source
//! preservation, and ownership transfer into an empty `SystemState`. Complete
//! persisted reconstruction is covered separately by `reader.rs`.

use std::error::Error as _;
use std::mem;
use std::path::PathBuf;

use serde_json::json;

use crate::storage::decoder::{Decoders, PayloadDecoder, StringDecoder, VecF64Decoder};
use crate::storage::error::StorageError;
use crate::system_state::{StateSpec, TimePoint};

/// Builds a compact state specification through the real metadata parser.
fn spec(fields: &[&str]) -> StateSpec {
    let fields = fields
        .iter()
        .map(|name| json!({"name": name, "description": format!("{name} payload")}))
        .collect::<Vec<_>>();
    StateSpec::parse(
        PathBuf::from("decoder-metadata.json"),
        &serde_json::to_vec(&json!({"fields": fields})).unwrap(),
    )
    .expect("generated decoder test schema must be valid")
}

#[test]
fn vec_f64_decoder_reconstructs_exact_owned_vectors() {
    let populated = VecF64Decoder
        .decode("[1.25,-2.5,0.0,3e2]")
        .expect("numeric JSON array must decode");
    let empty = VecF64Decoder
        .decode("[]")
        .expect("empty numeric JSON array must decode");

    assert_eq!(populated, vec![1.25, -2.5, 0.0, 300.0]);
    assert!(empty.is_empty());
    assert!(populated.capacity() >= populated.len());
}

#[test]
fn vec_f64_decoder_rejects_wrong_json_shapes_and_elements() {
    for raw in ["null", "4.5", "[1.0,\"bad\"]", "[[1.0]]", "["] {
        assert!(
            VecF64Decoder.decode(raw).is_err(),
            "unexpectedly decoded {raw}"
        );
    }
}

#[test]
fn string_decoder_preserves_content_and_decodes_json_escapes() {
    let decoded = StringDecoder
        .decode(r#""  line\n世界\t  ""#)
        .expect("valid escaped JSON string must decode");
    let empty = StringDecoder
        .decode(r#""""#)
        .expect("empty JSON string must decode");

    assert_eq!(decoded, "  line\n世界\t  ");
    assert!(empty.is_empty());
}

#[test]
fn string_decoder_rejects_every_non_string_json_kind() {
    for raw in ["null", "false", "12", "[]", "{}", "\""] {
        assert!(
            StringDecoder.decode(raw).is_err(),
            "unexpectedly decoded {raw}"
        );
    }
}

#[test]
fn built_in_decoders_are_zero_sized_copyable_and_thread_safe() {
    fn assert_copy<T: Copy>() {}
    fn assert_send_sync<T: Send + Sync>() {}

    assert_eq!(mem::size_of::<VecF64Decoder>(), 0);
    assert_eq!(mem::size_of::<StringDecoder>(), 0);
    assert_copy::<VecF64Decoder>();
    assert_copy::<StringDecoder>();
    assert_send_sync::<VecF64Decoder>();
    assert_send_sync::<StringDecoder>();
}

#[test]
fn registry_binds_each_exact_key_to_one_concrete_output_type() {
    let state_spec = spec(&["samples", "label"]);
    let mut state = state_spec.empty(TimePoint::new(17));
    let mut decoders = Decoders::with_capacity(2);
    decoders
        .add::<Vec<f64>, _>("samples", VecF64Decoder)
        .unwrap();
    decoders.add::<String, _>("label", StringDecoder).unwrap();

    decoders.require(["samples", "label"]).unwrap();
    decoders
        .decode_into("signal", 17, "samples", "[2.0,4.0,8.0]", &mut state)
        .unwrap();
    decoders
        .decode_into("signal", 17, "label", r#""sample-17""#, &mut state)
        .unwrap();

    assert_eq!(state.get::<Vec<f64>>("samples").unwrap(), &[2.0, 4.0, 8.0]);
    assert_eq!(state.get::<String>("label").unwrap(), "sample-17");
    assert_eq!(decoders.len(), 2);
    assert!(!decoders.is_empty());
    assert!(decoders.contains("samples"));
    assert!(decoders.contains("label"));
}

#[test]
fn registry_rejects_empty_duplicate_and_missing_key_configuration() {
    let mut decoders = Decoders::new();
    assert!(matches!(
        decoders.add::<String, _>("", StringDecoder),
        Err(StorageError::InvalidConfig {
            setting: "decoder.key",
            ..
        })
    ));
    decoders.add::<String, _>("label", StringDecoder).unwrap();
    assert!(matches!(
        decoders.add::<String, _>("label", StringDecoder),
        Err(StorageError::DuplicateDecoder { field }) if field == "label"
    ));
    assert!(matches!(
        decoders.require(["label", "samples"]),
        Err(StorageError::MissingDecoder { field }) if field == "samples"
    ));
}

#[test]
fn conversion_failure_preserves_stream_index_key_and_serde_source() {
    let state_spec = spec(&["samples"]);
    let mut state = state_spec.empty(TimePoint::new(29));
    let mut decoders = Decoders::new();
    decoders
        .add::<Vec<f64>, _>("samples", VecF64Decoder)
        .unwrap();

    let error = decoders
        .decode_into("space", 29, "samples", r#"[1.0,"bad"]"#, &mut state)
        .expect_err("wrong element type must fail reconstruction");

    assert!(matches!(
        &error,
        StorageError::DecodeField {
            stream,
            index: 29,
            field,
            ..
        } if stream == "space" && field == "samples"
    ));
    assert!(error.source().unwrap().is::<serde_json::Error>());
    assert!(!state.has("samples").unwrap());
}

#[test]
fn closure_decoders_remain_available_for_application_specific_payloads() {
    let state_spec = spec(&["count"]);
    let mut state = state_spec.empty(TimePoint::new(3));
    let mut decoders = Decoders::new();
    decoders
        .add::<u64, _>("count", |raw: &str| serde_json::from_str::<u64>(raw))
        .unwrap();

    decoders
        .decode_into("events", 3, "count", "42", &mut state)
        .unwrap();

    assert_eq!(*state.get::<u64>("count").unwrap(), 42);
}

#[test]
fn registry_debug_output_contains_sorted_keys_but_no_decoder_internals() {
    let mut decoders = Decoders::new();
    decoders.add::<String, _>("zeta", StringDecoder).unwrap();
    decoders.add::<Vec<f64>, _>("alpha", VecF64Decoder).unwrap();

    let debug = format!("{decoders:?}");
    assert!(debug.contains(r#"["alpha", "zeta"]"#));
    assert!(!debug.contains("serde_json"));
    assert!(!debug.contains("VecF64Decoder"));
}
