//! Private local persistence adapter and verified reconstruction.
//!
//! This module is not API. Runtime reaches it only through the parent
//! `PersistenceSession`, which supplies Study's immutable plan, inferred path,
//! observation plan, schema, and provenance. Models and applications cannot
//! construct or drive these writers.
//!
//! One bounded queue and worker serve every configured stream. Each stream
//! accumulates an independent byte-targeted chunk in reusable userspace memory
//! and performs filesystem IO only when publishing that chunk. A recording
//! owns exactly one authoritative `metadata.json` lifecycle.
//!
//! # Ownership and backpressure
//!
//! Sampling never clones, removes, or retains a scientific payload. The
//! selected values are borrowed only while Serde creates one owned JSONL
//! record. That record is then moved into the recording writer. If the configured
//! queue-byte budget is full, [`SystemStateWriter::observe_state`] blocks until the writer
//! commits enough queued bytes or reports a terminal error. Records are never
//! split between chunks.
//!
//! # Lifecycle
//!
//! [`SystemStateWriter::create`] refuses an existing output root, consumes an
//! observation plan already bound to the shared state schema, infers deterministic
//! stream layout, and publishes initial `running`
//! metadata, and then starts the recording writer. A complete buffered chunk is
//! written once, synchronized, described in metadata, and atomically sealed.
//! [`SystemStateWriter::complete_recording`] drains the writer, atomically commits
//! completion timing and terminal metadata;
//! [`SystemStateWriter::mark_recording_failed`] records an explicit failed
//! lifecycle instead. Dropping an active recording drains its writer thread for
//! memory and file safety but deliberately leaves metadata as `running`.
//!
//! # Reading
//!
//! [`StoredStateSeriesReader`] accepts a completed output directory and a [`JsonPayloadDecoderRegistry`]
//! registry. The reader validates metadata, chunks, checksums, record order,
//! and decoder coverage before reconstructing typed
//! [`StateSeries`](crate::state::advanced::StateSeries) values. Decoder
//! implementations remain per payload type and registrations remain per exact
//! state key. Latest-state reads verify and decode only the newest chunk.
//!
//! # Boundary
//!
//! This adapter owns local durable mechanics: run directories, stream chunking, queue
//! flushing, metadata transitions, and reconstruction integrity checks. The
//! observation subsystem owns scientific stream schemas, cadence, and
//! encoding; callers own simulation evolution and decoder registrations. The
//! adapter does not define
//! modeling APIs, RNG behavior, or artifact semantics.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::clock::{duration_nanoseconds, utc_now_rfc3339};
use crate::observation::advanced::{BoundObservationPlan, ObservationSession};
use crate::state::advanced::SystemState;

mod error;
mod json_payload_decoder;
mod jsonl_format;
mod queued_state_writer;
mod stored_state_series_reader;

pub use error::PersistenceError;
pub use json_payload_decoder::{
    JsonPayloadDecoder, JsonPayloadDecoderRegistry, JsonStringDecoder, JsonVecF64Decoder,
};
pub use stored_state_series_reader::StoredStateSeriesReader;

use jsonl_format::{
    EncodedStateRecord, RecordingMetadata, RecordingStatus, StateFieldMetadata,
    StateStreamMetadata, TimeAxisMetadata as StoredTimeAxis,
};
use queued_state_writer::{StateStreamStorageConfig, StateWriterWorker};

/// Stable name of the sole structural metadata file in one output root.
const METADATA_FILE: &str = "metadata.json";

/// Temporary sibling used for atomic metadata replacement.
const METADATA_TEMP_FILE: &str = ".metadata.json.tmp";

/// Immutable operational timing returned after successful recording completion.
///
/// These values describe host execution rather than scientific coordinates.
/// Scientific iteration and physical time remain part of each recorded
/// [`SystemState`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordingTiming {
    created_at_utc: String,
    finalized_at_utc: String,
    active_duration_ns: u64,
    continuation_count: u64,
}

