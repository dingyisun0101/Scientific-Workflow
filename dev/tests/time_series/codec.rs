//! Contract tests for the private `time_series/codec.rs` implementation.
//!
//! The public time-series facade remains intentionally unwired while source
//! files are reviewed individually. This suite includes the production codec
//! and series-error modules directly. A narrow test-only `system_state` module
//! supplies the same typed `FieldSpec` and `SystemState` operations consumed by
//! the codec; the production codec has separately been compiled against the
//! real SystemState module graph.
//!
//! These tests verify:
//!
//! - explicit, unique stable-tag registration;
//! - borrowed serialization without payload cloning;
//! - owned deserialization directly into a state;
//! - precise missing-codec and concrete-type failures;
//! - optional borrowed size estimation;
//! - bounded registry diagnostics and thread-safety.

use std::any::{Any, type_name};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::{Deserialize, Serialize};

/// Minimal typed state boundary required to isolate `codec.rs` behavior.
///
/// The implementation deliberately mirrors production semantics relevant to
/// codecs: fields have stable names and tags, insertion consumes an owned
/// payload, and access performs exact `Any` downcasts.
mod system_state {
    use super::{Any, HashMap, type_name};

    use thiserror::Error;

    /// State access failures used by the production `SeriesError` conversion.
    #[derive(Debug, Error)]
    pub enum StateError {
        /// A field name is absent from this test state's declared layout.
        #[error("state template does not declare field `{field}`")]
        UnknownField {
            /// Requested undeclared field.
            field: String,
        },

        /// A declared field does not currently own a payload.
        #[error("state field `{field}` does not contain a payload")]
        MissingValue {
            /// Requested empty field.
            field: String,
        },

        /// The requested concrete type differs from the stored payload type.
        #[error(
            "state field `{field}` contains `{actual}`, but the operation requested `{expected}`"
        )]
        TypeMismatch {
            /// Field containing the mismatched value.
            field: String,
            /// Fully qualified requested Rust type name.
            expected: &'static str,
            /// Fully qualified stored Rust type name.
            actual: &'static str,
        },
    }

    /// Stable field name and serialization tag consumed by `CodecRegistry`.
    pub struct FieldSpec {
        name: Box<str>,
        type_tag: Box<str>,
    }

    impl FieldSpec {
        /// Constructs one deterministic test field.
        pub fn new(name: &str, type_tag: &str) -> Self {
            Self {
                name: name.into(),
                type_tag: type_tag.into(),
            }
        }

        /// Returns the dictionary key used for state access.
        pub fn name(&self) -> &str {
            &self.name
        }

        /// Returns the stable tag used for codec lookup.
        pub fn type_tag(&self) -> &str {
            &self.type_tag
        }
    }

    /// Object-safe runtime type information retained by one test payload.
    trait StoredValue: Any + Send {
        /// Borrows this value through `Any` for exact downcasting.
        fn as_any(&self) -> &dyn Any;

        /// Returns the concrete diagnostic type name.
        fn type_name(&self) -> &'static str;
    }

    impl<T> StoredValue for T
    where
        T: Any + Send,
    {
        fn as_any(&self) -> &dyn Any {
            self
        }

        fn type_name(&self) -> &'static str {
            type_name::<T>()
        }
    }

    /// Small dictionary-like owner implementing the codec-facing state API.
    pub struct SystemState {
        declared: Vec<Box<str>>,
        values: HashMap<Box<str>, Box<dyn StoredValue>>,
    }

    impl SystemState {
        /// Creates an empty state declaring exactly the supplied field names.
        pub fn new(fields: &[&str]) -> Self {
            Self {
                declared: fields.iter().map(|field| (*field).into()).collect(),
                values: HashMap::new(),
            }
        }

        /// Consumes and stores a concrete payload under a declared field.
        pub fn set<T>(&mut self, field: &str, payload: T) -> Result<(), StateError>
        where
            T: Any + Clone + Send,
        {
            if !self
                .declared
                .iter()
                .any(|declared| declared.as_ref() == field)
            {
                return Err(StateError::UnknownField {
                    field: field.to_owned(),
                });
            }

            self.values.insert(field.into(), Box::new(payload));
            Ok(())
        }

        /// Borrows a populated field as the exact requested concrete type.
        pub fn get<T>(&self, field: &str) -> Result<&T, StateError>
        where
            T: Any,
        {
            if !self
                .declared
                .iter()
                .any(|declared| declared.as_ref() == field)
            {
                return Err(StateError::UnknownField {
                    field: field.to_owned(),
                });
            }

            let value = self
                .values
                .get(field)
                .ok_or_else(|| StateError::MissingValue {
                    field: field.to_owned(),
                })?;
            let actual = value.as_ref().type_name();

            value
                .as_ref()
                .as_any()
                .downcast_ref::<T>()
                .ok_or_else(|| StateError::TypeMismatch {
                    field: field.to_owned(),
                    expected: type_name::<T>(),
                    actual,
                })
        }
    }
}

