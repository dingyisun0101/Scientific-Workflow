//! Logged integration tests for filename-based crash recovery and append.
//!
//! Run with:
//!
//! ```text
//! cargo test --test resume_workflow -- --nocapture
//! ```

use std::fmt::Write as _;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use physics_in_parallel::prelude::advanced::Dense;
use physics_in_parallel::prelude::basic::Tensor;
use scientific_workflow::prelude::basic::*;
use scientific_workflow::state::advanced::{StateMaintenance, StateSchemaAccess};
use scientific_workflow::storage::*;
use serde_json::Value;
use sha2::{Digest, Sha256};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TempWorkspace {
    root: PathBuf,
}

impl TempWorkspace {
    fn new(label: &str) -> Self {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "scientific-workflow-{label}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        Self { root }
    }

    fn run(&self) -> PathBuf {
        self.root.join("run")
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.root) {
            eprintln!("[cleanup] {}: {error}", self.root.display());
        }
    }
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/state.json")
}

fn builder(run: &Path, spec: &SystemStateSchema) -> SystemStateWriterBuilder {
    builder_with_chunk_limit(run, spec, 1_000_000)
}

fn builder_with_chunk_limit(
    run: &Path,
    spec: &SystemStateSchema,
    max_chunk_bytes: u64,
) -> SystemStateWriterBuilder {
    SystemStateWriter::builder(run.to_path_buf(), spec)
        .with_writer(Writer::streams([Stream::all_fields("checkpoint").unwrap()]).unwrap())
        .with_shared_stream_storage(StateStreamStorage::chunked(
            NonZeroU64::new(max_chunk_bytes).unwrap(),
            NonZeroU64::new(1_000_000).unwrap(),
        ))
}

fn decoders() -> JsonPayloadDecoderRegistry {
    JsonPayloadDecoderRegistry::with_capacity(3)
        .with_json_field::<Vec<f64>>("population")
        .unwrap()
        .with_json_field::<String>("activity")
        .unwrap()
        .with_json_field::<Tensor<u64, Dense>>("space")
        .unwrap()
}

fn automatic_builder(run: &Path, spec: &SystemStateSchema) -> SystemStateWriterBuilder {
    automatic_builder_with_chunk_limit(run, spec, 1)
}

fn automatic_builder_with_chunk_limit(
    run: &Path,
    spec: &SystemStateSchema,
    max_chunk_bytes: u64,
) -> SystemStateWriterBuilder {
    SystemStateWriter::builder(run.to_path_buf(), spec)
        .with_writer(
            Writer::streams([
                Stream::fields("observations", ["activity"]).unwrap(),
                Stream::all_fields("checkpoint")
                    .unwrap()
                    .every_iterations(2)
                    .unwrap(),
            ])
            .unwrap(),
        )
        .with_shared_stream_storage(StateStreamStorage::chunked(
            NonZeroU64::new(max_chunk_bytes).unwrap(),
            NonZeroU64::new(1_000_000).unwrap(),
        ))
}

fn state(spec: &SystemStateSchema, index: u64) -> SystemState {
    let mut lattice = Tensor::<u64, Dense>::zeros(&[2]);
    lattice.set(&[0], index + 10);
    lattice.set(&[1], index + 20);
    let mut state = spec.create_empty_state(
        StateTime::from_iteration_and_physical_time(index, index as f64 * 0.5).unwrap(),
    );
    state
        .insert_payload("population", vec![index as f64, index as f64 + 0.25])
        .unwrap();
    state.insert_payload("space", lattice).unwrap();
    state
        .insert_payload("activity", format!("iteration-{index}"))
        .unwrap();
    state
}

fn metadata(run: &Path) -> Value {
    serde_json::from_slice(&fs::read(run.join("metadata.json")).unwrap()).unwrap()
}

fn write_metadata(run: &Path, metadata: &Value) {
    let mut bytes = serde_json::to_vec_pretty(metadata).unwrap();
    bytes.push(b'\n');
    fs::write(run.join("metadata.json"), bytes).unwrap();
}

/// Reproduces an interrupted lifecycle from a successfully sealed fixture.
fn mark_manifest_running(metadata: &mut Value) {
    metadata["status"] = serde_json::json!({"state": "running"});
    metadata["timing"]
        .as_object_mut()
        .unwrap()
        .remove("finalized_at_utc");
    metadata
        .as_object_mut()
        .unwrap()
        .remove("terminal_metadata");
}

