//! Bounded asynchronous persistence and tail recovery for one recording.
//!
//! [`StateWriterWorker`] accepts complete [`EncodedStateRecord`] values, applies strict
//! byte/count backpressure, and moves every stream through one FIFO worker.
//! Each private stream sink owns at most one open payload. The worker prepares
//! descriptors through the recording-level [`RecordingManifest`] and
//! atomically renames each payload into its immutable final name. No payload is
//! copied during this lifecycle.
//!
//! # Filename lifecycle
//!
//! `chunk-NNNNNN.jsonl.tmp` is the real open payload. After its bytes and open
//! directory entry are synchronized, its descriptor is committed to the sole
//! `metadata.json`; only then is it renamed to `chunk-NNNNNN.jsonl`. The final
//! name is the authoritative seal marker. This ordering ensures recovery never
//! needs to inspect sealed payload contents to rebuild missing metadata.
//!
//! # Recovery boundary
//!
//! Resume checks filenames for the committed ordinal prefix but never opens a
//! final `.jsonl` file. At most the highest `.jsonl.tmp` payload is opened. A
//! prepared open chunk is verified against its existing descriptor and renamed;
//! an unprepared open chunk retains its complete JSONL prefix, truncates only a
//! non-newline-terminated crash fragment, and is moved directly into the new
//! worker as its active chunk.
//!
//! # Backpressure and durability barriers
//!
//! [`MAX_OUTSTANDING_RECORDS`] bounds recording-wide queue-node overhead while
//! each configured stream byte limit bounds its encoded payload memory.
//! [`StateWriterWorker::submit_record`] blocks until both permits allow admission.
//! [`StateWriterWorker::flush_state_stream`] inserts an ordered barrier into the same
//! FIFO and blocks until all earlier work and that stream's current non-empty
//! chunk have been durably sealed.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};

use serde::Deserialize;
use serde_json::value::RawValue;
use sha2::{Digest, Sha256};

use super::RecordingManifest;
use super::error::StorageError;
use super::jsonl_format::{
    ChunkMetadata, EncodedStateRecord, StateStreamMetadata, chunk_filename, chunk_temp_filename,
};

/// Maximum accepted records not yet appended by one stream worker.
///
/// This internal general-purpose bound prevents tiny records from creating an
/// unbounded number of queue nodes. End users configure only the scientifically
/// meaningful encoded-byte budget.
pub(crate) const MAX_OUTSTANDING_RECORDS: usize = 1_024;

/// Immutable configuration shared by new and resumed writer construction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StateStreamStorageConfig {
    stream: String,
    directory: PathBuf,
    max_chunk_bytes: NonZeroU64,
    queue_bytes: NonZeroU64,
}

impl StateStreamStorageConfig {
    /// Validates stable identity/path facts without touching the filesystem.
    pub(crate) fn new(
        stream: impl Into<String>,
        directory: impl Into<PathBuf>,
        max_chunk_bytes: NonZeroU64,
        queue_bytes: NonZeroU64,
    ) -> Result<Self, StorageError> {
        let stream = stream.into();
        if stream.trim().is_empty() {
            return Err(StorageError::InvalidConfiguration {
                setting: "stream",
                reason: "stream name must not be empty".to_owned(),
            });
        }
        let directory = directory.into();
        if directory.as_os_str().is_empty() {
            return Err(StorageError::InvalidConfiguration {
                setting: "directory",
                reason: "stream output directory must not be empty".to_owned(),
            });
        }
        Ok(Self {
            stream,
            directory,
            max_chunk_bytes,
            queue_bytes,
        })
    }

    /// Creates the absent directory required by a new run.
    pub(crate) fn create_directory(&self) -> Result<(), StorageError> {
        create_output_directory(&self.directory)
    }
}

/// Recovered worker position and optional active payload.
///
/// The type owns the open file handle when an unprepared chunk survived. It is
/// intentionally opaque outside this module so callers cannot append around
/// writer ordering or publish a chunk without the manifest transaction.
pub(crate) struct RecoveredStateStream {
    next_ordinal: u64,
    last_iteration: Option<u64>,
    active: Option<ActiveChunk>,
    latest_open_record: Option<RecoveredUnsealedRecord>,
}

impl RecoveredStateStream {
    /// Creates the initial position for an empty new stream.
    fn empty() -> Self {
        Self {
            next_ordinal: 0,
            last_iteration: None,
            active: None,
            latest_open_record: None,
        }
    }

    /// Borrows the final complete record recovered from an unprepared chunk.
    pub(crate) fn latest_open_record(&self) -> Option<&RecoveredUnsealedRecord> {
        self.latest_open_record.as_ref()
    }

