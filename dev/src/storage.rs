//! Recording persistence and reconstruction for scientific state samples.
//!
//! This module is the complete public storage boundary. Simulations configure
//! named output streams with coordinate-aware sampling intervals through
//! [`SystemStateWriterBuilder`], then offer a borrowed live [`SystemState`] to
//! [`SystemStateWriter::observe_state`] after each evolution step. The writer
//! checks time before accessing any payload and encodes only streams whose
//! sampling interval includes the current iteration. One bounded queue
//! and worker serve every configured stream, while each stream retains an
//! independent byte-targeted chunk sequence. The recording owns exactly one
//! authoritative `metadata.json` lifecycle.
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
//! [`SystemStateWriterBuilder::create_new_recording`] refuses an existing output root, validates every
//! stream against one shared state specification, publishes initial `running`
//! metadata, and then starts the recording writer. Each chunk descriptor is committed
//! incrementally before that payload receives its sealed filename.
//! [`SystemStateWriter::complete_recording`] drains the writer, atomically commits
//! completion timing and terminal metadata, and returns [`CompletedRecording`];
//! [`SystemStateWriter::mark_recording_failed`] records an explicit failed
//! lifecycle instead. Dropping an active recording drains its writer thread for
//! memory and file safety but deliberately leaves metadata as `running`.
//!
//! [`SystemStateWriterBuilder::continue_existing_recording`] explicitly validates and appends an
//! existing running run. [`SystemStateWriterBuilder::continue_recording_from_latest_checkpoint`]
//! additionally reconstructs a complete owned checkpoint state through
//! caller-supplied payload decoders. Recovery examines only the highest
//! unsealed chunk per stream. Checkpoint-aware continuation also verifies the
//! selected latest sealed checkpoint chunk's exact byte count and SHA-256
//! checksum before decoding it or returning an append-capable writer.
//!
//! # Reading
//!
//! [`StoredStateSeriesReader`] accepts a completed output directory and a [`JsonPayloadDecoderRegistry`]
//! registry. The reader validates metadata, chunks, checksums, record order,
//! and decoder coverage before reconstructing typed
//! [`StateSeries`](crate::time_series::StateSeries) values. Decoder
//! implementations remain per payload type and registrations remain per exact
//! state key. Latest-state reads verify and decode only the newest chunk.

use std::collections::{HashMap, HashSet};
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
use crate::configuration::TaskParameters;
use crate::system_state::{SystemState, SystemStateSchema};

mod error;
mod json_payload_decoder;
mod json_state_record_encoder;
mod jsonl_format;
mod queued_state_writer;
mod stored_state_series_reader;

pub use error::StorageError;
pub use json_payload_decoder::{
    JsonPayloadDecoder, JsonPayloadDecoderRegistry, JsonStringDecoder, JsonVecF64Decoder,
};
pub use stored_state_series_reader::StoredStateSeriesReader;

use json_state_record_encoder::JsonStateRecordEncoder;
use jsonl_format::{
    RecordingMetadata, RecordingStatus, StateFieldMetadata, StateStreamMetadata,
    TimeAxisMetadata as StoredTimeAxis,
};
use queued_state_writer::{RecoveredStateStream, StateStreamStorageConfig, StateWriterWorker};

/// Stable name of the sole structural metadata file in one output root.
const METADATA_FILE: &str = "metadata.json";

/// Temporary sibling used for atomic metadata replacement.
const METADATA_TEMP_FILE: &str = ".metadata.json.tmp";

/// Public description of the temporal coordinates used by a run.
///
/// Every record always has an integer iteration. Physical time remains
/// optional, and its unit is legal only when a physical-coordinate name is
/// configured. Labels are documentation persisted once in `metadata.json`;
/// they do not change [`crate::system_state::SimulationTime`] representation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimeAxisMetadata {
    iteration_name: String,
    iteration_unit: Option<String>,
    physical_time_name: Option<String>,
    physical_time_unit: Option<String>,
}

impl TimeAxisMetadata {
    /// Creates a time-axis declaration with a mandatory iteration label.
    ///
    /// Whitespace is retained in the builder and rejected by
    /// [`SystemStateWriterBuilder::create_new_recording`], keeping fluent configuration infallible
    /// while ensuring persisted labels are never silently normalized.
    pub fn new(iteration_name: impl Into<String>) -> Self {
        Self {
            iteration_name: iteration_name.into(),
            iteration_unit: None,
            physical_time_name: None,
            physical_time_unit: None,
        }
    }

    /// Sets the optional unit of the iteration coordinate.
    #[must_use]
    pub fn with_iteration_unit(mut self, unit: impl Into<String>) -> Self {
        self.iteration_unit = Some(unit.into());
        self
    }

    /// Declares the optional floating-point physical coordinate.
    #[must_use]
    pub fn with_physical_time_name(mut self, name: impl Into<String>) -> Self {
        self.physical_time_name = Some(name.into());
        self
    }