#[path = "../../src/time_series/error.rs"]
#[allow(dead_code)]
mod error;

#[path = "../../src/time_series/codec.rs"]
mod codec;

use codec::CodecRegistry;
use error::SeriesError;
use system_state::{FieldSpec, StateError, SystemState};

/// Serializable payload whose explicit clones are observable.
///
/// The clone counter is skipped by Serde and reconstructed at zero, allowing
/// tests to distinguish serialization from the explicit deep-clone contract.
#[derive(Debug, Serialize, Deserialize)]
struct Payload {
    values: Vec<u64>,
    #[serde(skip)]
    clones: Arc<AtomicUsize>,
}

impl Payload {
    /// Creates one payload and a shared observer for explicit clone calls.
    fn tracked(values: Vec<u64>) -> (Self, Arc<AtomicUsize>) {
        let clones = Arc::new(AtomicUsize::new(0));
        (
            Self {
                values,
                clones: Arc::clone(&clones),
            },
            clones,
        )
    }
}

impl Clone for Payload {
    /// Records and performs the explicit payload clone required by a cloned
    /// SystemState, but never by codec borrowing or serialization.
    fn clone(&self) -> Self {
        self.clones.fetch_add(1, Ordering::SeqCst);
        Self {
            values: self.values.clone(),
            clones: Arc::clone(&self.clones),
        }
    }
}

/// Stable tag used by every payload registration in this isolated suite.
const PAYLOAD_TAG: &str = "example.payload.u64.v1";

/// Creates the declared field shared by state and registry fixtures.
fn payload_field() -> FieldSpec {
    FieldSpec::new("payload", PAYLOAD_TAG)
}

#[test]
fn registration_is_explicit_unique_and_exact() {
    let mut registry = CodecRegistry::new();

    assert!(registry.is_empty());
    assert_eq!(registry.len(), 0);
    assert!(!registry.contains(PAYLOAD_TAG));

    registry
        .register::<Payload>(PAYLOAD_TAG)
        .expect("the first stable-tag registration must succeed");

    assert!(!registry.is_empty());
    assert_eq!(registry.len(), 1);
    assert!(registry.contains(PAYLOAD_TAG));
    assert!(!registry.contains("example.payload.u64.V1"));

    let error = registry
        .register::<Vec<u64>>(PAYLOAD_TAG)
        .expect_err("a stable tag must not be replaced");
    assert!(matches!(
        error,
        SeriesError::DuplicateCodec { ref type_tag } if type_tag == PAYLOAD_TAG
    ));
    assert_eq!(registry.len(), 1);
}