    /// Returns the newest recovered iteration, if the stream is nonempty.
    pub(crate) fn last_iteration(&self) -> Option<u64> {
        self.last_iteration
    }
}

/// File range of the latest complete record in a recovered open payload.
///
/// The locator avoids copying potentially large encoded state bytes out of the
/// recovery scan buffer. Checkpoint reconstruction seeks to this range and
/// allocates decoder input exactly once.
pub(crate) struct RecoveredUnsealedRecord {
    path: PathBuf,
    offset: u64,
    bytes: u64,
}

impl RecoveredUnsealedRecord {
    /// Borrows the open payload path for reading and contextual errors.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the encoded JSON object's byte offset.
    pub(crate) fn offset(&self) -> u64 {
        self.offset
    }

    /// Returns its byte length without the framing newline.
    pub(crate) fn bytes(&self) -> u64 {
        self.bytes
    }
}

/// Exclusive asynchronous writer worker for every stream in one recording.
pub(crate) struct StateWriterWorker {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl StateWriterWorker {
    /// Starts every empty stream after recording metadata is durable.
    pub(crate) fn start_new_recording(
        configs: Vec<StateStreamStorageConfig>,
        manifest: Arc<RecordingManifest>,
    ) -> Result<Self, StorageError> {
        let streams = configs
            .into_iter()
            .map(|config| (config, RecoveredStateStream::empty()))
            .collect();
        Self::spawn(streams, manifest)
    }

    /// Starts every stream from its already recovered position.
    pub(crate) fn continue_recovered_recording(
        streams: Vec<(StateStreamStorageConfig, RecoveredStateStream)>,
        manifest: Arc<RecordingManifest>,
    ) -> Result<Self, StorageError> {
        Self::spawn(streams, manifest)
    }

    /// Reconciles filenames and reconstructs only the highest open chunk.
    ///
    /// Sealed files are checked by name and ordinal only. Their contents,
    /// lengths, and checksums are deliberately not opened or recomputed.
    pub(crate) fn recover_state_stream(
        config: &StateStreamStorageConfig,
        declaration: &StateStreamMetadata,
    ) -> Result<RecoveredStateStream, StorageError> {
        recover_stream(config, declaration)
    }

    /// Admits one complete record to the recording-wide FIFO queue.
    pub(crate) fn submit_record(
        &self,
        stream: &str,
        record: EncodedStateRecord,
    ) -> Result<(), StorageError> {
        let record_bytes =
            u64::try_from(record.len()).map_err(|_| StorageError::ByteCountOverflow {
                stream: stream.to_owned(),
            })?;
        let mut state = lock_state(&self.shared);
        let queue_bytes = state
            .streams
            .get(stream)
            .ok_or_else(|| StorageError::UnknownStateStream {
                stream: stream.to_owned(),
            })?
            .queue_bytes;
        if record_bytes > queue_bytes {
            return Err(StorageError::RecordTooLarge {
                stream: stream.to_owned(),
                bytes: record_bytes,
                limit: queue_bytes,
            });
        }

        let iteration = record.simulation_time().iteration();
        loop {
            ensure_accepting(&state)?;
            let stream_state = state
                .streams
                .get(stream)
                .expect("validated stream remains registered");
            if let Some(previous) = stream_state.last_accepted_iteration
                && iteration <= previous
            {
                return Err(StorageError::OutOfOrderIteration {
                    stream: stream.to_owned(),
                    iteration,
                    previous,
                });
            }

            let bytes_fit = stream_state
                .outstanding_bytes
                .checked_add(record_bytes)
                .is_some_and(|total| total <= queue_bytes);
            if state.outstanding_records < MAX_OUTSTANDING_RECORDS && bytes_fit {
                state.outstanding_records += 1;
                let stream_state = state
                    .streams
                    .get_mut(stream)
                    .expect("validated stream remains registered");
                stream_state.outstanding_bytes += record_bytes;
                stream_state.last_accepted_iteration = Some(iteration);
                state.queue.push_back(Work::Record {
                    stream: stream.to_owned(),
                    record,
                    bytes: record_bytes,
                });
                self.shared.work_ready.notify_one();
                return Ok(());
            }
            state = wait_for_change(&self.shared, state);
        }
    }

