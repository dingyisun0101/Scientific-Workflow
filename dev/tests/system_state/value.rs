//! Contract tests for the private `system_state/value.rs` implementation.
//!
//! These tests live outside the source tree by project convention. The value
//! implementation is included directly because `StateValue` is deliberately
//! crate-private and the public `SystemState` facade has not been implemented
//! yet. Once that facade exists, broader ownership behavior will also be
//! covered through public integration tests.
//!
//! The tests focus on the guarantees that justify the erased-value layer:
//!
//! - typed borrows address the original payload;
//! - explicit clones create independent payloads;
//! - consuming extraction preserves owned backing allocations;
//! - failed downcasts return the original erased owner;
//! - erased values remain transferable between threads.

#[path = "../../src/system_state/value.rs"]
mod value;

use value::StateValue;

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