    /// Sets the physical-coordinate unit.
    ///
    /// A matching [`TimeAxisMetadata::with_physical_time_name`] is required; construction fails
    /// at [`SystemStateWriterBuilder::create_new_recording`] if the unit is configured alone.
    #[must_use]
    pub fn with_physical_time_unit(mut self, unit: impl Into<String>) -> Self {
        self.physical_time_unit = Some(unit.into());
        self
    }

    /// Declares the physical-time name and unit together.
    #[must_use]
    pub fn with_physical_axis(mut self, name: impl Into<String>, unit: impl Into<String>) -> Self {
        self.physical_time_name = Some(name.into());
        self.physical_time_unit = Some(unit.into());
        self
    }

    /// Converts public configuration into the private persisted representation.
    fn into_stored(self) -> StoredTimeAxis {
        StoredTimeAxis {
            iteration_name: self.iteration_name,
            iteration_unit: self.iteration_unit,
            physical_time_name: self.physical_time_name,
            physical_time_unit: self.physical_time_unit,
        }
    }
}

impl Default for TimeAxisMetadata {
    /// Uses `iteration` as the integer-time label and declares no units or
    /// physical coordinate.
    fn default() -> Self {
        Self::new("iteration")
    }
}

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
    ) -> Result<Self, StorageError> {
        let finalized_at_utc =
            timing
                .finalized_at_utc
                .clone()
                .ok_or_else(|| StorageError::InvalidMetadata {
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

    /// Returns how many times this recording was reopened for continuation.
    pub fn continuation_count(&self) -> u64 {
        self.continuation_count
    }
}

/// Aggregate persisted facts for one stream in a completed recording.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletedStreamSummary {
    name: String,
    chunk_count: u64,
    record_count: u64,
    encoded_bytes: u64,
    first_iteration: Option<u64>,
    last_iteration: Option<u64>,
}

impl CompletedStreamSummary {
    /// Returns the logical stream name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the number of immutable chunk files.
    pub fn chunk_count(&self) -> u64 {
        self.chunk_count
    }

    /// Returns the total number of recorded states.
    pub fn record_count(&self) -> u64 {
        self.record_count
    }

    /// Returns the exact total framed bytes across all chunks.
    pub fn encoded_bytes(&self) -> u64 {
        self.encoded_bytes
    }

    /// Returns the first recorded iteration, or `None` for an empty stream.
    pub fn first_iteration(&self) -> Option<u64> {
        self.first_iteration
    }

    /// Returns the final recorded iteration, or `None` for an empty stream.
    pub fn last_iteration(&self) -> Option<u64> {
        self.last_iteration
    }
}

/// Durable result of a successfully completed recording lifecycle.
///
/// The active writer has been consumed and all metadata and chunks are durable
/// before this handle is created. It cannot append data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletedRecording {
    directory: PathBuf,
    timing: RecordingTiming,
    terminal_metadata: Map<String, Value>,
    streams: Vec<CompletedStreamSummary>,
}

impl CompletedRecording {
    /// Returns the completed recording directory.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Returns automatically captured operational timing.
    pub fn timing(&self) -> &RecordingTiming {
        &self.timing
    }

    /// Returns caller-supplied terminal metadata committed with completion.
    pub fn terminal_metadata(&self) -> &Map<String, Value> {
        &self.terminal_metadata
    }

    /// Returns stream summaries in declaration order.
    pub fn stream_summaries(&self) -> &[CompletedStreamSummary] {
        &self.streams
    }

    /// Looks up one completed stream summary by exact name.
    pub fn stream_summary(&self, name: &str) -> Option<&CompletedStreamSummary> {
        self.streams.iter().find(|stream| stream.name == name)
    }
}

/// Coordinate-aware interval used to select states for one output stream.
///
/// The noun variant identifies the coordinate on which the interval is
/// measured. The current storage format supports iteration-based sampling;
/// adding physical-time sampling later will not require overloading the word
/// `step` or changing the surrounding stream API.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SamplingInterval {
    /// Select iteration zero and each iteration divisible by this interval.
    Iterations(NonZeroU64),
}

impl SamplingInterval {
    /// Creates an iteration interval, returning `None` for zero.
    pub const fn iterations(interval: u64) -> Option<Self> {
        match NonZeroU64::new(interval) {
            Some(interval) => Some(Self::Iterations(interval)),
            None => None,
        }
    }

    /// Reports whether this interval selects `iteration`.
    const fn includes(self, iteration: u64) -> bool {
        match self {
            Self::Iterations(interval) => iteration.is_multiple_of(interval.get()),
        }
    }
}

/// Configuration for one independently sampled logical output stream.
///
/// Field names are exact keys from the run's [`SystemStateSchema`]. Their input order
/// is irrelevant: the encoder writes them in canonical template order. The
/// chunk byte limit is a rollover target, so a single larger record remains
/// intact in its own oversized chunk. The queue byte limit is strict; a record
/// larger than the complete queue budget is rejected because it can never be
/// admitted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StateStreamConfig {
    name: String,
    directory: String,
    sampling_interval: SamplingInterval,
    fields: Vec<String>,
    storage_limits: Option<(NonZeroU64, NonZeroU64)>,
}