impl RecordingTiming {
    /// Converts the validated private wire representation into the public view.
    fn from_stored(
        timing: &jsonl_format::RecordingTiming,
        metadata_path: &Path,
    ) -> Result<Self, PersistenceError> {
        let finalized_at_utc =
            timing
                .finalized_at_utc
                .clone()
                .ok_or_else(|| PersistenceError::InvalidMetadata {
                    path: metadata_path.to_path_buf(),
                    reason: "completed recording lacks finalized timestamp".to_owned(),
                })?;
        Ok(Self {
            created_at_utc: timing.created_at_utc.clone(),
            finalized_at_utc,
            active_duration_ns: timing.active_duration_ns,
            continuation_count: timing.continuation_count,
        })
    }

    /// Returns the recording's original UTC creation timestamp in RFC 3339 form.
    pub fn created_at_utc(&self) -> &str {
        &self.created_at_utc
    }

    /// Returns the successful completion timestamp in UTC RFC 3339 form.
    pub fn finalized_at_utc(&self) -> &str {
        &self.finalized_at_utc
    }

    /// Returns the accumulated active writer duration as exact nanoseconds.
    pub fn active_duration_ns(&self) -> u64 {
        self.active_duration_ns
    }

    /// Returns the accumulated active writer duration as a standard duration.
    pub fn active_duration(&self) -> Duration {
        Duration::from_nanos(self.active_duration_ns)
    }

    /// Returns the stored continuation count.
    ///
    /// Recordings created by this crate always report zero because no
    /// continuation or resume path is supported.
    pub fn continuation_count(&self) -> u64 {
        self.continuation_count
    }
}

/// Coordinate-aware interval used to select states for one output stream.
///
/// The noun variant identifies the coordinate on which the interval is
/// measured. The current storage format supports iteration-based sampling;
/// adding physical-time sampling later will not require overloading the word
/// `step` or changing the surrounding stream API.
///
/// Human-authored configuration may use the concise JSON value `10` for every
/// ten iterations. Deserialization also accepts the stable tagged form
/// `{"iterations": 10}` emitted by serialization.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SamplingInterval {
    /// Select iteration zero and each iteration divisible by this interval.
    Iterations(NonZeroU64),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum SamplingIntervalInput {
    /// Concise configuration form: `10` means every ten iterations.
    Iterations(NonZeroU64),
    /// Stable tagged form emitted by [`SamplingInterval`]'s serializer.
    Tagged { iterations: NonZeroU64 },
}

impl<'de> Deserialize<'de> for SamplingInterval {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        match SamplingIntervalInput::deserialize(deserializer)? {
            SamplingIntervalInput::Iterations(interval)
            | SamplingIntervalInput::Tagged {
                iterations: interval,
            } => Ok(Self::Iterations(interval)),
        }
    }
}

impl SamplingInterval {
    /// Creates an iteration interval, returning `None` for zero.
    pub(crate) const fn iterations(interval: u64) -> Option<Self> {
        match NonZeroU64::new(interval) {
            Some(interval) => Some(Self::Iterations(interval)),
            None => None,
        }
    }
}

/// Filesystem layout for one logical state stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StateStreamLayout {
    /// Accumulate encoded records in memory until the rollover target is met.
    Chunked {
        /// Approximate encoded-byte threshold at which a chunk is sealed.
        target_bytes: NonZeroU64,
    },
    /// Publish every encoded record as its own immutable file.
    IndividualFiles,
}

/// Persistence and backpressure policy for one logical state stream.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StateStreamStorage {
    layout: StateStreamLayout,
    storage_queue_bytes: NonZeroU64,
}

impl StateStreamStorage {
    /// Creates an in-memory chunking policy with a strict queue-byte budget.
    pub const fn chunked(target_bytes: NonZeroU64, storage_queue_bytes: NonZeroU64) -> Self {
        Self {
            layout: StateStreamLayout::Chunked { target_bytes },
            storage_queue_bytes,
        }
    }

    /// Returns the configured on-disk stream layout.
    pub const fn layout(self) -> StateStreamLayout {
        self.layout
    }

    /// Returns the strict byte capacity shared by queued records.
    pub const fn storage_queue_bytes(self) -> NonZeroU64 {
        self.storage_queue_bytes
    }
}

