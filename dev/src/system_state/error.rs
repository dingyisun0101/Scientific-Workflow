//! Errors produced while defining and manipulating system states.
//!
//! This module keeps the SystemState error surface in one place so template
//! loading, layout validation, and typed payload access report failures
//! consistently. The variants are deliberately specific enough for callers to
//! inspect programmatically while their display messages retain the field or
//! path context needed for logs.
//!
//! # Error sources
//!
//! Filesystem and JSON failures preserve their original errors through
//! [`std::error::Error::source`]. Semantic template failures and state-access
//! failures do not wrap another error because they are detected directly by
//! this crate.
//!
//! # Performance
//!
//! These errors are constructed only on failure paths. Owned paths and field
//! names are retained to make an error independent of the state or template
//! that produced it; successful state access does not allocate error context.

use std::io;
use std::path::PathBuf;

use thiserror::Error;

/// A failure encountered while loading a state template or accessing a state.
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

    /// A field definition used an empty or whitespace-only codec type tag.
    #[error("state template field `{field}` has an empty type tag")]
    EmptyTypeTag {
        /// Name of the field with the invalid type tag.
        field: String,
    },

    /// An operation addressed a key that is absent from the template layout.
    #[error("state template does not declare field `{field}`")]
    UnknownField {
        /// Requested field name.
        field: String,
    },

    /// An operation required a payload from a declared but currently empty
    /// field.
    #[error("state field `{field}` does not contain a payload")]
    MissingValue {
        /// Declared field whose slot is empty.
        field: String,
    },

    /// A typed operation requested a different Rust type from the stored one.
    #[error("state field `{field}` contains `{actual}`, but the operation requested `{expected}`")]
    TypeMismatch {
        /// Field on which the typed operation was attempted.
        field: String,
        /// Rust type requested by the caller.
        expected: &'static str,
        /// Rust type currently stored in the field.
        actual: &'static str,
    },

    /// A reconstructed state supplied a slot count incompatible with its
    /// template layout.
    ///
    /// Normal state creation allocates the correct number of slots. This
    /// variant primarily protects deserialization and future storage backends
    /// from constructing structurally invalid states.
    #[error("state payload has {actual} field slots, but its template declares {expected}")]
    FieldCountMismatch {
        /// Number of fields declared by the template.
        expected: usize,
        /// Number of payload slots supplied for the state.
        actual: usize,
    },
}
