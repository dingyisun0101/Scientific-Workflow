//! Contract tests for the private system_state/error.rs implementation.
//!
//! This suite includes the production error module directly and exercises its
//! ownership, formatting, source-chain, and checked-time diagnostics in
//! isolation. The tests deliberately use a payload without Debug to prove that
//! rejection diagnostics remain bounded and independent of scientific data.
//!
//! These tests verify:
//!
//! - borrowed inspection of the exact StateError and rejected payload;
//! - zero-clone recovery of the original owned payload;
//! - bounded Debug output without a T: Debug requirement;
//! - Display delegation and standard error-source traversal;
//! - preservation of nested filesystem error sources;
//! - Send support whenever the rejected payload is Send;
//! - exact context and source behavior for checked time-advance failures.

use std::error::Error as _;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[path = "../../src/system_state/error.rs"]
#[allow(dead_code)]
mod error;

use error::{SetError, StateError};

/// Scientific payload intentionally lacking Debug.
///
/// The vector pointer proves ownership identity, while the external counter
/// detects any unexpected Clone call during error construction or recovery.
struct OpaquePayload {
    values: Vec<u64>,
    clones: Arc<AtomicUsize>,
}

impl OpaquePayload {
    /// Creates a payload together with an independent clone observer.
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

impl Clone for OpaquePayload {
    /// Records explicit payload cloning; SetError must never call this method.
    fn clone(&self) -> Self {
        self.clones.fetch_add(1, Ordering::SeqCst);
        Self {
            values: self.values.clone(),
            clones: Arc::clone(&self.clones),
        }
    }
}

#[test]
fn rejection_inspection_and_recovery_preserve_the_original_payload() {
    let (payload, clones) = OpaquePayload::tracked(vec![13, 21, 34]);
    let original_buffer = payload.values.as_ptr();
    let rejection = SetError::new(
        StateError::UnknownField {
            field: "velocity".to_owned(),
        },
        payload,
    );

    assert!(matches!(
        rejection.error(),
        StateError::UnknownField { field } if field == "velocity"
    ));
    assert_eq!(rejection.payload().values.as_ptr(), original_buffer);
    assert_eq!(rejection.payload().values, vec![13, 21, 34]);
    assert_eq!(clones.load(Ordering::SeqCst), 0);
    assert_eq!(
        rejection.to_string(),
        "state template does not declare field `velocity`"
    );

    let debug = format!("{rejection:?}");
    assert!(debug.contains("SetError"));
    assert!(debug.contains("UnknownField"));
    assert!(debug.contains("OpaquePayload"));
    assert!(!debug.contains("13"));
    assert!(!debug.contains("21"));
    assert!(!debug.contains("34"));

    let source = rejection
        .source()
        .expect("SetError must expose its StateError reason");
    assert_eq!(
        source.to_string(),
        "state template does not declare field `velocity`"
    );

    let (reason, payload) = rejection.into_parts();
    assert!(matches!(
        reason,
        StateError::UnknownField { ref field } if field == "velocity"
    ));
    assert_eq!(payload.values.as_ptr(), original_buffer);
    assert_eq!(payload.values, vec![13, 21, 34]);
    assert_eq!(clones.load(Ordering::SeqCst), 0);
}

#[test]
fn nested_state_error_sources_remain_traversable() {
    let (payload, clones) = OpaquePayload::tracked(vec![1]);
    let rejection = SetError::new(
        StateError::TemplateRead {
            path: PathBuf::from("missing/state.json"),
            source: io::Error::new(io::ErrorKind::NotFound, "fixture missing"),
        },
        payload,
    );

    assert_eq!(
        rejection.to_string(),
        "failed to read state template `missing/state.json`"
    );
    let state_source = rejection.source().expect("SetError must expose StateError");
    let io_source = state_source
        .source()
        .expect("StateError must retain its filesystem source");
    assert_eq!(io_source.to_string(), "fixture missing");
    assert_eq!(clones.load(Ordering::SeqCst), 0);
}

#[test]
fn set_error_is_send_for_send_payloads() {
    fn assert_send<T: Send>() {}

    assert_send::<SetError<OpaquePayload>>();
}

#[test]
fn time_advance_errors_preserve_context_without_wrapped_sources() {
    let overflow = StateError::TimeIndexOverflow { index: u64::MAX };
    assert_eq!(
        overflow.to_string(),
        "cannot advance state time index 18446744073709551615: the next index exceeds u64::MAX"
    );
    assert!(overflow.source().is_none());
    assert!(matches!(
        overflow,
        StateError::TimeIndexOverflow { index } if index == u64::MAX
    ));

    let missing = StateError::MissingPhysicalTime { index: 17 };
    assert_eq!(
        missing.to_string(),
        "cannot advance physical time at state index 17: no physical coordinate is present"
    );
    assert!(missing.source().is_none());
    assert!(matches!(
        missing,
        StateError::MissingPhysicalTime { index } if index == 17
    ));

    let invalid = StateError::InvalidPhysicalAdvance {
        current: 1.25,
        delta: f64::INFINITY,
    };
    let display = invalid.to_string();
    assert!(display.contains("cannot advance physical time 1.25"));
    assert!(display.contains("by inf"));
    assert!(display.contains("delta and resulting coordinate must be finite"));
    assert!(invalid.source().is_none());
    assert!(matches!(
        invalid,
        StateError::InvalidPhysicalAdvance { current, delta }
            if current == 1.25 && delta == f64::INFINITY
    ));
}
