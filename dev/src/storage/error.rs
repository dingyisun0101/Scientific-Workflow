//! Errors produced by persistent scientific-workflow storage.
//!
//! This module owns diagnostic context for the complete storage boundary:
//! versioned metadata, record encoding and decoding, output directories,
//! immutable chunk files, bounded writer queues, and worker lifecycle. It does
//! not redefine errors that belong to the in-memory data model. Instead,
//! [`StorageError`] wraps [`StateError`](crate::system_state::StateError) or
//! [`SeriesError`](crate::time_series::SeriesError) when storage adds stream,
//! record, or filesystem context to one of those failures.
//!
//! # Context ownership
//!
//! Paths, stream names, field names, indices, and validation explanations are
//! owned by each error. An error therefore remains useful after its encoder,
//! reader, writer, decoder, or run coordinator has been dropped. These
//! allocations occur only on failure paths.
//!
//! # Source preservation
//!
//! IO, JSON, state access, series collection, custom field decoding, and
//! terminal worker failures preserve their underlying errors through
//! [`std::error::Error::source`]. Semantic format and lifecycle failures record
//! their complete conflicting values directly because they have no lower-level
//! source.
//!
//! # Responsibility boundary
//!
//! `StorageError` contains no scientific payload. In particular, a decoded
//! state that violates a series invariant is dropped before its
//! [`SeriesError`] is wrapped. Writer-terminal errors are shared through `Arc`
//! so every blocked or later submitter can observe one authoritative failure
//! without requiring `StorageError: Clone`.

use std::error::Error;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use thiserror::Error;

use crate::system_state::StateError;
use crate::time_series::SeriesError;