fn stream_position(metadata: &Value, name: &str) -> usize {
    metadata["streams"]
        .as_array()
        .unwrap()
        .iter()
        .position(|stream| stream["name"] == name)
        .unwrap()
}

fn retained_prefix(bytes: &[u8], checkpoint: u64) -> Vec<u8> {
    bytes
        .split_inclusive(|byte| *byte == b'\n')
        .take_while(|line| {
            serde_json::from_slice::<Value>(&line[..line.len() - 1]).unwrap()["iteration"]
                .as_u64()
                .unwrap()
                <= checkpoint
        })
        .flatten()
        .copied()
        .collect()
}

fn update_chunk_descriptor(descriptor: &mut Value, bytes: &[u8]) {
    let iterations = bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| {
            serde_json::from_slice::<Value>(line).unwrap()["iteration"]
                .as_u64()
                .unwrap()
        })
        .collect::<Vec<_>>();
    descriptor["records"] = (iterations.len() as u64).into();
    descriptor["bytes"] = (bytes.len() as u64).into();
    descriptor["first_iteration"] = iterations.first().copied().unwrap().into();
    descriptor["last_iteration"] = iterations.last().copied().unwrap().into();
    let mut checksum = String::from("sha256:");
    for byte in Sha256::digest(bytes) {
        write!(&mut checksum, "{byte:02x}").unwrap();
    }
    descriptor["checksum"] = checksum.into();
}

#[test]
fn automatic_resume_infers_checkpoint_and_rewinds_every_stream() {
    let workspace = TempWorkspace::new("automatic-resume");
    let run = workspace.run();
    let spec = SystemStateSchema::load_json_template(&fixture_path()).unwrap();

    let (mut writer, restored) = automatic_builder(&run, &spec)
        .open_or_resume_from_latest_checkpoint(decoders())
        .unwrap();
    assert!(restored.is_none());
    for iteration in 0..=3 {
        writer.observe_state(&state(&spec, iteration)).unwrap();
    }
    drop(writer);

    let (mut writer, restored) = automatic_builder(&run, &spec)
        .open_or_resume_from_latest_checkpoint(decoders())
        .unwrap();
    let restored = restored.unwrap();
    assert_eq!(restored.time().iteration(), 2);
    let rewound = metadata(&run);
    for stream in rewound["streams"].as_array().unwrap() {
        assert!(
            stream["chunks"]
                .as_array()
                .unwrap()
                .iter()
                .all(|chunk| chunk["last_iteration"].as_u64().unwrap() <= 2)
        );
    }

    writer.observe_state(&state(&spec, 3)).unwrap();
    writer.complete_recording().unwrap();
    let completed = metadata(&run);
    let observations = completed["streams"]
        .as_array()
        .unwrap()
        .iter()
        .find(|stream| stream["name"] == "observations")
        .unwrap();
    assert_eq!(
        observations["chunks"].as_array().unwrap().last().unwrap()["last_iteration"],
        3
    );
}

