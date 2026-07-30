//! Contract tests for `system_state/state.rs`.
//!
//! These tests live outside the source tree by project convention. The
//! production state and erased-value implementations are included directly
//! while the crate's public module facade remains intentionally unwired. A
//! narrow test-only specification supplies the exact interface required by
//! `SystemState`; loading and validating real JSON specifications is covered
//! independently by `tests/system_state/spec.rs`.
//!
//! The suite verifies the state-specific guarantees that matter to scientific
//! workloads:
//!
//! - time points reject non-finite physical coordinates;
//! - state shape is fixed by a shared specification;
//! - insertion, mutation, and extraction do not clone payloads;
//! - extraction preserves owned backing allocations;
//! - failed typed extraction restores the original payload;
//! - explicit state cloning deeply clones populated payloads;
//! - empty-state derivation shares only immutable layout metadata;
//! - field-access failures remain precise and non-destructive;
//! - debug formatting never traverses scientific payloads.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[path = "../../src/system_state/error.rs"]
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