/// A failure encountered while encoding, writing, reading, or decoding a run.
///
/// Variants are grouped by the boundary that detects them: configuration and
/// lifecycle, persisted-format validation, field processing, filesystem and
/// JSON mechanics, and asynchronous writer coordination.
///
/// The enum is non-exhaustive so future integrity checks or durability modes
/// can add precise variants without forcing downstream crates to exhaustively
/// match every storage failure.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum StorageError {
    // ---------------------------------------------------------------------
    // Configuration and lifecycle
    // ---------------------------------------------------------------------
    /// A new run refused to replace an existing output path.
    ///
    /// Storage never silently overwrites a previous run. A future explicit
    /// recovery or replacement policy must use a separate API.
    #[error("output path `{path}` already exists")]
    OutputExists {
        /// Existing path that prevented run creation.
        path: PathBuf,
    },

    /// One storage setting violates a constructor invariant.
    ///
    /// Settings represented by `NonZero*` types are rejected before this
    /// point. This variant covers relationships between values, unsafe relative
    /// paths, unsupported names, and similar semantic configuration failures.
    #[error("invalid storage setting `{setting}`: {reason}")]
    InvalidConfig {
        /// Stable setting name used by documentation and diagnostics.
        setting: &'static str,
        /// Concise explanation of the violated invariant.
        reason: String,
    },

    /// Two logical output streams were configured with the same name.
    #[error("output stream `{stream}` is configured more than once")]
    DuplicateStream {
        /// Repeated normalized stream name.
        stream: String,
    },

    /// A caller selected a stream absent from the run declaration.
    #[error("run does not declare output stream `{stream}`")]
    UnknownStream {
        /// Requested stream name.
        stream: String,
    },

    /// A caller submitted another record after one stream had been finished.
    #[error("output stream `{stream}` has already finished")]
    StreamFinished {
        /// Finished stream that rejected the operation.
        stream: String,
    },

    /// A caller repeated a run-level operation after successful termination.
    #[error("run output has already finished")]
    RunFinished,

    // ---------------------------------------------------------------------
    // Persisted format and integrity
    // ---------------------------------------------------------------------
    /// `metadata.json` declares a format version this crate cannot read.
    #[error(
        "metadata file `{path}` uses format version {found}, but this crate supports version {supported}"
    )]
    UnsupportedVersion {
        /// Metadata file containing the unsupported declaration.
        path: PathBuf,
        /// Version found in the file.
        found: u32,
        /// Version implemented by this crate.
        supported: u32,
    },

    /// Syntactically valid metadata violates a semantic storage invariant.
    #[error("invalid run metadata in `{path}`: {reason}")]
    InvalidMetadata {
        /// Authoritative metadata file that failed validation.
        path: PathBuf,
        /// Concise invariant violation.
        reason: String,
    },

    /// A reader requiring a successful run encountered incomplete metadata.
    #[error("run metadata `{path}` does not declare successful completion")]
    RunIncomplete {
        /// Metadata file whose lifecycle state is incomplete or failed.
        path: PathBuf,
    },

    /// A committed chunk named by metadata is absent from the filesystem.
    #[error("committed chunk `{path}` is missing")]
    MissingChunk {
        /// Expected chunk path.
        path: PathBuf,
    },

    /// A committed chunk's actual length differs from its metadata descriptor.
    #[error("chunk `{path}` has {actual} bytes, but metadata declares {expected}")]
    ChunkSizeMismatch {
        /// Chunk path whose filesystem length was checked.
        path: PathBuf,
        /// Authoritative encoded byte length from metadata.
        expected: u64,
        /// Encoded byte length reported by the filesystem.
        actual: u64,
    },

    /// A committed chunk's checksum differs from its metadata descriptor.
    #[error("chunk `{path}` checksum is `{actual}`, but metadata declares `{expected}`")]
    ChecksumMismatch {
        /// Chunk path whose contents were checked.
        path: PathBuf,
        /// Authoritative checksum encoded in metadata.
        expected: String,
        /// Checksum computed from the chunk contents.
        actual: String,
    },

    /// One syntactically readable JSONL record violates record invariants.
    #[error("invalid record at line {line} of `{path}`: {reason}")]
    InvalidRecord {
        /// Chunk file containing the invalid record.
        path: PathBuf,
        /// One-based JSONL line number.
        line: u64,
        /// Concise framing or semantic invariant violation.
        reason: String,
    },

    // ---------------------------------------------------------------------
    // State borrowing, encoding, and decoding
    // ---------------------------------------------------------------------
    /// The encoder could not borrow one declared field from the live state.
    #[error("cannot sample field `{field}` for stream `{stream}` at time index {index}: {source}")]
    StateAccess {
        /// Logical output stream being sampled.
        stream: String,
        /// Simulation index of the sampled state.
        index: u64,
        /// Declared stream field that could not be borrowed.
        field: String,
        /// Original SystemState access failure.
        #[source]
        source: StateError,
    },

    /// Serde failed while encoding one borrowed payload.
    #[error("failed to encode field `{field}` for stream `{stream}` at time index {index}")]
    EncodeField {
        /// Logical output stream being encoded.
        stream: String,
        /// Simulation index of the sampled state.
        index: u64,
        /// Field whose payload serializer failed.
        field: String,
        /// Underlying JSON serializer failure.
        #[source]
        source: serde_json::Error,
    },

    /// A decoder registration attempted to reuse one field key.
    #[error("a payload decoder is already registered for field `{field}`")]
    DuplicateDecoder {
        /// Repeated state field key.
        field: String,
    },

    /// No concrete payload decoder was declared for a persisted field key.
    #[error("no payload decoder is registered for field `{field}`")]
    MissingDecoder {
        /// Persisted state field key requiring reconstruction.
        field: String,
    },

    /// A user-supplied field decoder failed to reconstruct its concrete value.
    #[error("failed to decode field `{field}` for stream `{stream}` at time index {index}")]
    DecodeField {
        /// Logical stream being reconstructed.
        stream: String,
        /// Simulation index of the raw record.
        index: u64,
        /// Field whose registered decoder failed.
        field: String,
        /// Decoder-specific failure retained behind an object-safe boundary.
        #[source]
        source: Box<dyn Error + Send + Sync + 'static>,
    },

    /// A reconstructed state violated its destination series invariant.
    ///
    /// The rejected state is intentionally not retained in this error: it was
    /// created from persisted input and is dropped on failed reconstruction,
    /// preventing an error value from pinning arbitrarily large payloads.
    #[error("decoded state for stream `{stream}` at time index {index} cannot enter its series")]
    SeriesInvariant {
        /// Logical stream being reconstructed.
        stream: String,
        /// Simulation index of the rejected decoded state.
        index: u64,
        /// Original in-memory collection invariant failure.
        #[source]
        source: SeriesError,
    },

    // ---------------------------------------------------------------------
    // Filesystem and JSON mechanics
    // ---------------------------------------------------------------------
    /// A filesystem operation failed.
    #[error("failed to {operation} at `{path}`")]
    Io {
        /// Stable action description such as `create chunk` or `sync metadata`.
        operation: &'static str,
        /// Filesystem path involved in the failed operation.
        path: PathBuf,
        /// Underlying operating-system error.
        #[source]
        source: io::Error,
    },

    /// JSON framing, metadata serialization, or raw record parsing failed.
    #[error("failed to {operation} JSON at `{path}`")]
    Json {
        /// Stable action description such as `parse metadata`.
        operation: &'static str,
        /// Metadata or chunk path associated with the JSON operation.
        path: PathBuf,
        /// Underlying Serde JSON failure.
        #[source]
        source: serde_json::Error,
    },

    /// Exact byte accounting overflowed its `u64` persisted representation.
    #[error("encoded byte count overflowed while processing stream `{stream}`")]
    ByteCountOverflow {
        /// Logical stream whose accounting could not be represented.
        stream: String,
    },

    /// One indivisible record exceeds the stream's strict queue-byte budget.
    ///
    /// Returning immediately is essential: waiting for capacity can never
    /// make a record larger than the complete budget admissible.
    #[error(
        "encoded record for stream `{stream}` has {bytes} bytes, exceeding the queue limit of {limit}"
    )]
    RecordTooLarge {
        /// Logical stream that rejected the encoded record.
        stream: String,
        /// Exact framed size of the rejected record.
        bytes: u64,
        /// Configured strict queue-byte limit.
        limit: u64,
    },

    /// A stream submission did not advance its simulation index.
    #[error(
        "record index {index} for stream `{stream}` does not follow previously accepted index {previous}"
    )]
    OutOfOrderRecord {
        /// Logical stream receiving the record.
        stream: String,
        /// Rejected simulation index.
        index: u64,
        /// Most recently accepted simulation index.
        previous: u64,
    },

    // ---------------------------------------------------------------------
    // Queue and writer-worker lifecycle
    // ---------------------------------------------------------------------
    /// The bounded queue disconnected before accepting a complete record.
    #[error("writer queue for stream `{stream}` is disconnected")]
    QueueDisconnected {
        /// Stream whose worker no longer has a live queue receiver.
        stream: String,
    },

    /// A stream worker terminated with an authoritative storage failure.
    ///
    /// The shared source lets multiple blocked submitters observe the same
    /// terminal failure without cloning an IO or JSON error.
    #[error("writer for stream `{stream}` terminated: {source}")]
    WriterTerminated {
        /// Stream whose worker entered the terminal state.
        stream: String,
        /// Shared authoritative worker failure.
        #[source]
        source: Arc<StorageError>,
    },

    /// Joining a writer thread revealed an unexpected panic.
    #[error("writer worker for stream `{stream}` panicked")]
    WriterPanicked {
        /// Stream owned by the panicked worker.
        stream: String,
    },
}
