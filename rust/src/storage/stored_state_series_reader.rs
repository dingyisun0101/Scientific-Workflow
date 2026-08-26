//! All-in-one reconstruction of persisted streams into `StateSeries`.
//!
//! [`StoredStateSeriesReader`] is the complete public read boundary. It owns a recording
//! directory, one validated snapshot of its sole `metadata.json`, and a
//! caller-configured [`JsonPayloadDecoderRegistry`] registry. [`StoredStateSeriesReader::read_stream_as_state_series`] verifies a
//! selected stream's immutable chunks, dispatches each raw payload to the
//! decoder registered for its key, assembles complete `SystemState` values, and
//! returns one fully validated `StateSeries`.
//!
//! # Transactional result
//!
//! A read returns either the complete series or one [`StorageError`]. States
//! accumulated before a later checksum, record, decoder, or series failure are
//! dropped internally and never exposed as a partial success.
//!
//! # Memory behavior
//!
//! Chunk files are processed one buffered line at a time. Record structure is
//! deserialized with borrowed `serde_json::value::RawValue` field slices, so a
//! payload decoder reads directly from the line buffer into its final concrete
//! allocation. The reader does not construct an intermediate JSON value tree
//! for tensor elements and does not retain encoded chunk bytes after a record
//! has been reconstructed.
//!
//! The requested result is intentionally eager: `StateSeries` owns every
//! reconstructed state. A future out-of-core method may be added on this same
//! reader without exposing the private raw-record machinery.

