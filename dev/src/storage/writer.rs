//! Bounded asynchronous persistence for one logical output stream.
//!
//! A [`StateWriter`] accepts only complete [`EncodedRecord`] values. It never
//! borrows a [`SystemState`](crate::system_state::SystemState), invokes Serde,
//! or interprets payload bytes. One worker thread appends records in admission
//! order, rolls immutable chunks by exact framed byte length, and returns the
//! descriptors needed for the run-level `metadata.json` transaction.
//!
//! # Backpressure
//!
//! Admission is bounded in two dimensions. [`MAX_OUTSTANDING_RECORDS`] is an
//! internal safeguard against unbounded per-record overhead, while the caller
//! selects a strict encoded-byte budget through [`WriterConfig`]. Counts cover
//! every accepted but not-yet-appended record, including the record currently
//! owned by the worker. [`StateWriter::submit`] waits on a condition variable
//! until both budgets permit admission. A record larger than the complete byte
//! budget is rejected immediately because waiting could never make it fit.
//!
//! # Chunk commits
//!
//! Records are written to a temporary file and exposed under their
//! deterministic `chunk-NNNNNN.jsonl` name only after `sync_all` and rename.
//! The chunk-size setting is a rollover target rather than a splitting rule:
//! one oversized record becomes the sole record in an oversized chunk.
//! SHA-256 is accumulated while bytes are appended, so no second payload pass
//! is needed to construct [`ChunkMetadata`].
//!
//! # Failure and shutdown
//!
//! The first worker failure becomes the authoritative terminal error. The
//! worker closes admission and wakes all blocked submitters; they receive a
//! [`StorageError::WriterTerminated`] that shares the original error through
//! `Arc`. [`StateWriter::finish`] drains accepted work, seals the final chunk,
//! joins the worker, and returns [`WriterSummary`]. Dropping an unfinished
//! writer performs the same drain-and-join lifecycle but cannot report errors.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};

use sha2::{Digest, Sha256};

use super::error::StorageError;
use super::format::{ChunkMetadata, EncodedRecord, chunk_filename};

/// Maximum number of accepted but not-yet-appended records per stream.
///
/// This is deliberately an implementation policy rather than user-facing
/// configuration or persisted metadata. A limit of 1,024 permits substantial
/// batching of very small samples while bounding queue-node and allocator
/// overhead independently of the encoded-byte budget.
pub(crate) const MAX_OUTSTANDING_RECORDS: usize = 1_024;

/// Immutable startup configuration for one stream writer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WriterConfig {
    /// Logical stream name retained in diagnostics.
    stream: String,
    /// New directory that will contain this stream's immutable chunks.
    directory: PathBuf,
    /// Soft rollover target; records remain indivisible.
    max_chunk_bytes: NonZeroU64,
    /// Strict bound over accepted but not-yet-appended record bytes.
    queue_bytes: NonZeroU64,
}

