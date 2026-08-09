//! Recording persistence and reconstruction for scientific state samples.
//!
//! This module is the complete public storage boundary. Simulations configure
//! named output streams with fixed step cadences through
//! [`SystemStateWriterBuilder`], then offer a borrowed live [`SystemState`] to
//! [`SystemStateWriter::observe_state`] after each evolution step. The writer
//! checks time before accessing any payload and encodes only streams whose
//! cadence is due. One bounded queue
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
//! [`SystemStateWriter::complete_recording`] drains the writer and atomically transitions the
//! manifest to complete; [`SystemStateWriter::mark_recording_failed`] records an explicit failed
//! lifecycle instead. Dropping an active recording drains its writer thread for
//! memory and file safety but deliberately leaves metadata as `running`.
//!
//! [`SystemStateWriterBuilder::continue_existing_recording`] explicitly validates and appends an
//! existing running run. [`SystemStateWriterBuilder::continue_recording_from_latest_checkpoint`]
//! additionally reconstructs a complete owned checkpoint state through
//! caller-supplied payload decoders.
//! Resume trusts every sealed filename and examines only the highest unsealed
//! chunk per stream.
//!
//! # Reading
//!
//! [`StoredStateSeriesReader`] accepts a completed output directory and a [`JsonPayloadDecoderRegistry`]
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
use std::sync::{Arc, Mutex, MutexGuard};

use fs2::FileExt;
use serde_json::{Map, Value};

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
/// Every record always has an integer simulation index. Physical time remains
/// optional, and its unit is legal only when a physical-coordinate name is
/// configured. Labels are documentation persisted once in `metadata.json`;
/// they do not change [`crate::system_state::SimulationTime`] representation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimeAxisMetadata {
    step_name: String,
    step_unit: Option<String>,
    physical_time_name: Option<String>,
    physical_time_unit: Option<String>,
}

impl TimeAxisMetadata {
    /// Creates a time-axis declaration with a mandatory index label.
    ///
    /// Whitespace is retained in the builder and rejected by
    /// [`SystemStateWriterBuilder::create_new_recording`], keeping fluent configuration infallible
    /// while ensuring persisted labels are never silently normalized.
    pub fn new(step_name: impl Into<String>) -> Self {
        Self {
            step_name: step_name.into(),
            step_unit: None,
            physical_time_name: None,
            physical_time_unit: None,
        }
    }

    /// Sets the optional unit of the integer simulation index.
    #[must_use]
    pub fn with_step_unit(mut self, unit: impl Into<String>) -> Self {
        self.step_unit = Some(unit.into());
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
            step_name: self.step_name,
            step_unit: self.step_unit,
            physical_time_name: self.physical_time_name,
            physical_time_unit: self.physical_time_unit,
        }
    }
}

impl Default for TimeAxisMetadata {
    /// Uses `index` as the simulation-index label and declares no units or
    /// physical coordinate.
    fn default() -> Self {
        Self::new("index")
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
    every_steps: NonZeroU64,
    fields: Vec<String>,
    storage_limits: Option<(NonZeroU64, NonZeroU64)>,
}

impl StateStreamConfig {
    /// Creates a stream whose relative output directory initially equals its
    /// logical name.
    ///
    /// Non-zero types make cadence and both storage limits valid by
    /// construction. Names, paths, duplicate fields, and state-key membership
    /// are validated together by
    /// [`SystemStateWriterBuilder::create_new_recording`].
    pub fn new<I, K>(
        name: impl Into<String>,
        fields: I,
        every_steps: NonZeroU64,
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
            every_steps,
            fields: fields.into_iter().map(Into::into).collect(),
            storage_limits: Some((max_chunk_bytes, queue_bytes)),
        }
    }