    /// Blocks until all earlier records are durably sealed.
    ///
    /// The barrier is ordered in the same FIFO as records. It seals a non-empty
    /// active chunk even below the byte target and commits its descriptor before
    /// acknowledgement. An empty stream or a barrier immediately following a
    /// previous barrier performs no filesystem write but is still acknowledged.
    pub(crate) fn flush_state_stream(&self, stream: &str) -> Result<(), StorageError> {
        let mut state = lock_state(&self.shared);
        ensure_accepting(&state)?;
        if !state.streams.contains_key(stream) {
            return Err(StorageError::UnknownStateStream {
                stream: stream.to_owned(),
            });
        }
        let id =
            state
                .next_flush_id
                .checked_add(1)
                .ok_or_else(|| StorageError::ByteCountOverflow {
                    stream: stream.to_owned(),
                })?;
        state.next_flush_id = id;
        state.queue.push_back(Work::Flush {
            stream: stream.to_owned(),
            id,
        });
        self.shared.work_ready.notify_one();

        while state.completed_flush_id < id {
            if let Some(source) = &state.terminal {
                return Err(terminated(source));
            }
            state = wait_for_change(&self.shared, state);
        }
        Ok(())
    }

    /// Closes admission, drains accepted work, seals the tail, and joins.
    pub(crate) fn finish_recording(mut self) -> Result<(), StorageError> {
        self.close_admission();
        self.join_worker()?;
        let state = lock_state(&self.shared);
        if let Some(source) = &state.terminal {
            return Err(terminated(source));
        }
        if !state.finished {
            return Err(StorageError::WriterQueueDisconnected);
        }
        Ok(())
    }

    /// Creates one shared queue and transfers every stream sink into one worker.
    fn spawn(
        streams: Vec<(StateStreamStorageConfig, RecoveredStateStream)>,
        manifest: Arc<RecordingManifest>,
    ) -> Result<Self, StorageError> {
        let mut queue_streams = HashMap::with_capacity(streams.len());
        let mut sinks = BTreeMap::new();
        let mut error_path = None;
        for (config, mut recovered) in streams {
            error_path.get_or_insert_with(|| config.directory.clone());
            queue_streams.insert(
                config.stream.clone(),
                StreamQueueState {
                    queue_bytes: config.queue_bytes.get(),
                    outstanding_bytes: 0,
                    last_accepted_iteration: recovered.last_iteration,
                },
            );
            drop(recovered.latest_open_record.take());
            let sink = StateStreamSink::new(config, recovered);
            let replaced = sinks.insert(sink.stream.clone(), sink);
            debug_assert!(replaced.is_none());
        }
        let shared = Arc::new(Shared::new(queue_streams));
        let worker_shared = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name("scientific-workflow-state-writer".to_owned())
            .spawn(move || {
                worker_main(sinks, &manifest, &worker_shared);
            })
            .map_err(|source| StorageError::Io {
                operation: "start writer worker",
                path: error_path.unwrap_or_default(),
                source,
            })?;

        Ok(Self {
            shared,
            worker: Some(worker),
        })
    }

    /// Marks admission closed and wakes the worker and blocked callers.
    fn close_admission(&self) {
        let mut state = lock_state(&self.shared);
        state.accepting = false;
        self.shared.work_ready.notify_all();
        self.shared.changed.notify_all();
    }

    /// Joins the worker once and gives panics stable stream context.
    fn join_worker(&mut self) -> Result<(), StorageError> {
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        worker.join().map_err(|_| StorageError::StateWriterPanicked)
    }
}

impl Drop for StateWriterWorker {
    /// Prevents detached workers and closes every owned open file normally.
    fn drop(&mut self) {
        self.close_admission();
        let _ = self.join_worker();
    }
}

/// One queue item; barriers share record admission order without byte charge.
enum Work {
    Record {
        stream: String,
        record: EncodedStateRecord,
        bytes: u64,
    },
    Flush {
        stream: String,
        id: u64,
    },
}

/// Shared queue state guarded by one mutex and two condition variables.
struct Shared {
    state: Mutex<QueueState>,
    work_ready: Condvar,
    changed: Condvar,
}

impl Shared {
    /// Creates an open queue seeded with the recovered final accepted iteration.
    fn new(streams: HashMap<String, StreamQueueState>) -> Self {
        Self {
            state: Mutex::new(QueueState {
                queue: VecDeque::new(),
                outstanding_records: 0,
                streams,
                next_flush_id: 0,
                completed_flush_id: 0,
                accepting: true,
                terminal: None,
                finished: false,
            }),
            work_ready: Condvar::new(),
            changed: Condvar::new(),
        }
    }
}

/// Mutable coordination facts protected by [`Shared::state`].
struct QueueState {
    queue: VecDeque<Work>,
    outstanding_records: usize,
    streams: HashMap<String, StreamQueueState>,
    next_flush_id: u64,
    completed_flush_id: u64,
    accepting: bool,
    terminal: Option<Arc<StorageError>>,
    finished: bool,
}

/// Per-stream admission facts protected by the recording-wide queue mutex.
struct StreamQueueState {
    queue_bytes: u64,
    outstanding_bytes: u64,
    last_accepted_iteration: Option<u64>,
}

/// Worker-owned persistence state for one named stream.
struct StateStreamSink {
    stream: String,
    directory: PathBuf,
    max_chunk_bytes: u64,
    next_ordinal: u64,
    active: Option<ActiveChunk>,
}

impl StateStreamSink {
    /// Combines immutable configuration with recovered append position.
    fn new(config: StateStreamStorageConfig, recovered: RecoveredStateStream) -> Self {
        Self {
            stream: config.stream,
            directory: config.directory,
            max_chunk_bytes: config.max_chunk_bytes.get(),
            next_ordinal: recovered.next_ordinal,
            active: recovered.active,
        }
    }