impl StateStreamConfig {
    /// Creates a stream whose relative output directory initially equals its
    /// logical name.
    ///
    /// Non-zero types make the sampling interval and both storage limits valid by
    /// construction. Names, paths, duplicate fields, and state-key membership
    /// are validated together by
    /// [`SystemStateWriterBuilder::create_new_recording`].
    pub fn new<I, K>(
        name: impl Into<String>,
        fields: I,
        sampling_interval: SamplingInterval,
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
            sampling_interval,
            fields: fields.into_iter().map(Into::into).collect(),
            storage_limits: Some((max_chunk_bytes, queue_bytes)),
        }
    }

    /// Creates a sampled stream that inherits writer-wide storage limits.
    fn sampled<I, K>(
        name: impl Into<String>,
        fields: I,
        sampling_interval: SamplingInterval,
    ) -> Self
    where
        I: IntoIterator<Item = K>,
        K: Into<String>,
    {
        let name = name.into();
        Self {
            directory: name.clone(),
            name,
            sampling_interval,
            fields: fields.into_iter().map(Into::into).collect(),
            storage_limits: None,
        }
    }

    /// Overrides the stream's relative directory beneath the run root.
    ///
    /// Absolute paths, empty paths, and `.` or `..` components are rejected at
    /// start. Distinct streams must use distinct directories.
    #[must_use]
    pub fn with_relative_directory(mut self, directory: impl Into<String>) -> Self {
        self.directory = directory.into();
        self
    }
}

/// Builder for one exclusive state-recording directory.
///
/// The builder owns only paths, immutable configuration, and a cheap shared
/// [`SystemStateSchema`] handle. It opens no files and starts no threads before
/// [`SystemStateWriterBuilder::create_new_recording`], [`SystemStateWriterBuilder::continue_existing_recording`], or
/// [`SystemStateWriterBuilder::continue_recording_from_latest_checkpoint`].
#[derive(Debug)]
pub struct SystemStateWriterBuilder {
    root: PathBuf,
    spec: SystemStateSchema,
    time: TimeAxisMetadata,
    user_metadata: Map<String, Value>,
    shared_stream_limits: Option<(NonZeroU64, NonZeroU64)>,
    streams: Vec<StateStreamConfig>,
}

impl SystemStateWriterBuilder {
    /// Creates an empty run configuration using [`TimeAxisMetadata::default`].
    ///
    /// `spec` is cloned only as an `Arc`-backed metadata handle. No scientific
    /// state or payload exists in this builder.
    pub fn new(root: impl Into<PathBuf>, spec: &SystemStateSchema) -> Self {
        Self {
            root: root.into(),
            spec: spec.clone(),
            time: TimeAxisMetadata::default(),
            user_metadata: Map::new(),
            shared_stream_limits: None,
            streams: Vec::new(),
        }
    }

    /// Replaces the run's temporal-coordinate documentation.
    #[must_use]
    pub fn with_time_axis_metadata(mut self, time: TimeAxisMetadata) -> Self {
        self.time = time;
        self
    }

    /// Replaces caller-owned metadata persisted under `user_metadata`.
    ///
    /// Values must already be JSON-compatible. This metadata is structurally
    /// separate from scientific payloads and is written only to
    /// `metadata.json`.
    #[must_use]
    pub fn with_user_metadata(mut self, metadata: Map<String, Value>) -> Self {
        self.user_metadata = metadata;
        self
    }

    /// Uses one chunk target and one bounded-queue budget for concise stream declarations.
    ///
    /// Limits supplied directly through [`StateStreamConfig::new`] remain
    /// stream-specific and take precedence. Streams added through
    /// [`SystemStateWriterBuilder::add_sampled_state_stream`] require these
    /// shared limits.
    #[must_use]
    pub fn with_shared_stream_limits(
        mut self,
        max_chunk_bytes: NonZeroU64,
        queue_bytes: NonZeroU64,
    ) -> Self {
        self.shared_stream_limits = Some((max_chunk_bytes, queue_bytes));
        self
    }

    /// Records one resolved task dictionary as the recording's user metadata.
    ///
    /// Fixed and swept values retain their resolved JSON representation.
    /// The synthetic `task_ordinal` entry is always set from the task itself and
    /// therefore replaces any same-named input entry.
    #[must_use]
    pub fn with_task_parameters(mut self, parameters: &TaskParameters) -> Self {
        self.user_metadata = parameters
            .iter()
            .map(|(key, value)| (key.to_owned(), value.clone()))
            .collect();
        self.user_metadata.insert(
            "task_ordinal".to_owned(),
            Value::from(parameters.task_ordinal()),
        );
        self
    }

    /// Appends one logical stream declaration in deterministic metadata order.
    ///
    /// Duplicate names or directories are reported at start so fluent builder
    /// assembly remains infallible.
    #[must_use]
    pub fn add_state_stream(mut self, stream: StateStreamConfig) -> Self {
        self.streams.push(stream);
        self
    }

