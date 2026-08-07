//! Errors produced while defining and manipulating system states.
//!
//! This module keeps the SystemState error surface in one place so template
//! loading, layout validation, time advancement, and typed payload access
//! report failures consistently. The variants are deliberately specific
//! enough for callers to inspect programmatically while their display messages
//! retain the field, path, or time context needed for logs.
//!
//! # Error sources
//!
//! Filesystem and JSON failures preserve their original errors through
//! [`std::error::Error::source`]. Semantic template failures and state-access
//! failures do not wrap another error because they are detected directly by
//! this crate. Checked time-advance failures likewise retain their complete
//! numeric context directly in [`StateError`].
//!
//! # Performance
//!
//! These errors are constructed only on failure paths. Owned paths and field
//! names are retained to make an error independent of the state or template
//! that produced it; successful state access does not allocate error context.
//!
//! # Ownership-preserving insertion failures
//!
//! [`SetError`] is generic because it returns ownership of a payload that
//! [`SystemState::set`](super::state::SystemState::set) could not accept. Its
//! diagnostics deliberately omit the payload value, so scientific data does
//! not need to implement [`Debug`](std::fmt::Debug) and is never traversed or
//! copied merely to format an error.

use std::error::Error;
use std::fmt;
use std::io;
use std::path::PathBuf;

use thiserror::Error;

/// A failure encountered while defining, accessing, or advancing a state.
///
/// `StateError` is non-exhaustive because later workflow features may add
/// validation failures without forcing downstream crates to update exhaustive
/// matches. Callers should match variants of interest and retain a fallback
/// arm.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum StateError {
    /// A JSON state template could not be read from the filesystem.
    #[error("failed to read state template `{path}`")]
    TemplateRead {
        /// Path passed to the template loader.
        path: PathBuf,
        /// Underlying filesystem error.
        #[source]
        source: io::Error,
    },

    /// A state template was readable but did not contain valid JSON.
    #[error("failed to parse state template `{path}` as JSON")]
    TemplateParse {
        /// Path of the malformed template.
        path: PathBuf,
        /// Underlying JSON syntax or data-model error.
        #[source]
        source: serde_json::Error,
    },

    /// A field definition used an empty or whitespace-only name.
    #[error("state template field at index {index} has an empty name")]
    EmptyFieldName {
        /// Zero-based position of the invalid field in template order.
        index: usize,
    },

    /// Two field definitions used the same name.
    #[error("state template declares duplicate field `{field}`")]
    DuplicateField {
        /// Repeated field name.
        field: String,
    },

    /// An operation addressed a key that is absent from the template layout.
    #[error("state template does not declare field `{field}`")]
    UnknownField {
        /// Requested field name.
        field: String,
    },

    /// One coordinated borrow requested the same resolved field more than once.
    ///
    /// Mutable aliases to one payload would violate Rust's exclusivity rules.
    /// Immutable coordinated borrows reject the same input as well so
    /// `SystemState::borrow` and `SystemState::borrow_mut` retain identical,
    /// predictable request validation.
    #[error("coordinated state borrow repeats field `{field}`")]
    RepeatedBorrow {
        /// Field name at the first repeated tuple position.
        field: String,
    },

    /// An operation required a payload from a declared but currently empty
    /// field.
    #[error("state field `{field}` does not contain a payload")]
    MissingValue {
        /// Declared field whose slot is empty.
        field: String,
    },

    /// A typed operation requested a different Rust type from the retained
    /// field contract.
    #[error(
        "state field `{field}` is bound to `{actual}`, but the operation requested `{expected}`"
    )]
    TypeMismatch {
        /// Field on which the typed operation was attempted.
        field: String,
        /// Rust type requested by the caller.
        expected: &'static str,
        /// Rust type bound to the field during state assembly.
        actual: &'static str,
    },

    /// Incrementing the authoritative integer time index would overflow `u64`.
    ///
    /// `SystemState::advance` will detect this condition before mutating the
    /// state, so the original time point remains unchanged.
    #[error("cannot advance state time index {index}: the next index exceeds u64::MAX")]
    TimeIndexOverflow {
        /// Current index that cannot be incremented.
        index: u64,
    },

    /// A physical-time delta was requested for a state without a physical
    /// coordinate.
    ///
    /// Absence is not interpreted as zero: callers must establish a known
    /// origin explicitly before advancing physical time.
    #[error(
        "cannot advance physical time at state index {index}: no physical coordinate is present"
    )]
    MissingPhysicalTime {
        /// Integer index at which physical advancement was requested.
        index: u64,
    },

    /// A physical-time delta or its sum with the current coordinate is not
    /// finite.
    ///
    /// Both operands are retained for diagnosis. This variant covers a
    /// non-finite input delta and finite operands whose addition overflows to
    /// infinity. The state remains unchanged.
    #[error(
        "cannot advance physical time {current} by {delta}: the delta and resulting coordinate must be finite"
    )]
    InvalidPhysicalAdvance {
        /// Current finite physical coordinate.
        current: f64,
        /// Requested delta, which may itself be non-finite.
        delta: f64,
    },
}