    /// Appends one indivisible record, rolling over only between records.
    fn append(
        &mut self,
        record: &EncodedStateRecord,
        manifest: &RecordingManifest,
    ) -> Result<(), StorageError> {
        let record_bytes =
            u64::try_from(record.len()).map_err(|_| StorageError::ByteCountOverflow {
                stream: self.stream.clone(),
            })?;
        if self.active.as_ref().is_some_and(|chunk| {
            chunk.records > 0
                && chunk
                    .bytes
                    .checked_add(record_bytes)
                    .is_none_or(|bytes| bytes > self.max_chunk_bytes)
        }) {
            self.flush(manifest)?;
        }
        if self.active.is_none() {
            self.active = Some(ActiveChunk::create(
                &self.directory,
                self.next_ordinal,
                record.simulation_time().iteration(),
            )?);
            self.next_ordinal = self.next_ordinal.checked_add(1).ok_or_else(|| {
                StorageError::ByteCountOverflow {
                    stream: self.stream.clone(),
                }
            })?;
        }
        self.active
            .as_mut()
            .expect("active chunk was initialized")
            .append(&self.stream, record)?;
        if self
            .active
            .as_ref()
            .is_some_and(|chunk| chunk.bytes >= self.max_chunk_bytes)
        {
            self.flush(manifest)?;
        }
        Ok(())
    }

    /// Durably seals the current non-empty chunk, if one exists.
    fn flush(&mut self, manifest: &RecordingManifest) -> Result<(), StorageError> {
        seal_active(&mut self.active, &self.stream, manifest)
    }
}

/// Worker-owned open payload and its incremental descriptor facts.
struct ActiveChunk {
    ordinal: u64,
    temporary_path: PathBuf,
    final_path: PathBuf,
    file: File,
    hasher: Sha256,
    records: u64,
    bytes: u64,
    first_iteration: u64,
    last_iteration: u64,
}

impl ActiveChunk {
    /// Exclusively creates one new open payload under its lifecycle name.
    fn create(directory: &Path, ordinal: u64, iteration: u64) -> Result<Self, StorageError> {
        let final_path = directory.join(chunk_filename(ordinal));
        if final_path.exists() {
            return Err(StorageError::RecordingDirectoryExists { path: final_path });
        }
        let temporary_path = directory.join(chunk_temp_filename(ordinal));
        let file = OpenOptions::new()
            .append(true)
            .create_new(true)
            .open(&temporary_path)
            .map_err(|source| StorageError::Io {
                operation: "create open chunk",
                path: temporary_path.clone(),
                source,
            })?;
        Ok(Self {
            ordinal,
            temporary_path,
            final_path,
            file,
            hasher: Sha256::new(),
            records: 0,
            bytes: 0,
            first_iteration: iteration,
            last_iteration: iteration,
        })
    }

    /// Reopens the valid complete prefix found by recovery without copying it.
    fn recovered(
        directory: &Path,
        ordinal: u64,
        hasher: Sha256,
        records: u64,
        bytes: u64,
        first_iteration: u64,
        last_iteration: u64,
    ) -> Result<Self, StorageError> {
        let temporary_path = directory.join(chunk_temp_filename(ordinal));
        let file = OpenOptions::new()
            .append(true)
            .open(&temporary_path)
            .map_err(|source| StorageError::Io {
                operation: "reopen recovered chunk",
                path: temporary_path.clone(),
                source,
            })?;
        Ok(Self {
            ordinal,
            temporary_path,
            final_path: directory.join(chunk_filename(ordinal)),
            file,
            hasher,
            records,
            bytes,
            first_iteration,
            last_iteration,
        })
    }

