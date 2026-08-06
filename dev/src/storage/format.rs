//! Versioned metadata and record representations for JSON storage.
//!
//! This module defines the complete data contract shared by encoders, writers,
//! readers, and decoders. It performs structural validation but never opens a
//! file, starts a thread, accesses a live payload, or chooses a concrete decode
//! type. Filesystem mechanics belong to `reader.rs` and `writer.rs`.
//!
//! # On-disk layout
//!
//! Every run has one `metadata.json`. It contains the format/version marker,
//! record encoding, time-axis description, caller-supplied JSON metadata,
//! logical stream schemas, committed chunk descriptors, and run completion
//! state. Chunk files contain only compact JSON Lines records and never repeat
//! schemas or chunk metadata.
//!
//! # Record shape
//!
//! One logical partial state occupies exactly one line:
//!
//! ```json
//! {"index":12,"physical":0.25,"values":{"population":[1,2,3]}}
//! ```
//!
//! `physical` is omitted when absent. `values` retains field keys for readable
//! raw output and decoder dispatch. [`EncodedRecord`] owns the complete framed
//! line including its trailing newline, so writer byte accounting is exact and
//! no downstream layer can accidentally split a record.
//!
//! # Validation
//!
//! [`RunMetadata::validate`] rejects unknown format versions, unsafe relative
//! paths, duplicate stream or field names, non-deterministic chunk filenames,
//! empty committed chunks, inconsistent chunk ordinals or index ranges,
//! unsupported encoding labels, and malformed lifecycle descriptions. It does
//! not check filesystem existence, actual byte lengths, or checksums; readers
//! perform those external integrity checks after metadata validation.

use std::collections::HashSet;
use std::fmt;
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::system_state::TimePoint;

use super::error::StorageError;

/// Stable name written into every metadata file owned by this format.
pub(crate) const FORMAT_NAME: &str = "scientific-workflow-jsonl";

/// Current metadata and record schema version.
pub(crate) const FORMAT_VERSION: u32 = 1;

/// Payload encoding supported by the current storage stage.
pub(crate) const PAYLOAD_ENCODING: &str = "json";

/// Record framing supported by the current storage stage.
pub(crate) const RECORD_FRAMING: &str = "json_lines";

/// Complete contents of the sole run-level `metadata.json` file.
///
/// This representation is cloneable because writers commit small metadata
/// snapshots atomically. It never contains scientific payload data.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunMetadata {
    /// Stable format identifier validated before version-specific processing.
    pub(crate) format: String,
    /// Version of all structures in this metadata document and its chunks.
    pub(crate) version: u32,
    /// Current run lifecycle state.
    pub(crate) status: RunStatus,
    /// Payload encoding and record framing declaration.
    pub(crate) records: RecordFormat,
    /// Meanings and optional units of temporal coordinates.
    pub(crate) time: TimeAxis,
    /// Arbitrary JSON metadata supplied by the workflow or dispatcher.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub(crate) run: Map<String, Value>,
    /// Logical output streams in deterministic declaration order.
    pub(crate) streams: Vec<StreamMetadata>,
}

impl RunMetadata {
    /// Creates initial metadata for a run that has not yet accepted records.
    ///
    /// Stream order is preserved exactly. Semantic validation is deliberately
    /// separate through [`RunMetadata::validate`] so construction, parsed input,
    /// and pre-commit snapshots share one validation implementation.
    pub(crate) fn running(
        time: TimeAxis,
        run: Map<String, Value>,
        streams: Vec<StreamMetadata>,
    ) -> Self {
        Self {
            format: FORMAT_NAME.to_owned(),
            version: FORMAT_VERSION,
            status: RunStatus::Running,
            records: RecordFormat::json_lines(),
            time,
            run,
            streams,
        }
    }