    /// Creates a periodic stream that inherits the writer-wide storage limits.
    fn periodic<I, K>(name: impl Into<String>, fields: I, every_steps: NonZeroU64) -> Self
    where
        I: IntoIterator<Item = K>,
        K: Into<String>,
    {
        let name = name.into();
        Self {
            directory: name.clone(),
            name,
            every_steps,
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
    /// [`SystemStateWriterBuilder::add_periodic_state_stream`] require these
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
    /// The synthetic `task_index` entry is always set from the task itself and
    /// therefore replaces any same-named input entry.
    #[must_use]
    pub fn with_task_parameters(mut self, parameters: &TaskParameters) -> Self {
        self.user_metadata = parameters
            .iter()
            .map(|(key, value)| (key.to_owned(), value.clone()))
            .collect();
        self.user_metadata.insert(
            "task_index".to_owned(),
            Value::from(parameters.task_index()),
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

    /// Adds a cadence-controlled stream using writer-wide storage limits.
    ///
    /// The logical name is also its relative output directory. Applications
    /// needing a different directory or per-stream limits can use
    /// [`SystemStateWriterBuilder::add_state_stream`] with an explicit
    /// [`StateStreamConfig`].
    #[must_use]
    pub fn add_periodic_state_stream<I, K>(
        mut self,
        name: impl Into<String>,
        fields: I,
        every_steps: NonZeroU64,
    ) -> Self
    where
        I: IntoIterator<Item = K>,
        K: Into<String>,
    {
        self.streams
            .push(StateStreamConfig::periodic(name, fields, every_steps));
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
    /// metadata before any chunk is recovered. Sealed chunks are trusted by
    /// filename; only the highest open chunk in each stream may be examined.
    pub fn continue_existing_recording(self) -> Result<SystemStateWriter, StorageError> {
        SystemStateWriter::continue_recording(self, None).map(|(writer, _)| writer)
    }

    /// Resumes a run and reconstructs its newest complete checkpoint state.
    ///
    /// `stream` must cover the builder's complete state specification, and
    /// `decoders` must cover every field. The returned state owns all decoded
    /// payloads. Writer threads begin only after reconstruction succeeds.
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
    /// The writer first reads only the state's integer step. Streams that are
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
        let step = state.simulation_time().step();
        let writer = self
            .writer
            .as_ref()
            .expect("an active recording owns its writer worker");
        for name in &self.stream_order {
            let stream = self
                .streams
                .get_mut(name)
                .expect("stream order contains every configured stream");
            if step % stream.every_steps.get() != 0 || stream.last_recorded_step == Some(step) {
                continue;
            }
            let record = stream.encoder.encode(state)?;
            writer.submit_record(name, record)?;
            stream.last_recorded_step = Some(step);
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
    pub fn complete_recording(mut self) -> Result<(), StorageError> {
        if let Err(error) = self.finish_writer() {
            let _ = self.manifest.transition(RecordingStatus::Failed {
                message: error.to_string(),
            });
            return Err(error);
        }
        self.manifest.transition(RecordingStatus::Complete)
    }

    /// Records one final state to every stream exactly once, then completes.
    ///
    /// This terminal observation is independent of periodic cadence. A stream
    /// already recorded at the same step is skipped, while a non-aligned final
    /// step is encoded once. The writer therefore owns both periodic and final
    /// sampling decisions; the simulation supplies only a borrowed state.
    pub fn complete_recording_with_final_state(
        mut self,
        state: &SystemState,
    ) -> Result<(), StorageError> {
        self.record_final_state(state)?;
        self.complete_recording()
    }

    /// Encodes the supplied terminal state for streams that lack this step.
    fn record_final_state(&mut self, state: &SystemState) -> Result<(), StorageError> {
        let step = state.simulation_time().step();
        let writer = self
            .writer
            .as_ref()
            .expect("an active recording owns its writer worker");
        for name in &self.stream_order {
            let stream = self
                .streams
                .get_mut(name)
                .expect("stream order contains every configured stream");
            if stream.last_recorded_step == Some(step) {
                continue;
            }
            let record = stream.encoder.encode(state)?;
            writer.submit_record(name, record)?;
            stream.last_recorded_step = Some(step);
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
    pub fn mark_recording_failed(mut self, message: impl Into<String>) -> Result<(), StorageError> {
        let message = message.into();
        if message.trim().is_empty() {
            return Err(StorageError::InvalidConfiguration {
                setting: "failure_message",
                reason: "failed run message must not be empty".to_owned(),
            });
        }

        if let Err(error) = self.finish_writer() {
            let _ = self.manifest.transition(RecordingStatus::Failed {
                message: error.to_string(),
            });
            return Err(error);
        }
        self.manifest
            .transition(RecordingStatus::Failed { message })
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
        let existing = load_metadata(&prepared.metadata_path)?;
        if !matches!(existing.status, RecordingStatus::Running) {
            return Err(StorageError::RecordingNotContinuable {
                path: prepared.metadata_path,
            });
        }
        ensure_resume_match(&prepared.metadata_path, &prepared.metadata, &existing)?;

        let manifest = Arc::new(RecordingManifest::new(
            prepared.root.clone(),
            prepared.metadata_path.clone(),
            existing.clone(),
        ));
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
                    every_steps: prepared.every_steps,
                    last_recorded_step: None,
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
                    every_steps: prepared.every_steps,
                    last_recorded_step: seed.last_index(),
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
                every_steps: config.every_steps.get(),
                fields,
                max_chunk_bytes: max_chunk_bytes.get(),
                queue_bytes: queue_bytes.get(),
                chunks: Vec::new(),
            });
            streams.push(PreparedStateStream {
                name: config.name.clone(),
                encoder,
                every_steps: config.every_steps,
                writer: StateStreamStorageConfig::new(
                    &config.name,
                    builder.root.join(&config.directory),
                    max_chunk_bytes,
                    queue_bytes,
                )?,
            });
        }

        let metadata = RecordingMetadata::running(stored_time, builder.user_metadata, declarations);
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
    every_steps: NonZeroU64,
    writer: StateStreamStorageConfig,
}

/// Runtime sampling policy and encoder for one logical stream.
struct ScheduledStateStream {
    encoder: JsonStateRecordEncoder,
    every_steps: NonZeroU64,
    last_recorded_step: Option<u64>,
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

    /// Atomically changes the lifecycle after every writer has drained.
    fn transition(&self, status: RecordingStatus) -> Result<(), StorageError> {
        let mut current = lock_metadata(&self.metadata);
        let mut candidate = current.clone();
        candidate.status = status;
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
