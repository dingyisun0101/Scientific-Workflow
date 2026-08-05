//! Contract tests for `time_series/error.rs`.
//!
//! The public time-series facade remains disconnected until every component
//! file has completed review. This focused suite therefore includes the
//! production error module directly and supplies the real SystemState error at
//! the crate-root path expected by that module.
//!
//! The tests cover only in-memory collection concerns:
//!
//! - shared-layout and increasing-index invariant diagnostics;
//! - zero-based analysis-position diagnostics;
//! - position-aware preservation of typed `StateError` sources;
//! - non-exhaustive downstream matching; and
//! - thread-transfer marker traits needed by parallel analysis code.
//!
//! Persistence errors are intentionally absent. JSON, metadata, chunks,
//! filesystems, and writer lifecycles belong to the future storage module.

use std::error::Error as _;

#[path = "../../src/system_state/error.rs"]
#[allow(dead_code)]
mod state_error;

/// Reproduces the crate-root import used by the production time-series error
/// module without substituting a test double for `StateError`.
mod system_state {
    pub use super::state_error::StateError;
}

#[path = "../../src/time_series/error.rs"]
mod error;

use error::SeriesError;
use system_state::StateError;

#[test]
fn append_invariant_errors_retain_exact_time_context() {
    let mismatch = SeriesError::SpecMismatch { index: 41 };
    assert_eq!(
        mismatch.to_string(),
        "state at time index 41 does not share the series specification"
    );
    assert!(matches!(mismatch, SeriesError::SpecMismatch { index: 41 }));

    let ordering = SeriesError::NonIncreasingTime {
        previous: 41,
        next: 40,
    };
    assert_eq!(
        ordering.to_string(),
        "state time index 40 must be greater than the previous index 41"
    );
    assert!(matches!(
        ordering,
        SeriesError::NonIncreasingTime {
            previous: 41,
            next: 40
        }
    ));
}

#[test]
fn position_error_distinguishes_series_position_from_time_index() {
    let error = SeriesError::PositionOutOfBounds {
        position: 7,
        len: 3,
    };

    assert_eq!(
        error.to_string(),
        "state-series position 7 is out of bounds for length 3"
    );
    assert!(matches!(
        error,
        SeriesError::PositionOutOfBounds {
            position: 7,
            len: 3
        }
    ));
}

#[test]
fn field_access_adds_position_and_preserves_the_real_state_error() {
    let error = SeriesError::FieldAccess {
        position: 2,
        source: StateError::TypeMismatch {
            field: "population".to_owned(),
            expected: "alloc::vec::Vec<f64>",
            actual: "alloc::vec::Vec<u64>",
        },
    };

    assert_eq!(
        error.to_string(),
        "cannot access state-series position 2: state field `population` contains `alloc::vec::Vec<u64>`, but the operation requested `alloc::vec::Vec<f64>`"
    );

    let source = error
        .source()
        .expect("FieldAccess must retain its typed SystemState source");
    assert_eq!(
        source.to_string(),
        "state field `population` contains `alloc::vec::Vec<u64>`, but the operation requested `alloc::vec::Vec<f64>`"
    );
    assert!(matches!(
        source.downcast_ref::<StateError>(),
        Some(StateError::TypeMismatch {
            field,
            expected: "alloc::vec::Vec<f64>",
            actual: "alloc::vec::Vec<u64>"
        }) if field == "population"
    ));
    assert!(source.source().is_none());
}

#[test]
fn downstream_matching_retains_a_non_exhaustive_fallback() {
    /// Models the match shape required in a crate that consumes this public,
    /// non-exhaustive error enum.
    fn category(error: &SeriesError) -> &'static str {
        // This focused test includes the production enum in the same crate, so
        // rustc can see that the fallback is presently unreachable. External
        // consumers still require it because SeriesError is non-exhaustive.
        #[allow(
            unreachable_patterns,
            reason = "models the fallback required across the public crate boundary"
        )]
        match error {
            SeriesError::SpecMismatch { .. } | SeriesError::NonIncreasingTime { .. } => "append",
            SeriesError::PositionOutOfBounds { .. } | SeriesError::FieldAccess { .. } => "analysis",
            _ => "future",
        }
    }

    assert_eq!(category(&SeriesError::SpecMismatch { index: 1 }), "append");
    assert_eq!(
        category(&SeriesError::PositionOutOfBounds {
            position: 1,
            len: 0,
        }),
        "analysis"
    );
}

#[test]
fn series_error_is_send_and_sync() {
    /// Fails to compile if a future variant introduces a payload that cannot
    /// cross or be observed across analysis worker threads.
    fn assert_send_sync<T: Send + Sync>() {}

    assert_send_sync::<SeriesError>();
}
