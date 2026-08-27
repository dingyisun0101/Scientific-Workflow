//! Bounded asynchronous persistence for one recording.
//!
//! [`StateWriterWorker`] accepts complete [`EncodedStateRecord`] values, applies strict
//! byte/count backpressure, and moves every stream through one FIFO worker.
//! Each private stream sink owns one reusable userspace byte buffer. Records
//! cause no filesystem operation until the buffer reaches its chunk target.
//! The worker then writes the complete payload once, prepares its descriptor
//! through the recording-level [`RecordingManifest`], and atomically renames
//! it into its immutable final name.
//!
//! # Filename lifecycle
//!
//! `chunk-NNNNNN.jsonl.tmp` exists only during whole-chunk publication. After
//! its bytes and directory entry are synchronized, its descriptor is committed to the sole
//! `metadata.json`; only then is it renamed to `chunk-NNNNNN.jsonl`. The final
//! name is the ordinary seal marker. The metadata inventory remains authoritative
//! about which chunks belong to the recording.
//!
//! # Backpressure and durability
//!
//! [`MAX_OUTSTANDING_RECORDS`] bounds recording-wide queue-node overhead while
//! each configured stream byte limit bounds its encoded payload memory.
//! [`StateWriterWorker::submit_record`] blocks until both permits allow admission.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};

use sha2::{Digest, Sha256};

use super::error::PersistenceError;
use super::jsonl_format::{ChunkMetadata, EncodedStateRecord, chunk_filename, chunk_temp_filename};
use super::{RecordingManifest, StateStreamLayout, StateStreamStorage};

/// Maximum accepted records not yet appended by one stream worker.
///
/// This internal general-purpose bound prevents tiny records from creating an
/// unbounded number of queue nodes. End users configure only the scientifically
/// meaningful encoded-byte budget.
pub(crate) const MAX_OUTSTANDING_RECORDS: usize = 1_024;

/// Immutable configuration used while starting a new stream writer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StateStreamStorageConfig {
    stream: String,
    directory: PathBuf,
    storage: StateStreamStorage,
}

impl StateStreamStorageConfig {
    /// Validates stable identity/path facts without touching the filesystem.
    pub(crate) fn new(
        stream: impl Into<String>,
        directory: impl Into<PathBuf>,
        storage: StateStreamStorage,
    ) -> Result<Self, PersistenceError> {
        let stream = stream.into();
        if stream.trim().is_empty() {
            return Err(PersistenceError::InvalidConfiguration {
                setting: "stream",
                reason: "stream name must not be empty".to_owned(),
            });
        }
        let directory = directory.into();
        if directory.as_os_str().is_empty() {
            return Err(PersistenceError::InvalidConfiguration {
                setting: "directory",
                reason: "stream output directory must not be empty".to_owned(),
            });
        }
        Ok(Self {
            stream,
            directory,
            storage,
        })
    }

    /// Creates the absent directory required by a new run.
    pub(crate) fn create_directory(&self) -> Result<(), PersistenceError> {
        create_output_directory(&self.directory)
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
    ) -> Result<Self, PersistenceError> {
        Self::spawn(configs, manifest)
    }

