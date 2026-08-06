//! Run-level persistence and reconstruction for scientific state samples.
//!
//! This module is the complete public storage boundary. Simulations configure
//! named output streams through [`RunOutputBuilder`], borrow their live
//! [`SystemState`] at each sampling cadence, and hand the resulting encoded
//! record to [`RunOutput::sample`]. Each stream owns an independent bounded
//! writer queue and byte-targeted chunk sequence, while the run owns exactly
//! one authoritative `metadata.json` lifecycle.
//!
//! # Ownership and backpressure
//!
//! Sampling never clones, removes, or retains a scientific payload. The
//! selected values are borrowed only while Serde creates one owned JSONL
//! record. That record is then moved into its stream writer. If the configured
//! queue-byte budget is full, [`RunOutput::sample`] blocks until the writer
//! commits enough queued bytes or reports a terminal error. Records are never
//! split between chunks.
//!
//! # Lifecycle
//!
//! [`RunOutputBuilder::start`] refuses an existing output root, validates every
//! stream against one shared state specification, starts the stream writers,
//! and atomically publishes initial `running` metadata before returning.
//! [`RunOutput::finish`] drains every writer and atomically replaces that file
//! with a complete chunk inventory. [`RunOutput::fail`] performs the same safe
//! drain but records an explicit failed lifecycle instead. Dropping an active
//! output drains its writer threads for memory and file safety, but deliberately
//! leaves metadata as `running`; an implicit drop cannot claim successful or
//! intentional termination.
//!
//! # Reading
//!
//! [`SeriesReader`] accepts a completed output directory and a [`Decoders`]
//! registry. The reader validates metadata, chunks, checksums, record order,
//! and decoder coverage before reconstructing typed
//! [`StateSeries`](crate::time_series::StateSeries) values. Decoder
//! implementations remain per payload type and registrations remain per exact
//! state key.

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::system_state::{StateSpec, SystemState};

mod decoder;
mod encoder;
mod error;
mod format;
mod reader;
mod writer;

pub use decoder::{Decoders, PayloadDecoder, StringDecoder, VecF64Decoder};
pub use error::StorageError;
pub use reader::SeriesReader;

use encoder::JsonEncoder;
use format::{FieldMetadata, RunMetadata, RunStatus, StreamMetadata, TimeAxis as StoredTimeAxis};
use writer::{StateWriter, WriterConfig};

/// Stable name of the sole structural metadata file in one output root.
const METADATA_FILE: &str = "metadata.json";

/// Temporary sibling used for atomic metadata replacement.
const METADATA_TEMP_FILE: &str = ".metadata.json.tmp";

/// Public description of the temporal coordinates used by a run.
///
/// Every record always has an integer simulation index. Physical time remains
/// optional, and its unit is legal only when a physical-coordinate name is
/// configured. Labels are documentation persisted once in `metadata.json`;
/// they do not change [`crate::system_state::TimePoint`] representation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimeAxis {
    index_name: String,
    index_unit: Option<String>,
    physical_name: Option<String>,
    physical_unit: Option<String>,
}

impl TimeAxis {
    /// Creates a time-axis declaration with a mandatory index label.
    ///
    /// Whitespace is retained in the builder and rejected by
    /// [`RunOutputBuilder::start`], keeping fluent configuration infallible
    /// while ensuring persisted labels are never silently normalized.
    pub fn new(index_name: impl Into<String>) -> Self {
        Self {
            index_name: index_name.into(),
            index_unit: None,
            physical_name: None,
            physical_unit: None,
        }
    }

    /// Sets the optional unit of the integer simulation index.
    #[must_use]
    pub fn index_unit(mut self, unit: impl Into<String>) -> Self {
        self.index_unit = Some(unit.into());
        self
    }

    /// Declares the optional floating-point physical coordinate.
    #[must_use]
    pub fn physical_name(mut self, name: impl Into<String>) -> Self {
        self.physical_name = Some(name.into());
        self
    }

