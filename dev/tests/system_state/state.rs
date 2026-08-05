//! Contract tests for `system_state/state.rs`.
//!
//! These tests live outside the source tree by project convention. The
//! production state, error, and erased-value implementations are included
//! directly. A narrow test-only specification supplies the exact interface
//! required by `SystemState`; loading and validating real JSON specifications
//! is covered independently by `tests/system_state/spec.rs`.
//!
//! The suite verifies the state-specific guarantees that matter to scientific
//! workloads:
//!
//! - time points reject non-finite physical coordinates;
//! - complete time replacement leaves payload ownership untouched;
//! - checked time advancement is transactional on every failure;
//! - state shape is fixed by a shared specification;
//! - insertion, replacement, mutation, and extraction do not clone payloads;
//! - rejected insertion returns the incoming payload and preserves the state;
//! - extraction preserves owned backing allocations;
//! - failed typed extraction restores the original payload;
//! - explicit state cloning deeply clones populated payloads;
//! - erased serialization borrows payloads without cloning or replacement;
//! - empty-state derivation shares only immutable layout metadata;
//! - field-access failures remain precise and non-destructive;
//! - debug formatting never traverses scientific payloads.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::{Serialize, Serializer};

#[path = "../../src/system_state/error.rs"]
#[allow(dead_code, unfulfilled_lint_expectations)]
mod error;

/// Minimal fixed-layout specification used to isolate the behavior implemented
/// by `state.rs`.
///
/// This is deliberately not a second specification implementation. It only
/// supplies deterministic fields, lookup, and shared ownership so these tests
/// can compile before the production module facade is connected.
mod spec {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use super::error::StateError;

    /// Minimal field metadata needed by `SystemState::fields`.
    #[derive(Debug, Eq, PartialEq)]
    pub struct FieldSpec {
        name: &'static str,
    }

    impl FieldSpec {
        /// Returns the declared field name.
        pub const fn name(&self) -> &str {
            self.name
        }
    }

    /// Shared immutable test layout matching the production specification's
    /// ownership and lookup semantics.
    #[derive(Debug)]
    struct Layout {
        source: PathBuf,
        fields: Vec<FieldSpec>,
    }

    /// Cheaply cloneable handle to the test layout.
    #[derive(Clone, Debug)]
    pub struct StateSpec {
        inner: Arc<Layout>,
    }

    impl StateSpec {
        /// Creates the deterministic three-field layout used by this suite.
        pub fn fixture() -> Self {
            Self {
                inner: Arc::new(Layout {
                    source: PathBuf::from("state.test.json"),
                    fields: vec![
                        FieldSpec { name: "population" },
                        FieldSpec { name: "space" },
                        FieldSpec { name: "status" },
                    ],
                }),
            }
        }

        /// Returns the number of payload slots required by the layout.
        pub fn len(&self) -> usize {
            self.inner.fields.len()
        }

        /// Returns field metadata in stable declaration order.
        pub fn fields(&self) -> &[FieldSpec] {
            &self.inner.fields
        }

        /// Returns the synthetic provenance path used in debug output.
        pub fn source(&self) -> &Path {
            &self.inner.source
        }

        /// Resolves a field name to its stable payload-slot index.
        pub(crate) fn index_of(&self, name: &str) -> Result<usize, StateError> {
            self.inner
                .fields
                .iter()
                .position(|field| field.name == name)
                .ok_or_else(|| StateError::UnknownField {
                    field: name.to_owned(),
                })
        }

        /// Reports whether two handles share the same immutable allocation.
        pub fn shares_layout_with(&self, other: &Self) -> bool {
            Arc::ptr_eq(&self.inner, &other.inner)
        }
    }
}

#[path = "../../src/system_state/value.rs"]
mod value;

#[path = "../../src/system_state/state.rs"]
mod state;

use error::StateError;
use spec::StateSpec;
use state::{SystemState, TimePoint};

/// Payload whose explicit clones are observable without changing its owned
/// vector's allocation semantics.
#[derive(Debug)]
struct CloneTracked {
    values: Vec<u64>,
    clones: Arc<AtomicUsize>,
}