    /// Validates all format invariants without consulting the filesystem.
    ///
    /// `path` is retained in any error as the provenance of this metadata. It
    /// may identify a parsed file or the destination of a pending atomic
    /// commit.
    pub(crate) fn validate(&self, path: &Path) -> Result<(), StorageError> {
        if self.format != FORMAT_NAME {
            return Err(invalid_metadata(
                path,
                format!("format must be `{FORMAT_NAME}`, got `{}`", self.format),
            ));
        }
        if self.version != FORMAT_VERSION {
            return Err(StorageError::UnsupportedVersion {
                path: path.to_path_buf(),
                found: self.version,
                supported: FORMAT_VERSION,
            });
        }
        self.records.validate(path)?;
        self.time.validate(path)?;
        self.status.validate(path)?;
        if self.streams.is_empty() {
            return Err(invalid_metadata(
                path,
                "at least one output stream must be declared",
            ));
        }

        let mut names = HashSet::with_capacity(self.streams.len());
        let mut directories = HashSet::with_capacity(self.streams.len());
        for stream in &self.streams {
            if !names.insert(stream.name.as_str()) {
                return Err(StorageError::DuplicateStream {
                    stream: stream.name.clone(),
                });
            }
            if !directories.insert(stream.directory.as_str()) {
                return Err(invalid_metadata(
                    path,
                    format!(
                        "streams use the same output directory `{}`",
                        stream.directory
                    ),
                ));
            }
            stream.validate(path)?;
        }
        Ok(())
    }

    /// Returns one stream declaration by exact configured name.
    pub(crate) fn stream(&self, name: &str) -> Option<&StreamMetadata> {
        self.streams.iter().find(|stream| stream.name == name)
    }

    /// Returns one mutable stream declaration by exact configured name.
    ///
    /// This crate-private boundary lets the writer append committed chunk
    /// descriptors. Callers must re-run [`RunMetadata::validate`] before an
    /// atomic metadata commit.
    pub(crate) fn stream_mut(&mut self, name: &str) -> Option<&mut StreamMetadata> {
        self.streams.iter_mut().find(|stream| stream.name == name)
    }
}

/// Run lifecycle persisted atomically in `metadata.json`.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum RunStatus {
    /// Writers may still accept or commit records.
    Running,
    /// Every writer drained and committed its final non-empty chunk.
    Complete,
    /// The run terminated without a successful completion transition.
    Failed {
        /// Stable human-readable terminal explanation.
        message: String,
    },
}

impl RunStatus {
    /// Validates lifecycle-specific metadata fields.
    fn validate(&self, path: &Path) -> Result<(), StorageError> {
        if let Self::Failed { message } = self {
            if message.trim().is_empty() {
                return Err(invalid_metadata(
                    path,
                    "failed run status requires a non-empty message",
                ));
            }
        }
        Ok(())
    }
}

/// Encoding declaration shared by every stream in one run.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordFormat {
    /// Payload representation; currently always `json`.
    pub(crate) encoding: String,
    /// Record boundary convention; currently always `json_lines`.
    pub(crate) framing: String,
}

impl RecordFormat {
    /// Returns the only encoding/framing pair supported by this version.
    fn json_lines() -> Self {
        Self {
            encoding: PAYLOAD_ENCODING.to_owned(),
            framing: RECORD_FRAMING.to_owned(),
        }
    }

    /// Rejects unsupported encoding labels before records are inspected.
    fn validate(&self, path: &Path) -> Result<(), StorageError> {
        if self.encoding != PAYLOAD_ENCODING || self.framing != RECORD_FRAMING {
            return Err(invalid_metadata(
                path,
                format!(
                    "record format must be `{PAYLOAD_ENCODING}` with `{RECORD_FRAMING}` framing"
                ),
            ));
        }
        Ok(())
    }
}

/// Names and optional units for the two supported temporal coordinates.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TimeAxis {
    /// Human-facing name for the mandatory integer simulation index.
    pub(crate) index_name: String,
    /// Optional unit for the integer index, such as `step` or `sweep`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) index_unit: Option<String>,
    /// Optional name for the floating physical coordinate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) physical_name: Option<String>,
    /// Optional physical-coordinate unit. A unit requires a physical name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) physical_unit: Option<String>,
}