    /// Adds a sampled stream using writer-wide storage limits.
    ///
    /// The logical name is also its relative output directory. Applications
    /// needing a different directory or per-stream limits can use
    /// [`SystemStateWriterBuilder::add_state_stream`] with an explicit
    /// [`StateStreamConfig`].
    #[must_use]
    pub fn add_sampled_state_stream<I, K>(
        mut self,
        name: impl Into<String>,
        fields: I,
        sampling_interval: SamplingInterval,
    ) -> Self
    where
        I: IntoIterator<Item = K>,
        K: Into<String>,
    {
        self.streams
            .push(StateStreamConfig::sampled(name, fields, sampling_interval));
        self
    }

    /// Validates the complete run, creates its exclusive output root, starts
    /// each bounded writer, and publishes initial metadata atomically.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::RecordingDirectoryExists`] rather than replacing any
    /// existing filesystem entry. Configuration, state-key selection,
    /// directory creation, thread startup, JSON, and metadata durability
    /// failures retain their precise [`StorageError`] context. If startup fails
    /// after the root is created, the path is retained as diagnostic evidence
    /// and is never silently removed.
    pub fn create_new_recording(self) -> Result<SystemStateWriter, StorageError> {
        SystemStateWriter::create_new_recording(self)
    }

    /// Continues append writing in an existing running recording directory.
    ///
    /// The complete builder configuration is compared with authoritative
    /// metadata before any chunk is recovered. Only the highest open chunk in
    /// each stream may be examined. This append-only entry point does not
    /// reconstruct scientific state; callers requiring a verified checkpoint
    /// must use [`Self::continue_recording_from_latest_checkpoint`].
    pub fn continue_existing_recording(self) -> Result<SystemStateWriter, StorageError> {
        SystemStateWriter::continue_recording(self, None).map(|(writer, _)| writer)
    }

    /// Resumes a run and reconstructs its newest complete checkpoint state.
    ///
    /// `stream` must cover the builder's complete state specification, and
    /// `decoders` must cover every field. The returned state owns all decoded
    /// payloads. When reconstruction selects a sealed chunk, its exact byte
    /// count and SHA-256 checksum are verified before its final record is
    /// decoded. Writer threads begin only after reconstruction succeeds.
    pub fn continue_recording_from_latest_checkpoint(
        self,
        stream: &str,
        decoders: JsonPayloadDecoderRegistry,
    ) -> Result<(SystemStateWriter, SystemState), StorageError> {
        let (writer, state) =
            SystemStateWriter::continue_recording(self, Some((stream, decoders)))?;
        Ok((
            writer,
            state.expect("checkpoint-aware resume always reconstructs one state"),
        ))
    }
}

/// Exclusive queued writer for all persistent streams in one recording.
///
/// This type is intentionally non-Clone. It owns the only writer handles and
/// the only legal transition from `running` metadata to a terminal status.
/// It owns no [`SystemState`] and never extends a payload borrow beyond one
/// synchronous [`SystemStateWriter::observe_state`] call.
pub struct SystemStateWriter {
    root: PathBuf,
    stream_order: Vec<String>,
    manifest: Arc<RecordingManifest>,
    streams: HashMap<String, ScheduledStateStream>,
    writer: Option<StateWriterWorker>,
    session_started: Instant,
    /// Held after writers so normal field drop keeps the lease until every
    /// worker has drained and released its manifest handle.
    _lease: RecordingLease,
}

impl SystemStateWriter {
    /// Begins configuring a new exclusive state-recording directory.
    pub fn builder(root: impl Into<PathBuf>, spec: &SystemStateSchema) -> SystemStateWriterBuilder {
        SystemStateWriterBuilder::new(root, spec)
    }

    /// Returns the recording directory exactly as configured.
    pub fn recording_directory(&self) -> &Path {
        &self.root
    }

