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

use scientific_workflow::prelude::*;
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

fn spec() -> StateSpec {
    StateSpec::load(fixture_path()).unwrap()
}

fn stream(queue_bytes: u64) -> StreamConfig {
    StreamConfig::new(
        "signal",
        ["population", "activity"],
        NonZeroU64::new(128).unwrap(),
        NonZeroU64::new(queue_bytes).unwrap(),
    )
}

fn populated_state(spec: &StateSpec, index: u64) -> SystemState {
    let mut state = spec.empty(TimePoint::new(index));
    state.set("population", vec![index as f64, 2.5]).unwrap();
    state.set("activity", format!("sample-{index}")).unwrap();
    state
}

fn decoders() -> Decoders {
    let mut decoders = Decoders::with_capacity(2);
    decoders.add("population", VecF64Decoder).unwrap();
    decoders.add("activity", StringDecoder).unwrap();
    decoders
}

/// Produces a completed public run with two strictly ordered records.
fn write_valid_run(workspace: &TempWorkspace) {
    let spec = spec();
    let output = RunOutput::builder(workspace.run(), &spec)
        .stream(stream(4_096))
        .start()
        .unwrap();
    output.sample("signal", &populated_state(&spec, 2)).unwrap();
    output.sample("signal", &populated_state(&spec, 5)).unwrap();
    output.finish().unwrap();
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
    descriptor["first_index"] = first.into();
    descriptor["last_index"] = last.into();
    descriptor["checksum"] = sha256_checksum(bytes).into();
    fs::write(first_chunk(run), bytes).unwrap();
    fs::write(metadata_file, serde_json::to_vec_pretty(&metadata).unwrap()).unwrap();
}