use std::fmt;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde::de::{SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::value::RawValue;
use sha2::{Digest, Sha256};

use crate::state::advanced::{
    StateSchemaAccess, StateSeries, StateTime, SystemState, SystemStateSchema,
};

use super::RecordingTiming;
use super::error::StorageError;
use super::json_payload_decoder::JsonPayloadDecoderRegistry;
use super::jsonl_format::{ChunkMetadata, RecordingMetadata, RecordingStatus, StateStreamMetadata};

/// Name of the only metadata document in one recording directory.
const METADATA_FILE: &str = "metadata.json";

/// Reader that reconstructs complete in-memory series from one completed recording.
///
/// Construction consumes the decoder registry so decoder configuration and
/// metadata remain one coherent read authority. The type is intentionally
/// non-Clone because registered decoders may not have meaningful clone
/// semantics.
pub struct StoredStateSeriesReader {
    root: PathBuf,
    metadata_path: PathBuf,
    metadata: RecordingMetadata,
    timing: RecordingTiming,
    decoders: JsonPayloadDecoderRegistry,
}

impl StoredStateSeriesReader {
    /// Opens one recording directory and validates its authoritative metadata.
    ///
    /// The metadata snapshot must declare successful completion. Reading an
    /// active or failed recording is deliberately rejected because its chunk
    /// inventory is not a complete analysis result.
    ///
    /// Decoder coverage is checked per selected stream by
    /// [`StoredStateSeriesReader::read_stream_as_state_series`], allowing one registry to serve only the streams
    /// an analysis intends to load.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Io`] when `metadata.json` cannot be read,
    /// [`StorageError::Json`] when it is not syntactically valid JSON, semantic
    /// metadata errors from internal `RecordingMetadata::validate`, or
    /// [`StorageError::RecordingNotComplete`] unless status is complete.
    pub fn open_completed_recording(
        root: impl AsRef<Path>,
        decoders: JsonPayloadDecoderRegistry,
    ) -> Result<Self, StorageError> {
        let root = root.as_ref().to_path_buf();
        let metadata_path = root.join(METADATA_FILE);
        let bytes = fs::read(&metadata_path).map_err(|source| StorageError::Io {
            operation: "read metadata",
            path: metadata_path.clone(),
            source,
        })?;
        let metadata: RecordingMetadata =
            serde_json::from_slice(&bytes).map_err(|source| StorageError::Json {
                operation: "parse metadata",
                path: metadata_path.clone(),
                source,
            })?;
        metadata.validate(&metadata_path)?;
        if !matches!(metadata.status, RecordingStatus::Complete) {
            return Err(StorageError::RecordingNotComplete {
                path: metadata_path,
            });
        }
        let timing = RecordingTiming::from_stored(&metadata.timing, &metadata_path)?;
        Ok(Self {
            root,
            metadata_path,
            metadata,
            timing,
            decoders,
        })
    }

    /// Returns the recording directory exactly as supplied at construction.
    pub fn recording_directory(&self) -> &Path {
        &self.root
    }

    /// Iterates declared stream names in deterministic metadata order.
    pub fn stream_names(&self) -> impl ExactSizeIterator<Item = &str> {
        self.metadata
            .streams
            .iter()
            .map(|stream| stream.name.as_str())
    }

    /// Returns the validated storage format version.
    pub fn format_version(&self) -> u32 {
        self.metadata.version
    }

    /// Borrows immutable metadata supplied when recording began.
    pub fn user_metadata(&self) -> &serde_json::Map<String, serde_json::Value> {
        &self.metadata.user_metadata
    }

    /// Borrows values committed only at successful completion.
    pub fn terminal_metadata(&self) -> &serde_json::Map<String, serde_json::Value> {
        &self.metadata.terminal_metadata
    }

    /// Borrows automatic operational timing for the completed recording.
    pub fn recording_timing(&self) -> &RecordingTiming {
        &self.timing
    }

    /// Returns the metadata-declared record count for one completed stream.
    pub fn stream_record_count(&self, stream: &str) -> Result<u64, StorageError> {
        let declaration =
            self.metadata
                .stream(stream)
                .ok_or_else(|| StorageError::UnknownStateStream {
                    stream: stream.to_owned(),
                })?;
        declaration
            .chunks
            .iter()
            .try_fold(0_u64, |total, chunk| total.checked_add(chunk.records))
            .ok_or_else(|| StorageError::ByteCountOverflow {
                stream: declaration.name.clone(),
            })
    }

    /// Returns the exact metadata-declared encoded bytes for one stream.
    pub fn stream_encoded_bytes(&self, stream: &str) -> Result<u64, StorageError> {
        let declaration =
            self.metadata
                .stream(stream)
                .ok_or_else(|| StorageError::UnknownStateStream {
                    stream: stream.to_owned(),
                })?;
        declaration
            .chunks
            .iter()
            .try_fold(0_u64, |total, chunk| total.checked_add(chunk.bytes))
            .ok_or_else(|| StorageError::ByteCountOverflow {
                stream: declaration.name.clone(),
            })
    }

    /// Reconstructs one named logical stream as a complete `StateSeries`.
    ///
    /// The method validates decoder coverage before opening chunk files. It
    /// then checks every chunk's filesystem length, processes its complete
    /// JSONL records in order, verifies descriptor record/index facts and
    /// SHA-256, and appends only fully decoded states to the private series.
    ///
    /// # Errors
    ///
    /// Returns precise stream-selection, decoder, filesystem, integrity,
    /// record, payload-conversion, or series-invariant errors. No partially
    /// reconstructed series is returned on failure.
    pub fn read_stream_as_state_series(&self, stream: &str) -> Result<StateSeries, StorageError> {
        let declaration =
            self.metadata
                .stream(stream)
                .ok_or_else(|| StorageError::UnknownStateStream {
                    stream: stream.to_owned(),
                })?;
        self.decoders
            .require(declaration.fields.iter().map(|field| field.name.as_str()))?;

        let spec = stream_spec(&self.metadata_path, declaration)?;
        let total_records = self.stream_record_count(stream)?;
        let capacity =
            usize::try_from(total_records).map_err(|_| StorageError::ByteCountOverflow {
                stream: declaration.name.clone(),
            })?;
        let mut series = StateSeries::with_capacity(spec, capacity);
        let mut previous_iteration = None;

        for chunk in &declaration.chunks {
            self.read_chunk(declaration, chunk, &mut previous_iteration, &mut series)?;
        }
        Ok(series)
    }

    /// Reconstructs every declared stream in metadata order.
    ///
    /// Distinct stream schemas and sampling intervals remain distinct series. If any
    /// stream fails, already reconstructed series are dropped and no partial
    /// vector is returned.
    pub fn read_all_streams_as_state_series(
        &self,
    ) -> Result<Vec<(String, StateSeries)>, StorageError> {
        self.metadata
            .streams
            .iter()
            .map(|stream| {
                self.read_stream_as_state_series(&stream.name)
                    .map(|series| (stream.name.clone(), series))
            })
            .collect()
    }

    /// Reconstructs only the latest state in one completed stream.
    ///
    /// Earlier chunks are not opened. The newest chunk's length and checksum
    /// are verified, then its final newline-terminated record is decoded into
    /// the stream's partial schema. This is suitable for final-value analysis;
    /// use checkpoint continuation when the state must cover a complete model
    /// schema and remain appendable.
    pub fn read_latest_state_from_stream(&self, stream: &str) -> Result<SystemState, StorageError> {
        let declaration =
            self.metadata
                .stream(stream)
                .ok_or_else(|| StorageError::UnknownStateStream {
                    stream: stream.to_owned(),
                })?;
        self.decoders
            .require(declaration.fields.iter().map(|field| field.name.as_str()))?;
        let chunk = declaration
            .chunks
            .last()
            .ok_or_else(|| StorageError::NoRecordedState {
                stream: declaration.name.clone(),
            })?;
        let path = self.root.join(&declaration.directory).join(&chunk.file);
        let bytes = read_verified_chunk(&self.metadata_path, &path, chunk)?;
        let record = final_jsonl_record(&path, &bytes)?;
        let spec = stream_spec(&self.metadata_path, declaration)?;
        let state =
            decode_state_record_with_decoders(record, &path, declaration, &spec, &self.decoders)?;
        if state.time().iteration() != chunk.last_iteration {
            return Err(invalid_record(
                &path,
                chunk.records,
                format!(
                    "latest record iteration {} differs from chunk descriptor {}",
                    state.time().iteration(),
                    chunk.last_iteration
                ),
            ));
        }
        Ok(state)
    }

    /// Verifies and reconstructs one immutable committed chunk.
    fn read_chunk(
        &self,
        stream: &StateStreamMetadata,
        chunk: &ChunkMetadata,
        previous_iteration: &mut Option<u64>,
        series: &mut StateSeries,
    ) -> Result<(), StorageError> {
        let path = self.root.join(&stream.directory).join(&chunk.file);
        verify_file_size(&path, chunk.bytes)?;
        let file = File::open(&path).map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                StorageError::MissingChunk { path: path.clone() }
            } else {
                StorageError::Io {
                    operation: "open chunk",
                    path: path.clone(),
                    source,
                }
            }
        })?;
        let mut input = BufReader::new(file);
        let mut line = Vec::new();
        let mut line_number = 0_u64;
        let mut records = 0_u64;
        let mut first_iteration = None;
        let mut last_iteration = None;
        let mut hasher = Sha256::new();

        loop {
            line.clear();
            let bytes_read =
                input
                    .read_until(b'\n', &mut line)
                    .map_err(|source| StorageError::Io {
                        operation: "read chunk",
                        path: path.clone(),
                        source,
                    })?;
            if bytes_read == 0 {
                break;
            }
            line_number =
                line_number
                    .checked_add(1)
                    .ok_or_else(|| StorageError::ByteCountOverflow {
                        stream: stream.name.clone(),
                    })?;
            hasher.update(&line);
            if line.last() != Some(&b'\n') {
                return Err(invalid_record(
                    &path,
                    line_number,
                    "record is not terminated by a newline",
                ));
            }
            line.pop();
            if line.is_empty() {
                return Err(invalid_record(
                    &path,
                    line_number,
                    "record line must not be empty",
                ));
            }

            let record: BorrowedRecord<'_> = serde_json::from_slice(&line).map_err(|source| {
                invalid_record(&path, line_number, format!("invalid JSON record: {source}"))
            })?;
            validate_iteration(&path, line_number, record.iteration, *previous_iteration)?;
            let iteration = record.iteration;
            first_iteration.get_or_insert(iteration);
            last_iteration = Some(iteration);
            *previous_iteration = Some(iteration);

            let time = match record.physical_time {
                Some(physical_time) => {
                    StateTime::from_iteration_and_physical_time(iteration, physical_time)
                        .ok_or_else(|| {
                            invalid_record(&path, line_number, "physical time must be finite")
                        })?
                }
                None => StateTime::from_iteration(iteration),
            };
            let mut state = series.schema().create_empty_state(time);
            decode_values(
                &self.decoders,
                stream,
                &path,
                line_number,
                record.values,
                &mut state,
            )?;
            series.push_state(state).map_err(|rejection| {
                let (source, state) = rejection.into_parts();
                let index = state.time().iteration();
                drop(state);
                StorageError::StateSeriesInvariant {
                    stream: stream.name.clone(),
                    iteration: index,
                    source,
                }
            })?;
            records = records
                .checked_add(1)
                .ok_or_else(|| StorageError::ByteCountOverflow {
                    stream: stream.name.clone(),
                })?;
        }

        validate_chunk_facts(
            &self.metadata_path,
            stream,
            chunk,
            records,
            first_iteration,
            last_iteration,
        )?;
        verify_checksum(
            &self.metadata_path,
            &path,
            &chunk.checksum,
            hasher.finalize(),
        )
    }
}

