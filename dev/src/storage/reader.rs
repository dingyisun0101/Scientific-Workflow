//! All-in-one reconstruction of persisted streams into `StateSeries`.
//!
//! [`SeriesReader`] is the complete public read boundary. It owns an output
//! directory, one validated snapshot of its sole `metadata.json`, and a
//! caller-configured [`Decoders`] registry. [`SeriesReader::read`] verifies a
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

use std::borrow::Cow;
use std::fmt;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::value::RawValue;
use sha2::{Digest, Sha256};

use crate::system_state::{StateSpec, TimePoint};
use crate::time_series::StateSeries;

use super::decoder::Decoders;
use super::error::StorageError;
use super::format::{ChunkMetadata, RunMetadata, RunStatus, StreamMetadata};

/// Name of the only metadata document in one run output directory.
const METADATA_FILE: &str = "metadata.json";

/// Reader that reconstructs complete in-memory series from one finished run.
///
/// Construction consumes the decoder registry so decoder configuration and
/// metadata remain one coherent read authority. The type is intentionally
/// non-Clone because registered decoders may not have meaningful clone
/// semantics.
pub struct SeriesReader {
    root: PathBuf,
    metadata_path: PathBuf,
    metadata: RunMetadata,
    decoders: Decoders,
}

impl SeriesReader {
    /// Opens one output directory and validates its authoritative metadata.
    ///
    /// The metadata snapshot must declare successful completion. Reading an
    /// active or failed run is deliberately rejected because its chunk
    /// inventory is not a complete analysis result.
    ///
    /// Decoder coverage is checked per selected stream by
    /// [`SeriesReader::read`], allowing one registry to serve only the streams
    /// an analysis intends to load.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Io`] when `metadata.json` cannot be read,
    /// [`StorageError::Json`] when it is not syntactically valid JSON, semantic
    /// metadata errors from [`RunMetadata::validate`], or
    /// [`StorageError::RunIncomplete`] unless status is complete.
    pub fn open(root: impl AsRef<Path>, decoders: Decoders) -> Result<Self, StorageError> {
        let root = root.as_ref().to_path_buf();
        let metadata_path = root.join(METADATA_FILE);
        let bytes = fs::read(&metadata_path).map_err(|source| StorageError::Io {
            operation: "read metadata",
            path: metadata_path.clone(),
            source,
        })?;
        let metadata: RunMetadata =
            serde_json::from_slice(&bytes).map_err(|source| StorageError::Json {
                operation: "parse metadata",
                path: metadata_path.clone(),
                source,
            })?;
        metadata.validate(&metadata_path)?;
        if !matches!(metadata.status, RunStatus::Complete) {
            return Err(StorageError::RunIncomplete {
                path: metadata_path,
            });
        }
        Ok(Self {
            root,
            metadata_path,
            metadata,
            decoders,
        })
    }

    /// Returns the run output directory exactly as supplied at construction.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Iterates declared stream names in deterministic metadata order.
    pub fn streams(&self) -> impl ExactSizeIterator<Item = &str> {
        self.metadata
            .streams
            .iter()
            .map(|stream| stream.name.as_str())
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
    pub fn read(&self, stream: &str) -> Result<StateSeries, StorageError> {
        let declaration =
            self.metadata
                .stream(stream)
                .ok_or_else(|| StorageError::UnknownStream {
                    stream: stream.to_owned(),
                })?;
        self.decoders
            .require(declaration.fields.iter().map(|field| field.name.as_str()))?;

        let spec = stream_spec(&self.metadata_path, declaration)?;
        let capacity = declaration
            .chunks
            .iter()
            .try_fold(0_u64, |total, chunk| total.checked_add(chunk.records))
            .and_then(|total| usize::try_from(total).ok())
            .ok_or_else(|| StorageError::ByteCountOverflow {
                stream: declaration.name.clone(),
            })?;
        let mut series = StateSeries::with_capacity(spec, capacity);
        let mut previous_index = None;

        for chunk in &declaration.chunks {
            self.read_chunk(declaration, chunk, &mut previous_index, &mut series)?;
        }
        Ok(series)
    }

    /// Reconstructs every declared stream in metadata order.
    ///
    /// Distinct stream schemas and cadences remain distinct series. If any
    /// stream fails, already reconstructed series are dropped and no partial
    /// vector is returned.
    pub fn read_all(&self) -> Result<Vec<(String, StateSeries)>, StorageError> {
        self.metadata
            .streams
            .iter()
            .map(|stream| {
                self.read(&stream.name)
                    .map(|series| (stream.name.clone(), series))
            })
            .collect()
    }

    /// Verifies and reconstructs one immutable committed chunk.
    fn read_chunk(
        &self,
        stream: &StreamMetadata,
        chunk: &ChunkMetadata,
        previous_index: &mut Option<u64>,
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
        let mut first_index = None;
        let mut last_index = None;
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
            validate_index(&path, line_number, record.index, *previous_index)?;
            let index = record.index;
            first_index.get_or_insert(index);
            last_index = Some(index);
            *previous_index = Some(index);

            let time = match record.physical {
                Some(physical) => TimePoint::from_physical(index, physical).ok_or_else(|| {
                    invalid_record(&path, line_number, "physical time must be finite")
                })?,
                None => TimePoint::new(index),
            };
            let mut state = series.spec().empty(time);
            decode_values(
                &self.decoders,
                stream,
                &path,
                line_number,
                record.values,
                &mut state,
            )?;
            series.push(state).map_err(|rejection| {
                let (source, state) = rejection.into_parts();
                let index = state.time().index();
                drop(state);
                StorageError::SeriesInvariant {
                    stream: stream.name.clone(),
                    index,
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
            first_index,
            last_index,
        )?;
        verify_checksum(
            &self.metadata_path,
            &path,
            &chunk.checksum,
            hasher.finalize(),
        )
    }
}

impl fmt::Debug for SeriesReader {
    /// Formats bounded configuration without decoder internals or payloads.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SeriesReader")
            .field("root", &self.root)
            .field("streams", &self.metadata.streams.len())
            .field("decoders", &self.decoders)
            .finish_non_exhaustive()
    }
}

/// Borrowed JSONL record whose payload values point into one line buffer.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BorrowedRecord<'a> {
    index: u64,
    #[serde(default)]
    physical: Option<f64>,
    #[serde(borrow)]
    values: BorrowedValues<'a>,
}

/// Duplicate-preserving field collection backed by borrowed raw JSON slices.
struct BorrowedValues<'a> {
    entries: Vec<(String, &'a RawValue)>,
}

impl<'de: 'a, 'a> Deserialize<'de> for BorrowedValues<'a> {
    /// Rejects duplicate keys while retaining raw value boundaries.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(BorrowedValuesVisitor {
            output: std::marker::PhantomData,
        })
    }
}

