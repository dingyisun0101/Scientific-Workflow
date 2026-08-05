//! Contract tests for the private `system_state/value.rs` implementation.
//!
//! These tests live outside the source tree by project convention. The value
//! implementation is included directly because `StateValue` is deliberately
//! crate-private. Broader ownership behavior is independently covered through
//! the public `SystemState` integration test.
//!
//! The tests focus on the guarantees that justify the erased-value layer:
//!
//! - typed borrows address the original payload;
//! - explicit clones create independent payloads;
//! - consuming extraction preserves owned backing allocations;
//! - failed downcasts return the original erased owner;
//! - erased serialization borrows the original value without cloning it;
//! - erased values remain transferable between threads.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde::{Serialize, Serializer};

#[path = "../../src/system_state/value.rs"]
mod value;

use value::StateValue;

/// Serializable payload whose Clone calls are externally observable.
struct CloneTracked {
    values: Vec<u64>,
    clones: Arc<AtomicUsize>,
}

impl CloneTracked {
    /// Creates a payload and its independent clone counter.
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
    /// Records deliberate payload cloning.
    fn clone(&self) -> Self {
        self.clones.fetch_add(1, Ordering::SeqCst);
        Self {
            values: self.values.clone(),
            clones: Arc::clone(&self.clones),
        }
    }
}

impl Serialize for CloneTracked {
    /// Serializes only the scientific values; the test counter is not data.
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.values.serialize(serializer)
    }
}

#[test]
fn borrowed_downcasts_access_the_original_payload() {
    let mut value = StateValue::new(vec![1_u64, 2, 3]);

    assert!(value.is::<Vec<u64>>());
    assert!(!value.is::<Vec<u32>>());
    assert_eq!(value.downcast_ref::<Vec<u64>>(), Some(&vec![1, 2, 3]));

    value
        .downcast_mut::<Vec<u64>>()
        .expect("the requested type matches")
        .push(4);

    assert_eq!(value.downcast_ref::<Vec<u64>>(), Some(&vec![1, 2, 3, 4]));
}

#[test]
fn clone_deep_clones_the_concrete_payload() {
    let original = StateValue::new(vec![1_u64, 2, 3]);
    let mut cloned = original.clone();

    cloned
        .downcast_mut::<Vec<u64>>()
        .expect("the requested type matches")
        .push(4);

    assert_eq!(original.downcast_ref::<Vec<u64>>(), Some(&vec![1, 2, 3]));
    assert_eq!(cloned.downcast_ref::<Vec<u64>>(), Some(&vec![1, 2, 3, 4]));
}

#[test]
fn consuming_downcast_preserves_a_vec_backing_allocation() {
    let payload = vec![10_u64, 20, 30, 40];
    let original_pointer = payload.as_ptr();
    let original_capacity = payload.capacity();

    let value = StateValue::new(payload);
    let extracted = value
        .downcast::<Vec<u64>>()
        .expect("the requested type matches");

    assert_eq!(extracted.as_ptr(), original_pointer);
    assert_eq!(extracted.capacity(), original_capacity);
    assert_eq!(extracted, vec![10, 20, 30, 40]);
}

#[test]
fn failed_consuming_downcast_returns_the_original_value() {
    let value = StateValue::new(vec![1_u64, 2, 3]);

    let value = value
        .downcast::<Vec<u32>>()
        .expect_err("the requested type does not match");

    assert!(value.is::<Vec<u64>>());
    assert_eq!(value.downcast_ref::<Vec<u64>>(), Some(&vec![1, 2, 3]));
}

#[test]
fn erased_serialization_borrows_without_cloning_or_replacing_the_payload() {
    let (payload, clones) = CloneTracked::new(vec![3, 5, 8]);
    let original_pointer = payload.values.as_ptr();
    let mut value = StateValue::new(payload);
    let mut encoded = Vec::new();
    let mut serializer = serde_json::Serializer::new(&mut encoded);

    erased_serde::serialize(value.serializable(), &mut serializer)
        .expect("borrowed erased payload must serialize");

    assert_eq!(encoded, br#"[3,5,8]"#);
    assert_eq!(clones.load(Ordering::SeqCst), 0);
    assert_eq!(
        value
            .downcast_ref::<CloneTracked>()
            .expect("serialization must preserve the concrete value")
            .values
            .as_ptr(),
        original_pointer
    );

    value
        .downcast_mut::<CloneTracked>()
        .expect("payload must remain mutable after serialization")
        .values
        .push(13);
    assert_eq!(
        value
            .downcast_ref::<CloneTracked>()
            .expect("payload type must remain unchanged")
            .values,
        vec![3, 5, 8, 13]
    );
}

#[test]
fn debug_output_describes_the_type_without_printing_the_payload() {
    let value = StateValue::new(vec![10_u64, 20, 30]);
    let output = format!("{value:?}");

    assert!(output.contains("StateValue"));
    assert!(output.contains("Vec<u64>"));
    assert!(!output.contains("[10, 20, 30]"));
}

#[test]
fn state_value_is_send_when_its_payload_is_send() {
    fn assert_send<T: Send>() {}

    assert_send::<StateValue>();
}
