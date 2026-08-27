//! Logged public-API fault injection for storage lifecycle and integrity.
//!
//! Run with:
//!
//! ```text
//! cargo test --test storage_resilience -- --nocapture
//! ```

use std::error::Error as _;
use std::fmt::Write as _;
use std::fs;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use scientific_workflow::prelude::basic::*;
use scientific_workflow::state::advanced::StateMaintenance;
use scientific_workflow::storage::*;
use serde::{Serialize, Serializer};
use serde_json::Value;
use sha2::{Digest, Sha256};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Owns one exact workspace and leaves its run child absent for public startup.
struct TempWorkspace {
    root: PathBuf,
}

impl TempWorkspace {
    fn new(label: &str) -> Self {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "scientific-workflow-resilience-{label}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("unique resilience workspace must be creatable");
        Self { root }
    }

    fn run(&self) -> PathBuf {
        self.root.join("run")
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.root) {
            eprintln!(
                "[cleanup] failed to remove {}: {error}",
                self.root.display()
            );
        }
    }
}

#[derive(Clone)]
struct RejectEncoding;

impl Serialize for RejectEncoding {
    fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        Err(serde::ser::Error::custom("deliberate resilience failure"))
    }
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/state.json")
}

fn spec() -> SystemStateSchema {
    SystemStateSchema::load_json_template(&fixture_path()).unwrap()
}

fn writer(every_iterations: u64) -> Writer {
    Writer::streams([Stream::fields("signal", ["population", "activity"])
        .unwrap()
        .every_iterations(every_iterations)
        .unwrap()])
    .unwrap()
}

fn recording_builder(
    root: PathBuf,
    state: &SystemState,
    queue_bytes: u64,
) -> SystemStateWriterBuilder {
    SystemStateWriter::builder(root, state)
        .with_writer(writer(1))
        .with_shared_stream_storage(StateStreamStorage::chunked(
            NonZeroU64::new(128).unwrap(),
            NonZeroU64::new(queue_bytes).unwrap(),
        ))
}

fn populated_state(spec: &SystemStateSchema, index: u64) -> SystemState {
    let mut state = spec.create_empty_state(StateTime::from_iteration(index));
    state
        .insert_payload("population", vec![index as f64, 2.5])
        .unwrap();
    state
        .insert_payload("activity", format!("sample-{index}"))
        .unwrap();
    state
}

fn decoders() -> JsonPayloadDecoderRegistry {
    let mut decoders = JsonPayloadDecoderRegistry::with_capacity(2);
    decoders
        .register_for_field("population", JsonVecF64Decoder)
        .unwrap();
    decoders
        .register_for_field("activity", JsonStringDecoder)
        .unwrap();
    decoders
}

/// Produces a completed public run with two strictly ordered records.
fn write_valid_run(workspace: &TempWorkspace) {
    let spec = spec();
    let initial_state = populated_state(&spec, 0);
    let mut output = recording_builder(workspace.run(), &initial_state, 4_096)
        .create_new_recording()
        .unwrap();
    output.observe_state(&populated_state(&spec, 2)).unwrap();
    output.observe_state(&populated_state(&spec, 5)).unwrap();
    output.complete_recording().unwrap();
}

fn metadata_path(run: &Path) -> PathBuf {
    run.join("metadata.json")
}

fn first_chunk(run: &Path) -> PathBuf {
    let metadata: Value = serde_json::from_slice(&fs::read(metadata_path(run)).unwrap()).unwrap();
    run.join(metadata["streams"][0]["directory"].as_str().unwrap())
        .join(
            metadata["streams"][0]["chunks"][0]["file"]
                .as_str()
                .unwrap(),
        )
}

/// Replaces the first chunk and updates its authoritative structural facts.
fn replace_first_chunk(run: &Path, bytes: &[u8], records: u64, first: u64, last: u64) {
    let metadata_file = metadata_path(run);
    let mut metadata: Value = serde_json::from_slice(&fs::read(&metadata_file).unwrap()).unwrap();
    let descriptor = &mut metadata["streams"][0]["chunks"][0];
    descriptor["records"] = records.into();
    descriptor["bytes"] = (bytes.len() as u64).into();
    descriptor["first_iteration"] = first.into();
    descriptor["last_iteration"] = last.into();
    descriptor["checksum"] = sha256_checksum(bytes).into();
    fs::write(first_chunk(run), bytes).unwrap();
    fs::write(metadata_file, serde_json::to_vec_pretty(&metadata).unwrap()).unwrap();
}