    /// Sets the physical-coordinate unit.
    ///
    /// A matching [`TimeAxis::physical_name`] is required; construction fails
    /// at [`RunOutputBuilder::start`] if the unit is configured alone.
    #[must_use]
    pub fn physical_unit(mut self, unit: impl Into<String>) -> Self {
        self.physical_unit = Some(unit.into());
        self
    }

    /// Converts public configuration into the private persisted representation.
    fn into_stored(self) -> StoredTimeAxis {
        StoredTimeAxis {
            index_name: self.index_name,
            index_unit: self.index_unit,
            physical_name: self.physical_name,
            physical_unit: self.physical_unit,
        }
    }
}

impl Default for TimeAxis {
    /// Uses `index` as the simulation-index label and declares no units or
    /// physical coordinate.
    fn default() -> Self {
        Self::new("index")
    }
}

/// Configuration for one independently sampled logical output stream.
///
/// Field names are exact keys from the run's [`StateSpec`]. Their input order
/// is irrelevant: the encoder writes them in canonical template order. The
/// chunk byte limit is a rollover target, so a single larger record remains
/// intact in its own oversized chunk. The queue byte limit is strict; a record
/// larger than the complete queue budget is rejected because it can never be
/// admitted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamConfig {
    name: String,
    directory: String,
    cadence: Option<String>,
    fields: Vec<String>,
    max_chunk_bytes: NonZeroU64,
    queue_bytes: NonZeroU64,
}

impl StreamConfig {
    /// Creates a stream whose relative output directory initially equals its
    /// logical name.
    ///
    /// Non-zero byte types make both storage limits valid by construction.
    /// Names, paths, duplicate fields, and state-key membership are validated
    /// together by [`RunOutputBuilder::start`].
    pub fn new<I, K>(
        name: impl Into<String>,
        fields: I,
        max_chunk_bytes: NonZeroU64,
        queue_bytes: NonZeroU64,
    ) -> Self
    where
        I: IntoIterator<Item = K>,
        K: Into<String>,
    {
        let name = name.into();
        Self {
            directory: name.clone(),
            name,
            cadence: None,
            fields: fields.into_iter().map(Into::into).collect(),
            max_chunk_bytes,
            queue_bytes,
        }
    }

    /// Overrides the stream's relative directory beneath the run root.
    ///
    /// Absolute paths, empty paths, and `.` or `..` components are rejected at
    /// start. Distinct streams must use distinct directories.
    #[must_use]
    pub fn directory(mut self, directory: impl Into<String>) -> Self {
        self.directory = directory.into();
        self
    }

    /// Adds an optional human-readable cadence description to metadata.
    ///
    /// Cadence is descriptive only. The simulation remains responsible for
    /// deciding when to call [`RunOutput::sample`].
    #[must_use]
    pub fn cadence(mut self, cadence: impl Into<String>) -> Self {
        self.cadence = Some(cadence.into());
        self
    }
}

/// Builder for one exclusive run output directory.
///
/// The builder owns only paths, immutable configuration, and a cheap shared
/// [`StateSpec`] handle. It opens no files and starts no threads before
/// [`RunOutputBuilder::start`].
#[derive(Debug)]
pub struct RunOutputBuilder {
    root: PathBuf,
    spec: StateSpec,
    time: TimeAxis,
    run_metadata: Map<String, Value>,
    streams: Vec<StreamConfig>,
}

impl RunOutputBuilder {
    /// Creates an empty run configuration using [`TimeAxis::default`].
    ///
    /// `spec` is cloned only as an `Arc`-backed metadata handle. No scientific
    /// state or payload exists in this builder.
    pub fn new(root: impl Into<PathBuf>, spec: &StateSpec) -> Self {
        Self {
            root: root.into(),
            spec: spec.clone(),
            time: TimeAxis::default(),
            run_metadata: Map::new(),
            streams: Vec::new(),
        }
    }

    /// Replaces the run's temporal-coordinate documentation.
    #[must_use]
    pub fn time_axis(mut self, time: TimeAxis) -> Self {
        self.time = time;
        self
    }