impl WriterConfig {
    /// Creates configuration without touching the filesystem.
    ///
    /// The output directory itself must not already exist when
    /// [`StateWriter::start`] is called. Non-zero integer types make both byte
    /// limits valid by construction.
    pub(crate) fn new(
        stream: impl Into<String>,
        directory: impl Into<PathBuf>,
        max_chunk_bytes: NonZeroU64,
        queue_bytes: NonZeroU64,
    ) -> Result<Self, StorageError> {
        let stream = stream.into();
        if stream.trim().is_empty() {
            return Err(StorageError::InvalidConfig {
                setting: "stream",
                reason: "stream name must not be empty".to_owned(),
            });
        }
        let directory = directory.into();
        if directory.as_os_str().is_empty() {
            return Err(StorageError::InvalidConfig {
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

    /// Returns the logical stream name.
    pub(crate) fn stream(&self) -> &str {
        &self.stream
    }

    /// Returns the stream's output directory.
    pub(crate) fn directory(&self) -> &Path {
        &self.directory
    }

    /// Returns the preferred maximum chunk size.
    pub(crate) fn max_chunk_bytes(&self) -> NonZeroU64 {
        self.max_chunk_bytes
    }

    /// Returns the strict outstanding encoded-byte limit.
    pub(crate) fn queue_bytes(&self) -> NonZeroU64 {
        self.queue_bytes
    }
}

/// Final immutable statistics and chunk inventory for one stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WriterSummary {
    stream: String,
    chunks: Vec<ChunkMetadata>,
    records: u64,
    bytes: u64,
}

impl WriterSummary {
    /// Returns the logical stream represented by this summary.
    pub(crate) fn stream(&self) -> &str {
        &self.stream
    }

    /// Returns committed chunks in increasing ordinal order.
    pub(crate) fn chunks(&self) -> &[ChunkMetadata] {
        &self.chunks
    }

    /// Returns the total number of committed records.
    pub(crate) fn records(&self) -> u64 {
        self.records
    }

    /// Returns the exact total bytes across committed chunks.
    pub(crate) fn bytes(&self) -> u64 {
        self.bytes
    }
}

/// Exclusive asynchronous writer for one logical stream.
///
/// The type is intentionally non-Clone. One instance owns admission closure,
/// its worker join handle, and the only successful finish transition.
pub(crate) struct StateWriter {
    stream: String,
    queue_bytes: u64,
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl StateWriter {
    /// Creates the stream directory and starts its dedicated worker.
    ///
    /// Existing output is never overwritten. The caller should create the run
    /// directory first, then give each stream a distinct absent child path.
    pub(crate) fn start(config: WriterConfig) -> Result<Self, StorageError> {
        create_output_directory(&config.directory)?;

        let stream = config.stream.clone();
        let queue_bytes = config.queue_bytes.get();
        let shared = Arc::new(Shared::new());
        let worker_shared = Arc::clone(&shared);
        let worker_stream = stream.clone();
        let directory = config.directory.clone();
        let spawn_error_path = config.directory;
        let max_chunk_bytes = config.max_chunk_bytes.get();
        let worker = thread::Builder::new()
            .name(format!("scientific-workflow-{stream}"))
            .spawn(move || {
                worker_main(&worker_stream, &directory, max_chunk_bytes, &worker_shared);
            })
            .map_err(|source| StorageError::Io {
                operation: "start writer worker",
                path: spawn_error_path,
                source,
            })?;

        Ok(Self {
            stream,
            queue_bytes,
            shared,
            worker: Some(worker),
        })
    }

    /// Admits one complete record, blocking while either queue budget is full.
    ///
    /// Success transfers ownership into the worker FIFO; it does not promise
    /// immediate durability. The caller retains ownership when this method
    /// rejects a record only conceptually—the error path drops the supplied
    /// encoded buffer because the API is designed as a consuming handoff.
    pub(crate) fn submit(&self, record: EncodedRecord) -> Result<(), StorageError> {
        let record_bytes =
            u64::try_from(record.len()).map_err(|_| StorageError::ByteCountOverflow {
                stream: self.stream.clone(),
            })?;
        if record_bytes > self.queue_bytes {
            return Err(StorageError::RecordTooLarge {
                stream: self.stream.clone(),
                bytes: record_bytes,
                limit: self.queue_bytes,
            });
        }

        let index = record.time().index();
        let mut state = lock_state(&self.shared);
        loop {
            if let Some(source) = &state.terminal {
                return Err(terminated(&self.stream, source));
            }
            if !state.accepting {
                return Err(StorageError::StreamFinished {
                    stream: self.stream.clone(),
                });
            }
            if let Some(previous) = state.last_accepted_index
                && index <= previous
            {
                return Err(StorageError::OutOfOrderRecord {
                    stream: self.stream.clone(),
                    index,
                    previous,
                });
            }

            let bytes_fit = state
                .outstanding_bytes
                .checked_add(record_bytes)
                .is_some_and(|total| total <= self.queue_bytes);
            if state.outstanding_records < MAX_OUTSTANDING_RECORDS && bytes_fit {
                state.outstanding_records += 1;
                state.outstanding_bytes += record_bytes;
                state.last_accepted_index = Some(index);
                state.queue.push_back(record);
                self.shared.work_ready.notify_one();
                return Ok(());
            }
            state = wait_for_capacity(&self.shared, state);
        }
    }

    /// Stops admission, drains accepted records, seals the final chunk, and
    /// joins the worker.
    pub(crate) fn finish(mut self) -> Result<WriterSummary, StorageError> {
        self.close_admission();
        self.join_worker()?;
        let mut state = lock_state(&self.shared);
        if let Some(source) = &state.terminal {
            return Err(terminated(&self.stream, source));
        }
        state
            .summary
            .take()
            .ok_or_else(|| StorageError::QueueDisconnected {
                stream: self.stream.clone(),
            })
    }

    /// Marks admission closed and wakes the worker and blocked submitters.
    fn close_admission(&self) {
        let mut state = lock_state(&self.shared);
        state.accepting = false;
        self.shared.work_ready.notify_all();
        self.shared.capacity_available.notify_all();
    }

    /// Joins the worker once and translates a panic into a stable error.
    fn join_worker(&mut self) -> Result<(), StorageError> {
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        worker.join().map_err(|_| StorageError::WriterPanicked {
            stream: self.stream.clone(),
        })
    }
}

impl Drop for StateWriter {
    /// Prevents a detached worker and incomplete temporary-file lifetime.
    fn drop(&mut self) {
        self.close_admission();
        let _ = self.join_worker();
    }
}

/// Shared queue state guarded by one mutex and two condition variables.
struct Shared {
    state: Mutex<QueueState>,
    work_ready: Condvar,
    capacity_available: Condvar,
}

impl Shared {
    /// Creates an open empty queue.
    fn new() -> Self {
        Self {
            state: Mutex::new(QueueState {
                queue: VecDeque::new(),
                outstanding_records: 0,
                outstanding_bytes: 0,
                last_accepted_index: None,
                accepting: true,
                terminal: None,
                summary: None,
            }),
            work_ready: Condvar::new(),
            capacity_available: Condvar::new(),
        }
    }
}

/// Mutable coordination facts protected by [`Shared::state`].
struct QueueState {
    queue: VecDeque<EncodedRecord>,
    outstanding_records: usize,
    outstanding_bytes: u64,
    last_accepted_index: Option<u64>,
    accepting: bool,
    terminal: Option<Arc<StorageError>>,
    summary: Option<WriterSummary>,
}

/// Worker-owned active chunk before atomic publication.
struct ActiveChunk {
    ordinal: u64,
    temporary_path: PathBuf,
    final_path: PathBuf,
    file: File,
    hasher: Sha256,
    records: u64,
    bytes: u64,
    first_index: u64,
    last_index: u64,
}

impl ActiveChunk {
    /// Creates a new temporary chunk without replacing any existing file.
    fn create(directory: &Path, ordinal: u64, index: u64) -> Result<Self, StorageError> {
        let filename = chunk_filename(ordinal);
        let final_path = directory.join(&filename);
        if final_path.exists() {
            return Err(StorageError::OutputExists { path: final_path });
        }
        let temporary_path = directory.join(format!(".{filename}.tmp"));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
            .map_err(|source| StorageError::Io {
                operation: "create temporary chunk",
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
            first_index: index,
            last_index: index,
        })
    }

    /// Appends one indivisible framed record and updates exact statistics.
    fn append(&mut self, stream: &str, record: &EncodedRecord) -> Result<(), StorageError> {
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
        self.last_index = record.time().index();
        Ok(())
    }

    /// Synchronizes and atomically publishes this non-empty chunk.
    fn seal(self) -> Result<ChunkMetadata, StorageError> {
        self.file.sync_all().map_err(|source| StorageError::Io {
            operation: "synchronize chunk",
            path: self.temporary_path.clone(),
            source,
        })?;
        drop(self.file);
        fs::rename(&self.temporary_path, &self.final_path).map_err(|source| StorageError::Io {
            operation: "commit chunk",
            path: self.final_path.clone(),
            source,
        })?;
        let digest = lowercase_hex(&self.hasher.finalize());
        Ok(ChunkMetadata {
            ordinal: self.ordinal,
            file: chunk_filename(self.ordinal),
            records: self.records,
            bytes: self.bytes,
            checksum: format!("sha256:{digest}"),
            first_index: self.first_index,
            last_index: self.last_index,
        })
    }
}

/// Encodes digest bytes without allocating an intermediate byte collection.
fn lowercase_hex(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing to String cannot fail");
    }
    encoded
}

/// Runs the append loop and publishes either one summary or one terminal error.
fn worker_main(stream: &str, directory: &Path, max_chunk_bytes: u64, shared: &Shared) {
    let result = write_records(stream, directory, max_chunk_bytes, shared);
    let mut state = lock_state(shared);
    match result {
        Ok(summary) => state.summary = Some(summary),
        Err(error) => {
            state.terminal = Some(Arc::new(error));
            state.accepting = false;
            state.queue.clear();
            state.outstanding_records = 0;
            state.outstanding_bytes = 0;
        }
    }
    shared.capacity_available.notify_all();
    shared.work_ready.notify_all();
}

/// Drains admitted records and performs byte-targeted rollover.
fn write_records(
    stream: &str,
    directory: &Path,
    max_chunk_bytes: u64,
    shared: &Shared,
) -> Result<WriterSummary, StorageError> {
    let mut chunks = Vec::new();
    let mut active: Option<ActiveChunk> = None;
    let mut total_records = 0_u64;
    let mut total_bytes = 0_u64;

    loop {
        let Some(record) = next_record(shared) else {
            break;
        };
        let record_bytes =
            u64::try_from(record.len()).map_err(|_| StorageError::ByteCountOverflow {
                stream: stream.to_owned(),
            })?;
        if active.as_ref().is_some_and(|chunk| {
            chunk.records > 0
                && chunk
                    .bytes
                    .checked_add(record_bytes)
                    .is_none_or(|bytes| bytes > max_chunk_bytes)
        }) {
            seal_active(&mut active, &mut chunks)?;
        }
        if active.is_none() {
            active = Some(ActiveChunk::create(
                directory,
                chunks.len() as u64,
                record.time().index(),
            )?);
        }
        active
            .as_mut()
            .expect("active chunk was just initialized")
            .append(stream, &record)?;
        total_records =
            total_records
                .checked_add(1)
                .ok_or_else(|| StorageError::ByteCountOverflow {
                    stream: stream.to_owned(),
                })?;
        total_bytes = total_bytes.checked_add(record_bytes).ok_or_else(|| {
            StorageError::ByteCountOverflow {
                stream: stream.to_owned(),
            }
        })?;
        // Release the owned encoded allocation before advertising its byte
        // permits to blocked producers.
        drop(record);
        release_capacity(shared, record_bytes);

        if active
            .as_ref()
            .is_some_and(|chunk| chunk.bytes >= max_chunk_bytes)
        {
            seal_active(&mut active, &mut chunks)?;
        }
    }
    seal_active(&mut active, &mut chunks)?;
    Ok(WriterSummary {
        stream: stream.to_owned(),
        chunks,
        records: total_records,
        bytes: total_bytes,
    })
}

/// Removes the next record or returns `None` after a closed queue is drained.
fn next_record(shared: &Shared) -> Option<EncodedRecord> {
    let mut state = lock_state(shared);
    loop {
        if let Some(record) = state.queue.pop_front() {
            return Some(record);
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

/// Releases one record and its exact byte charge after successful append.
fn release_capacity(shared: &Shared, record_bytes: u64) {
    let mut state = lock_state(shared);
    state.outstanding_records -= 1;
    state.outstanding_bytes -= record_bytes;
    shared.capacity_available.notify_all();
}

/// Seals a present active chunk and appends its descriptor.
fn seal_active(
    active: &mut Option<ActiveChunk>,
    chunks: &mut Vec<ChunkMetadata>,
) -> Result<(), StorageError> {
    if let Some(chunk) = active.take() {
        chunks.push(chunk.seal()?);
    }
    Ok(())
}

/// Creates a new stream directory while distinguishing existing output.
fn create_output_directory(path: &Path) -> Result<(), StorageError> {
    match fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(StorageError::OutputExists {
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

/// Locks shared state while recovering from a worker or submitter panic.
fn lock_state(shared: &Shared) -> MutexGuard<'_, QueueState> {
    shared
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Waits for a commit, shutdown, or worker failure to change admission state.
fn wait_for_capacity<'a>(
    shared: &'a Shared,
    state: MutexGuard<'a, QueueState>,
) -> MutexGuard<'a, QueueState> {
    shared
        .capacity_available
        .wait(state)
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Wraps one authoritative terminal failure for a particular stream.
fn terminated(stream: &str, source: &Arc<StorageError>) -> StorageError {
    StorageError::WriterTerminated {
        stream: stream.to_owned(),
        source: Arc::clone(source),
    }
}