#[test]
fn metadata_authority_completes_interrupted_rewind_idempotently() {
    let spec = SystemStateSchema::load_json_template(&fixture_path()).unwrap();

    // Crash before metadata commit: the old sealed file still matches metadata,
    // so the uncommitted staged replacement is discarded before rewind retries.
    let before_metadata = TempWorkspace::new("rewind-before-metadata");
    let before_run = before_metadata.run();
    let mut writer = automatic_builder_with_chunk_limit(&before_run, &spec, 1_000_000)
        .create_new_recording()
        .unwrap();
    for iteration in 0..=3 {
        writer.observe_state(&state(&spec, iteration)).unwrap();
    }
    drop(writer);
    let before_manifest = metadata(&before_run);
    let observations = stream_position(&before_manifest, "observations");
    let directory = before_manifest["streams"][observations]["directory"]
        .as_str()
        .unwrap();
    let filename = before_manifest["streams"][observations]["chunks"][0]["file"]
        .as_str()
        .unwrap();
    let sealed = before_run.join(directory).join(filename);
    let staged = sealed.with_extension("jsonl.tmp");
    let prefix = retained_prefix(&fs::read(&sealed).unwrap(), 2);
    fs::write(&staged, &prefix).unwrap();

    let (writer, restored) = automatic_builder_with_chunk_limit(&before_run, &spec, 1_000_000)
        .open_or_resume_from_latest_checkpoint(decoders())
        .unwrap();
    assert_eq!(restored.unwrap().time().iteration(), 2);
    assert_eq!(fs::read(&sealed).unwrap(), prefix);
    assert!(!staged.exists());
    drop(writer);

    // Crash after metadata commit: the staged bytes now match metadata and
    // replace the obsolete sealed file when recovery next enters the stream.
    let after_metadata = TempWorkspace::new("rewind-after-metadata");
    let after_run = after_metadata.run();
    let mut writer = automatic_builder_with_chunk_limit(&after_run, &spec, 1_000_000)
        .create_new_recording()
        .unwrap();
    for iteration in 0..=3 {
        writer.observe_state(&state(&spec, iteration)).unwrap();
    }
    drop(writer);
    let mut after_manifest = metadata(&after_run);
    let observations = stream_position(&after_manifest, "observations");
    let directory = after_manifest["streams"][observations]["directory"]
        .as_str()
        .unwrap()
        .to_owned();
    let filename = after_manifest["streams"][observations]["chunks"][0]["file"]
        .as_str()
        .unwrap()
        .to_owned();
    let sealed = after_run.join(directory).join(filename);
    let staged = sealed.with_extension("jsonl.tmp");
    let prefix = retained_prefix(&fs::read(&sealed).unwrap(), 2);
    fs::write(&staged, &prefix).unwrap();
    update_chunk_descriptor(
        &mut after_manifest["streams"][observations]["chunks"][0],
        &prefix,
    );
    write_metadata(&after_run, &after_manifest);

    let (writer, restored) = automatic_builder_with_chunk_limit(&after_run, &spec, 1_000_000)
        .open_or_resume_from_latest_checkpoint(decoders())
        .unwrap();
    assert_eq!(restored.unwrap().time().iteration(), 2);
    assert_eq!(fs::read(&sealed).unwrap(), prefix);
    assert!(!staged.exists());
    drop(writer);

    // Committed omission similarly authorizes cleanup of a later sealed chunk.
    let omitted = TempWorkspace::new("rewind-omitted-chunk");
    let omitted_run = omitted.run();
    let mut writer = automatic_builder(&omitted_run, &spec)
        .create_new_recording()
        .unwrap();
    for iteration in 0..=3 {
        writer.observe_state(&state(&spec, iteration)).unwrap();
    }
    drop(writer);
    let mut omitted_manifest = metadata(&omitted_run);
    let observations = stream_position(&omitted_manifest, "observations");
    let directory = omitted_manifest["streams"][observations]["directory"]
        .as_str()
        .unwrap()
        .to_owned();
    let removed_file = omitted_manifest["streams"][observations]["chunks"]
        .as_array_mut()
        .unwrap()
        .pop()
        .unwrap()["file"]
        .as_str()
        .unwrap()
        .to_owned();
    let removed_path = omitted_run.join(directory).join(removed_file);
    assert!(removed_path.is_file());
    write_metadata(&omitted_run, &omitted_manifest);
    let (writer, restored) = automatic_builder(&omitted_run, &spec)
        .open_or_resume_from_latest_checkpoint(decoders())
        .unwrap();
    assert_eq!(restored.unwrap().time().iteration(), 2);
    assert!(!removed_path.exists());
    drop(writer);
}