/// A failed [`SystemState::set`](super::state::SystemState::set) operation that
/// retains ownership of the unchanged incoming payload.
///
/// A set operation can fail before moving `payload` into a state because the
/// requested field is undeclared or because its assembly-retained type contract
/// names a different concrete Rust type. The latter remains true even when the
/// field is temporarily empty after `take` or `clear`. Returning only
/// [`StateError`] in those cases would drop the caller's payload while unwinding
/// the failed call. `SetError` instead keeps the rejection reason and original
/// `T` together, following the ownership-preserving pattern of channel send
/// errors.
///
/// The payload remains private so diagnostics cannot accidentally expose or
/// traverse large scientific data. Borrow it through [`SetError::payload`] or
/// recover ownership of both components through [`SetError::into_parts`].
/// Neither operation invokes [`Clone`].
///
/// # Formatting
///
/// [`Display`](fmt::Display) delegates to the contained [`StateError`]. The
/// bounded [`Debug`](fmt::Debug) representation includes only that error and
/// the compile-time Rust type name of `T`; it never requires `T: Debug` or
/// formats the payload value.
#[must_use = "the rejected payload remains owned by this error until it is recovered or dropped"]
pub struct SetError<T> {
    error: StateError,
    payload: T,
}

impl<T> SetError<T> {
    /// Creates an ownership-preserving set rejection.
    ///
    /// This constructor is crate-private because only SystemState validation
    /// may determine that a payload was rejected. Public callers receive a
    /// `SetError<T>` from [`SystemState::set`](super::state::SystemState::set)
    /// and recover its contents through the accessors below.
    pub(crate) const fn new(error: StateError, payload: T) -> Self {
        Self { error, payload }
    }

    /// Returns the state-validation error that rejected the payload.
    ///
    /// Borrowing the reason leaves the incoming payload owned by this error,
    /// allowing callers to inspect the failure before deciding how to recover
    /// or dispose of the scientific data.
    pub const fn error(&self) -> &StateError {
        &self.error
    }

    /// Returns the unchanged rejected payload by shared reference.
    ///
    /// The returned reference points to the same concrete `T` moved into
    /// [`SystemState::set`](super::state::SystemState::set). No payload clone,
    /// serialization, downcast, or backing-buffer copy occurs.
    pub const fn payload(&self) -> &T {
        &self.payload
    }

    /// Consumes the rejection and returns its reason and original payload.
    ///
    /// The tuple is ordered as `(StateError, T)`, matching the borrowed
    /// [`SetError::error`] then [`SetError::payload`] inspection order. The
    /// payload moves directly out of the error and retains its original owned
    /// allocations.
    pub fn into_parts(self) -> (StateError, T) {
        (self.error, self.payload)
    }
}

impl<T> fmt::Debug for SetError<T> {
    /// Formats bounded diagnostic context without requiring or inspecting
    /// `T: Debug`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SetError")
            .field("error", &self.error)
            .field("payload_type", &std::any::type_name::<T>())
            .finish_non_exhaustive()
    }
}

impl<T> fmt::Display for SetError<T> {
    /// Delegates the user-facing message to the state-validation reason.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.error, formatter)
    }
}

impl<T> Error for SetError<T> {
    /// Exposes the contained [`StateError`] for standard error-chain traversal.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.error)
    }
}