    /// Appends one indivisible record and advances descriptor facts.
    fn append(&mut self, stream: &str, record: &EncodedStateRecord) -> Result<(), StorageError> {
        self.file
            .write_all(record.bytes())
            .map_err(|source| StorageError::Io {
                operation: "append chunk",
                path: self.temporary_path.clone(),
                source,
            })?;
        self.hasher.update(record.bytes());
        let record_bytes =
            u64::try_from(record.len()).map_err(|_| StorageError::ByteCountOverflow {
                stream: stream.to_owned(),
            })?;
        self.bytes = self.bytes.checked_add(record_bytes).ok_or_else(|| {
            StorageError::ByteCountOverflow {
                stream: stream.to_owned(),
            }
        })?;
        self.records =
            self.records
                .checked_add(1)
                .ok_or_else(|| StorageError::ByteCountOverflow {
                    stream: stream.to_owned(),
                })?;
        self.last_iteration = record.simulation_time().iteration();
        Ok(())
    }

    /// Builds the authoritative descriptor without consuming the open owner.
    fn descriptor(&self) -> ChunkMetadata {
        let digest = lowercase_hex(&self.hasher.clone().finalize());
        ChunkMetadata {
            ordinal: self.ordinal,
            file: chunk_filename(self.ordinal),
            records: self.records,
            bytes: self.bytes,
            checksum: format!("sha256:{digest}"),
            first_iteration: self.first_iteration,
            last_iteration: self.last_iteration,
        }
    }

    /// Prepares metadata and then atomically changes the seal filename.
    fn seal(self, stream: &str, manifest: &RecordingManifest) -> Result<(), StorageError> {
        self.file.sync_all().map_err(|source| StorageError::Io {
            operation: "synchronize open chunk",
            path: self.temporary_path.clone(),
            source,
        })?;
        let directory = self
            .temporary_path
            .parent()
            .expect("every chunk has its configured stream directory");
        sync_directory(directory, "synchronize open chunk directory entry")?;
        let descriptor = self.descriptor();
        manifest.prepare_chunk(stream, descriptor)?;
        drop(self.file);
        fs::rename(&self.temporary_path, &self.final_path).map_err(|source| StorageError::Io {
            operation: "seal chunk",
            path: self.final_path.clone(),
            source,
        })?;
        sync_directory(directory, "synchronize sealed chunk filename")
    }

    /// Completes a crash-interrupted rename for an already prepared descriptor.
    fn finish_prepared(self) -> Result<(), StorageError> {
        drop(self.file);
        fs::rename(&self.temporary_path, &self.final_path).map_err(|source| StorageError::Io {
            operation: "finish prepared chunk seal",
            path: self.final_path.clone(),
            source,
        })?;
        let directory = self
            .final_path
            .parent()
            .expect("every chunk has its configured stream directory");
        sync_directory(directory, "synchronize recovered sealed filename")
    }
}

/// Minimal borrowed envelope used while scanning one open JSONL prefix.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryRecord<'a> {
    iteration: u64,
    #[serde(default)]
    physical_time: Option<f64>,
    #[serde(borrow)]
    values: &'a RawValue,
}

/// Directory inventory for one deterministic ordinal.
#[derive(Default)]
struct ChunkNames {
    sealed: bool,
    open: bool,
}

/// Scanned complete prefix of one open payload.
struct OpenScan {
    active: Option<ActiveChunk>,
    latest_record: Option<RecoveredUnsealedRecord>,
}

/// Runs the append/barrier loop and publishes one terminal outcome.
fn worker_main(
    mut streams: BTreeMap<String, StateStreamSink>,
    manifest: &RecordingManifest,
    shared: &Shared,
) {
    let result = write_records(&mut streams, manifest, shared);
    let mut state = lock_state(shared);
    match result {
        Ok(()) => state.finished = true,
        Err(error) => {
            state.terminal = Some(Arc::new(error));
            state.accepting = false;
            state.queue.clear();
            state.outstanding_records = 0;
            for stream in state.streams.values_mut() {
                stream.outstanding_bytes = 0;
            }
        }
    }
    shared.changed.notify_all();
    shared.work_ready.notify_all();
}