/// Exclusive queued writer for all persistent streams in one recording.
///
/// This type is intentionally non-Clone. It owns the only writer handles and
/// the only legal transition from `running` metadata to a terminal status.
/// It owns no [`SystemState`] and never extends a payload borrow beyond one
/// synchronous [`SystemStateWriter::observe_state`] call.
pub struct SystemStateWriter {
    manifest: Arc<RecordingManifest>,
    session: ObservationSession,
    writer: Option<StateWriterWorker>,
    session_started: Instant,
    /// Held after writers so normal field drop keeps the lease until every
    /// worker has drained and released its manifest handle.
    _lease: RecordingLease,
}

impl SystemStateWriter {
    /// Creates the sole local writer from settings already resolved by Study.
    pub(crate) fn create(
        root: PathBuf,
        descriptor: BoundObservationPlan,
        user_metadata: Map<String, Value>,
        storage: StateStreamStorage,
    ) -> Result<Self, PersistenceError> {
        ensure_absent(&root)?;
        let prepared = PreparedRecording::new(root, descriptor, user_metadata, storage)?;
        create_root(&prepared.root)?;
        let lease = RecordingLease::acquire(&prepared.root)?;
        for stream in &prepared.streams {
            stream.create_directory()?;
        }
        commit_metadata(&prepared.root, &prepared.metadata_path, &prepared.metadata)?;
        let manifest = Arc::new(RecordingManifest::new(
            prepared.root.clone(),
            prepared.metadata_path,
            prepared.metadata,
        ));
        Self::start_new_prepared(prepared.descriptor, prepared.streams, manifest, lease)
    }

    /// Offers the current live state to every configured sampling stream.
    ///
    /// The writer first reads only the state's iteration. Streams that are
    /// not due perform no field lookup, payload borrow, serialization,
    /// allocation, or queue operation. Every due stream encodes its selected
    /// fields before bounded queue admission, so backpressure retains only
    /// owned bytes and never extends a scientific payload borrow.
    ///
    /// # Errors
    ///
    /// Returns state or payload serialization errors from a due stream,
    /// queue-limit and ordering errors, or the writer's authoritative terminal
    /// failure.
    pub fn observe_state(&mut self, state: &SystemState) -> Result<(), PersistenceError> {
        let observations = self
            .session
            .observe(state)
            .map_err(|source| PersistenceError::Observation { source })?;
        let writer = self
            .writer
            .as_ref()
            .expect("an active recording owns its writer worker");
        for observation in observations {
            let (stream, time, bytes) = observation.into_parts();
            let record = EncodedStateRecord::new(time, bytes);
            writer.submit_record(stream, record)?;
        }
        Ok(())
    }

    /// Drains every stream, seals all chunks, and atomically publishes complete
    /// metadata.
    ///
    /// The method consumes the coordinator, making repeated finish or sampling
    /// impossible in safe Rust. If a writer fails, all remaining writers are
    /// still drained and a best-effort failed metadata transition is attempted
    /// before the originating writer error is returned.
    pub(crate) fn complete_recording(self) -> Result<(), PersistenceError> {
        self.complete_recording_with_terminal_metadata(Map::new())
    }

    /// Completes the recording and atomically commits values known only at the
    /// terminal boundary.
    ///
    /// Terminal values are stored separately from immutable creation-time user
    /// metadata and therefore cannot silently replace configuration parameters.
    pub(crate) fn complete_recording_with_terminal_metadata(
        mut self,
        terminal_metadata: Map<String, Value>,
    ) -> Result<(), PersistenceError> {
        if let Err(error) = self.finish_writer() {
            let _ = self.transition_terminal(
                RecordingStatus::Failed {
                    message: error.to_string(),
                },
                Map::new(),
            );
            return Err(error);
        }
        self.transition_terminal(RecordingStatus::Complete, terminal_metadata)
    }

    /// Records one final state to every stream exactly once, then completes.
    ///
    /// This terminal observation is independent of the sampling interval. A stream
    /// already recorded at the same iteration is skipped, while a non-aligned final
    /// iteration is encoded once. The writer therefore owns both interval-based and
    /// terminal sampling decisions; the simulation supplies only a borrowed state.
    pub(crate) fn complete_recording_with_final_state(
        mut self,
        state: &SystemState,
    ) -> Result<(), PersistenceError> {
        if let Err(error) = self.record_final_state(state) {
            let _ = self.mark_recording_failed(error.to_string());
            return Err(error);
        }
        self.complete_recording()
    }

