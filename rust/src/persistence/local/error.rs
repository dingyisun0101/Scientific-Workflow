//! Errors produced by scientific-workflow persistence.
//!
//! This module owns diagnostic context for the complete persistence boundary:
//! versioned metadata, observation integration, record decoding, output directories,
//! immutable chunk files, bounded writer queues, and worker lifecycle. It does
//! not redefine errors that belong to the in-memory data model. Instead,
//! [`PersistenceError`] wraps [`ObservationError`] or [`StateSeriesError`] when persistence adds stream,
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
//! `PersistenceError` contains no scientific payload. In particular, a decoded
//! state that violates a series invariant is dropped before its
//! [`StateSeriesError`] is wrapped. Writer-terminal errors are shared through `Arc`
//! so every blocked or later submitter can observe one authoritative failure
//! without requiring `PersistenceError: Clone`.

use std::error::Error;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use thiserror::Error;

use crate::observation::advanced::ObservationError;
use crate::state::advanced::StateSeriesError;

/// A failure encountered while encoding, writing, reading, or decoding a run.
///
/// Variants are grouped by the boundary that detects them: configuration and
/// lifecycle, persisted-format validation, field processing, filesystem and
/// JSON mechanics, and asynchronous writer coordination.
///
/// The enum is non-exhaustive so future integrity checks or durability modes
/// can add precise variants without forcing downstream crates to exhaustively
/// match every persistence failure.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PersistenceError {
    /// The scientific observation layer rejected a plan or live observation.
    #[error("scientific observation rejected the recording operation: {source}")]
    Observation {
        /// Original observation-layer failure.
        #[source]
        source: ObservationError,
    },

    // ---------------------------------------------------------------------
    // Configuration and lifecycle
    // ---------------------------------------------------------------------
    /// A new recording refused to replace an existing path.
    ///
    /// Persistence never silently overwrites a previous recording. Existing
    /// Running recordings are accepted only through continuation APIs.
    #[error("recording directory `{path}` already exists")]
    RecordingDirectoryExists {
        /// Existing path that prevented recording creation.
        path: PathBuf,
    },

    /// One persistence setting violates a constructor invariant.
    ///
    /// Settings represented by `NonZero*` types are rejected before this
    /// point. This variant covers relationships between values, unsafe relative
    /// paths, unsupported names, and similar semantic configuration failures.
    #[error("invalid persistence setting `{setting}`: {reason}")]
    InvalidConfiguration {
        /// Stable setting name used by documentation and diagnostics.
        setting: &'static str,
        /// Concise explanation of the violated invariant.
        reason: String,
    },

    /// Two logical output streams were configured with the same name.
    #[error("output stream `{stream}` is configured more than once")]
    DuplicateStateStream {
        /// Repeated normalized stream name.
        stream: String,
    },

    /// A caller selected a stream absent from the recording declaration.
    #[error("recording does not declare state stream `{stream}`")]
    UnknownStateStream {
        /// Requested stream name.
        stream: String,
    },

    /// The recording-wide writer has stopped accepting new work.
    #[error("system-state writer has stopped accepting records")]
    StateWriterClosed,

    /// A caller repeated a recording operation after successful termination.
    #[error("state recording has already finished")]
    RecordingFinished,

    /// Another writer currently owns the recording directory.
    #[error("state recording `{path}` is already owned by another writer")]
    RecordingDirectoryInUse {
        /// Output root whose advisory exclusive lease could not be acquired.
        path: PathBuf,
    },

    /// A completed stream contains no record to reconstruct.
    #[error("stream `{stream}` contains no recorded state")]
    NoRecordedState {
        /// Logical completed stream searched for its latest state.
        stream: String,
    },

    /// The host UTC clock could not be represented in the canonical metadata
    /// timestamp format.
    #[error("failed to format the operational timestamp while attempting to {operation}")]
    OperationalTimestamp {
        /// Lifecycle action requesting the timestamp.
        operation: &'static str,
        /// Timestamp-formatting failure.
        #[source]
        source: time::error::Format,
    },

    /// A monotonic writer session exceeded the exact persisted duration range.
    #[error("active recording duration exceeds the supported u64 nanosecond range")]
    OperationalDurationOverflow,

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

    /// Syntactically valid metadata violates a semantic persistence invariant.
    #[error("invalid recording metadata in `{path}`: {reason}")]
    InvalidMetadata {
        /// Authoritative metadata file that failed validation.
        path: PathBuf,
        /// Concise invariant violation.
        reason: String,
    },

    /// A reader requiring a completed recording encountered terminally
    /// unsuitable metadata.
    #[error("recording metadata `{path}` does not declare successful completion")]
    RecordingNotComplete {
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
    #[error("failed to decode field `{field}` for stream `{stream}` at iteration {iteration}")]
    DecodeField {
        /// Logical stream being reconstructed.
        stream: String,
        /// Iteration of the raw record.
        iteration: u64,
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
    #[error("decoded state for stream `{stream}` at iteration {iteration} cannot enter its series")]
    StateSeriesInvariant {
        /// Logical stream being reconstructed.
        stream: String,
        /// Iteration of the rejected decoded state.
        iteration: u64,
        /// Original in-memory collection invariant failure.
        #[source]
        source: StateSeriesError,
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

    /// A stream submission did not advance its iteration.
    #[error(
        "record iteration {iteration} for stream `{stream}` does not follow previously accepted iteration {previous}"
    )]
    OutOfOrderIteration {
        /// Logical stream receiving the record.
        stream: String,
        /// Rejected iteration.
        iteration: u64,
        /// Most recently accepted iteration.
        previous: u64,
    },

    // ---------------------------------------------------------------------
    // Queue and writer-worker lifecycle
    // ---------------------------------------------------------------------
    /// The bounded recording queue disconnected before shutdown completed.
    #[error("system-state writer queue disconnected before shutdown completed")]
    WriterQueueDisconnected,

    /// The recording worker terminated with an authoritative persistence failure.
    ///
    /// The shared source lets multiple blocked submitters observe the same
    /// terminal failure without cloning an IO or JSON error.
    #[error("system-state writer terminated: {source}")]
    StateWriterTerminated {
        /// Shared authoritative worker failure.
        #[source]
        source: Arc<PersistenceError>,
    },

    /// Joining a writer thread revealed an unexpected panic.
    #[error("system-state writer worker panicked")]
    StateWriterPanicked,
}