impl TimeAxis {
    /// Validates non-empty labels and physical-name/unit consistency.
    fn validate(&self, path: &Path) -> Result<(), StorageError> {
        if self.index_name.trim().is_empty() {
            return Err(invalid_metadata(path, "time.index_name must not be empty"));
        }
        if self
            .index_unit
            .as_deref()
            .is_some_and(|unit| unit.trim().is_empty())
        {
            return Err(invalid_metadata(
                path,
                "time.index_unit must not be empty when present",
            ));
        }
        if self
            .physical_name
            .as_deref()
            .is_some_and(|name| name.trim().is_empty())
        {
            return Err(invalid_metadata(
                path,
                "time.physical_name must not be empty when present",
            ));
        }
        if self
            .physical_unit
            .as_deref()
            .is_some_and(|unit| unit.trim().is_empty())
        {
            return Err(invalid_metadata(
                path,
                "time.physical_unit must not be empty when present",
            ));
        }
        if self.physical_unit.is_some() && self.physical_name.is_none() {
            return Err(invalid_metadata(
                path,
                "time.physical_unit requires time.physical_name",
            ));
        }
        Ok(())
    }
}

/// Metadata and committed chunk inventory for one logical output stream.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StreamMetadata {
    /// Unique normalized stream name used by the sampling API.
    pub(crate) name: String,
    /// Safe relative directory beneath the run output root.
    pub(crate) directory: String,
    /// Optional human-readable sampling cadence description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) cadence: Option<String>,
    /// Ordered partial-state schema persisted once for this stream.
    pub(crate) fields: Vec<FieldMetadata>,
    /// Soft maximum chunk size; complete oversized records remain indivisible.
    pub(crate) max_chunk_bytes: u64,
    /// Strict maximum number of accepted but uncommitted encoded bytes.
    pub(crate) queue_bytes: u64,
    /// Committed chunks in monotonically increasing ordinal order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) chunks: Vec<ChunkMetadata>,
}

impl StreamMetadata {
    /// Validates stream names, paths, limits, fields, and chunk continuity.
    fn validate(&self, path: &Path) -> Result<(), StorageError> {
        if self.name.trim().is_empty() {
            return Err(invalid_metadata(path, "stream name must not be empty"));
        }
        validate_relative_path(path, "stream directory", &self.directory)?;
        if self
            .cadence
            .as_deref()
            .is_some_and(|cadence| cadence.trim().is_empty())
        {
            return Err(invalid_metadata(
                path,
                format!("stream `{}` has an empty cadence", self.name),
            ));
        }
        if self.max_chunk_bytes == 0 || self.queue_bytes == 0 {
            return Err(invalid_metadata(
                path,
                format!("stream `{}` has a zero storage limit", self.name),
            ));
        }

        let mut fields = HashSet::with_capacity(self.fields.len());
        for field in &self.fields {
            field.validate(path, &self.name)?;
            if !fields.insert(field.name.as_str()) {
                return Err(invalid_metadata(
                    path,
                    format!(
                        "stream `{}` declares duplicate field `{}`",
                        self.name, field.name
                    ),
                ));
            }
        }

        let mut previous_last = None;
        for (expected_ordinal, chunk) in self.chunks.iter().enumerate() {
            chunk.validate(path, &self.name, expected_ordinal as u64)?;
            if let Some(previous) = previous_last {
                if chunk.first_index <= previous {
                    return Err(invalid_metadata(
                        path,
                        format!(
                            "stream `{}` chunk {} begins at index {}, not after {}",
                            self.name, chunk.ordinal, chunk.first_index, previous
                        ),
                    ));
                }
            }
            previous_last = Some(chunk.last_index);
        }
        Ok(())
    }
}

/// One key and optional description in a persisted partial-state schema.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FieldMetadata {
    /// Exact SystemState key serialized into each record's `values` object.
    pub(crate) name: String,
    /// Optional natural-language payload description; never a Rust type tag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) description: Option<String>,
}

impl FieldMetadata {
    /// Validates normalized field documentation.
    fn validate(&self, path: &Path, stream: &str) -> Result<(), StorageError> {
        if self.name.trim().is_empty() {
            return Err(invalid_metadata(
                path,
                format!("stream `{stream}` contains an empty field name"),
            ));
        }
        if self
            .description
            .as_deref()
            .is_some_and(|description| description.trim().is_empty())
        {
            return Err(invalid_metadata(
                path,
                format!(
                    "stream `{stream}` field `{}` has an empty description",
                    self.name
                ),
            ));
        }
        Ok(())
    }
}