    /// Encodes the supplied terminal state for streams that lack this iteration.
    pub(crate) fn record_final_state(
        &mut self,
        state: &SystemState,
    ) -> Result<(), PersistenceError> {
        let observations = self
            .session
            .observe_final(state)
            .map_err(|source| PersistenceError::Observation { source })?;
        let writer = self
            .writer
            .as_ref()
            .expect("an active recording owns its writer worker");
        for observation in observations {
            let (stream, time, bytes) = observation.into_parts();
            let record = EncodedStateRecord::new(time, bytes);
            writer.submit_record(stream, record)?;
        }
        Ok(())
    }

    /// Drains every stream and atomically records an intentional failed run.
    ///
    /// This is appropriate when the simulation itself fails after storage has
    /// started. The supplied message is structural recording metadata and must not be
    /// empty or whitespace-only. Successfully accepted records remain as
    /// immutable chunks and are listed in the failed metadata, but
    /// [`StoredStateSeriesReader`] deliberately reconstructs only completed runs.
    ///
    /// If a writer also fails, its error takes precedence as the returned and
    /// persisted reason; the caller's message would no longer describe the
    /// authoritative storage termination.
    pub(crate) fn mark_recording_failed(
        self,
        message: impl Into<String>,
    ) -> Result<(), PersistenceError> {
        self.mark_recording_failed_with_terminal_metadata(message, Map::new())
    }

    /// Records an intentional failure with terminal-only user metadata.
    pub(crate) fn mark_recording_failed_with_terminal_metadata(
        mut self,
        message: impl Into<String>,
        terminal_metadata: Map<String, Value>,
    ) -> Result<(), PersistenceError> {
        let message = message.into();
        if message.trim().is_empty() {
            return Err(PersistenceError::InvalidConfiguration {
                setting: "failure_message",
                reason: "failed run message must not be empty".to_owned(),
            });
        }

        if let Err(error) = self.finish_writer() {
            let _ = self.transition_terminal(
                RecordingStatus::Failed {
                    message: error.to_string(),
                },
                Map::new(),
            );
            return Err(error);
        }
        self.transition_terminal(RecordingStatus::Failed { message }, terminal_metadata)
    }

    /// Spawns every empty writer after the initial manifest is durable.
    fn start_new_prepared(
        descriptor: BoundObservationPlan,
        streams: Vec<StateStreamStorageConfig>,
        manifest: Arc<RecordingManifest>,
        lease: RecordingLease,
    ) -> Result<Self, PersistenceError> {
        let mut configs = Vec::with_capacity(streams.len());
        configs.extend(streams);
        let writer = StateWriterWorker::start_new_recording(configs, Arc::clone(&manifest))?;
        Ok(Self {
            manifest,
            session: ObservationSession::new(descriptor),
            writer: Some(writer),
            session_started: Instant::now(),
            _lease: lease,
        })
    }

    /// Drains and joins the recording's sole queued writer worker.
    fn finish_writer(&mut self) -> Result<(), PersistenceError> {
        let Some(writer) = self.writer.take() else {
            return Ok(());
        };
        writer.finish_recording()
    }

    /// Commits one terminal status, timestamp, duration, and metadata map.
    fn transition_terminal(
        &self,
        status: RecordingStatus,
        terminal_metadata: Map<String, Value>,
    ) -> Result<(), PersistenceError> {
        let finalized_at_utc =
            utc_now_rfc3339().map_err(|source| PersistenceError::OperationalTimestamp {
                operation: "finalize recording",
                source,
            })?;
        let active_duration_ns = duration_nanoseconds(self.session_started.elapsed())
            .ok_or(PersistenceError::OperationalDurationOverflow)?;
        self.manifest.transition_terminal(
            status,
            finalized_at_utc,
            active_duration_ns,
            terminal_metadata,
        )
    }
}

/// Fully validated builder output before any writer thread starts.
struct PreparedRecording {
    root: PathBuf,
    metadata_path: PathBuf,
    descriptor: BoundObservationPlan,
    metadata: RecordingMetadata,
    streams: Vec<StateStreamStorageConfig>,
}