#[test]
fn unpublished_tail_is_discarded_and_resume_uses_latest_sealed_checkpoint() {
    let workspace = TempWorkspace::new("unprepared-resume");
    let run = workspace.run();
    let spec = SystemStateSchema::load_json_template(&fixture_path()).unwrap();

    let recording_builder = || builder_with_chunk_limit(&run, &spec, 1);
    let mut output = recording_builder().create_new_recording().unwrap();
    output.observe_state(&state(&spec, 0)).unwrap();
    output.observe_state(&state(&spec, 1)).unwrap();
    let completed = output.complete_recording().unwrap();
    assert_eq!(completed.timing().continuation_count(), 0);

    // Reproduce a crash while publishing chunk 1, before descriptor
    // preparation. Chunk 0 remains the latest authoritative checkpoint.
    let mut manifest = metadata(&run);
    mark_manifest_running(&mut manifest);
    manifest["streams"][0]["chunks"]
        .as_array_mut()
        .unwrap()
        .pop();
    write_metadata(&run, &manifest);
    let sealed = run.join("stream_0000/chunk-000001.jsonl");
    let open = run.join("stream_0000/chunk-000001.jsonl.tmp");
    fs::rename(&sealed, &open).unwrap();
    OpenOptions::new()
        .append(true)
        .open(&open)
        .unwrap()
        .write_all(br#"{"iteration":2,"physical_time":"#)
        .unwrap();

    let (mut output, resumed) = recording_builder()
        .continue_recording_from_latest_checkpoint("checkpoint", decoders())
        .unwrap();
    assert_eq!(resumed.time().iteration(), 0);
    assert_eq!(resumed.time().physical_time(), Some(0.0));
    assert_eq!(
        resumed.payload::<Vec<f64>>("population").unwrap(),
        &[0.0, 0.25]
    );
    assert_eq!(
        resumed.payload::<String>("activity").unwrap(),
        "iteration-0"
    );
    assert_eq!(
        resumed
            .payload::<Tensor<u64, Dense>>("space")
            .unwrap()
            .get(&[1]),
        20
    );
    assert_eq!(resumed.populated_field_count(), resumed.schema().len());
    println!(
        "[resume-state] iteration={} physical_time={:?} fields={} complete=true",
        resumed.time().iteration(),
        resumed.time().physical_time(),
        resumed.populated_field_count()
    );

    output.observe_state(&state(&spec, 2)).unwrap();
    output.flush_stream_to_storage("checkpoint").unwrap();
    assert!(sealed.is_file());
    assert!(!open.exists());
    let running = metadata(&run);
    assert_eq!(running["status"]["state"], "running");
    assert_eq!(running["streams"][0]["chunks"].as_array().unwrap().len(), 2);
    println!(
        "[recovery] unpublished_tail_discarded=true resumed_sealed_chunk=true durable_barrier=true"
    );
    let completed = output.complete_recording().unwrap();
    assert_eq!(completed.timing().continuation_count(), 1);

    let reader = StoredStateSeriesReader::open_completed_recording(&run, decoders()).unwrap();
    let series = reader.read_stream_as_state_series("checkpoint").unwrap();
    assert_eq!(series.len(), 2);
    assert_eq!(series.last_state().unwrap().time().iteration(), 2);
    assert_eq!(
        series
            .iter()
            .map(|state| state.time().iteration())
            .collect::<Vec<_>>(),
        [0, 2]
    );
    println!(
        "[result] buffered_resume=passed final_states={}",
        series.len()
    );
}

#[test]
fn prepared_tail_finishes_rename_and_exclusive_lease_rejects_competitors() {
    let workspace = TempWorkspace::new("prepared-resume");
    let run = workspace.run();
    let spec = SystemStateSchema::load_json_template(&fixture_path()).unwrap();

    let mut output = builder(&run, &spec).create_new_recording().unwrap();
    output.observe_state(&state(&spec, 4)).unwrap();
    output.complete_recording().unwrap();

    // Reproduce the narrow crash window after metadata preparation but before
    // the lifecycle rename. The descriptor remains authoritative.
    let mut manifest = metadata(&run);
    mark_manifest_running(&mut manifest);
    write_metadata(&run, &manifest);
    let sealed = run.join("stream_0000/chunk-000000.jsonl");
    let open = run.join("stream_0000/chunk-000000.jsonl.tmp");
    fs::rename(&sealed, &open).unwrap();

    let (mut output, resumed) = builder(&run, &spec)
        .continue_recording_from_latest_checkpoint("checkpoint", decoders())
        .unwrap();
    assert_eq!(resumed.time().iteration(), 4);
    assert!(sealed.is_file());
    assert!(!open.exists());
    assert!(matches!(
        builder(&run, &spec).continue_existing_recording(),
        Err(StorageError::RecordingDirectoryInUse { .. })
    ));
    println!(
        "[prepared] descriptor_verified=true rename_completed=true sealed_history_scanned=false lease_exclusive=true"
    );

    output.observe_state(&state(&spec, 5)).unwrap();
    output.complete_recording().unwrap();
    let reader = StoredStateSeriesReader::open_completed_recording(&run, decoders()).unwrap();
    let series = reader.read_stream_as_state_series("checkpoint").unwrap();
    assert_eq!(
        series
            .iter()
            .map(|state| state.time().iteration())
            .collect::<Vec<_>>(),
        [4, 5]
    );
    println!(
        "[result] prepared_resume=passed final_states={}",
        series.len()
    );
}

#[test]
fn sealed_checkpoint_integrity_is_mandatory_before_continuation() {
    let size_workspace = TempWorkspace::new("resume-size-integrity");
    let size_run = size_workspace.run();
    let spec = SystemStateSchema::load_json_template(&fixture_path()).unwrap();
    let mut output = builder(&size_run, &spec).create_new_recording().unwrap();
    output.observe_state(&state(&spec, 1)).unwrap();
    output.complete_recording().unwrap();
    let mut manifest = metadata(&size_run);
    mark_manifest_running(&mut manifest);
    write_metadata(&size_run, &manifest);
    let size_chunk = size_run.join("stream_0000/chunk-000000.jsonl");
    OpenOptions::new()
        .append(true)
        .open(&size_chunk)
        .unwrap()
        .write_all(b"x")
        .unwrap();

    assert!(matches!(
        builder(&size_run, &spec)
            .continue_recording_from_latest_checkpoint("checkpoint", decoders()),
        Err(StorageError::ChunkSizeMismatch { path, .. }) if path == size_chunk
    ));
    assert_eq!(metadata(&size_run)["timing"]["continuation_count"], 0);

    let checksum_workspace = TempWorkspace::new("resume-checksum-integrity");
    let checksum_run = checksum_workspace.run();
    let mut output = builder(&checksum_run, &spec)
        .create_new_recording()
        .unwrap();
    output.observe_state(&state(&spec, 2)).unwrap();
    output.complete_recording().unwrap();
    let mut manifest = metadata(&checksum_run);
    mark_manifest_running(&mut manifest);
    write_metadata(&checksum_run, &manifest);
    let checksum_chunk = checksum_run.join("stream_0000/chunk-000000.jsonl");
    let mut bytes = fs::read(&checksum_chunk).unwrap();
    bytes[0] ^= 1;
    fs::write(&checksum_chunk, bytes).unwrap();

    assert!(matches!(
        builder(&checksum_run, &spec)
            .continue_recording_from_latest_checkpoint("checkpoint", decoders()),
        Err(StorageError::ChecksumMismatch { path, .. }) if path == checksum_chunk
    ));
    assert_eq!(metadata(&checksum_run)["timing"]["continuation_count"], 0);
    println!("[resume-integrity] byte_count=true checksum=true writer_not_exposed=true");
}

#[test]
fn partial_stream_continues_output_but_cannot_construct_a_full_state() {
    let workspace = TempWorkspace::new("partial-resume");
    let run = workspace.run();
    let spec = SystemStateSchema::load_json_template(&fixture_path()).unwrap();
    let partial_builder = || {
        SystemStateWriter::builder(run.clone(), &spec)
            .with_writer(
                Writer::streams([Stream::fields("signal", ["population"]).unwrap()]).unwrap(),
            )
            .with_shared_stream_storage(StateStreamStorage::chunked(
                NonZeroU64::new(1_000_000).unwrap(),
                NonZeroU64::new(1_000_000).unwrap(),
            ))
    };

    let mut output = partial_builder().create_new_recording().unwrap();
    output.observe_state(&state(&spec, 8)).unwrap();
    output.complete_recording().unwrap();
    let mut manifest = metadata(&run);
    mark_manifest_running(&mut manifest);
    write_metadata(&run, &manifest);

    assert!(matches!(
        partial_builder().continue_recording_from_latest_checkpoint("signal", decoders()),
        Err(StorageError::IncompleteCheckpointStream { stream, .. }) if stream == "signal"
    ));
    let mut output = partial_builder().continue_existing_recording().unwrap();
    output.observe_state(&state(&spec, 9)).unwrap();
    output.complete_recording().unwrap();

    let mut population_decoder = JsonPayloadDecoderRegistry::new();
    population_decoder
        .register_for_field("population", JsonVecF64Decoder)
        .unwrap();
    let series = StoredStateSeriesReader::open_completed_recording(&run, population_decoder)
        .unwrap()
        .read_stream_as_state_series("signal")
        .unwrap();
    assert_eq!(series.len(), 2);
    println!(
        "[schema] partial_checkpoint_rejected=true output_continued=true final_states={}",
        series.len()
    );
}

#[test]
fn earlier_sealed_chunks_are_not_scanned_when_discarding_an_unpublished_tail() {
    let workspace = TempWorkspace::new("multi-chunk-resume");
    let run = workspace.run();
    let spec = SystemStateSchema::load_json_template(&fixture_path()).unwrap();
    let recording_builder = || builder_with_chunk_limit(&run, &spec, 1);

    let mut output = recording_builder().create_new_recording().unwrap();
    for index in 0..3 {
        output.observe_state(&state(&spec, index)).unwrap();
    }
    output.complete_recording().unwrap();

    let mut manifest = metadata(&run);
    mark_manifest_running(&mut manifest);
    let chunks = manifest["streams"][0]["chunks"].as_array_mut().unwrap();
    assert_eq!(chunks.len(), 3);
    chunks.pop();
    write_metadata(&run, &manifest);

    let stream_directory = run.join("stream_0000");
    let sealed_zero = stream_directory.join("chunk-000000.jsonl");
    let sealed_two = stream_directory.join("chunk-000002.jsonl");
    let open_two = stream_directory.join("chunk-000002.jsonl.tmp");
    fs::write(&sealed_zero, b"deliberately invalid sealed history\n").unwrap();
    fs::rename(&sealed_two, &open_two).unwrap();

    let (mut output, resumed) = recording_builder()
        .continue_recording_from_latest_checkpoint("checkpoint", decoders())
        .expect("resume must discard the open tail and decode the latest sealed chunk");
    assert_eq!(resumed.time().iteration(), 1);
    assert_eq!(
        resumed.payload::<String>("activity").unwrap(),
        "iteration-1"
    );

    output.observe_state(&state(&spec, 3)).unwrap();
    output.complete_recording().unwrap();

    let completed = metadata(&run);
    let chunks = completed["streams"][0]["chunks"].as_array().unwrap();
    assert_eq!(chunks.len(), 3);
    assert_eq!(
        chunks
            .iter()
            .map(|chunk| chunk["ordinal"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        [0, 1, 2]
    );
    assert!((0..3).all(|ordinal| {
        stream_directory
            .join(format!("chunk-{ordinal:06}.jsonl"))
            .is_file()
    }));
    assert!(!open_two.exists());
    println!(
        "[multi-chunk] sealed_history_trusted=true unpublished_tail_discarded=true resumed_index=1 next_ordinal=2"
    );
}

#[test]
fn continuation_rejects_terminal_mismatched_and_empty_recordings() {
    let workspace = TempWorkspace::new("resume-rejections");
    let spec = SystemStateSchema::load_json_template(&fixture_path()).unwrap();

    let terminal = workspace.root.join("terminal");
    builder(&terminal, &spec)
        .create_new_recording()
        .unwrap()
        .complete_recording()
        .unwrap();
    assert!(matches!(
        builder(&terminal, &spec).continue_existing_recording(),
        Err(StorageError::RecordingNotContinuable { path }) if path == terminal.join("metadata.json")
    ));

    let mismatched = workspace.root.join("mismatched");
    drop(builder(&mismatched, &spec).create_new_recording().unwrap());
    assert!(matches!(
        builder(&mismatched, &spec)
            .with_writer(
                Writer::streams([Stream::all_fields("checkpoint").unwrap()])
                    .unwrap()
                    .with_iteration_unit("cycle")
                    .unwrap(),
            )
            .continue_existing_recording(),
        Err(StorageError::RecordingConfigurationMismatch { path, .. })
            if path == mismatched.join("metadata.json")
    ));

    let empty = workspace.root.join("empty");
    drop(builder(&empty, &spec).create_new_recording().unwrap());
    assert!(matches!(
        builder(&empty, &spec)
            .continue_recording_from_latest_checkpoint("checkpoint", decoders()),
        Err(StorageError::NoCheckpointState { stream }) if stream == "checkpoint"
    ));

    println!("[resume-rejections] terminal=true configuration_mismatch=true no_checkpoint=true");
}