/// Borrows the final nonempty newline-terminated record from one chunk image.
fn final_jsonl_record<'a>(path: &Path, bytes: &'a [u8]) -> Result<&'a [u8], StorageError> {
    if !bytes.ends_with(b"\n") {
        return Err(invalid_record(
            path,
            1,
            "latest chunk is not terminated by a newline",
        ));
    }
    let without_final_newline = &bytes[..bytes.len() - 1];
    let start = without_final_newline
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |position| position + 1);
    let record = &without_final_newline[start..];
    if record.is_empty() {
        return Err(invalid_record(path, 1, "latest record must not be empty"));
    }
    Ok(record)
}

impl fmt::Debug for StoredStateSeriesReader {
    /// Formats bounded configuration without decoder internals or payloads.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoredStateSeriesReader")
            .field("root", &self.root)
            .field("streams", &self.metadata.streams.len())
            .field("decoders", &self.decoders)
            .finish_non_exhaustive()
    }
}

/// Reconstructs the newest complete state used by coordinated run resume.
///
/// The newest sealed chunk's declared byte count and SHA-256 checksum are
/// verified before its final record is decoded. Earlier sealed chunks are not
/// opened, and unpublished buffered chunks are never checkpoint state.
pub(crate) fn decode_resume_state(
    root: &Path,
    metadata_path: &Path,
    stream: &StateStreamMetadata,
    full_spec: &SystemStateSchema,
    decoders: &JsonPayloadDecoderRegistry,
) -> Result<SystemState, StorageError> {
    validate_complete_resume_schema(stream, full_spec)?;
    decoders.require(full_spec.field_schemas().iter().map(|field| field.name()))?;

    let chunk = stream
        .chunks
        .last()
        .ok_or_else(|| StorageError::NoCheckpointState {
            stream: stream.name.clone(),
        })?;
    let path = root.join(&stream.directory).join(&chunk.file);
    let bytes = read_verified_chunk(metadata_path, &path, chunk)?;
    let record = final_jsonl_record(&path, &bytes)?;
    let state = decode_state_record_with_decoders(record, &path, stream, full_spec, decoders)?;
    if state.time().iteration() != chunk.last_iteration {
        return Err(invalid_record(
            &path,
            chunk.records,
            format!(
                "latest record iteration {} differs from chunk descriptor {}",
                state.time().iteration(),
                chunk.last_iteration
            ),
        ));
    }
    Ok(state)
}