impl CloneTracked {
    /// Creates an un-cloned payload and its shared clone counter.
    fn new(values: Vec<u64>) -> (Self, Arc<AtomicUsize>) {
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

impl Clone for CloneTracked {
    /// Records and performs an explicit deep payload clone.
    fn clone(&self) -> Self {
        self.clones.fetch_add(1, Ordering::SeqCst);
        Self {
            values: self.values.clone(),
            clones: Arc::clone(&self.clones),
        }
    }
}

impl Serialize for CloneTracked {
    /// Serializes scientific values while excluding the test-only counter.
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.values.serialize(serializer)
    }
}

/// Creates a blank state through the crate-private constructor under test.
fn blank_state(index: u64) -> SystemState {
    SystemState::new(StateSpec::fixture(), TimePoint::new(index))
}

#[test]
fn time_points_preserve_valid_coordinates_and_reject_non_finite_values() {
    let indexed = TimePoint::new(7);
    assert_eq!(indexed.index(), 7);
    assert_eq!(indexed.physical(), None);

    let physical =
        TimePoint::from_physical(8, -0.25).expect("negative finite coordinates are valid");
    assert_eq!(physical.index(), 8);
    assert_eq!(physical.physical(), Some(-0.25));

    assert!(TimePoint::from_physical(9, f64::NAN).is_none());
    assert!(TimePoint::from_physical(9, f64::INFINITY).is_none());
    assert!(TimePoint::from_physical(9, f64::NEG_INFINITY).is_none());
}

#[test]
fn set_time_replaces_only_time_and_returns_the_previous_coordinate() {
    let (payload, clones) = CloneTracked::new(vec![1, 2, 3]);
    let payload_pointer = payload.values.as_ptr();
    let mut state = blank_state(4);
    assert!(
        state
            .set("population", payload)
            .expect("declared empty field must accept payload")
            .is_none()
    );

    let next = TimePoint::from_physical(40, 1.5).expect("physical time must be finite");
    let previous = state.set_time(next);

    assert_eq!(previous, TimePoint::new(4));
    assert_eq!(state.time(), next);
    assert_eq!(clones.load(Ordering::SeqCst), 0);
    assert_eq!(
        state
            .get::<CloneTracked>("population")
            .expect("time replacement must preserve payload")
            .values
            .as_ptr(),
        payload_pointer
    );
}

#[test]
fn advance_increments_index_and_optionally_adds_physical_time() {
    let mut indexed = blank_state(3);
    assert_eq!(
        indexed.advance(None).expect("index-only time must advance"),
        TimePoint::new(4)
    );

    let initial = TimePoint::from_physical(10, 2.0).expect("physical time must be finite");
    let mut physical = SystemState::new(StateSpec::fixture(), initial);

    let preserved = physical
        .advance(None)
        .expect("None must preserve physical time");
    assert_eq!(preserved.index(), 11);
    assert_eq!(preserved.physical(), Some(2.0));

    let increased = physical
        .advance(Some(0.5))
        .expect("positive finite delta must advance");
    assert_eq!(increased.index(), 12);
    assert_eq!(increased.physical(), Some(2.5));

    let unchanged_physical = physical
        .advance(Some(0.0))
        .expect("zero delta must still advance the integer index");
    assert_eq!(unchanged_physical.index(), 13);
    assert_eq!(unchanged_physical.physical(), Some(2.5));

    let decreased = physical
        .advance(Some(-1.25))
        .expect("negative finite delta remains valid");
    assert_eq!(decreased.index(), 14);
    assert_eq!(decreased.physical(), Some(1.25));
    assert_eq!(physical.time(), decreased);
}

#[test]
fn failed_advance_is_transactional_for_every_error_class() {
    let mut overflow = blank_state(u64::MAX);
    let before = overflow.time();
    assert!(matches!(
        overflow.advance(None),
        Err(StateError::TimeIndexOverflow { index }) if index == u64::MAX
    ));
    assert_eq!(overflow.time(), before);

    let mut missing = blank_state(9);
    let before = missing.time();
    assert!(matches!(
        missing.advance(Some(0.25)),
        Err(StateError::MissingPhysicalTime { index }) if index == 9
    ));
    assert_eq!(missing.time(), before);

    for delta in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let initial = TimePoint::from_physical(2, 1.0).expect("fixture time must be finite");
        let mut invalid = SystemState::new(StateSpec::fixture(), initial);
        assert!(matches!(
            invalid.advance(Some(delta)),
            Err(StateError::InvalidPhysicalAdvance {
                current,
                delta: rejected,
            }) if current == 1.0
                && (rejected == delta || (rejected.is_nan() && delta.is_nan()))
        ));
        assert_eq!(invalid.time(), initial);
    }