impl PreparedRecording {
    /// Builds persisted metadata from one already-bound observation plan.
    fn new(
        root: PathBuf,
        descriptor: BoundObservationPlan,
        user_metadata: Map<String, Value>,
        storage: StateStreamStorage,
    ) -> Result<Self, PersistenceError> {
        let metadata_path = root.join(METADATA_FILE);
        let stored_time = StoredTimeAxis {
            iteration_name: "iteration".to_owned(),
            iteration_unit: descriptor.iteration_unit().map(str::to_owned),
            physical_time_name: Some("physical_time".to_owned()),
            physical_time_unit: descriptor.physical_time_unit().map(str::to_owned),
        };
        let mut streams = Vec::with_capacity(descriptor.streams().len());
        let mut declarations = Vec::with_capacity(descriptor.streams().len());

        for (index, stream) in descriptor.streams().iter().enumerate() {
            let directory = format!("stream_{index:04}");
            let sampling_interval = SamplingInterval::iterations(stream.every_iterations())
                .expect("bound observation streams contain positive sampling intervals");
            let fields = stream
                .fields()
                .iter()
                .map(|field| StateFieldMetadata {
                    name: field.name().to_owned(),
                    description: field.description().map(str::to_owned),
                })
                .collect::<Vec<_>>();
            declarations.push(StateStreamMetadata {
                name: stream.name().to_owned(),
                directory: directory.clone(),
                sampling_interval,
                fields,
                storage,
                chunks: Vec::new(),
            });
            streams.push(StateStreamStorageConfig::new(
                stream.name(),
                root.join(&directory),
                storage,
            )?);
        }

        let created_at_utc =
            utc_now_rfc3339().map_err(|source| PersistenceError::OperationalTimestamp {
                operation: "create recording",
                source,
            })?;
        let metadata =
            RecordingMetadata::running(stored_time, user_metadata, declarations, created_at_utc);
        metadata.validate(&metadata_path)?;
        Ok(Self {
            root,
            metadata_path,
            descriptor,
            metadata,
            streams,
        })
    }
}

/// Serialized authority over the sole mutable metadata document.
///
/// Every worker shares this small coordinator. A transaction clones metadata,
/// validates and persists the candidate, then replaces the in-memory snapshot
/// only after the atomic filesystem commit succeeds.
pub(crate) struct RecordingManifest {
    root: PathBuf,
    path: PathBuf,
    metadata: Mutex<RecordingMetadata>,
}

impl RecordingManifest {
    /// Creates an authority from the exact snapshot already present on disk.
    fn new(root: PathBuf, path: PathBuf, metadata: RecordingMetadata) -> Self {
        Self {
            root,
            path,
            metadata: Mutex::new(metadata),
        }
    }

    /// Appends one prepared descriptor and commits it before filename sealing.
    pub(crate) fn prepare_chunk(
        &self,
        stream: &str,
        descriptor: jsonl_format::ChunkMetadata,
    ) -> Result<(), PersistenceError> {
        let mut current = lock_metadata(&self.metadata);
        if !matches!(current.status, RecordingStatus::Running) {
            return Err(PersistenceError::RecordingFinished);
        }
        let mut candidate = current.clone();
        let declaration =
            candidate
                .stream_mut(stream)
                .ok_or_else(|| PersistenceError::UnknownStateStream {
                    stream: stream.to_owned(),
                })?;
        let expected = u64::try_from(declaration.chunks.len()).map_err(|_| {
            PersistenceError::ByteCountOverflow {
                stream: stream.to_owned(),
            }
        })?;
        if descriptor.ordinal != expected {
            return Err(PersistenceError::InvalidMetadata {
                path: self.path.clone(),
                reason: format!(
                    "stream `{stream}` prepared chunk ordinal {}, expected {expected}",
                    descriptor.ordinal
                ),
            });
        }
        declaration.chunks.push(descriptor);
        commit_metadata(&self.root, &self.path, &candidate)?;
        *current = candidate;
        Ok(())
    }