/// Requires exact key order and descriptions for a full-state checkpoint.
fn validate_complete_resume_schema(
    stream: &StateStreamMetadata,
    full_spec: &SystemStateSchema,
) -> Result<(), StorageError> {
    let Some(mismatch) = checkpoint_schema_mismatch(stream, full_spec) else {
        return Ok(());
    };
    Err(match mismatch {
        CheckpointSchemaMismatch::Count {
            stream_fields,
            full_spec_fields,
        } => StorageError::IncompleteCheckpointStream {
            stream: stream.name.clone(),
            reason: format!(
                "stream declares {stream_fields} fields but the full state declares {full_spec_fields}",
            ),
        },
        CheckpointSchemaMismatch::Field {
            position,
            stored,
            expected,
        } => StorageError::IncompleteCheckpointStream {
            stream: stream.name.clone(),
            reason: format!("field {position} is `{stored}` but full state requires `{expected}`"),
        },
    })
}

pub(crate) fn is_complete_checkpoint_stream(
    stream: &StateStreamMetadata,
    full_spec: &SystemStateSchema,
) -> bool {
    checkpoint_schema_mismatch(stream, full_spec).is_none()
}

enum CheckpointSchemaMismatch {
    Count {
        stream_fields: usize,
        full_spec_fields: usize,
    },
    Field {
        position: usize,
        stored: String,
        expected: String,
    },
}