    /// Iterates logical stream names in deterministic declaration order.
    pub fn stream_names(&self) -> impl ExactSizeIterator<Item = &str> {
        self.stream_order.iter().map(String::as_str)
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
    pub fn observe_state(&mut self, state: &SystemState) -> Result<(), StorageError> {
        let iteration = state.simulation_time().iteration();
        let writer = self
            .writer
            .as_ref()
            .expect("an active recording owns its writer worker");
        for name in &self.stream_order {
            let stream = self
                .streams
                .get_mut(name)
                .expect("stream order contains every configured stream");
            if !stream.sampling_interval.includes(iteration)
                || stream.last_recorded_iteration == Some(iteration)
            {
                continue;
            }
            let record = stream.encoder.encode(state)?;
            writer.submit_record(name, record)?;
            stream.last_recorded_iteration = Some(iteration);
        }
        Ok(())
    }

    /// Durably seals every record accepted earlier by one logical stream.
    ///
    /// This is an ordered per-stream checkpoint barrier, not merely a buffered
    /// file flush. A non-empty open chunk is synchronized, prepared in the sole
    /// metadata document, renamed to its sealed filename, and directory-synced
    /// before this method returns.
    pub fn flush_stream_to_storage(&self, stream: &str) -> Result<(), StorageError> {
        if !self.streams.contains_key(stream) {
            return Err(StorageError::UnknownStateStream {
                stream: stream.to_owned(),
            });
        }
        self.writer
            .as_ref()
            .expect("an active recording owns its writer worker")
            .flush_state_stream(stream)
    }

    /// Drains every stream, seals all chunks, and atomically publishes complete
    /// metadata.
    ///
    /// The method consumes the coordinator, making repeated finish or sampling
    /// impossible in safe Rust. If a writer fails, all remaining writers are
    /// still drained and a best-effort failed metadata transition is attempted
    /// before the originating writer error is returned.
    pub fn complete_recording(self) -> Result<CompletedRecording, StorageError> {
        self.complete_recording_with_terminal_metadata(Map::new())
    }

    /// Completes the recording and atomically commits values known only at the
    /// terminal boundary.
    ///
    /// Terminal values are stored separately from immutable creation-time user
    /// metadata and therefore cannot silently replace task parameters.
    pub fn complete_recording_with_terminal_metadata(
        mut self,
        terminal_metadata: Map<String, Value>,
    ) -> Result<CompletedRecording, StorageError> {
        if let Err(error) = self.finish_writer() {
            let _ = self.transition_terminal(
                RecordingStatus::Failed {
                    message: error.to_string(),
                },
                Map::new(),
            );
            return Err(error);
        }
        self.transition_terminal(RecordingStatus::Complete, terminal_metadata)?;
        self.completed_recording()
    }

    /// Records one final state to every stream exactly once, then completes.
    ///
    /// This terminal observation is independent of the sampling interval. A stream
    /// already recorded at the same iteration is skipped, while a non-aligned final
    /// iteration is encoded once. The writer therefore owns both interval-based and
    /// terminal sampling decisions; the simulation supplies only a borrowed state.
    pub fn complete_recording_with_final_state(
        mut self,
        state: &SystemState,
    ) -> Result<CompletedRecording, StorageError> {
        self.record_final_state(state)?;
        self.complete_recording()
    }

    /// Records the final state exactly once and atomically commits terminal
    /// user metadata with successful status and operational timing.
    pub fn complete_recording_with_final_state_and_terminal_metadata(
        mut self,
        state: &SystemState,
        terminal_metadata: Map<String, Value>,
    ) -> Result<CompletedRecording, StorageError> {
        self.record_final_state(state)?;
        self.complete_recording_with_terminal_metadata(terminal_metadata)
    }

    /// Encodes the supplied terminal state for streams that lack this iteration.
    fn record_final_state(&mut self, state: &SystemState) -> Result<(), StorageError> {
        let iteration = state.simulation_time().iteration();
        let writer = self
            .writer
            .as_ref()
            .expect("an active recording owns its writer worker");
        for name in &self.stream_order {
            let stream = self
                .streams
                .get_mut(name)
                .expect("stream order contains every configured stream");
            if stream.last_recorded_iteration == Some(iteration) {
                continue;
            }
            let record = stream.encoder.encode(state)?;
            writer.submit_record(name, record)?;
            stream.last_recorded_iteration = Some(iteration);
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
    pub fn mark_recording_failed(self, message: impl Into<String>) -> Result<(), StorageError> {
        self.mark_recording_failed_with_terminal_metadata(message, Map::new())
    }

    /// Records an intentional failure with terminal-only user metadata.
    pub fn mark_recording_failed_with_terminal_metadata(
        mut self,
        message: impl Into<String>,
        terminal_metadata: Map<String, Value>,
    ) -> Result<(), StorageError> {
        let message = message.into();
        if message.trim().is_empty() {
            return Err(StorageError::InvalidConfiguration {
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

    /// Performs complete validation before creating or mutating the run root.
    fn create_new_recording(builder: SystemStateWriterBuilder) -> Result<Self, StorageError> {
        ensure_absent(&builder.root)?;
        let prepared = PreparedRecording::from_builder(builder)?;
        create_root(&prepared.root)?;
        let lease = RecordingLease::acquire(&prepared.root)?;
        for stream in &prepared.streams {
            stream.writer.create_directory()?;
        }
        commit_metadata(&prepared.root, &prepared.metadata_path, &prepared.metadata)?;
        let manifest = Arc::new(RecordingManifest::new(
            prepared.root.clone(),
            prepared.metadata_path.clone(),
            prepared.metadata,
        ));
        Self::start_new_prepared(prepared.root, prepared.streams, manifest, lease)
    }

    /// Validates, recovers, optionally reconstructs, and starts an append run.
    fn continue_recording(
        builder: SystemStateWriterBuilder,
        checkpoint: Option<(&str, JsonPayloadDecoderRegistry)>,
    ) -> Result<(Self, Option<SystemState>), StorageError> {
        let prepared = PreparedRecording::from_builder(builder)?;
        let lease = RecordingLease::acquire(&prepared.root)?;
        remove_stale_metadata_temp(&prepared.root)?;
        let mut existing = load_metadata(&prepared.metadata_path)?;
        if !matches!(existing.status, RecordingStatus::Running) {
            return Err(StorageError::RecordingNotContinuable {
                path: prepared.metadata_path,
            });
        }
        ensure_resume_match(&prepared.metadata_path, &prepared.metadata, &existing)?;

        let mut recovered = Vec::with_capacity(prepared.streams.len());
        for stream in prepared.streams {
            let declaration = existing
                .stream(&stream.name)
                .expect("matched metadata contains every prepared stream");
            let seed = StateWriterWorker::recover_state_stream(&stream.writer, declaration)?;
            recovered.push((stream, seed));
        }

        let state = if let Some((checkpoint_stream, decoders)) = checkpoint {
            let declaration = existing.stream(checkpoint_stream).ok_or_else(|| {
                StorageError::UnknownStateStream {
                    stream: checkpoint_stream.to_owned(),
                }
            })?;
            let seed = recovered
                .iter()
                .find(|(stream, _)| stream.name == checkpoint_stream)
                .map(|(_, seed)| seed)
                .expect("matched stream has one recovered seed");
            Some(stored_state_series_reader::decode_resume_state(
                &prepared.root,
                &prepared.metadata_path,
                declaration,
                &prepared.spec,
                &decoders,
                seed.latest_open_record(),
            )?)
        } else {
            None
        };

        existing.timing.continuation_count = existing
            .timing
            .continuation_count
            .checked_add(1)
            .ok_or_else(|| StorageError::InvalidMetadata {
                path: prepared.metadata_path.clone(),
                reason: "timing.continuation_count overflowed".to_owned(),
            })?;
        commit_metadata(&prepared.root, &prepared.metadata_path, &existing)?;
        let manifest = Arc::new(RecordingManifest::new(
            prepared.root.clone(),
            prepared.metadata_path.clone(),
            existing,
        ));

        let output = Self::start_resumed_prepared(prepared.root, recovered, manifest, lease)?;
        Ok((output, state))
    }

    /// Spawns every empty writer after the initial manifest is durable.
    fn start_new_prepared(
        root: PathBuf,
        streams: Vec<PreparedStateStream>,
        manifest: Arc<RecordingManifest>,
        lease: RecordingLease,
    ) -> Result<Self, StorageError> {
        let mut scheduled = HashMap::with_capacity(streams.len());
        let mut configs = Vec::with_capacity(streams.len());
        let mut stream_order = Vec::with_capacity(streams.len());
        for prepared in streams {
            let name = prepared.name;
            stream_order.push(name.clone());
            scheduled.insert(
                name,
                ScheduledStateStream {
                    encoder: prepared.encoder,
                    sampling_interval: prepared.sampling_interval,
                    last_recorded_iteration: None,
                },
            );
            configs.push(prepared.writer);
        }
        let writer = StateWriterWorker::start_new_recording(configs, Arc::clone(&manifest))?;
        Ok(Self {
            root,
            stream_order,
            manifest,
            streams: scheduled,
            writer: Some(writer),
            session_started: Instant::now(),
            _lease: lease,
        })
    }

    /// Spawns every append writer from its recovered active owner and indices.
    fn start_resumed_prepared(
        root: PathBuf,
        streams: Vec<(PreparedStateStream, RecoveredStateStream)>,
        manifest: Arc<RecordingManifest>,
        lease: RecordingLease,
    ) -> Result<Self, StorageError> {
        let mut scheduled = HashMap::with_capacity(streams.len());
        let mut recovered_streams = Vec::with_capacity(streams.len());
        let mut stream_order = Vec::with_capacity(streams.len());
        for (prepared, seed) in streams {
            let name = prepared.name;
            stream_order.push(name.clone());
            scheduled.insert(
                name,
                ScheduledStateStream {
                    encoder: prepared.encoder,
                    sampling_interval: prepared.sampling_interval,
                    last_recorded_iteration: seed.last_iteration(),
                },
            );
            recovered_streams.push((prepared.writer, seed));
        }
        let writer = StateWriterWorker::continue_recovered_recording(
            recovered_streams,
            Arc::clone(&manifest),
        )?;
        Ok(Self {
            root,
            stream_order,
            manifest,
            streams: scheduled,
            writer: Some(writer),
            session_started: Instant::now(),
            _lease: lease,
        })
    }

    /// Drains and joins the recording's sole queued writer worker.
    fn finish_writer(&mut self) -> Result<(), StorageError> {
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
    ) -> Result<(), StorageError> {
        let finalized_at_utc =
            utc_now_rfc3339().map_err(|source| StorageError::OperationalTimestamp {
                operation: "finalize recording",
                source,
            })?;
        let active_duration_ns = duration_nanoseconds(self.session_started.elapsed())
            .ok_or(StorageError::OperationalDurationOverflow)?;
        self.manifest.transition_terminal(
            status,
            finalized_at_utc,
            active_duration_ns,
            terminal_metadata,
        )
    }

    /// Builds the immutable public result from the durable manifest snapshot.
    fn completed_recording(&self) -> Result<CompletedRecording, StorageError> {
        let metadata = self.manifest.snapshot();
        let timing = RecordingTiming::from_stored(&metadata.timing, &self.manifest.path)?;
        let streams = metadata
            .streams
            .iter()
            .map(completed_stream_summary)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(CompletedRecording {
            directory: self.root.clone(),
            timing,
            terminal_metadata: metadata.terminal_metadata,
            streams,
        })
    }
}

/// Derives one public stream aggregate without opening any chunk file.
fn completed_stream_summary(
    stream: &StateStreamMetadata,
) -> Result<CompletedStreamSummary, StorageError> {
    let overflow = || StorageError::ByteCountOverflow {
        stream: stream.name.clone(),
    };
    let chunk_count = u64::try_from(stream.chunks.len()).map_err(|_| overflow())?;
    let record_count = stream
        .chunks
        .iter()
        .try_fold(0_u64, |total, chunk| total.checked_add(chunk.records))
        .ok_or_else(&overflow)?;
    let encoded_bytes = stream
        .chunks
        .iter()
        .try_fold(0_u64, |total, chunk| total.checked_add(chunk.bytes))
        .ok_or_else(overflow)?;
    Ok(CompletedStreamSummary {
        name: stream.name.clone(),
        chunk_count,
        record_count,
        encoded_bytes,
        first_iteration: stream.chunks.first().map(|chunk| chunk.first_iteration),
        last_iteration: stream.chunks.last().map(|chunk| chunk.last_iteration),
    })
}

/// Fully validated builder output before any writer thread starts.
struct PreparedRecording {
    root: PathBuf,
    metadata_path: PathBuf,
    spec: SystemStateSchema,
    metadata: RecordingMetadata,
    streams: Vec<PreparedStateStream>,
}

impl PreparedRecording {
    /// Canonicalizes stream field order and builds expected persisted metadata.
    fn from_builder(builder: SystemStateWriterBuilder) -> Result<Self, StorageError> {
        let metadata_path = builder.root.join(METADATA_FILE);
        let stored_time = builder.time.into_stored();
        let mut names = HashSet::with_capacity(builder.streams.len());
        let mut directories = HashSet::with_capacity(builder.streams.len());
        let mut streams = Vec::with_capacity(builder.streams.len());
        let mut declarations = Vec::with_capacity(builder.streams.len());

        for config in builder.streams {
            if !names.insert(config.name.clone()) {
                return Err(StorageError::DuplicateStateStream {
                    stream: config.name,
                });
            }
            if !directories.insert(config.directory.clone()) {
                return Err(StorageError::InvalidConfiguration {
                    setting: "stream.directory",
                    reason: format!(
                        "multiple streams use relative directory `{}`",
                        config.directory
                    ),
                });
            }

            let (max_chunk_bytes, queue_bytes) = config
                .storage_limits
                .or(builder.shared_stream_limits)
                .ok_or_else(|| StorageError::InvalidConfiguration {
                    setting: "stream.storage_limits",
                    reason: format!(
                        "stream `{}` has no explicit limits and the writer has no shared limits",
                        config.name
                    ),
                })?;
            let encoder = JsonStateRecordEncoder::new(&config.name, &builder.spec, &config.fields)?;
            let fields = encoder
                .fields()
                .map(|name| {
                    let field = builder
                        .spec
                        .field_schema(name)
                        .expect("encoder fields were validated against this specification");
                    StateFieldMetadata {
                        name: name.to_owned(),
                        description: field.description().map(str::to_owned),
                    }
                })
                .collect::<Vec<_>>();
            declarations.push(StateStreamMetadata {
                name: config.name.clone(),
                directory: config.directory.clone(),
                sampling_interval: config.sampling_interval,
                fields,
                max_chunk_bytes: max_chunk_bytes.get(),
                queue_bytes: queue_bytes.get(),
                chunks: Vec::new(),
            });
            streams.push(PreparedStateStream {
                name: config.name.clone(),
                encoder,
                sampling_interval: config.sampling_interval,
                writer: StateStreamStorageConfig::new(
                    &config.name,
                    builder.root.join(&config.directory),
                    max_chunk_bytes,
                    queue_bytes,
                )?,
            });
        }

        let created_at_utc =
            utc_now_rfc3339().map_err(|source| StorageError::OperationalTimestamp {
                operation: "create recording",
                source,
            })?;
        let metadata = RecordingMetadata::running(
            stored_time,
            builder.user_metadata,
            declarations,
            created_at_utc,
        );
        metadata.validate(&metadata_path)?;
        Ok(Self {
            root: builder.root,
            metadata_path,
            spec: builder.spec,
            metadata,
            streams,
        })
    }
}

/// One canonical encoder paired with its immutable writer configuration.
struct PreparedStateStream {
    name: String,
    encoder: JsonStateRecordEncoder,
    sampling_interval: SamplingInterval,
    writer: StateStreamStorageConfig,
}

/// Runtime sampling policy and encoder for one logical stream.
struct ScheduledStateStream {
    encoder: JsonStateRecordEncoder,
    sampling_interval: SamplingInterval,
    last_recorded_iteration: Option<u64>,
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
    ) -> Result<(), StorageError> {
        let mut current = lock_metadata(&self.metadata);
        if !matches!(current.status, RecordingStatus::Running) {
            return Err(StorageError::RecordingFinished);
        }
        let mut candidate = current.clone();
        let declaration =
            candidate
                .stream_mut(stream)
                .ok_or_else(|| StorageError::UnknownStateStream {
                    stream: stream.to_owned(),
                })?;
        let expected = u64::try_from(declaration.chunks.len()).map_err(|_| {
            StorageError::ByteCountOverflow {
                stream: stream.to_owned(),
            }
        })?;
        if descriptor.ordinal != expected {
            return Err(StorageError::InvalidMetadata {
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
    ) -> Result<(), StorageError> {
        let mut current = lock_metadata(&self.metadata);
        let mut candidate = current.clone();
        candidate.status = status;
        candidate.timing.finalized_at_utc = Some(finalized_at_utc);
        candidate.timing.active_duration_ns = candidate
            .timing
            .active_duration_ns
            .checked_add(active_duration_ns)
            .ok_or(StorageError::OperationalDurationOverflow)?;
        candidate.terminal_metadata = terminal_metadata;
        commit_metadata(&self.root, &self.path, &candidate)?;
        *current = candidate;
        Ok(())
    }

    /// Clones the small durable metadata snapshot for a public terminal result.
    fn snapshot(&self) -> RecordingMetadata {
        lock_metadata(&self.metadata).clone()
    }
}

/// Advisory exclusive ownership of the output root directory itself.
///
/// Locking the directory handle creates no lockfile or status artifact and the
/// operating system releases the lease automatically after process death.
struct RecordingLease {
    _directory: File,
}

impl RecordingLease {
    /// Acquires non-blocking exclusive writer ownership.
    fn acquire(root: &Path) -> Result<Self, StorageError> {
        let directory = File::open(root).map_err(|source| StorageError::Io {
            operation: "open output root for exclusive ownership",
            path: root.to_path_buf(),
            source,
        })?;
        match FileExt::try_lock_exclusive(&directory) {
            Ok(()) => Ok(Self {
                _directory: directory,
            }),
            Err(source) if source.kind() == std::io::ErrorKind::WouldBlock => {
                Err(StorageError::RecordingDirectoryInUse {
                    path: root.to_path_buf(),
                })
            }
            Err(source) => Err(StorageError::Io {
                operation: "acquire exclusive output ownership",
                path: root.to_path_buf(),
                source,
            }),
        }
    }
}

/// Loads and semantically validates the authoritative metadata snapshot.
fn load_metadata(path: &Path) -> Result<RecordingMetadata, StorageError> {
    let bytes = fs::read(path).map_err(|source| StorageError::Io {
        operation: "read metadata for resume",
        path: path.to_path_buf(),
        source,
    })?;
    let metadata: RecordingMetadata =
        serde_json::from_slice(&bytes).map_err(|source| StorageError::Json {
            operation: "parse metadata for resume",
            path: path.to_path_buf(),
            source,
        })?;
    metadata.validate(path)?;
    Ok(metadata)
}

/// Compares every immutable run/stream setting while ignoring chunk progress.
fn ensure_resume_match(
    path: &Path,
    expected: &RecordingMetadata,
    existing: &RecordingMetadata,
) -> Result<(), StorageError> {
    let mut configuration = existing.clone();
    for stream in &mut configuration.streams {
        stream.chunks.clear();
    }
    configuration.status = RecordingStatus::Running;
    configuration.timing = expected.timing.clone();
    configuration.terminal_metadata.clear();
    if &configuration != expected {
        return Err(StorageError::RecordingConfigurationMismatch {
            path: path.to_path_buf(),
            reason: "builder time axis, user metadata, or stream declarations differ".to_owned(),
        });
    }
    Ok(())
}

/// Removes only the known atomic-replacement remnant after acquiring the lease.
fn remove_stale_metadata_temp(root: &Path) -> Result<(), StorageError> {
    let path = root.join(METADATA_TEMP_FILE);
    match fs::remove_file(&path) {
        Ok(()) => File::open(root)
            .and_then(|directory| directory.sync_all())
            .map_err(|source| StorageError::Io {
                operation: "synchronize stale metadata cleanup",
                path: root.to_path_buf(),
                source,
            }),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(StorageError::Io {
            operation: "remove stale temporary metadata",
            path,
            source,
        }),
    }
}

/// Locks the metadata snapshot while recovering from a participant panic.
fn lock_metadata(metadata: &Mutex<RecordingMetadata>) -> MutexGuard<'_, RecordingMetadata> {
    metadata
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Rejects every existing filesystem object and preserves IO inspection errors.
fn ensure_absent(root: &Path) -> Result<(), StorageError> {
    match root.try_exists() {
        Ok(false) => Ok(()),
        Ok(true) => Err(StorageError::RecordingDirectoryExists {
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
            Err(StorageError::RecordingDirectoryExists {
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
    metadata: &RecordingMetadata,
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