#[test]
fn storage_failures_are_detected_with_context_and_without_partial_success() {
    let state_spec = spec();
    let writer_state = populated_state(&state_spec, 0);

    let existing = TempWorkspace::new("existing");
    fs::create_dir(existing.run()).unwrap();
    assert!(matches!(
        recording_builder(existing.run(), &writer_state, 4_096)
            .create_new_recording(),
        Err(StorageError::RecordingDirectoryExists { path }) if path == existing.run()
    ));

    let invalid = TempWorkspace::new("invalid");
    assert!(matches!(
        SystemStateWriter::builder(invalid.run(), &writer_state)
            .with_writer(Writer::fields(["absent"]).unwrap())
            .create_new_recording(),
        Err(StorageError::Writer {
            source: WriterError::UnknownField { field, .. }
        }) if field == "absent"
    ));
    assert!(!invalid.run().exists());

    let duplicate = TempWorkspace::new("duplicate");
    assert!(matches!(
        Writer::streams([
            Stream::all_fields("signal").unwrap(),
            Stream::all_fields("signal").unwrap(),
        ]),
        Err(WriterError::DuplicateStreamName { stream }) if stream == "signal"
    ));
    assert!(!duplicate.run().exists());

    let invalid_time = TempWorkspace::new("time");
    assert!(matches!(
        Writer::all_fields().with_physical_time_unit(" "),
        Err(WriterError::EmptyAxisUnit {
            axis: "physical_time"
        })
    ));
    assert!(!invalid_time.run().exists());
    println!("[configuration] existing=true fields=true duplicate=true time=true");

    let oversized = TempWorkspace::new("oversized");
    let mut output = recording_builder(oversized.run(), &writer_state, 64)
        .create_new_recording()
        .unwrap();
    let mut huge = state_spec.create_empty_state(StateTime::from_iteration(1));
    huge.insert_payload("population", vec![1.0]).unwrap();
    huge.insert_payload("activity", "x".repeat(512)).unwrap();
    assert!(matches!(
        output.observe_state(&huge),
        Err(StorageError::RecordTooLarge { limit: 64, .. })
    ));
    output
        .mark_recording_failed("expected oversized sample")
        .unwrap();

    let ordering = TempWorkspace::new("ordering");
    let mut output = recording_builder(ordering.run(), &writer_state, 4_096)
        .create_new_recording()
        .unwrap();
    let mut state = populated_state(&state_spec, 5);
    output.observe_state(&state).unwrap();
    output.observe_state(&state).unwrap();
    state.replace_time(StateTime::from_iteration(4));
    assert!(matches!(
        output.observe_state(&state),
        Err(StorageError::Writer {
            source: WriterError::NonIncreasingObservation {
                next: 4,
                previous: 5,
                ..
            }
        })
    ));
    output.complete_recording().unwrap();

    let terminal = TempWorkspace::new("terminal");
    let mut output = recording_builder(terminal.run(), &writer_state, 4_096)
        .create_new_recording()
        .unwrap();
    fs::remove_dir(terminal.run().join("stream_0000")).unwrap();
    output
        .observe_state(&populated_state(&state_spec, 1))
        .unwrap();
    assert!(matches!(
        output.complete_recording(),
        Err(StorageError::StateWriterTerminated { .. })
    ));
    println!(
        "[backpressure] oversized_rejected=true ordering_rejected=true terminal_propagated=true"
    );

    let encoding = TempWorkspace::new("encoding");
    let mut output = SystemStateWriter::builder(encoding.run(), &writer_state)
        .with_writer(writer(2))
        .with_shared_stream_storage(StateStreamStorage::chunked(
            NonZeroU64::new(128).unwrap(),
            NonZeroU64::new(4_096).unwrap(),
        ))
        .create_new_recording()
        .unwrap();
    let mut empty = state_spec.create_empty_state(StateTime::from_iteration(3));
    output.observe_state(&empty).unwrap();
    empty.replace_time(StateTime::from_iteration(4));
    assert!(matches!(
        output.observe_state(&empty),
        Err(StorageError::Writer {
            source: WriterError::StateAccess { iteration: 4, .. }
        })
    ));
    empty.insert_payload("population", RejectEncoding).unwrap();
    empty
        .insert_payload("activity", String::from("valid"))
        .unwrap();
    let encode_error = output.observe_state(&empty).unwrap_err();
    assert!(matches!(
        &encode_error,
        StorageError::Writer {
            source: WriterError::EncodeField {
                stream,
                iteration: 4,
                field,
                ..
            }
        } if stream == "signal" && field == "population"
    ));
    assert_eq!(
        encode_error.source().unwrap().source().unwrap().to_string(),
        "deliberate resilience failure"
    );
    output
        .mark_recording_failed("expected encoding failures")
        .unwrap();

    let running = TempWorkspace::new("running");
    let output = recording_builder(running.run(), &writer_state, 4_096)
        .create_new_recording()
        .unwrap();
    assert!(matches!(
        StoredStateSeriesReader::open_completed_recording(running.run(), decoders()),
        Err(StorageError::RecordingNotComplete { .. })
    ));
    let mut terminal_metadata = serde_json::Map::new();
    terminal_metadata.insert("completed_step_count".to_owned(), Value::from(7));
    output
        .mark_recording_failed_with_terminal_metadata(
            "simulation stopped deliberately",
            terminal_metadata,
        )
        .unwrap();
    let failed_metadata: Value =
        serde_json::from_slice(&fs::read(metadata_path(&running.run())).unwrap()).unwrap();
    assert_eq!(failed_metadata["status"]["state"], "failed");
    assert_eq!(
        failed_metadata["status"]["message"],
        "simulation stopped deliberately"
    );
    assert_eq!(
        failed_metadata["terminal_metadata"]["completed_step_count"],
        7
    );
    assert!(
        failed_metadata["timing"]["finalized_at_utc"]
            .as_str()
            .unwrap()
            .ends_with('Z')
    );
    assert!(matches!(
        StoredStateSeriesReader::open_completed_recording(running.run(), decoders()),
        Err(StorageError::RecordingNotComplete { .. })
    ));

    let coverage = TempWorkspace::new("coverage");
    write_valid_run(&coverage);
    let reader = StoredStateSeriesReader::open_completed_recording(
        coverage.run(),
        JsonPayloadDecoderRegistry::new(),
    )
    .unwrap();
    assert!(matches!(
        reader.read_stream_as_state_series("absent"),
        Err(StorageError::UnknownStateStream { stream }) if stream == "absent"
    ));
    assert!(matches!(
        reader.read_stream_as_state_series("signal"),
        Err(StorageError::MissingDecoder { field }) if field == "population"
    ));
    let mut registry = JsonPayloadDecoderRegistry::new();
    assert!(matches!(
        registry.register_for_field::<String, _>("", JsonStringDecoder),
        Err(StorageError::InvalidConfiguration { .. })
    ));
    registry
        .register_for_field("activity", JsonStringDecoder)
        .unwrap();
    assert!(matches!(
        registry.register_for_field::<String, _>("activity", JsonStringDecoder),
        Err(StorageError::DuplicateDecoder { field }) if field == "activity"
    ));

    let wrong_type = TempWorkspace::new("wrong-type");
    write_valid_run(&wrong_type);
    let wrong_bytes = b"{\"iteration\":2,\"values\":[\"bad\",\"valid\"]}\n";
    replace_first_chunk(&wrong_type.run(), wrong_bytes, 1, 2, 2);
    let error = StoredStateSeriesReader::open_completed_recording(wrong_type.run(), decoders())
        .unwrap()
        .read_stream_as_state_series("signal")
        .unwrap_err();
    assert!(matches!(
        &error,
        StorageError::DecodeField {
            stream,
            iteration: 2,
            field,
            ..
        } if stream == "signal" && field == "population"
    ));
    assert!(error.source().unwrap().is::<serde_json::Error>());
    println!("[decoder] missing=true wrong_type=true source_preserved=true");

    let malformed = TempWorkspace::new("malformed");
    write_valid_run(&malformed);
    replace_first_chunk(&malformed.run(), b"{\n", 1, 2, 2);
    assert!(matches!(
        StoredStateSeriesReader::open_completed_recording(malformed.run(), decoders())
            .unwrap()
            .read_stream_as_state_series("signal"),
        Err(StorageError::InvalidRecord { line: 1, .. })
    ));

    let missing_field = TempWorkspace::new("missing-field");
    write_valid_run(&missing_field);
    let missing_field_bytes = b"{\"iteration\":2,\"values\":[[2.0]]}\n";
    replace_first_chunk(&missing_field.run(), missing_field_bytes, 1, 2, 2);
    assert!(matches!(
        StoredStateSeriesReader::open_completed_recording(missing_field.run(), decoders())
            .unwrap()
            .read_stream_as_state_series("signal"),
        Err(StorageError::InvalidRecord { reason, .. }) if reason.contains("contains 1 payload values")
    ));

    let object_values = TempWorkspace::new("object-values");
    write_valid_run(&object_values);
    let object_bytes = b"{\"iteration\":2,\"values\":{\"population\":[2.0],\"activity\":\"a\"}}\n";
    replace_first_chunk(&object_values.run(), object_bytes, 1, 2, 2);
    assert!(matches!(
        StoredStateSeriesReader::open_completed_recording(object_values.run(), decoders())
            .unwrap()
            .read_stream_as_state_series("signal"),
        Err(StorageError::InvalidRecord { .. })
    ));

    let additional_field = TempWorkspace::new("additional-field");
    write_valid_run(&additional_field);
    let additional_bytes = b"{\"iteration\":2,\"values\":[[2.0],\"a\",0]}\n";
    replace_first_chunk(&additional_field.run(), additional_bytes, 1, 2, 2);
    assert!(matches!(
        StoredStateSeriesReader::open_completed_recording(additional_field.run(), decoders())
            .unwrap()
            .read_stream_as_state_series("signal"),
        Err(StorageError::InvalidRecord { reason, .. }) if reason.contains("contains 3 payload values")
    ));

    let invalid_physical = TempWorkspace::new("invalid-physical");
    write_valid_run(&invalid_physical);
    let physical_bytes = b"{\"iteration\":2,\"physical_time\":1e400,\"values\":[[2.0],\"a\"]}\n";
    replace_first_chunk(&invalid_physical.run(), physical_bytes, 1, 2, 2);
    assert!(matches!(
        StoredStateSeriesReader::open_completed_recording(invalid_physical.run(), decoders())
            .unwrap()
            .read_stream_as_state_series("signal"),
        Err(StorageError::InvalidRecord { .. })
    ));

    let non_increasing = TempWorkspace::new("non-increasing");
    write_valid_run(&non_increasing);
    let repeated =
        b"{\"iteration\":2,\"values\":[[2.0],\"a\"]}\n{\"iteration\":2,\"values\":[[3.0],\"b\"]}\n";
    replace_first_chunk(&non_increasing.run(), repeated, 2, 2, 2);
    assert!(matches!(
        StoredStateSeriesReader::open_completed_recording(non_increasing.run(), decoders())
            .unwrap()
            .read_stream_as_state_series("signal"),
        Err(StorageError::InvalidRecord { reason, .. }) if reason.contains("not greater")
    ));

    let missing = TempWorkspace::new("missing-chunk");
    write_valid_run(&missing);
    let missing_chunk = first_chunk(&missing.run());
    fs::remove_file(&missing_chunk).unwrap();
    assert!(matches!(
        StoredStateSeriesReader::open_completed_recording(missing.run(), decoders())
            .unwrap()
            .read_stream_as_state_series("signal"),
        Err(StorageError::MissingChunk { path }) if path == missing_chunk
    ));

    let size = TempWorkspace::new("size");
    write_valid_run(&size);
    let size_chunk = first_chunk(&size.run());
    let mut size_bytes = fs::read(&size_chunk).unwrap();
    size_bytes.push(b' ');
    fs::write(&size_chunk, size_bytes).unwrap();
    assert!(matches!(
        StoredStateSeriesReader::open_completed_recording(size.run(), decoders())
            .unwrap()
            .read_stream_as_state_series("signal"),
        Err(StorageError::ChunkSizeMismatch { path, .. }) if path == size_chunk
    ));

    let checksum = TempWorkspace::new("checksum");
    write_valid_run(&checksum);
    let checksum_chunk = first_chunk(&checksum.run());
    let mut checksum_bytes = fs::read(&checksum_chunk).unwrap();
    let position = checksum_bytes
        .iter()
        .position(|byte| *byte == b'5')
        .unwrap();
    checksum_bytes[position] = b'6';
    fs::write(&checksum_chunk, checksum_bytes).unwrap();
    assert!(matches!(
        StoredStateSeriesReader::open_completed_recording(checksum.run(), decoders())
            .unwrap()
            .read_stream_as_state_series("signal"),
        Err(StorageError::ChecksumMismatch { path, .. }) if path == checksum_chunk
    ));

    let unterminated = TempWorkspace::new("unterminated");
    write_valid_run(&unterminated);
    let bytes = b"{\"iteration\":2,\"values\":[[2.0],\"a\"]}";
    replace_first_chunk(&unterminated.run(), bytes, 1, 2, 2);
    assert!(matches!(
        StoredStateSeriesReader::open_completed_recording(unterminated.run(), decoders())
            .unwrap()
            .read_stream_as_state_series("signal"),
        Err(StorageError::InvalidRecord { reason, .. }) if reason.contains("not terminated")
    ));

    let version = TempWorkspace::new("version");
    write_valid_run(&version);
    let version_path = metadata_path(&version.run());
    let mut version_metadata: Value =
        serde_json::from_slice(&fs::read(&version_path).unwrap()).unwrap();
    version_metadata["version"] = 999.into();
    fs::write(
        &version_path,
        serde_json::to_vec_pretty(&version_metadata).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        StoredStateSeriesReader::open_completed_recording(version.run(), decoders()),
        Err(StorageError::UnsupportedVersion { found: 999, .. })
    ));

    println!("[integrity] missing=true size=true checksum=true record=true");
    println!(
        "[expected-error] families=configuration,writer,lifecycle,decoder,record,integrity context_verified=true"
    );
    println!("[result] storage_resilience=passed");
}

fn sha256_checksum(bytes: &[u8]) -> String {
    let mut checksum = String::from("sha256:");
    for byte in Sha256::digest(bytes) {
        write!(&mut checksum, "{byte:02x}").unwrap();
    }
    checksum
}
