//! End-to-end reconstruction tests for `storage/reader.rs`.
//!
//! Every successful fixture traverses the real forward and reverse path:
//! `SystemState` -> borrowed `JsonEncoder` -> bounded `StateWriter` -> JSONL
//! chunks and `metadata.json` -> per-key decoders -> `SeriesReader` ->
//! `StateSeries`. Filesystem mutations stay inside uniquely owned temporary
//! directories removed at test completion.

use std::error::Error as _;
use std::fs;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Map, json};

use crate::storage::decoder::{Decoders, StringDecoder, VecF64Decoder};
use crate::storage::encoder::JsonEncoder;
use crate::storage::error::StorageError;
use crate::storage::format::{FieldMetadata, RunMetadata, RunStatus, StreamMetadata, TimeAxis};
use crate::storage::reader::SeriesReader;
use crate::storage::writer::{StateWriter, WriterConfig};
use crate::system_state::{StateSpec, TimePoint};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Owns one exact run root and removes only that root on drop.
struct TempRun {
    root: PathBuf,
}

impl TempRun {
    /// Creates a collision-resistant empty run directory.
    fn new(label: &str) -> Self {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "scientific-workflow-reader-{label}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("unique reader test root must be creatable");
        Self { root }
    }

    /// Returns the authoritative metadata path.
    fn metadata(&self) -> PathBuf {
        self.root.join("metadata.json")
    }

    /// Returns the directory for the test's single logical stream.
    fn stream(&self) -> PathBuf {
        self.root.join("signal")
    }
}

impl Drop for TempRun {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.root) {
            eprintln!(
                "failed to remove reader test directory {}: {error}",
                self.root.display()
            );
        }
    }
}

/// Expected payloads for one persisted sample.
struct Sample {
    index: u64,
    physical: f64,
    values: Vec<f64>,
    label: String,
}

/// Parses the shared two-field schema through the real state-spec boundary.
fn sample_spec(source: impl Into<PathBuf>) -> StateSpec {
    StateSpec::parse(
        source.into(),
        &serde_json::to_vec(&json!({
            "fields": [
                {"name": "values", "description": "Sample vector"},
                {"name": "label", "description": "Sample label"}
            ]
        }))
        .unwrap(),
    )
    .expect("static reader test schema must be valid")
}

/// Returns the two approved default decoders bound to exact field keys.
fn default_decoders() -> Decoders {
    let mut decoders = Decoders::with_capacity(2);
    decoders
        .add::<Vec<f64>, _>("values", VecF64Decoder)
        .unwrap();
    decoders.add::<String, _>("label", StringDecoder).unwrap();
    decoders
}

/// Writes metadata after all chunks have been durably committed.
fn commit_metadata(
    run: &TempRun,
    status: RunStatus,
    chunks: Vec<crate::storage::format::ChunkMetadata>,
) {
    let mut metadata = RunMetadata::running(
        TimeAxis {
            index_name: "step".to_owned(),
            index_unit: Some("iteration".to_owned()),
            physical_name: Some("time".to_owned()),
            physical_unit: Some("s".to_owned()),
        },
        Map::new(),
        vec![StreamMetadata {
            name: "signal".to_owned(),
            directory: "signal".to_owned(),
            cadence: Some("selected simulation steps".to_owned()),
            fields: vec![
                FieldMetadata {
                    name: "values".to_owned(),
                    description: Some("Sample vector".to_owned()),
                },
                FieldMetadata {
                    name: "label".to_owned(),
                    description: Some("Sample label".to_owned()),
                },
            ],
            max_chunk_bytes: 100,
            queue_bytes: 16_384,
            chunks,
        }],
    );
    metadata.status = status;
    metadata
        .validate(&run.metadata())
        .expect("generated metadata must satisfy the storage contract");
    fs::write(
        run.metadata(),
        serde_json::to_vec_pretty(&metadata).unwrap(),
    )
    .expect("metadata fixture must be writable");
}