    /// Replaces caller-owned run metadata persisted under the `run` property.
    ///
    /// Values must already be JSON-compatible. This metadata is structurally
    /// separate from scientific payloads and is written only to
    /// `metadata.json`.
    #[must_use]
    pub fn run_metadata(mut self, metadata: Map<String, Value>) -> Self {
        self.run_metadata = metadata;
        self
    }

    /// Appends one logical stream declaration in deterministic metadata order.
    ///
    /// Duplicate names or directories are reported at start so fluent builder
    /// assembly remains infallible.
    #[must_use]
    pub fn stream(mut self, stream: StreamConfig) -> Self {
        self.streams.push(stream);
        self
    }

    /// Validates the complete run, creates its exclusive output root, starts
    /// each bounded writer, and publishes initial metadata atomically.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::OutputExists`] rather than replacing any
    /// existing filesystem entry. Configuration, state-key selection,
    /// directory creation, thread startup, JSON, and metadata durability
    /// failures retain their precise [`StorageError`] context. If startup fails
    /// after the root is created, the path is retained as diagnostic evidence
    /// and is never silently removed.
    pub fn start(self) -> Result<RunOutput, StorageError> {
        RunOutput::start(self)
    }
}

/// Exclusive coordinator for all persistent streams in one scientific run.
///
/// This type is intentionally non-Clone. It owns the only writer handles and
/// the only legal transition from `running` metadata to a terminal status.
/// It owns no [`SystemState`] and never extends a payload borrow beyond one
/// synchronous [`RunOutput::sample`] call.
pub struct RunOutput {
    root: PathBuf,
    metadata_path: PathBuf,
    metadata: RunMetadata,
    streams: HashMap<String, ActiveStream>,
}

impl RunOutput {
    /// Begins configuring a new exclusive run output directory.
    pub fn builder(root: impl Into<PathBuf>, spec: &StateSpec) -> RunOutputBuilder {
        RunOutputBuilder::new(root, spec)
    }

    /// Returns the run output root exactly as configured.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Iterates logical stream names in deterministic declaration order.
    pub fn streams(&self) -> impl ExactSizeIterator<Item = &str> {
        self.metadata
            .streams
            .iter()
            .map(|stream| stream.name.as_str())
    }

    /// Samples selected fields from one live state and transfers the encoded
    /// record into that stream's bounded writer.
    ///
    /// The encoder borrows payloads only during this call. Queue admission may
    /// then block on owned encoded bytes, but the state borrow and every payload
    /// borrow have already ended before writer backpressure begins.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::UnknownStream`] for an undeclared name, state or
    /// payload serialization errors from borrowed encoding, queue-limit and
    /// ordering errors, or the writer's authoritative terminal failure.
    pub fn sample(&self, stream: &str, state: &SystemState) -> Result<(), StorageError> {
        let active = self
            .streams
            .get(stream)
            .ok_or_else(|| StorageError::UnknownStream {
                stream: stream.to_owned(),
            })?;
        let record = active.encoder.encode(state)?;
        active.writer.submit(record)
    }

    /// Drains every stream, seals all chunks, and atomically publishes complete
    /// metadata.
    ///
    /// The method consumes the coordinator, making repeated finish or sampling
    /// impossible in safe Rust. If a writer fails, all remaining writers are
    /// still drained and a best-effort failed metadata transition is attempted
    /// before the originating writer error is returned.
    pub fn finish(mut self) -> Result<(), StorageError> {
        let first_error = self.finish_writers();
        if let Some(error) = first_error {
            self.metadata.status = RunStatus::Failed {
                message: error.to_string(),
            };
            let _ = commit_metadata(&self.root, &self.metadata_path, &self.metadata);
            return Err(error);
        }

        self.metadata.status = RunStatus::Complete;
        commit_metadata(&self.root, &self.metadata_path, &self.metadata)
    }