fn checkpoint_schema_mismatch(
    stream: &StateStreamMetadata,
    full_spec: &SystemStateSchema,
) -> Option<CheckpointSchemaMismatch> {
    if stream.fields.len() != full_spec.len() {
        return Some(CheckpointSchemaMismatch::Count {
            stream_fields: stream.fields.len(),
            full_spec_fields: full_spec.len(),
        });
    }
    for (position, (stored, expected)) in stream
        .fields
        .iter()
        .zip(full_spec.field_schemas())
        .enumerate()
    {
        if stored.name != expected.name() || stored.description.as_deref() != expected.description()
        {
            return Some(CheckpointSchemaMismatch::Field {
                position,
                stored: stored.name.clone(),
                expected: expected.name().to_string(),
            });
        }
    }
    None
}

/// Parses one record and dispatches each raw field into its owned final type.
fn decode_state_record_with_decoders(
    record: &[u8],
    path: &Path,
    stream: &StateStreamMetadata,
    full_spec: &SystemStateSchema,
    decoders: &JsonPayloadDecoderRegistry,
) -> Result<SystemState, StorageError> {
    let record: BorrowedRecord<'_> = serde_json::from_slice(record)
        .map_err(|source| invalid_record(path, 1, format!("invalid JSON record: {source}")))?;
    let time = match record.physical_time {
        Some(physical_time) => {
            StateTime::from_iteration_and_physical_time(record.iteration, physical_time)
                .ok_or_else(|| invalid_record(path, 1, "physical time must be finite"))?
        }
        None => StateTime::from_iteration(record.iteration),
    };
    let mut state = full_spec.create_empty_state(time);
    decode_values(decoders, stream, path, 1, record.values, &mut state)?;
    Ok(state)
}

/// Borrowed JSONL record whose payload values point into one line buffer.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BorrowedRecord<'a> {
    iteration: u64,
    #[serde(default)]
    physical_time: Option<f64>,
    #[serde(borrow)]
    values: BorrowedValues<'a>,
}

/// Positional field collection backed by borrowed raw JSON slices.
struct BorrowedValues<'a> {
    entries: Vec<&'a RawValue>,
}

impl<'de: 'a, 'a> Deserialize<'de> for BorrowedValues<'a> {
    /// Retains each positional raw value boundary without decoding its payload.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_seq(BorrowedValuesVisitor {
            output: std::marker::PhantomData,
        })
    }
}

/// Serde visitor for one record's positional `values` array.
struct BorrowedValuesVisitor<'a> {
    /// Selects the shorter lifetime exposed by the containing record.
    output: std::marker::PhantomData<&'a RawValue>,
}

impl<'de: 'a, 'a> Visitor<'de> for BorrowedValuesVisitor<'a> {
    type Value = BorrowedValues<'a>;

    /// Describes the required JSON representation.
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an array of raw JSON payload values")
    }

    /// Collects each potentially large value as a borrow into the input line.
    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut entries = Vec::with_capacity(sequence.size_hint().unwrap_or(0));
        while let Some(value) = sequence.next_element::<&'de RawValue>()? {
            let value: &'a RawValue = value;
            entries.push(value);
        }
        Ok(BorrowedValues { entries })
    }
}

/// Serde representation accepted by the crate-private SystemStateSchema parser.
#[derive(Serialize)]
struct StreamTemplateRef<'a> {
    fields: &'a [super::jsonl_format::StateFieldMetadata],
}

/// Reconstructs one stream's immutable key/description specification.
fn stream_spec(
    metadata_path: &Path,
    stream: &StateStreamMetadata,
) -> Result<SystemStateSchema, StorageError> {
    let bytes = serde_json::to_vec(&StreamTemplateRef {
        fields: &stream.fields,
    })
    .map_err(|source| StorageError::Json {
        operation: "serialize stream schema",
        path: metadata_path.to_path_buf(),
        source,
    })?;
    SystemStateSchema::parse(metadata_path.to_path_buf(), &bytes).map_err(|source| {
        StorageError::InvalidMetadata {
            path: metadata_path.to_path_buf(),
            reason: format!(
                "stream `{}` has an invalid state schema: {source}",
                stream.name
            ),
        }
    })
}

/// Verifies filesystem length before allocating decoded payloads.
fn verify_file_size(path: &Path, expected: u64) -> Result<(), StorageError> {
    let actual = match fs::metadata(path) {
        Ok(metadata) => metadata.len(),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Err(StorageError::MissingChunk {
                path: path.to_path_buf(),
            });
        }
        Err(source) => {
            return Err(StorageError::Io {
                operation: "inspect chunk",
                path: path.to_path_buf(),
                source,
            });
        }
    };
    if actual != expected {
        return Err(StorageError::ChunkSizeMismatch {
            path: path.to_path_buf(),
            expected,
            actual,
        });
    }
    Ok(())
}