/// Persists samples through the real encoder and asynchronous writer.
fn write_samples(run: &TempRun, samples: &[Sample]) {
    let spec = sample_spec(run.metadata());
    let encoder = JsonEncoder::new("signal", &spec, ["values", "label"]).unwrap();
    let writer = StateWriter::start(
        WriterConfig::new(
            "signal",
            run.stream(),
            NonZeroU64::new(100).unwrap(),
            NonZeroU64::new(16_384).unwrap(),
        )
        .unwrap(),
    )
    .expect("reader fixture writer must start");

    for sample in samples {
        let mut state = spec.empty(
            TimePoint::from_physical(sample.index, sample.physical)
                .expect("test physical time must be finite"),
        );
        assert!(
            state
                .set("values", sample.values.clone())
                .unwrap()
                .is_none()
        );
        assert!(state.set("label", sample.label.clone()).unwrap().is_none());
        writer
            .submit(encoder.encode(&state).expect("sample must encode"))
            .expect("encoded sample must enter the bounded writer");
    }

    let summary = writer.finish().expect("reader fixture must commit");
    commit_metadata(run, RunStatus::Complete, summary.chunks().to_vec());
}

/// Creates the canonical three-sample round-trip fixture.
fn samples() -> Vec<Sample> {
    vec![
        Sample {
            index: 2,
            physical: 0.125,
            values: vec![1.0, 2.0, 3.0],
            label: "initial".to_owned(),
        },
        Sample {
            index: 5,
            physical: 0.5,
            values: vec![-4.5, 8.25],
            label: "middle\nstate".to_owned(),
        },
        Sample {
            index: 11,
            physical: 1.75,
            values: Vec::new(),
            label: "final 世界".to_owned(),
        },
    ]
}

#[test]
fn writer_output_round_trips_into_a_complete_typed_series() {
    let run = TempRun::new("round-trip");
    let expected = samples();
    write_samples(&run, &expected);

    let reader = SeriesReader::open(&run.root, default_decoders())
        .expect("completed generated run must open");
    let series = reader.read("signal").expect("stream must reconstruct");

    assert_eq!(reader.root(), run.root.as_path());
    assert_eq!(reader.streams().collect::<Vec<_>>(), vec!["signal"]);
    assert_eq!(series.len(), expected.len());
    assert_eq!(series.spec().fields()[0].name(), "values");
    assert_eq!(series.spec().fields()[1].name(), "label");
    assert_eq!(
        series.spec().fields()[0].description(),
        Some("Sample vector")
    );

    for (state, sample) in series.iter().zip(&expected) {
        assert_eq!(state.time().index(), sample.index);
        assert_eq!(state.time().physical(), Some(sample.physical));
        assert_eq!(state.get::<Vec<f64>>("values").unwrap(), &sample.values);
        assert_eq!(state.get::<String>("label").unwrap(), &sample.label);
    }

    let metadata_files = fs::read_dir(&run.root)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .count();
    assert_eq!(metadata_files, 1, "run root must contain one metadata JSON");
    assert!(fs::read_dir(run.stream()).unwrap().count() >= 2);
}

#[test]
fn read_all_preserves_stream_order_and_returns_independent_owned_results() {
    let run = TempRun::new("read-all");
    write_samples(&run, &samples());
    let reader = SeriesReader::open(&run.root, default_decoders()).unwrap();

    let all = reader.read_all().expect("all declared streams must decode");

    assert_eq!(all.len(), 1);
    assert_eq!(all[0].0, "signal");
    assert_eq!(all[0].1.len(), 3);
    assert!(format!("{reader:?}").contains("streams: 1"));
}

#[test]
fn reader_rejects_unknown_streams_and_missing_decoder_coverage_before_io() {
    let run = TempRun::new("coverage");
    write_samples(&run, &samples());
    let reader = SeriesReader::open(&run.root, Decoders::new()).unwrap();

    assert!(matches!(
        reader.read("absent"),
        Err(StorageError::UnknownStream { stream }) if stream == "absent"
    ));
    assert!(matches!(
        reader.read("signal"),
        Err(StorageError::MissingDecoder { field }) if field == "values"
    ));
}

#[test]
fn reader_rejects_metadata_that_does_not_declare_completion() {
    let run = TempRun::new("incomplete");
    fs::create_dir(run.stream()).unwrap();
    commit_metadata(&run, RunStatus::Running, Vec::new());

    assert!(matches!(
        SeriesReader::open(&run.root, default_decoders()),
        Err(StorageError::RunIncomplete { path }) if path == run.metadata()
    ));
}