    /// Drains every stream and atomically records an intentional failed run.
    ///
    /// This is appropriate when the simulation itself fails after storage has
    /// started. The supplied message is structural run metadata and must not be
    /// empty or whitespace-only. Successfully accepted records remain as
    /// immutable chunks and are listed in the failed metadata, but
    /// [`SeriesReader`] deliberately reconstructs only completed runs.
    ///
    /// If a writer also fails, its error takes precedence as the returned and
    /// persisted reason; the caller's message would no longer describe the
    /// authoritative storage termination.
    pub fn fail(mut self, message: impl Into<String>) -> Result<(), StorageError> {
        let message = message.into();
        if message.trim().is_empty() {
            return Err(StorageError::InvalidConfig {
                setting: "failure_message",
                reason: "failed run message must not be empty".to_owned(),
            });
        }

        if let Some(error) = self.finish_writers() {
            self.metadata.status = RunStatus::Failed {
                message: error.to_string(),
            };
            let _ = commit_metadata(&self.root, &self.metadata_path, &self.metadata);
            return Err(error);
        }

        self.metadata.status = RunStatus::Failed { message };
        commit_metadata(&self.root, &self.metadata_path, &self.metadata)
    }

    /// Performs complete validation before creating or mutating the run root.
    fn start(builder: RunOutputBuilder) -> Result<Self, StorageError> {
        ensure_absent(&builder.root)?;
        let metadata_path = builder.root.join(METADATA_FILE);
        let stored_time = builder.time.into_stored();
        let mut names = HashSet::with_capacity(builder.streams.len());
        let mut directories = HashSet::with_capacity(builder.streams.len());
        let mut prepared = Vec::with_capacity(builder.streams.len());
        let mut declarations = Vec::with_capacity(builder.streams.len());

        for config in builder.streams {
            if !names.insert(config.name.clone()) {
                return Err(StorageError::DuplicateStream {
                    stream: config.name,
                });
            }
            if !directories.insert(config.directory.clone()) {
                return Err(StorageError::InvalidConfig {
                    setting: "stream.directory",
                    reason: format!(
                        "multiple streams use relative directory `{}`",
                        config.directory
                    ),
                });
            }

            let encoder = JsonEncoder::new(&config.name, &builder.spec, &config.fields)?;
            let fields = encoder
                .fields()
                .map(|name| {
                    let field = builder
                        .spec
                        .get(name)
                        .expect("encoder fields were validated against this specification");
                    FieldMetadata {
                        name: name.to_owned(),
                        description: field.description().map(str::to_owned),
                    }
                })
                .collect();
            declarations.push(StreamMetadata {
                name: config.name.clone(),
                directory: config.directory.clone(),
                cadence: config.cadence,
                fields,
                max_chunk_bytes: config.max_chunk_bytes.get(),
                queue_bytes: config.queue_bytes.get(),
                chunks: Vec::new(),
            });
            prepared.push((
                config.name,
                encoder,
                config.directory,
                config.max_chunk_bytes,
                config.queue_bytes,
            ));
        }

        let mut metadata = RunMetadata::running(stored_time, builder.run_metadata, declarations);
        metadata.validate(&metadata_path)?;
        create_root(&builder.root)?;

        let mut streams = HashMap::with_capacity(prepared.len());
        for (name, encoder, directory, max_chunk_bytes, queue_bytes) in prepared {
            let writer_config = WriterConfig::new(
                &name,
                builder.root.join(directory),
                max_chunk_bytes,
                queue_bytes,
            )?;
            match StateWriter::start(writer_config) {
                Ok(writer) => {
                    streams.insert(name, ActiveStream { encoder, writer });
                }
                Err(error) => {
                    metadata.status = RunStatus::Failed {
                        message: error.to_string(),
                    };
                    let _ = commit_metadata(&builder.root, &metadata_path, &metadata);
                    return Err(error);
                }
            }
        }

        commit_metadata(&builder.root, &metadata_path, &metadata)?;
        Ok(Self {
            root: builder.root,
            metadata_path,
            metadata,
            streams,
        })
    }