/// Drains ordered work while applying rollover and explicit seal barriers.
fn write_records(
    streams: &mut BTreeMap<String, StateStreamSink>,
    manifest: &RecordingManifest,
    shared: &Shared,
) -> Result<(), StorageError> {
    for stream in streams.values_mut() {
        if stream
            .active
            .as_ref()
            .is_some_and(|chunk| chunk.bytes >= stream.max_chunk_bytes)
        {
            stream.flush(manifest)?;
        }
    }
    while let Some(work) = next_work(shared) {
        match work {
            Work::Record {
                stream,
                record,
                bytes,
            } => {
                streams
                    .get_mut(&stream)
                    .expect("queued record names a registered stream")
                    .append(&record, manifest)?;
                drop(record);
                release_capacity(shared, &stream, bytes);
            }
            Work::Flush { stream, id } => {
                streams
                    .get_mut(&stream)
                    .expect("queued flush names a registered stream")
                    .flush(manifest)?;
                complete_flush(shared, id);
            }
        }
    }
    for stream in streams.values_mut() {
        stream.flush(manifest)?;
    }
    Ok(())
}

/// Removes the next FIFO item or ends after closed admission drains.
fn next_work(shared: &Shared) -> Option<Work> {
    let mut state = lock_state(shared);
    loop {
        if let Some(work) = state.queue.pop_front() {
            return Some(work);
        }
        if !state.accepting {
            return None;
        }
        state = shared
            .work_ready
            .wait(state)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
    }
}

/// Releases exact queue capacity after the owned record allocation is dropped.
fn release_capacity(shared: &Shared, stream: &str, record_bytes: u64) {
    let mut state = lock_state(shared);
    state.outstanding_records -= 1;
    state
        .streams
        .get_mut(stream)
        .expect("completed record names a registered stream")
        .outstanding_bytes -= record_bytes;
    shared.changed.notify_all();
}

/// Acknowledges one ordered durability barrier.
fn complete_flush(shared: &Shared, id: u64) {
    let mut state = lock_state(shared);
    state.completed_flush_id = id;
    shared.changed.notify_all();
}

/// Seals a present chunk through the shared manifest transaction.
fn seal_active(
    active: &mut Option<ActiveChunk>,
    stream: &str,
    manifest: &RecordingManifest,
) -> Result<(), StorageError> {
    if let Some(chunk) = active.take() {
        chunk.seal(stream, manifest)?;
    }
    Ok(())
}

/// Reconciles one stream directory without inspecting sealed contents.
fn recover_stream(
    config: &StateStreamStorageConfig,
    declaration: &StateStreamMetadata,
) -> Result<RecoveredStateStream, StorageError> {
    let inventory = inventory_directory(&config.directory)?;
    let open_chunks = inventory.values().filter(|names| names.open).count();
    if open_chunks > 1 {
        return Err(StorageError::RecoveryConflict {
            path: config.directory.clone(),
            reason: format!(
                "found {open_chunks} unsealed chunks; a stream may have only its highest chunk open"
            ),
        });
    }
    let committed =
        u64::try_from(declaration.chunks.len()).map_err(|_| StorageError::ByteCountOverflow {
            stream: config.stream.clone(),
        })?;

    for descriptor in &declaration.chunks {
        let names =
            inventory
                .get(&descriptor.ordinal)
                .ok_or_else(|| StorageError::RecoveryConflict {
                    path: config.directory.clone(),
                    reason: format!(
                        "metadata declares chunk {}, but neither lifecycle filename exists",
                        descriptor.ordinal
                    ),
                })?;
        let is_last = descriptor.ordinal + 1 == committed;
        if names.sealed && !names.open {
            continue;
        }
        if is_last && names.open && !names.sealed {
            let prior = descriptor
                .ordinal
                .checked_sub(1)
                .and_then(|ordinal| declaration.chunks.get(ordinal as usize))
                .map(|chunk| chunk.last_iteration);
            let scan = scan_open_chunk(config, descriptor.ordinal, prior)?;
            let active = scan.active.ok_or_else(|| StorageError::RecoveryConflict {
                path: config
                    .directory
                    .join(chunk_temp_filename(descriptor.ordinal)),
                reason: "prepared chunk contains no complete record".to_owned(),
            })?;
            if active.descriptor() != *descriptor {
                return Err(StorageError::RecoveryConflict {
                    path: active.temporary_path.clone(),
                    reason: "prepared chunk bytes do not match metadata descriptor".to_owned(),
                });
            }
            active.finish_prepared()?;
            continue;
        }
        return Err(StorageError::RecoveryConflict {
            path: config.directory.clone(),
            reason: format!(
                "chunk {} has conflicting open/sealed lifecycle names",
                descriptor.ordinal
            ),
        });
    }

    for (&ordinal, names) in &inventory {
        if ordinal < committed {
            continue;
        }
        if ordinal > committed || names.sealed || !names.open {
            return Err(StorageError::RecoveryConflict {
                path: config.directory.clone(),
                reason: format!(
                    "unexpected chunk lifecycle entry at ordinal {ordinal}; next open ordinal is {committed}"
                ),
            });
        }
    }

    let previous = declaration.chunks.last().map(|chunk| chunk.last_iteration);
    let Some(names) = inventory.get(&committed) else {
        return Ok(RecoveredStateStream {
            next_ordinal: committed,
            last_iteration: previous,
            active: None,
            latest_open_record: None,
        });
    };
    if !names.open || names.sealed {
        return Err(StorageError::RecoveryConflict {
            path: config.directory.clone(),
            reason: format!("ordinal {committed} is not one unsealed chunk"),
        });
    }

    let scan = scan_open_chunk(config, committed, previous)?;
    let Some(active) = scan.active else {
        let path = config.directory.join(chunk_temp_filename(committed));
        fs::remove_file(&path).map_err(|source| StorageError::Io {
            operation: "remove empty recovered chunk",
            path: path.clone(),
            source,
        })?;
        sync_directory(&config.directory, "synchronize empty chunk removal")?;
        return Ok(RecoveredStateStream {
            next_ordinal: committed,
            last_iteration: previous,
            active: None,
            latest_open_record: None,
        });
    };
    let next_ordinal = committed
        .checked_add(1)
        .ok_or_else(|| StorageError::ByteCountOverflow {
            stream: config.stream.clone(),
        })?;
    let last_iteration = Some(active.last_iteration);
    Ok(RecoveredStateStream {
        next_ordinal,
        last_iteration,
        active: Some(active),
        latest_open_record: scan.latest_record,
    })
}