#[test]
fn reader_preserves_decoder_context_for_a_key_with_the_wrong_payload_kind() {
    let run = TempRun::new("wrong-kind");
    let spec = sample_spec(run.metadata());
    let encoder = JsonEncoder::new("signal", &spec, ["values", "label"]).unwrap();
    let writer = StateWriter::start(
        WriterConfig::new(
            "signal",
            run.stream(),
            NonZeroU64::new(1_024).unwrap(),
            NonZeroU64::new(4_096).unwrap(),
        )
        .unwrap(),
    )
    .unwrap();
    let mut state = spec.empty(TimePoint::new(7));
    assert!(
        state
            .set("values", "not a vector".to_owned())
            .unwrap()
            .is_none()
    );
    assert!(
        state
            .set("label", "valid label".to_owned())
            .unwrap()
            .is_none()
    );
    writer.submit(encoder.encode(&state).unwrap()).unwrap();
    let summary = writer.finish().unwrap();
    commit_metadata(&run, RunStatus::Complete, summary.chunks().to_vec());
    let reader = SeriesReader::open(&run.root, default_decoders()).unwrap();

    let error = reader
        .read("signal")
        .expect_err("String cannot reconstruct as Vec<f64>");

    assert!(matches!(
        &error,
        StorageError::DecodeField {
            stream,
            index: 7,
            field,
            ..
        } if stream == "signal" && field == "values"
    ));
    assert!(error.source().unwrap().is::<serde_json::Error>());
}

#[test]
fn reader_detects_same_length_chunk_corruption_by_checksum() {
    let run = TempRun::new("checksum");
    write_samples(&run, &samples());
    let chunk = run.stream().join("chunk-000000.jsonl");
    let mut bytes = fs::read(&chunk).unwrap();
    let digit = bytes
        .iter_mut()
        .find(|byte| **byte == b'1')
        .expect("fixture must contain a replaceable digit");
    *digit = b'9';
    fs::write(&chunk, bytes).unwrap();
    let reader = SeriesReader::open(&run.root, default_decoders()).unwrap();

    assert!(matches!(
        reader.read("signal"),
        Err(StorageError::ChecksumMismatch { path, .. }) if path == chunk
    ));
}

#[test]
fn reader_detects_chunk_size_changes_before_payload_decoding() {
    let run = TempRun::new("size");
    write_samples(&run, &samples());
    let chunk = run.stream().join("chunk-000000.jsonl");
    let mut bytes = fs::read(&chunk).unwrap();
    bytes.push(b' ');
    fs::write(&chunk, bytes).unwrap();
    let reader = SeriesReader::open(&run.root, default_decoders()).unwrap();

    assert!(matches!(
        reader.read("signal"),
        Err(StorageError::ChunkSizeMismatch { path, .. }) if path == chunk
    ));
}

#[test]
fn reader_detects_missing_committed_chunks() {
    let run = TempRun::new("missing");
    write_samples(&run, &samples());
    let chunk = run.stream().join("chunk-000000.jsonl");
    fs::remove_file(&chunk).unwrap();
    let reader = SeriesReader::open(&run.root, default_decoders()).unwrap();

    assert!(matches!(
        reader.read("signal"),
        Err(StorageError::MissingChunk { path }) if path == chunk
    ));
}

#[test]
fn metadata_is_real_json_and_round_trips_semantically_before_reading() {
    let run = TempRun::new("metadata-json");
    write_samples(&run, &samples());
    let bytes = fs::read(run.metadata()).unwrap();
    let parsed: RunMetadata = serde_json::from_slice(&bytes).unwrap();
    let reparsed: RunMetadata =
        serde_json::from_slice(&serde_json::to_vec(&parsed).unwrap()).unwrap();

    assert_eq!(parsed, reparsed);
    assert_eq!(parsed.status, RunStatus::Complete);
    assert_eq!(parsed.streams.len(), 1);
    assert_eq!(parsed.streams[0].fields.len(), 2);
}

#[test]
fn reader_root_is_preserved_without_canonicalization() {
    let run = TempRun::new("root");
    write_samples(&run, &samples());
    let root: &Path = &run.root;
    let reader = SeriesReader::open(root, default_decoders()).unwrap();

    assert_eq!(reader.root(), root);
}