/// Reads one immutable chunk only after enforcing its authoritative descriptor.
pub(crate) fn read_verified_chunk(
    metadata_path: &Path,
    path: &Path,
    chunk: &ChunkMetadata,
) -> Result<Vec<u8>, StorageError> {
    verify_file_size(path, chunk.bytes)?;
    let bytes = fs::read(path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            StorageError::MissingChunk {
                path: path.to_path_buf(),
            }
        } else {
            StorageError::Io {
                operation: "read verified chunk",
                path: path.to_path_buf(),
                source,
            }
        }
    })?;
    verify_checksum(metadata_path, path, &chunk.checksum, Sha256::digest(&bytes))?;
    Ok(bytes)
}

/// Enforces strict iteration order across chunk boundaries.
fn validate_iteration(
    path: &Path,
    line: u64,
    iteration: u64,
    previous: Option<u64>,
) -> Result<(), StorageError> {
    if let Some(previous) = previous
        && iteration <= previous
    {
        return Err(invalid_record(
            path,
            line,
            format!("iteration {iteration} is not greater than previous iteration {previous}"),
        ));
    }
    Ok(())
}

/// Validates positional width, dispatches canonical decoders, and fills one state.
fn decode_values(
    decoders: &JsonPayloadDecoderRegistry,
    stream: &StateStreamMetadata,
    path: &Path,
    line: u64,
    values: BorrowedValues<'_>,
    state: &mut crate::state::advanced::SystemState,
) -> Result<(), StorageError> {
    if values.entries.len() != stream.fields.len() {
        return Err(invalid_record(
            path,
            line,
            format!(
                "record contains {} payload values but stream `{}` declares {} fields",
                values.entries.len(),
                stream.name,
                stream.fields.len()
            ),
        ));
    }
    for (field, raw) in stream.fields.iter().zip(values.entries) {
        decoders.decode_into(
            &stream.name,
            state.time().iteration(),
            &field.name,
            raw.get(),
            state,
        )?;
    }
    Ok(())
}

/// Compares parsed chunk facts with the authoritative metadata descriptor.
fn validate_chunk_facts(
    metadata_path: &Path,
    stream: &StateStreamMetadata,
    chunk: &ChunkMetadata,
    records: u64,
    first_iteration: Option<u64>,
    last_iteration: Option<u64>,
) -> Result<(), StorageError> {
    if records != chunk.records
        || first_iteration != Some(chunk.first_iteration)
        || last_iteration != Some(chunk.last_iteration)
    {
        return Err(StorageError::InvalidMetadata {
            path: metadata_path.to_path_buf(),
            reason: format!(
                "stream `{}` chunk {} declares {} records at {}..={}, but contains {} records at {:?}..={:?}",
                stream.name,
                chunk.ordinal,
                chunk.records,
                chunk.first_iteration,
                chunk.last_iteration,
                records,
                first_iteration,
                last_iteration
            ),
        });
    }
    Ok(())
}

/// Compares the streamed SHA-256 digest with the descriptor checksum.
fn verify_checksum(
    metadata_path: &Path,
    path: &Path,
    expected: &str,
    digest: impl AsRef<[u8]>,
) -> Result<(), StorageError> {
    let Some(expected_digest) = expected.strip_prefix("sha256:") else {
        return Err(StorageError::InvalidMetadata {
            path: metadata_path.to_path_buf(),
            reason: format!("unsupported chunk checksum algorithm in `{expected}`"),
        });
    };
    let actual_digest = lowercase_hex(digest.as_ref());
    if actual_digest != expected_digest {
        return Err(StorageError::ChecksumMismatch {
            path: path.to_path_buf(),
            expected: expected.to_owned(),
            actual: format!("sha256:{actual_digest}"),
        });
    }
    Ok(())
}

/// Encodes digest bytes in the persisted lowercase hexadecimal notation.
fn lowercase_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

/// Constructs one line-aware record error without retaining payload bytes.
fn invalid_record(path: &Path, line: u64, reason: impl Into<String>) -> StorageError {
    StorageError::InvalidRecord {
        path: path.to_path_buf(),
        line,
        reason: reason.into(),
    }
}