/// Reads names only and rejects every non-format artifact in a stream folder.
fn inventory_directory(path: &Path) -> Result<BTreeMap<u64, ChunkNames>, StorageError> {
    let entries = fs::read_dir(path).map_err(|source| StorageError::Io {
        operation: "inspect stream directory",
        path: path.to_path_buf(),
        source,
    })?;
    let mut inventory = BTreeMap::<u64, ChunkNames>::new();
    for entry in entries {
        let entry = entry.map_err(|source| StorageError::Io {
            operation: "inspect stream directory entry",
            path: path.to_path_buf(),
            source,
        })?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| StorageError::RecoveryConflict {
                path: entry.path(),
                reason: "chunk filename is not valid UTF-8".to_owned(),
            })?;
        let (ordinal, open) =
            parse_chunk_name(name).ok_or_else(|| StorageError::RecoveryConflict {
                path: entry.path(),
                reason: "unexpected artifact in stream directory".to_owned(),
            })?;
        let names = inventory.entry(ordinal).or_default();
        if open {
            names.open = true;
        } else {
            names.sealed = true;
        }
    }
    Ok(inventory)
}

/// Parses only names exactly reproducible by the deterministic format helpers.
fn parse_chunk_name(name: &str) -> Option<(u64, bool)> {
    let (candidate, open) = match name.strip_suffix(".tmp") {
        Some(candidate) => (candidate, true),
        None => (name, false),
    };
    let digits = candidate.strip_prefix("chunk-")?.strip_suffix(".jsonl")?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let ordinal = digits.parse().ok()?;
    let expected = if open {
        chunk_temp_filename(ordinal)
    } else {
        chunk_filename(ordinal)
    };
    (name == expected).then_some((ordinal, open))
}