#[test]
fn encoding_borrows_the_original_payload_without_cloning() {
    let mut registry = CodecRegistry::new();
    registry
        .register::<Payload>(PAYLOAD_TAG)
        .expect("payload codec must register");

    let field = payload_field();
    let (payload, clones) = Payload::tracked(vec![3, 5, 8]);
    let original_pointer = payload.values.as_ptr();
    let mut state = SystemState::new(&[field.name()]);
    state
        .set(field.name(), payload)
        .expect("declared field must accept payload");

    let value = registry
        .value(&state, &field)
        .expect("registered concrete payload must encode");
    let json = serde_json::to_string(value).expect("erased payload must serialize as JSON");

    assert_eq!(json, r#"{"values":[3,5,8]}"#);
    assert_eq!(clones.load(Ordering::SeqCst), 0);
    assert_eq!(
        state
            .get::<Payload>(field.name())
            .expect("payload must remain stored")
            .values
            .as_ptr(),
        original_pointer
    );
}

#[test]
fn decoding_constructs_and_moves_the_registered_concrete_type() {
    let mut registry = CodecRegistry::new();
    registry
        .register::<Payload>(PAYLOAD_TAG)
        .expect("payload codec must register");

    let field = payload_field();
    let mut state = SystemState::new(&[field.name()]);
    let mut json = serde_json::Deserializer::from_str(r#"{"values":[13,21,34]}"#);
    let mut erased = <dyn erased_serde::Deserializer>::erase(&mut json);

    registry
        .decode(&mut state, &field, &mut erased)
        .expect("valid JSON must decode into the registered type");
    json.end()
        .expect("the codec must consume exactly one payload");

    let decoded = state
        .get::<Payload>(field.name())
        .expect("decoded payload must be moved into the state");
    assert_eq!(decoded.values, vec![13, 21, 34]);
    assert_eq!(decoded.clones.load(Ordering::SeqCst), 0);
}

#[test]
fn lookup_and_type_failures_preserve_stable_and_concrete_context() {
    let field = payload_field();
    let mut state = SystemState::new(&[field.name()]);
    state
        .set(field.name(), vec![1_u64, 2, 3])
        .expect("declared field must accept payload");

    let empty_registry = CodecRegistry::new();
    let missing = match empty_registry.value(&state, &field) {
        Err(error) => error,
        Ok(_) => panic!("unregistered tags must be rejected"),
    };
    assert!(matches!(
        missing,
        SeriesError::MissingCodec { ref type_tag } if type_tag == PAYLOAD_TAG
    ));

    let mut registry = CodecRegistry::new();
    registry
        .register::<Payload>(PAYLOAD_TAG)
        .expect("payload codec must register");
    let mismatch = match registry.value(&state, &field) {
        Err(error) => error,
        Ok(_) => panic!("the stored concrete type must match its codec"),
    };
    assert!(matches!(
        mismatch,
        SeriesError::CodecTypeMismatch {
            ref field,
            ref type_tag,
            expected,
            actual,
        } if field == "payload"
            && type_tag == PAYLOAD_TAG
            && expected == type_name::<Payload>()
            && actual == type_name::<Vec<u64>>()
    ));

    let blank = SystemState::new(&[field.name()]);
    let missing_value = match registry.value(&blank, &field) {
        Err(error) => error,
        Ok(_) => panic!("empty fields must preserve their state error"),
    };
    assert!(matches!(
        missing_value,
        SeriesError::State(StateError::MissingValue { ref field }) if field == "payload"
    ));
}

#[test]
fn malformed_payloads_report_decode_context_and_source() {
    use std::error::Error as _;

    let mut registry = CodecRegistry::new();
    registry
        .register::<Payload>(PAYLOAD_TAG)
        .expect("payload codec must register");

    let field = payload_field();
    let mut state = SystemState::new(&[field.name()]);
    let mut json = serde_json::Deserializer::from_str(r#"{"values":"not-an-array"}"#);
    let mut erased = <dyn erased_serde::Deserializer>::erase(&mut json);
    let error = registry
        .decode(&mut state, &field, &mut erased)
        .expect_err("invalid payload JSON must fail decoding");

    assert!(matches!(
        error,
        SeriesError::DecodePayload {
            ref field,
            ref type_tag,
            ..
        } if field == "payload" && type_tag == PAYLOAD_TAG
    ));
    assert!(
        error
            .source()
            .expect("Serde decode source must be retained")
            .to_string()
            .contains("sequence")
    );
}

#[test]
fn size_estimation_is_optional_borrowed_and_type_checked() {
    let field = payload_field();
    let (payload, clones) = Payload::tracked(vec![1, 2, 3, 4]);
    let mut state = SystemState::new(&[field.name()]);
    state
        .set(field.name(), payload)
        .expect("declared field must accept payload");

    let mut without_size = CodecRegistry::new();
    without_size
        .register::<Payload>(PAYLOAD_TAG)
        .expect("payload codec must register");
    assert_eq!(
        without_size
            .estimate(&state, &field)
            .expect("registered payload type must validate"),
        None
    );

    let mut sized = CodecRegistry::new();
    sized
        .register_with_size::<Payload, _>(PAYLOAD_TAG, |payload| {
            payload.values.len() * size_of::<u64>()
        })
        .expect("sized payload codec must register");
    assert_eq!(
        sized
            .estimate(&state, &field)
            .expect("registered estimator must run"),
        Some(32)
    );
    assert_eq!(clones.load(Ordering::SeqCst), 0);

    let mut wrong_state = SystemState::new(&[field.name()]);
    wrong_state
        .set(field.name(), vec![1_u64])
        .expect("declared field must accept payload");
    assert!(matches!(
        without_size
            .estimate(&wrong_state, &field)
            .expect_err("missing estimator must not bypass type validation"),
        SeriesError::CodecTypeMismatch { .. }
    ));
}

#[test]
fn registry_debug_is_bounded_and_registry_is_thread_safe() {
    fn assert_send_sync<T: Send + Sync>() {}

    assert_send_sync::<CodecRegistry>();

    let mut registry = CodecRegistry::new();
    registry
        .register::<Payload>(PAYLOAD_TAG)
        .expect("payload codec must register");

    let output = format!("{registry:?}");
    assert!(output.contains("CodecRegistry"));
    assert!(output.contains("codecs: 1"));
    assert!(!output.contains(PAYLOAD_TAG));
}