    /// Finishes every writer and installs successful chunk inventories.
    ///
    /// The first failure is returned after every remaining stream has received
    /// its drain-and-join opportunity. Later failures cannot replace the first
    /// authoritative error.
    fn finish_writers(&mut self) -> Option<StorageError> {
        let names = self
            .metadata
            .streams
            .iter()
            .map(|stream| stream.name.clone())
            .collect::<Vec<_>>();
        let mut first_error = None;

        for name in names {
            let active = self
                .streams
                .remove(&name)
                .expect("each metadata stream owns exactly one active writer");
            match active.writer.finish() {
                Ok(summary) => {
                    let declaration = self
                        .metadata
                        .stream_mut(&name)
                        .expect("active stream must have a metadata declaration");
                    declaration.chunks = summary.chunks().to_vec();
                }
                Err(error) if first_error.is_none() => first_error = Some(error),
                Err(_) => {}
            }
        }
        first_error
    }
}

/// Private pairing of one stream's immutable encoder and exclusive writer.
struct ActiveStream {
    encoder: JsonEncoder,
    writer: StateWriter,
}

/// Rejects every existing filesystem object and preserves IO inspection errors.
fn ensure_absent(root: &Path) -> Result<(), StorageError> {
    match root.try_exists() {
        Ok(false) => Ok(()),
        Ok(true) => Err(StorageError::OutputExists {
            path: root.to_path_buf(),
        }),
        Err(source) => Err(StorageError::Io {
            operation: "inspect output root",
            path: root.to_path_buf(),
            source,
        }),
    }
}

/// Exclusively creates the run root, closing the check/create race safely.
fn create_root(root: &Path) -> Result<(), StorageError> {
    match fs::create_dir(root) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(StorageError::OutputExists {
                path: root.to_path_buf(),
            })
        }
        Err(source) => Err(StorageError::Io {
            operation: "create output root",
            path: root.to_path_buf(),
            source,
        }),
    }
}

/// Atomically replaces the sole authoritative metadata document.
///
/// The temporary file is created exclusively, serialized once, flushed with
/// `sync_all`, renamed over the previous snapshot, and followed by a directory
/// sync. A failed attempt removes only its precisely owned temporary path when
/// possible; the previous authoritative metadata remains untouched until the
/// rename succeeds.
fn commit_metadata(
    root: &Path,
    metadata_path: &Path,
    metadata: &RunMetadata,
) -> Result<(), StorageError> {
    metadata.validate(metadata_path)?;
    let mut bytes = serde_json::to_vec_pretty(metadata).map_err(|source| StorageError::Json {
        operation: "serialize metadata",
        path: metadata_path.to_path_buf(),
        source,
    })?;
    bytes.push(b'\n');

    let temporary_path = root.join(METADATA_TEMP_FILE);
    let result = write_and_replace_metadata(root, metadata_path, &temporary_path, &bytes);
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

/// Performs the fallible filesystem portion of one metadata transaction.
fn write_and_replace_metadata(
    root: &Path,
    metadata_path: &Path,
    temporary_path: &Path,
    bytes: &[u8],
) -> Result<(), StorageError> {
    let mut temporary = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temporary_path)
        .map_err(|source| StorageError::Io {
            operation: "create temporary metadata",
            path: temporary_path.to_path_buf(),
            source,
        })?;
    temporary
        .write_all(bytes)
        .map_err(|source| StorageError::Io {
            operation: "write temporary metadata",
            path: temporary_path.to_path_buf(),
            source,
        })?;
    temporary.sync_all().map_err(|source| StorageError::Io {
        operation: "sync temporary metadata",
        path: temporary_path.to_path_buf(),
        source,
    })?;
    drop(temporary);

    fs::rename(temporary_path, metadata_path).map_err(|source| StorageError::Io {
        operation: "publish metadata",
        path: metadata_path.to_path_buf(),
        source,
    })?;

    File::open(root)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| StorageError::Io {
            operation: "sync output root",
            path: root.to_path_buf(),
            source,
        })
}