/// Scans, validates, and reopens the complete prefix of the sole open chunk.
fn scan_open_chunk(
    config: &StateStreamStorageConfig,
    ordinal: u64,
    previous_iteration: Option<u64>,
) -> Result<OpenScan, StorageError> {
    let path = config.directory.join(chunk_temp_filename(ordinal));
    let file = File::open(&path).map_err(|source| StorageError::Io {
        operation: "open recoverable chunk",
        path: path.clone(),
        source,
    })?;
    let mut input = BufReader::new(file);
    let mut line = Vec::new();
    let mut line_number = 0_u64;
    let mut valid_bytes = 0_u64;
    let mut records = 0_u64;
    let mut first_iteration = None;
    let mut last_iteration = previous_iteration;
    let mut latest_record = None;
    let mut hasher = Sha256::new();

    loop {
        line.clear();
        let read = input
            .read_until(b'\n', &mut line)
            .map_err(|source| StorageError::Io {
                operation: "scan recoverable chunk",
                path: path.clone(),
                source,
            })?;
        if read == 0 {
            break;
        }
        if line.last() != Some(&b'\n') {
            break;
        }
        line_number =
            line_number
                .checked_add(1)
                .ok_or_else(|| StorageError::ByteCountOverflow {
                    stream: config.stream.clone(),
                })?;
        let record: RecoveryRecord<'_> =
            serde_json::from_slice(&line[..line.len() - 1]).map_err(|source| {
                StorageError::Json {
                    operation: "parse recoverable record",
                    path: path.clone(),
                    source,
                }
            })?;
        if record
            .physical_time
            .is_some_and(|physical_time| !physical_time.is_finite())
        {
            return Err(StorageError::InvalidRecord {
                path: path.clone(),
                line: line_number,
                reason: "physical time must be finite".to_owned(),
            });
        }
        let values = record.values.get().trim();
        if !(values.starts_with('[') && values.ends_with(']')) {
            return Err(StorageError::InvalidRecord {
                path: path.clone(),
                line: line_number,
                reason: "record values must be a JSON array".to_owned(),
            });
        }
        if let Some(previous) = last_iteration
            && record.iteration <= previous
        {
            return Err(StorageError::InvalidRecord {
                path: path.clone(),
                line: line_number,
                reason: format!(
                    "iteration {} is not greater than previous iteration {previous}",
                    record.iteration
                ),
            });
        }
        first_iteration.get_or_insert(record.iteration);
        last_iteration = Some(record.iteration);
        records = records
            .checked_add(1)
            .ok_or_else(|| StorageError::ByteCountOverflow {
                stream: config.stream.clone(),
            })?;
        let line_bytes =
            u64::try_from(line.len()).map_err(|_| StorageError::ByteCountOverflow {
                stream: config.stream.clone(),
            })?;
        let record_offset = valid_bytes;
        valid_bytes =
            valid_bytes
                .checked_add(line_bytes)
                .ok_or_else(|| StorageError::ByteCountOverflow {
                    stream: config.stream.clone(),
                })?;
        hasher.update(&line);
        latest_record = Some(RecoveredUnsealedRecord {
            path: path.clone(),
            offset: record_offset,
            bytes: line_bytes - 1,
        });
    }
    drop(input);

    let writable = OpenOptions::new()
        .write(true)
        .open(&path)
        .map_err(|source| StorageError::Io {
            operation: "truncate recoverable chunk",
            path: path.clone(),
            source,
        })?;
    writable
        .set_len(valid_bytes)
        .map_err(|source| StorageError::Io {
            operation: "truncate incomplete record",
            path: path.clone(),
            source,
        })?;
    drop(writable);

    let Some(first_iteration) = first_iteration else {
        return Ok(OpenScan {
            active: None,
            latest_record: None,
        });
    };
    let last_iteration = last_iteration.expect("a recovered first record has a final iteration");
    let active = ActiveChunk::recovered(
        &config.directory,
        ordinal,
        hasher,
        records,
        valid_bytes,
        first_iteration,
        last_iteration,
    )?;
    Ok(OpenScan {
        active: Some(active),
        latest_record,
    })
}

/// Creates a stream directory while distinguishing pre-existing output.
fn create_output_directory(path: &Path) -> Result<(), StorageError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| StorageError::Io {
            operation: "create stream parent directories",
            path: parent.to_path_buf(),
            source,
        })?;
    }
    match fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(StorageError::RecordingDirectoryExists {
                path: path.to_path_buf(),
            })
        }
        Err(source) => Err(StorageError::Io {
            operation: "create stream directory",
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Synchronizes a directory entry transition with stable error context.
fn sync_directory(path: &Path, operation: &'static str) -> Result<(), StorageError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| StorageError::Io {
            operation,
            path: path.to_path_buf(),
            source,
        })
}

/// Validates queue lifecycle while retaining authoritative worker failures.
fn ensure_accepting(state: &QueueState) -> Result<(), StorageError> {
    if let Some(source) = &state.terminal {
        return Err(terminated(source));
    }
    if !state.accepting {
        return Err(StorageError::StateWriterClosed);
    }
    Ok(())
}

/// Locks shared state while recovering from a participant panic.
fn lock_state(shared: &Shared) -> MutexGuard<'_, QueueState> {
    shared
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Waits for queue capacity, a barrier acknowledgement, or failure.
fn wait_for_change<'a>(
    shared: &'a Shared,
    state: MutexGuard<'a, QueueState>,
) -> MutexGuard<'a, QueueState> {
    shared
        .changed
        .wait(state)
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Wraps one authoritative terminal failure for a particular stream.
fn terminated(source: &Arc<StorageError>) -> StorageError {
    StorageError::StateWriterTerminated {
        source: Arc::clone(source),
    }
}

/// Encodes digest bytes without an intermediate byte collection.
fn lowercase_hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}
