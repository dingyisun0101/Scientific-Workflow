//! Errors produced by in-memory state series and their persistent JSON form.
//!
//! A time-series operation crosses several distinct boundaries: state-layout
//! validation, temporal ordering, payload codecs, versioned JSON structures,
//! filesystem durability, and writer lifecycle. [`SeriesError`] keeps those
//! failures in one public, inspectable type without exposing private manifest
//! or chunk representations.
//!
//! # Error context
//!
//! Paths, field names, type tags, time indices, and byte counts are owned by
//! error values. An error therefore remains useful after the state, reader, or
//! writer that produced it has been dropped. These allocations occur only on
//! failure paths; successful series operations do not allocate error context.
//!
//! # Source preservation
//!
//! Filesystem, JSON, state-access, and payload-codec failures retain their
//! original errors through [`std::error::Error::source`]. Semantic validation
//! failures record the conflicting values directly because they do not wrap a
//! lower-level error.
//!
//! # Extensibility
//!
//! The enum is non-exhaustive. Future persistence backends, integrity checks,
//! and recovery policies may add variants without requiring downstream crates
//! to exhaustively match every possible failure.

use std::error::Error;
use std::io;
use std::path::PathBuf;

use thiserror::Error;

use crate::system_state::StateError;

/// A failure encountered while collecting, encoding, writing, or reading a
/// time series of system states.
///
/// Variants are grouped by the boundary that detects them. Callers may match a
/// specific recoverable condition, such as [`SeriesError::MissingCodec`] or
/// [`SeriesError::WriterFinished`], and should retain a fallback arm for later
/// additions.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SeriesError {
    /// A state does not share the exact immutable specification allocation
    /// owned by the destination series.
    ///
    /// Structural equality is intentionally insufficient. Requiring shared
    /// identity makes compatibility checks constant-time and guarantees that
    /// every state in a series refers to one canonical [`StateSpec`].
    ///
    /// [`StateSpec`]: crate::system_state::StateSpec
    #[error("state at time index {index} does not share the series specification")]
    SpecMismatch {
        /// Time index of the state rejected by the series.
        index: u64,
    },

    /// A state was appended at an index that was not greater than the current
    /// final index.
    ///
    /// Gaps are permitted, but duplicate or decreasing indices would make
    /// chunk ranges and ordered iteration ambiguous.
    #[error("state time index {next} must be greater than the previous index {previous}")]
    NonIncreasingTime {
        /// Current final index in the destination series.
        previous: u64,
        /// Index of the rejected state.
        next: u64,
    },

    /// No payload codec is registered for a stable type tag declared by the
    /// state specification.
    #[error("no payload codec is registered for type tag `{type_tag}`")]
    MissingCodec {
        /// Stable serialization tag that could not be resolved.
        type_tag: String,
    },

    /// A codec registration attempted to reuse an existing stable type tag.
    ///
    /// Rejecting duplicates prevents registration order from silently changing
    /// the concrete Rust type or persisted meaning associated with a tag.
    #[error("a payload codec is already registered for type tag `{type_tag}`")]
    DuplicateCodec {
        /// Stable serialization tag supplied by both registrations.
        type_tag: String,
    },

    /// A registered codec and a populated state field disagree about the
    /// concrete Rust payload type.
    #[error(
        "state field `{field}` with type tag `{type_tag}` contains `{actual}`, but its codec expects `{expected}`"
    )]
    CodecTypeMismatch {
        /// Human-facing field name from the shared state specification.
        field: String,
        /// Stable serialization tag used to resolve the codec.
        type_tag: String,
        /// Fully qualified Rust type name registered by the codec.
        expected: &'static str,
        /// Fully qualified Rust type name stored in the state.
        actual: &'static str,
    },

    /// A codec failed while encoding one populated field.
    ///
    /// The boxed source permits user-defined payload codecs to retain their
    /// native error types without making this core error enum generic.
    #[error("failed to encode state field `{field}` with codec `{type_tag}`")]
    EncodePayload {
        /// Field whose payload could not be encoded.
        field: String,
        /// Stable tag of the codec that reported the failure.
        type_tag: String,
        /// Codec-specific underlying failure.
        #[source]
        source: Box<dyn Error + Send + Sync + 'static>,
    },

    /// A codec failed while reconstructing one populated field.
    ///
    /// The boxed source permits user-defined payload codecs to retain their
    /// native error types without making this core error enum generic.
    #[error("failed to decode state field `{field}` with codec `{type_tag}`")]
    DecodePayload {
        /// Field whose payload could not be reconstructed.
        field: String,
        /// Stable tag of the codec that reported the failure.
        type_tag: String,
        /// Codec-specific underlying failure.
        #[source]
        source: Box<dyn Error + Send + Sync + 'static>,
    },

    /// A metadata file declares a storage-format version this crate does not
    /// understand.
    #[error(
        "metadata file `{path}` uses format version {found}, but this crate supports version {supported}"
    )]
    UnsupportedVersion {
        /// Path of the metadata file containing the version declaration.
        path: PathBuf,
        /// Version read from the file.
        found: u32,
        /// Version implemented by this crate.
        supported: u32,
    },

    /// Parsed `series.json` metadata violates a semantic format invariant.
    ///
    /// JSON syntax failures use [`SeriesError::Json`]. This variant covers
    /// valid JSON with inconsistent chunk ranges, counts, paths, completion
    /// state, or other version-specific metadata.
    #[error("invalid series metadata in `{path}`: {reason}")]
    InvalidMetadata {
        /// Path of the invalid `series.json` file.
        path: PathBuf,
        /// Concise invariant violation suitable for logs.
        reason: String,
    },

    /// A parsed payload chunk violates a semantic format invariant.
    ///
    /// Examples include incorrect state ordering, undeclared field keys, or a
    /// state count that disagrees with its metadata descriptor.
    #[error("invalid payload chunk `{path}`: {reason}")]
    InvalidChunk {
        /// Path of the invalid chunk file.
        path: PathBuf,
        /// Concise invariant violation suitable for logs.
        reason: String,
    },

    /// A payload chunk referenced by `series.json` does not exist.
    #[error("payload chunk `{path}` is missing")]
    MissingChunk {
        /// Referenced chunk path that could not be found.
        path: PathBuf,
    },

    /// A payload chunk's filesystem length differs from its metadata
    /// descriptor.
    ///
    /// This provides inexpensive truncation or replacement detection before
    /// JSON decoding. Future checksums may provide stronger integrity checks.
    #[error("payload chunk `{path}` has {actual} bytes, but its metadata declares {expected}")]
    ChunkSizeMismatch {
        /// Path of the chunk whose length was checked.
        path: PathBuf,
        /// Encoded byte length declared in `series.json`.
        expected: u64,
        /// Encoded byte length reported by the filesystem.
        actual: u64,
    },

    /// A filesystem operation required by the reader or writer failed.
    ///
    /// `operation` is a static description such as `"create chunk"`,
    /// `"synchronize metadata"`, or `"rename temporary file"`; it does not
    /// allocate on the error path.
    #[error("failed to {operation} at `{path}`")]
    Io {
        /// Static description of the attempted filesystem operation.
        operation: &'static str,
        /// Path on which the operation was attempted.
        path: PathBuf,
        /// Underlying operating-system error.
        #[source]
        source: io::Error,
    },

    /// JSON encoding or decoding failed for a metadata or chunk file.
    #[error("failed to process JSON at `{path}`")]
    Json {
        /// Metadata, final chunk, or temporary output path being processed.
        path: PathBuf,
        /// Underlying Serde JSON error.
        #[source]
        source: serde_json::Error,
    },

    /// A state-level operation failed while a series codec reconstructed or
    /// inspected a [`SystemState`].
    ///
    /// [`SystemState`]: crate::system_state::SystemState
    #[error(transparent)]
    State(#[from] StateError),

    /// An append or flush was requested after successful writer finalization.
    #[error("the series writer has already been finished")]
    WriterFinished,

    /// An operation was requested after an earlier commit failure placed the
    /// writer in its terminal failed state.
    ///
    /// The operation that first fails returns its detailed originating error.
    /// This variant reports subsequent calls without duplicating or discarding
    /// ownership of the retained failed chunk.
    #[error("the series writer is unavailable after an earlier commit failure")]
    WriterFailed,
}