/// Serde visitor for one record's `values` object.
struct BorrowedValuesVisitor<'a> {
    /// Selects the shorter lifetime exposed by the containing record.
    output: std::marker::PhantomData<&'a RawValue>,
}

impl<'de: 'a, 'a> Visitor<'de> for BorrowedValuesVisitor<'a> {
    type Value = BorrowedValues<'a>;

    /// Describes the required JSON representation.
    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an object of unique field keys and raw JSON values")
    }

    /// Collects small owned keys while every potentially large value is
    /// borrowed directly from the input line.
    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut entries = Vec::with_capacity(map.size_hint().unwrap_or(0));
        while let Some((key, value)) = map.next_entry::<Cow<'de, str>, &'de RawValue>()? {
            if entries.iter().any(|(existing, _)| existing == key.as_ref()) {
                return Err(serde::de::Error::custom(format!(
                    "duplicate payload field `{key}`"
                )));
            }
            let value: &'a RawValue = value;
            entries.push((key.into_owned(), value));
        }
        Ok(BorrowedValues { entries })
    }
}

/// Serde representation accepted by the crate-private StateSpec parser.
#[derive(Serialize)]
struct StreamTemplateRef<'a> {
    fields: &'a [super::format::FieldMetadata],
}

/// Reconstructs one stream's immutable key/description specification.
fn stream_spec(metadata_path: &Path, stream: &StreamMetadata) -> Result<StateSpec, StorageError> {
    let bytes = serde_json::to_vec(&StreamTemplateRef {
        fields: &stream.fields,
    })
    .map_err(|source| StorageError::Json {
        operation: "serialize stream schema",
        path: metadata_path.to_path_buf(),
        source,
    })?;
    StateSpec::parse(metadata_path.to_path_buf(), &bytes).map_err(|source| {
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

/// Enforces strict simulation-index order across chunk boundaries.
fn validate_index(
    path: &Path,
    line: u64,
    index: u64,
    previous: Option<u64>,
) -> Result<(), StorageError> {
    if let Some(previous) = previous
        && index <= previous
    {
        return Err(invalid_record(
            path,
            line,
            format!("time index {index} is not greater than previous index {previous}"),
        ));
    }
    Ok(())
}

/// Validates exact keys, dispatches canonical decoders, and fills one state.
fn decode_values(
    decoders: &Decoders,
    stream: &StreamMetadata,
    path: &Path,
    line: u64,
    mut values: BorrowedValues<'_>,
    state: &mut crate::system_state::SystemState,
) -> Result<(), StorageError> {
    for field in &stream.fields {
        let Some(position) = values
            .entries
            .iter()
            .position(|(name, _)| name == &field.name)
        else {
            return Err(invalid_record(
                path,
                line,
                format!("missing payload field `{}`", field.name),
            ));
        };
        let (_, raw) = values.entries.swap_remove(position);
        decoders.decode_into(
            &stream.name,
            state.time().index(),
            &field.name,
            raw.get(),
            state,
        )?;
    }
    if !values.entries.is_empty() {
        let mut extra = values
            .entries
            .into_iter()
            .map(|(name, _)| name)
            .collect::<Vec<_>>();
        extra.sort_unstable();
        return Err(invalid_record(
            path,
            line,
            format!("undeclared payload fields: {}", extra.join(", ")),
        ));
    }
    Ok(())
}

/// Compares parsed chunk facts with the authoritative metadata descriptor.
fn validate_chunk_facts(
    metadata_path: &Path,
    stream: &StreamMetadata,
    chunk: &ChunkMetadata,
    records: u64,
    first_index: Option<u64>,
    last_index: Option<u64>,
) -> Result<(), StorageError> {
    if records != chunk.records
        || first_index != Some(chunk.first_index)
        || last_index != Some(chunk.last_index)
    {
        return Err(StorageError::InvalidMetadata {
            path: metadata_path.to_path_buf(),
            reason: format!(
                "stream `{}` chunk {} declares {} records at {}..={}, but contains {} records at {:?}..={:?}",
                stream.name,
                chunk.ordinal,
                chunk.records,
                chunk.first_index,
                chunk.last_index,
                records,
                first_index,
                last_index
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