    /// Admits one complete record to the recording-wide FIFO queue.
    pub(crate) fn submit_record(
        &self,
        stream: &str,
        record: EncodedStateRecord,
    ) -> Result<(), PersistenceError> {
        let record_bytes =
            u64::try_from(record.len()).map_err(|_| PersistenceError::ByteCountOverflow {
                stream: stream.to_owned(),
            })?;
        let mut state = lock_state(&self.shared);
        let queue_bytes = state
            .streams
            .get(stream)
            .ok_or_else(|| PersistenceError::UnknownStateStream {
                stream: stream.to_owned(),
            })?
            .queue_bytes;
        if record_bytes > queue_bytes {
            return Err(PersistenceError::RecordTooLarge {
                stream: stream.to_owned(),
                bytes: record_bytes,
                limit: queue_bytes,
            });
        }

        let iteration = record.time().iteration();
        loop {
            ensure_accepting(&state)?;
            let stream_state = state
                .streams
                .get(stream)
                .expect("validated stream remains registered");
            if let Some(previous) = stream_state.last_accepted_iteration
                && iteration <= previous
            {
                return Err(PersistenceError::OutOfOrderIteration {
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

    /// Closes admission, drains accepted work, seals the tail, and joins.
    pub(crate) fn finish_recording(mut self) -> Result<(), PersistenceError> {
        self.close_admission();
        self.join_worker()?;
        let state = lock_state(&self.shared);
        if let Some(source) = &state.terminal {
            return Err(terminated(source));
        }
        if !state.finished {
            return Err(PersistenceError::WriterQueueDisconnected);
        }
        Ok(())
    }

    /// Creates one shared queue and transfers every stream sink into one worker.
    fn spawn(
        streams: Vec<StateStreamStorageConfig>,
        manifest: Arc<RecordingManifest>,
    ) -> Result<Self, PersistenceError> {
        let mut queue_streams = HashMap::with_capacity(streams.len());
        let mut sinks = BTreeMap::new();
        let mut error_path = None;
        for config in streams {
            error_path.get_or_insert_with(|| config.directory.clone());
            queue_streams.insert(
                config.stream.clone(),
                StreamQueueState {
                    queue_bytes: config.storage.storage_queue_bytes().get(),
                    outstanding_bytes: 0,
                    last_accepted_iteration: None,
                },
            );
            let sink = StateStreamSink::new(config, 0);
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
            .map_err(|source| PersistenceError::Io {
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
    fn join_worker(&mut self) -> Result<(), PersistenceError> {
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        worker
            .join()
            .map_err(|_| PersistenceError::StateWriterPanicked)
    }
}

impl Drop for StateWriterWorker {
    /// Prevents detached workers and closes every owned open file normally.
    fn drop(&mut self) {
        self.close_admission();
        let _ = self.join_worker();
    }
}

/// One queue item carrying one complete encoded record.
enum Work {
    Record {
        stream: String,
        record: EncodedStateRecord,
        bytes: u64,
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
    accepting: bool,
    terminal: Option<Arc<PersistenceError>>,
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
    layout: StateStreamLayout,
    next_ordinal: u64,
    buffer: Vec<u8>,
    active: Option<BufferedChunkState>,
}

impl StateStreamSink {
    /// Combines immutable configuration with recovered append position.
    fn new(config: StateStreamStorageConfig, next_ordinal: u64) -> Self {
        Self {
            stream: config.stream,
            directory: config.directory,
            layout: config.storage.layout(),
            next_ordinal,
            buffer: Vec::new(),
            active: None,
        }
    }

    /// Appends one indivisible record, rolling over only between records.
    fn append(
        &mut self,
        record: &EncodedStateRecord,
        manifest: &RecordingManifest,
    ) -> Result<(), PersistenceError> {
        let record_bytes =
            u64::try_from(record.len()).map_err(|_| PersistenceError::ByteCountOverflow {
                stream: self.stream.clone(),
            })?;
        if let StateStreamLayout::Chunked { target_bytes } = self.layout
            && self.active.as_ref().is_some_and(|chunk| {
                chunk.records > 0
                    && chunk
                        .bytes
                        .checked_add(record_bytes)
                        .is_none_or(|bytes| bytes > target_bytes.get())
            })
        {
            self.flush(manifest)?;
        }
        if self.active.is_none() {
            self.active = Some(BufferedChunkState::new(
                self.next_ordinal,
                record.time().iteration(),
            ));
            self.next_ordinal = self.next_ordinal.checked_add(1).ok_or_else(|| {
                PersistenceError::ByteCountOverflow {
                    stream: self.stream.clone(),
                }
            })?;
        }
        self.buffer.extend_from_slice(record.bytes());
        self.active
            .as_mut()
            .expect("active chunk was initialized")
            .append(&self.stream, record)?;
        let should_seal = match self.layout {
            StateStreamLayout::Chunked { target_bytes } => self
                .active
                .as_ref()
                .is_some_and(|chunk| chunk.bytes >= target_bytes.get()),
            StateStreamLayout::IndividualFiles => true,
        };
        if should_seal {
            self.flush(manifest)?;
        }
        Ok(())
    }

    /// Durably seals the current non-empty chunk, if one exists.
    fn flush(&mut self, manifest: &RecordingManifest) -> Result<(), PersistenceError> {
        let Some(chunk) = self.active.take() else {
            return Ok(());
        };
        chunk.seal(&self.directory, &self.stream, &self.buffer, manifest)?;
        self.buffer.clear();
        Ok(())
    }
}

/// Descriptor state for one chunk accumulated entirely in userspace memory.
struct BufferedChunkState {
    ordinal: u64,
    hasher: Sha256,
    records: u64,
    bytes: u64,
    first_iteration: u64,
    last_iteration: u64,
}

impl BufferedChunkState {
    fn new(ordinal: u64, iteration: u64) -> Self {
        Self {
            ordinal,
            hasher: Sha256::new(),
            records: 0,
            bytes: 0,
            first_iteration: iteration,
            last_iteration: iteration,
        }
    }

    fn append(
        &mut self,
        stream: &str,
        record: &EncodedStateRecord,
    ) -> Result<(), PersistenceError> {
        self.hasher.update(record.bytes());
        let record_bytes =
            u64::try_from(record.len()).map_err(|_| PersistenceError::ByteCountOverflow {
                stream: stream.to_owned(),
            })?;
        self.bytes = self.bytes.checked_add(record_bytes).ok_or_else(|| {
            PersistenceError::ByteCountOverflow {
                stream: stream.to_owned(),
            }
        })?;
        self.records =
            self.records
                .checked_add(1)
                .ok_or_else(|| PersistenceError::ByteCountOverflow {
                    stream: stream.to_owned(),
                })?;
        self.last_iteration = record.time().iteration();
        Ok(())
    }

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

    /// Publishes one complete buffered payload with a single file write.
    fn seal(
        self,
        directory: &Path,
        stream: &str,
        bytes: &[u8],
        manifest: &RecordingManifest,
    ) -> Result<(), PersistenceError> {
        debug_assert_eq!(self.bytes, bytes.len() as u64);
        let final_path = directory.join(chunk_filename(self.ordinal));
        if final_path.exists() {
            return Err(PersistenceError::RecordingDirectoryExists { path: final_path });
        }
        let temporary_path = directory.join(chunk_temp_filename(self.ordinal));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
            .map_err(|source| PersistenceError::Io {
                operation: "create buffered chunk",
                path: temporary_path.clone(),
                source,
            })?;
        file.write_all(bytes)
            .map_err(|source| PersistenceError::Io {
                operation: "write buffered chunk",
                path: temporary_path.clone(),
                source,
            })?;
        file.sync_all().map_err(|source| PersistenceError::Io {
            operation: "synchronize buffered chunk",
            path: temporary_path.clone(),
            source,
        })?;
        sync_directory(directory, "synchronize buffered chunk directory entry")?;
        manifest.prepare_chunk(stream, self.descriptor())?;
        drop(file);
        fs::rename(&temporary_path, &final_path).map_err(|source| PersistenceError::Io {
            operation: "seal buffered chunk",
            path: final_path.clone(),
            source,
        })?;
        sync_directory(directory, "synchronize sealed chunk filename")
    }
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

/// Drains ordered work while applying byte-target rollover.
fn write_records(
    streams: &mut BTreeMap<String, StateStreamSink>,
    manifest: &RecordingManifest,
    shared: &Shared,
) -> Result<(), PersistenceError> {
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

fn create_output_directory(path: &Path) -> Result<(), PersistenceError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| PersistenceError::Io {
            operation: "create stream parent directories",
            path: parent.to_path_buf(),
            source,
        })?;
    }
    match fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(PersistenceError::RecordingDirectoryExists {
                path: path.to_path_buf(),
            })
        }
        Err(source) => Err(PersistenceError::Io {
            operation: "create stream directory",
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Synchronizes a directory entry transition with stable error context.
fn sync_directory(path: &Path, operation: &'static str) -> Result<(), PersistenceError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| PersistenceError::Io {
            operation,
            path: path.to_path_buf(),
            source,
        })
}

/// Validates queue lifecycle while retaining authoritative worker failures.
fn ensure_accepting(state: &QueueState) -> Result<(), PersistenceError> {
    if let Some(source) = &state.terminal {
        return Err(terminated(source));
    }
    if !state.accepting {
        return Err(PersistenceError::StateWriterClosed);
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
fn terminated(source: &Arc<PersistenceError>) -> PersistenceError {
    PersistenceError::StateWriterTerminated {
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