#[test]
fn storage_failures_are_detected_with_context_and_without_partial_success() {
    let state_spec = spec();

    let existing = TempWorkspace::new("existing");
    fs::create_dir(existing.run()).unwrap();
    assert!(matches!(
        RunOutput::builder(existing.run(), &state_spec)
            .stream(stream(4_096))
            .start(),
        Err(StorageError::OutputExists { path }) if path == existing.run()
    ));

    let invalid = TempWorkspace::new("invalid");
    assert!(matches!(
        RunOutput::builder(invalid.run(), &state_spec)
            .stream(StreamConfig::new(
                "signal",
                ["absent"],
                NonZeroU64::new(1).unwrap(),
                NonZeroU64::new(1).unwrap(),
            ))
            .start(),
        Err(StorageError::InvalidConfig {
            setting: "fields",
            ..
        })
    ));
    assert!(!invalid.run().exists());

    let duplicate = TempWorkspace::new("duplicate");
    assert!(matches!(
        RunOutput::builder(duplicate.run(), &state_spec)
            .stream(stream(4_096))
            .stream(stream(4_096))
            .start(),
        Err(StorageError::DuplicateStream { stream }) if stream == "signal"
    ));

    let invalid_time = TempWorkspace::new("time");
    assert!(matches!(
        RunOutput::builder(invalid_time.run(), &state_spec)
            .time_axis(TimeAxis::new("step").physical_unit("s"))
            .stream(stream(4_096))
            .start(),
        Err(StorageError::InvalidMetadata { .. })
    ));
    println!("[configuration] existing=true fields=true duplicate=true time=true");

    let oversized = TempWorkspace::new("oversized");
    let output = RunOutput::builder(oversized.run(), &state_spec)
        .stream(stream(64))
        .start()
        .unwrap();
    let mut huge = state_spec.empty(TimePoint::new(1));
    huge.set("population", vec![1.0]).unwrap();
    huge.set("activity", "x".repeat(512)).unwrap();
    assert!(matches!(
        output.sample("signal", &huge),
        Err(StorageError::RecordTooLarge { limit: 64, .. })
    ));
    output.fail("expected oversized sample").unwrap();

    let ordering = TempWorkspace::new("ordering");
    let output = RunOutput::builder(ordering.run(), &state_spec)
        .stream(stream(4_096))
        .start()
        .unwrap();
    let mut state = populated_state(&state_spec, 5);
    output.sample("signal", &state).unwrap();
    assert!(matches!(
        output.sample("signal", &state),
        Err(StorageError::OutOfOrderRecord {
            index: 5,
            previous: 5,
            ..
        })
    ));
    state.set_time(TimePoint::new(4));
    assert!(matches!(
        output.sample("signal", &state),
        Err(StorageError::OutOfOrderRecord {
            index: 4,
            previous: 5,
            ..
        })
    ));
    output.finish().unwrap();

    let terminal = TempWorkspace::new("terminal");
    let output = RunOutput::builder(terminal.run(), &state_spec)
        .stream(stream(4_096))
        .start()
        .unwrap();
    fs::remove_dir(terminal.run().join("signal")).unwrap();
    output
        .sample("signal", &populated_state(&state_spec, 1))
        .unwrap();
    assert!(matches!(
        output.finish(),
        Err(StorageError::WriterTerminated { .. })
    ));
    println!(
        "[backpressure] oversized_rejected=true ordering_rejected=true terminal_propagated=true"
    );

    let encoding = TempWorkspace::new("encoding");
    let output = RunOutput::builder(encoding.run(), &state_spec)
        .stream(stream(4_096))
        .start()
        .unwrap();
    let mut empty = state_spec.empty(TimePoint::new(3));
    assert!(matches!(
        output.sample("signal", &empty),
        Err(StorageError::StateAccess { index: 3, .. })
    ));
    empty.set("population", RejectEncoding).unwrap();
    empty.set("activity", String::from("valid")).unwrap();
    let encode_error = output.sample("signal", &empty).unwrap_err();
    assert!(matches!(
        &encode_error,
        StorageError::EncodeField {
            stream,
            index: 3,
            field,
            ..
        } if stream == "signal" && field == "population"
    ));
    assert_eq!(
        encode_error.source().unwrap().to_string(),
        "deliberate resilience failure"
    );
    output.fail("expected encoding failures").unwrap();

    let running = TempWorkspace::new("running");
    let output = RunOutput::builder(running.run(), &state_spec)
        .stream(stream(4_096))
        .start()
        .unwrap();
    assert!(matches!(
        SeriesReader::open(running.run(), decoders()),
        Err(StorageError::RunIncomplete { .. })
    ));
    output.fail("simulation stopped deliberately").unwrap();
    let failed_metadata: Value =
        serde_json::from_slice(&fs::read(metadata_path(&running.run())).unwrap()).unwrap();
    assert_eq!(failed_metadata["status"]["state"], "failed");
    assert_eq!(
        failed_metadata["status"]["message"],
        "simulation stopped deliberately"
    );
    assert!(matches!(
        SeriesReader::open(running.run(), decoders()),
        Err(StorageError::RunIncomplete { .. })
    ));

    let coverage = TempWorkspace::new("coverage");
    write_valid_run(&coverage);
    let reader = SeriesReader::open(coverage.run(), Decoders::new()).unwrap();
    assert!(matches!(
        reader.read("absent"),
        Err(StorageError::UnknownStream { stream }) if stream == "absent"
    ));
    assert!(matches!(
        reader.read("signal"),
        Err(StorageError::MissingDecoder { field }) if field == "population"
    ));
    let mut registry = Decoders::new();
    assert!(matches!(
        registry.add::<String, _>("", StringDecoder),
        Err(StorageError::InvalidConfig { .. })
    ));
    registry.add("activity", StringDecoder).unwrap();
    assert!(matches!(
        registry.add::<String, _>("activity", StringDecoder),
        Err(StorageError::DuplicateDecoder { field }) if field == "activity"
    ));

    let wrong_type = TempWorkspace::new("wrong-type");
    write_valid_run(&wrong_type);
    let wrong_bytes = b"{\"index\":2,\"values\":{\"population\":\"bad\",\"activity\":\"valid\"}}\n";
    replace_first_chunk(&wrong_type.run(), wrong_bytes, 1, 2, 2);
    let error = SeriesReader::open(wrong_type.run(), decoders())
        .unwrap()
        .read("signal")
        .unwrap_err();
    assert!(matches!(
        &error,
        StorageError::DecodeField {
            stream,
            index: 2,
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
        SeriesReader::open(malformed.run(), decoders())
            .unwrap()
            .read("signal"),
        Err(StorageError::InvalidRecord { line: 1, .. })
    ));

    let missing_field = TempWorkspace::new("missing-field");
    write_valid_run(&missing_field);
    let missing_field_bytes = b"{\"index\":2,\"values\":{\"population\":[2.0]}}\n";
    replace_first_chunk(&missing_field.run(), missing_field_bytes, 1, 2, 2);
    assert!(matches!(
        SeriesReader::open(missing_field.run(), decoders())
            .unwrap()
            .read("signal"),
        Err(StorageError::InvalidRecord { reason, .. }) if reason.contains("missing payload field `activity`")
    ));

    let duplicate_field = TempWorkspace::new("duplicate-field");
    write_valid_run(&duplicate_field);
    let duplicate_bytes =
        b"{\"index\":2,\"values\":{\"population\":[2.0],\"activity\":\"a\",\"activity\":\"b\"}}\n";
    replace_first_chunk(&duplicate_field.run(), duplicate_bytes, 1, 2, 2);
    assert!(matches!(
        SeriesReader::open(duplicate_field.run(), decoders())
            .unwrap()
            .read("signal"),
        Err(StorageError::InvalidRecord { reason, .. }) if reason.contains("duplicate payload field")
    ));

    let additional_field = TempWorkspace::new("additional-field");
    write_valid_run(&additional_field);
    let additional_bytes =
        b"{\"index\":2,\"values\":{\"population\":[2.0],\"activity\":\"a\",\"extra\":0}}\n";
    replace_first_chunk(&additional_field.run(), additional_bytes, 1, 2, 2);
    assert!(matches!(
        SeriesReader::open(additional_field.run(), decoders())
            .unwrap()
            .read("signal"),
        Err(StorageError::InvalidRecord { reason, .. }) if reason.contains("undeclared payload fields: extra")
    ));

    let invalid_physical = TempWorkspace::new("invalid-physical");
    write_valid_run(&invalid_physical);
    let physical_bytes =
        b"{\"index\":2,\"physical\":1e400,\"values\":{\"population\":[2.0],\"activity\":\"a\"}}\n";
    replace_first_chunk(&invalid_physical.run(), physical_bytes, 1, 2, 2);
    assert!(matches!(
        SeriesReader::open(invalid_physical.run(), decoders())
            .unwrap()
            .read("signal"),
        Err(StorageError::InvalidRecord { .. })
    ));

    let non_increasing = TempWorkspace::new("non-increasing");
    write_valid_run(&non_increasing);
    let repeated = b"{\"index\":2,\"values\":{\"population\":[2.0],\"activity\":\"a\"}}\n{\"index\":2,\"values\":{\"population\":[3.0],\"activity\":\"b\"}}\n";
    replace_first_chunk(&non_increasing.run(), repeated, 2, 2, 2);
    assert!(matches!(
        SeriesReader::open(non_increasing.run(), decoders())
            .unwrap()
            .read("signal"),
        Err(StorageError::InvalidRecord { reason, .. }) if reason.contains("not greater")
    ));

    let missing = TempWorkspace::new("missing-chunk");
    write_valid_run(&missing);
    let missing_chunk = first_chunk(&missing.run());
    fs::remove_file(&missing_chunk).unwrap();
    assert!(matches!(
        SeriesReader::open(missing.run(), decoders())
            .unwrap()
            .read("signal"),
        Err(StorageError::MissingChunk { path }) if path == missing_chunk
    ));

    let size = TempWorkspace::new("size");
    write_valid_run(&size);
    let size_chunk = first_chunk(&size.run());
    let mut size_bytes = fs::read(&size_chunk).unwrap();
    size_bytes.push(b' ');
    fs::write(&size_chunk, size_bytes).unwrap();
    assert!(matches!(
        SeriesReader::open(size.run(), decoders())
            .unwrap()
            .read("signal"),
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
        SeriesReader::open(checksum.run(), decoders())
            .unwrap()
            .read("signal"),
        Err(StorageError::ChecksumMismatch { path, .. }) if path == checksum_chunk
    ));

    let unterminated = TempWorkspace::new("unterminated");
    write_valid_run(&unterminated);
    let bytes = b"{\"index\":2,\"values\":{\"population\":[2.0],\"activity\":\"a\"}}";
    replace_first_chunk(&unterminated.run(), bytes, 1, 2, 2);
    assert!(matches!(
        SeriesReader::open(unterminated.run(), decoders())
            .unwrap()
            .read("signal"),
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
        SeriesReader::open(version.run(), decoders()),
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