    let initial =
        TimePoint::from_physical(5, f64::MAX).expect("maximum finite coordinate is valid");
    let mut non_finite_sum = SystemState::new(StateSpec::fixture(), initial);
    assert!(matches!(
        non_finite_sum.advance(Some(f64::MAX)),
        Err(StateError::InvalidPhysicalAdvance { current, delta })
            if current == f64::MAX && delta == f64::MAX
    ));
    assert_eq!(non_finite_sum.time(), initial);
}

#[test]
fn blank_state_has_fixed_shape_and_deterministic_fields() {
    let state = blank_state(12);

    assert_eq!(state.time(), TimePoint::new(12));
    assert_eq!(state.len(), 3);
    assert!(!state.is_empty());
    assert_eq!(state.loaded(), 0);
    assert!(state.is_blank());
    assert_eq!(
        state
            .fields()
            .iter()
            .map(|field| field.name())
            .collect::<Vec<_>>(),
        vec!["population", "space", "status"]
    );
    assert!(!state.has("population").expect("field is declared"));
}

#[test]
fn set_mutate_and_take_transfer_payload_without_cloning() {
    let (payload, clones) = CloneTracked::new(vec![10, 20, 30, 40]);
    let original_pointer = payload.values.as_ptr();
    let original_capacity = payload.values.capacity();
    let mut state = blank_state(0);

    state
        .set("population", payload)
        .expect("declared field must accept payload");
    assert_eq!(clones.load(Ordering::SeqCst), 0);
    assert!(state.has("population").expect("field is declared"));
    assert!(
        state
            .is::<CloneTracked>("population")
            .expect("field is declared")
    );
    assert_eq!(state.loaded(), 1);

    state
        .get_mut::<CloneTracked>("population")
        .expect("stored type must match")
        .values[0] = 11;
    assert_eq!(clones.load(Ordering::SeqCst), 0);
    assert_eq!(
        state
            .get::<CloneTracked>("population")
            .expect("stored type must match")
            .values,
        vec![11, 20, 30, 40]
    );

    let extracted = state
        .take::<CloneTracked>("population")
        .expect("stored type must match");

    assert_eq!(clones.load(Ordering::SeqCst), 0);
    assert_eq!(extracted.values.as_ptr(), original_pointer);
    assert_eq!(extracted.values.capacity(), original_capacity);
    assert_eq!(extracted.values, vec![11, 20, 30, 40]);
    assert!(!state.has("population").expect("field is declared"));
    assert!(state.is_blank());
}