/// Authoritative descriptor for one immutable committed JSONL chunk.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChunkMetadata {
    /// Zero-based ordinal within its logical stream.
    pub(crate) ordinal: u64,
    /// Deterministic filename relative to the stream directory.
    pub(crate) file: String,
    /// Number of complete JSONL records in the chunk.
    pub(crate) records: u64,
    /// Exact file length including every record newline.
    pub(crate) bytes: u64,
    /// Checksum string including its algorithm prefix.
    pub(crate) checksum: String,
    /// Simulation index of the first record.
    pub(crate) first_index: u64,
    /// Simulation index of the final record.
    pub(crate) last_index: u64,
}

impl ChunkMetadata {
    /// Validates deterministic naming and non-empty ordered contents.
    fn validate(
        &self,
        path: &Path,
        stream: &str,
        expected_ordinal: u64,
    ) -> Result<(), StorageError> {
        if self.ordinal != expected_ordinal {
            return Err(invalid_metadata(
                path,
                format!(
                    "stream `{stream}` expected chunk ordinal {expected_ordinal}, got {}",
                    self.ordinal
                ),
            ));
        }
        let expected_file = chunk_filename(self.ordinal);
        if self.file != expected_file {
            return Err(invalid_metadata(
                path,
                format!(
                    "stream `{stream}` chunk {} filename must be `{expected_file}`",
                    self.ordinal
                ),
            ));
        }
        validate_relative_path(path, "chunk file", &self.file)?;
        if self.records == 0 || self.bytes == 0 {
            return Err(invalid_metadata(
                path,
                format!("stream `{stream}` chunk {} is empty", self.ordinal),
            ));
        }
        if self.first_index > self.last_index {
            return Err(invalid_metadata(
                path,
                format!(
                    "stream `{stream}` chunk {} index range {}..={} is reversed",
                    self.ordinal, self.first_index, self.last_index
                ),
            ));
        }
        if !valid_checksum(&self.checksum) {
            return Err(invalid_metadata(
                path,
                format!(
                    "stream `{stream}` chunk {} has an invalid checksum",
                    self.ordinal
                ),
            ));
        }
        Ok(())
    }
}

/// One complete owned JSONL record moved through a writer queue.
///
/// The type is intentionally non-Clone. Its buffer is created once by the
/// encoder, moved through bounded queue ownership, and appended as one
/// indivisible unit by the writer.
pub(crate) struct EncodedRecord {
    time: TimePoint,
    bytes: Vec<u8>,
}

impl EncodedRecord {
    /// Frames compact JSON bytes as one complete newline-terminated record.
    ///
    /// `json` must contain one complete compact object produced by the encoder.
    /// The framing newline is appended here so [`EncodedRecord::len`] exactly
    /// matches the bytes presented to chunk rollover and file writing.
    pub(crate) fn new(time: TimePoint, mut json: Vec<u8>) -> Self {
        json.push(b'\n');
        Self { time, bytes: json }
    }

    /// Returns the record's complete temporal coordinate.
    pub(crate) fn time(&self) -> TimePoint {
        self.time
    }

    /// Returns the exact framed byte count, including the newline.
    pub(crate) fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Borrows the complete framed bytes for writing or checksum updates.
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl fmt::Debug for EncodedRecord {
    /// Formats time and byte length without formatting encoded payload bytes.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EncodedRecord")
            .field("time", &self.time)
            .field("bytes", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

/// Returns the only valid committed filename for `ordinal`.
pub(crate) fn chunk_filename(ordinal: u64) -> String {
    format!("chunk-{ordinal:06}.jsonl")
}

/// Constructs a semantic metadata error with owned provenance.
fn invalid_metadata(path: &Path, reason: impl Into<String>) -> StorageError {
    StorageError::InvalidMetadata {
        path: path.to_path_buf(),
        reason: reason.into(),
    }
}

/// Rejects absolute, parent, root, prefix, empty, and current-directory paths.
fn validate_relative_path(
    metadata_path: &Path,
    label: &str,
    value: &str,
) -> Result<(), StorageError> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || !path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(invalid_metadata(
            metadata_path,
            format!("{label} `{value}` must be a safe relative path"),
        ));
    }
    Ok(())
}

/// Validates the `algorithm:lowercase-hex` checksum representation.
fn valid_checksum(checksum: &str) -> bool {
    let Some((algorithm, digest)) = checksum.split_once(':') else {
        return false;
    };
    !algorithm.is_empty()
        && algorithm
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && !digest.is_empty()
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