    /// Atomically commits terminal lifecycle, timing, and user metadata.
    fn transition_terminal(
        &self,
        status: RecordingStatus,
        finalized_at_utc: String,
        active_duration_ns: u64,
        terminal_metadata: Map<String, Value>,
    ) -> Result<(), PersistenceError> {
        let mut current = lock_metadata(&self.metadata);
        let mut candidate = current.clone();
        candidate.status = status;
        candidate.timing.finalized_at_utc = Some(finalized_at_utc);
        candidate.timing.active_duration_ns = candidate
            .timing
            .active_duration_ns
            .checked_add(active_duration_ns)
            .ok_or(PersistenceError::OperationalDurationOverflow)?;
        candidate.terminal_metadata = terminal_metadata;
        commit_metadata(&self.root, &self.path, &candidate)?;
        *current = candidate;
        Ok(())
    }
}

/// Advisory exclusive ownership of the output root directory itself.
///
/// Locking the directory handle creates no lockfile or status artifact and the
/// operating system releases the lease automatically after process death.
struct RecordingLease {
    directory: File,
}

impl RecordingLease {
    /// Acquires non-blocking exclusive writer ownership.
    fn acquire(root: &Path) -> Result<Self, PersistenceError> {
        let directory = File::open(root).map_err(|source| PersistenceError::Io {
            operation: "open output root for exclusive ownership",
            path: root.to_path_buf(),
            source,
        })?;
        match FileExt::try_lock_exclusive(&directory) {
            Ok(()) => Ok(Self { directory }),
            Err(source) if source.kind() == std::io::ErrorKind::WouldBlock => {
                Err(PersistenceError::RecordingDirectoryInUse {
                    path: root.to_path_buf(),
                })
            }
            Err(source) => Err(PersistenceError::Io {
                operation: "acquire exclusive output ownership",
                path: root.to_path_buf(),
                source,
            }),
        }
    }
}

impl Drop for RecordingLease {
    fn drop(&mut self) {
        // Release explicitly instead of relying on file-descriptor teardown.
        // Several recording attempts can occur back-to-back in one process,
        // and the next attempt must never observe the previous lease.
        let _ = FileExt::unlock(&self.directory);
    }
}

/// Locks the metadata snapshot while recovering from a participant panic.
fn lock_metadata(metadata: &Mutex<RecordingMetadata>) -> MutexGuard<'_, RecordingMetadata> {
    metadata
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Rejects every existing filesystem object and preserves IO inspection errors.
fn ensure_absent(root: &Path) -> Result<(), PersistenceError> {
    match root.try_exists() {
        Ok(false) => Ok(()),
        Ok(true) => Err(PersistenceError::RecordingDirectoryExists {
            path: root.to_path_buf(),
        }),
        Err(source) => Err(PersistenceError::Io {
            operation: "inspect output root",
            path: root.to_path_buf(),
            source,
        }),
    }
}

/// Exclusively creates the run root, closing the check/create race safely.
fn create_root(root: &Path) -> Result<(), PersistenceError> {
    if let Some(parent) = root.parent() {
        fs::create_dir_all(parent).map_err(|source| PersistenceError::Io {
            operation: "create recording parent directories",
            path: parent.to_path_buf(),
            source,
        })?;
    }
    match fs::create_dir(root) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(PersistenceError::RecordingDirectoryExists {
                path: root.to_path_buf(),
            })
        }
        Err(source) => Err(PersistenceError::Io {
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
    metadata: &RecordingMetadata,
) -> Result<(), PersistenceError> {
    metadata.validate(metadata_path)?;
    let mut bytes =
        serde_json::to_vec_pretty(metadata).map_err(|source| PersistenceError::Json {
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
) -> Result<(), PersistenceError> {
    let mut temporary = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temporary_path)
        .map_err(|source| PersistenceError::Io {
            operation: "create temporary metadata",
            path: temporary_path.to_path_buf(),
            source,
        })?;
    temporary
        .write_all(bytes)
        .map_err(|source| PersistenceError::Io {
            operation: "write temporary metadata",
            path: temporary_path.to_path_buf(),
            source,
        })?;
    temporary
        .sync_all()
        .map_err(|source| PersistenceError::Io {
            operation: "sync temporary metadata",
            path: temporary_path.to_path_buf(),
            source,
        })?;
    drop(temporary);

    fs::rename(temporary_path, metadata_path).map_err(|source| PersistenceError::Io {
        operation: "publish metadata",
        path: metadata_path.to_path_buf(),
        source,
    })?;

    File::open(root)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| PersistenceError::Io {
            operation: "sync output root",
            path: root.to_path_buf(),
            source,
        })
}