#[test]
fn serializable_borrows_the_original_payload_and_preserves_mutability() {
    let (payload, clones) = CloneTracked::new(vec![2, 4, 8]);
    let original_pointer = payload.values.as_ptr();
    let mut state = blank_state(0);
    state
        .set("population", payload)
        .expect("declared field must accept payload");
    let mut encoded = Vec::new();
    let mut serializer = serde_json::Serializer::new(&mut encoded);

    erased_serde::serialize(
        state
            .serializable("population")
            .expect("populated field must expose Serialize"),
        &mut serializer,
    )
    .expect("borrowed field must serialize");

    assert_eq!(encoded, br#"[2,4,8]"#);
    assert_eq!(clones.load(Ordering::SeqCst), 0);
    assert_eq!(
        state
            .get::<CloneTracked>("population")
            .expect("serialization must preserve the concrete payload")
            .values
            .as_ptr(),
        original_pointer
    );

    state
        .get_mut::<CloneTracked>("population")
        .expect("payload must remain mutable")
        .values
        .push(16);
    assert_eq!(
        state
            .get::<CloneTracked>("population")
            .expect("payload must remain populated")
            .values,
        vec![2, 4, 8, 16]
    );

    assert!(matches!(
        state.serializable("space"),
        Err(StateError::MissingValue { field }) if field == "space"
    ));
    assert!(matches!(
        state.serializable("unknown"),
        Err(StateError::UnknownField { field }) if field == "unknown"
    ));
}

#[test]
fn same_type_replacement_returns_the_previous_payload_without_cloning() {
    let (original, original_clones) = CloneTracked::new(vec![1, 2, 3]);
    let original_pointer = original.values.as_ptr();
    let (replacement, replacement_clones) = CloneTracked::new(vec![5, 8, 13]);
    let replacement_pointer = replacement.values.as_ptr();
    let mut state = blank_state(0);

    assert!(
        state
            .set("population", original)
            .expect("first insertion must succeed")
            .is_none()
    );
    let displaced = state
        .set("population", replacement)
        .expect("same-type replacement must succeed")
        .expect("occupied slot must return its previous payload");

    assert_eq!(displaced.values.as_ptr(), original_pointer);
    assert_eq!(displaced.values, vec![1, 2, 3]);
    assert_eq!(
        state
            .get::<CloneTracked>("population")
            .expect("replacement must remain stored")
            .values
            .as_ptr(),
        replacement_pointer
    );
    assert_eq!(original_clones.load(Ordering::SeqCst), 0);
    assert_eq!(replacement_clones.load(Ordering::SeqCst), 0);
}

#[test]
fn rejected_set_returns_incoming_payload_and_preserves_existing_state() {
    let (existing, clones) = CloneTracked::new(vec![21, 34]);
    let existing_pointer = existing.values.as_ptr();
    let mut state = blank_state(0);
    assert!(
        state
            .set("population", existing)
            .expect("first insertion must succeed")
            .is_none()
    );

    let unknown = vec![55_u64, 89];
    let unknown_pointer = unknown.as_ptr();
    let rejection = state
        .set("unknown", unknown)
        .expect_err("undeclared field must reject its payload");
    assert!(matches!(
        rejection.error(),
        StateError::UnknownField { field } if field == "unknown"
    ));
    let (_, unknown) = rejection.into_parts();
    assert_eq!(unknown.as_ptr(), unknown_pointer);
    assert_eq!(unknown, vec![55, 89]);

    let wrong_type = vec![144_u64, 233];
    let wrong_type_pointer = wrong_type.as_ptr();
    let rejection = state
        .set("population", wrong_type)
        .expect_err("occupied different type must reject its payload");
    assert!(matches!(
        rejection.error(),
        StateError::TypeMismatch {
            field,
            expected,
            actual,
        } if field == "population"
            && *expected == std::any::type_name::<Vec<u64>>()
            && *actual == std::any::type_name::<CloneTracked>()
    ));
    let (_, wrong_type) = rejection.into_parts();
    assert_eq!(wrong_type.as_ptr(), wrong_type_pointer);
    assert_eq!(wrong_type, vec![144, 233]);

    assert_eq!(
        state
            .get::<CloneTracked>("population")
            .expect("rejections must preserve existing payload")
            .values
            .as_ptr(),
        existing_pointer
    );
    assert_eq!(clones.load(Ordering::SeqCst), 0);
}

#[test]
fn failed_take_restores_the_original_payload() {
    let (payload, clones) = CloneTracked::new(vec![1, 2, 3]);
    let original_pointer = payload.values.as_ptr();
    let mut state = blank_state(0);
    state
        .set("space", payload)
        .expect("declared field must accept payload");

    let error = state
        .take::<Vec<u64>>("space")
        .expect_err("requested type must not match");

    assert!(matches!(
        error,
        StateError::TypeMismatch {
            ref field,
            expected,
            actual,
        } if field == "space"
            && expected == std::any::type_name::<Vec<u64>>()
            && actual == std::any::type_name::<CloneTracked>()
    ));
    assert_eq!(clones.load(Ordering::SeqCst), 0);

    let restored = state
        .get::<CloneTracked>("space")
        .expect("failed take must restore the payload");
    assert_eq!(restored.values.as_ptr(), original_pointer);
    assert_eq!(restored.values, vec![1, 2, 3]);
}

#[test]
fn explicit_clone_deep_clones_payloads_but_shares_the_layout() {
    let (payload, clones) = CloneTracked::new(vec![4, 5, 6]);
    let mut original = blank_state(21);
    original
        .set("population", payload)
        .expect("declared field must accept payload");

    let mut cloned = original.clone();

    assert_eq!(clones.load(Ordering::SeqCst), 1);
    assert!(original.spec().shares_layout_with(cloned.spec()));
    assert_eq!(cloned.time(), original.time());
    assert_ne!(
        original
            .get::<CloneTracked>("population")
            .expect("payload must exist")
            .values
            .as_ptr(),
        cloned
            .get::<CloneTracked>("population")
            .expect("cloned payload must exist")
            .values
            .as_ptr()
    );

    cloned
        .get_mut::<CloneTracked>("population")
        .expect("cloned payload must exist")
        .values
        .push(7);

    assert_eq!(
        original
            .get::<CloneTracked>("population")
            .expect("original payload must remain")
            .values,
        vec![4, 5, 6]
    );
    assert_eq!(
        cloned
            .get::<CloneTracked>("population")
            .expect("cloned payload must remain")
            .values,
        vec![4, 5, 6, 7]
    );
}

#[test]
fn derived_empty_state_shares_layout_without_cloning_payloads() {
    let (payload, clones) = CloneTracked::new(vec![100, 200]);
    let mut populated = blank_state(3);
    populated
        .set("population", payload)
        .expect("declared field must accept payload");

    let derived = populated.empty(TimePoint::new(4));

    assert_eq!(clones.load(Ordering::SeqCst), 0);
    assert!(populated.spec().shares_layout_with(derived.spec()));
    assert_eq!(derived.time(), TimePoint::new(4));
    assert_eq!(derived.len(), populated.len());
    assert!(derived.is_blank());
    assert_eq!(populated.loaded(), 1);
}

#[test]
fn access_errors_distinguish_unknown_missing_and_mismatched_fields() {
    let mut state = blank_state(0);

    assert!(matches!(
        state.get::<u64>("unknown"),
        Err(StateError::UnknownField { ref field }) if field == "unknown"
    ));
    assert!(matches!(
        state.get::<u64>("status"),
        Err(StateError::MissingValue { ref field }) if field == "status"
    ));

    state
        .set("status", String::from("running"))
        .expect("declared field must accept payload");
    assert!(matches!(
        state.get::<u64>("status"),
        Err(StateError::TypeMismatch {
            ref field,
            expected,
            actual,
        }) if field == "status"
            && expected == std::any::type_name::<u64>()
            && actual == std::any::type_name::<String>()
    ));
    assert_eq!(
        state
            .get::<String>("status")
            .expect("mismatched borrow must not alter payload"),
        "running"
    );
}

#[test]
fn clear_operations_drop_payloads_without_changing_shape() {
    let mut state = blank_state(5);
    state
        .set("population", vec![1_u64])
        .expect("declared field must accept payload");
    state
        .set("status", String::from("ready"))
        .expect("declared field must accept payload");

    assert!(state.clear("population").expect("field is declared"));
    assert!(!state.clear("population").expect("field is declared"));
    assert_eq!(state.loaded(), 1);
    assert_eq!(state.len(), 3);

    state.clear_all();
    assert!(state.is_blank());
    assert_eq!(state.loaded(), 0);
    assert_eq!(state.len(), 3);
}

#[test]
fn debug_output_reports_structure_without_formatting_payloads() {
    let mut state = blank_state(42);
    state
        .set("status", String::from("SECRET_PAYLOAD_CONTENT"))
        .expect("declared field must accept payload");

    let output = format!("{state:?}");

    assert!(output.contains("SystemState"));
    assert!(output.contains("state.test.json"));
    assert!(output.contains("fields"));
    assert!(output.contains('3'));
    assert!(output.contains("loaded"));
    assert!(output.contains('1'));
    assert!(!output.contains("SECRET_PAYLOAD_CONTENT"));
}
