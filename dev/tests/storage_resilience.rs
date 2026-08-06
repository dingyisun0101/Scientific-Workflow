//! Logged fault-injection workflow for storage resilience.

use std::error::Error as _;
use std::fs;
use std::num::NonZeroU64;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Serialize, Serializer};
use serde_json::Map;

#[allow(dead_code)]
mod system_state {
    #[path = "../../src/system_state/error.rs"]
    mod error;
    #[path = "../../src/system_state/spec.rs"]
    mod spec;
    #[path = "../../src/system_state/state.rs"]
    mod state;
    #[path = "../../src/system_state/value.rs"]
    mod value;

    pub use error::StateError;
    pub use spec::StateSpec;
    pub use state::{SystemState, TimePoint};
}

#[allow(dead_code)]
mod time_series {
    #[path = "../../src/time_series/error.rs"]
    mod error;
    #[path = "../../src/time_series/series.rs"]
    mod series;

    pub use error::SeriesError;
    pub use series::StateSeries;
}

#[allow(dead_code)]
mod storage {
    #[path = "../../src/storage/decoder.rs"]
    pub mod decoder;
    #[path = "../../src/storage/encoder.rs"]
    pub mod encoder;
    #[path = "../../src/storage/error.rs"]
    pub mod error;
    #[path = "../../src/storage/format.rs"]
    pub mod format;
    #[path = "../../src/storage/reader.rs"]
    pub mod reader;
    #[path = "../../src/storage/writer.rs"]
    pub mod writer;
}

use storage::decoder::{Decoders, StringDecoder, VecF64Decoder};
use storage::encoder::JsonEncoder;
use storage::error::StorageError;
use storage::format::{
    EncodedRecord, FieldMetadata, RunMetadata, RunStatus, StreamMetadata, TimeAxis,
};
use storage::reader::SeriesReader;
use storage::writer::{StateWriter, WriterConfig};
use system_state::{StateSpec, TimePoint};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Owns one exact run root and removes only that root on drop.
struct TempRun(PathBuf);

impl TempRun {
    fn new(label: &str) -> Self {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "scientific-workflow-resilience-{label}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("unique resilience root must be creatable");
        Self(root)
    }

    fn metadata(&self) -> PathBuf {
        self.0.join("metadata.json")
    }

    fn stream(&self) -> PathBuf {
        self.0.join("signal")
    }
}

impl Drop for TempRun {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0) {
            eprintln!("[cleanup] failed to remove {}: {error}", self.0.display());
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

fn sample_spec(source: impl Into<PathBuf>) -> StateSpec {
    StateSpec::parse(
        source.into(),
        br#"{"fields":[{"name":"values","description":"Sample vector"},{"name":"label","description":"Sample label"}]}"#,
    )
    .expect("resilience schema must parse")
}

fn default_decoders() -> Decoders {
    let mut decoders = Decoders::with_capacity(2);
    decoders
        .add::<Vec<f64>, _>("values", VecF64Decoder)
        .unwrap();
    decoders.add::<String, _>("label", StringDecoder).unwrap();
    decoders
}

fn metadata_with(run: &TempRun, status: RunStatus, chunks: Vec<storage::format::ChunkMetadata>) {
    let mut metadata = RunMetadata::running(
        TimeAxis {
            index_name: "step".to_owned(),
            index_unit: None,
            physical_name: Some("time".to_owned()),
            physical_unit: Some("s".to_owned()),
        },
        Map::new(),
        vec![StreamMetadata {
            name: "signal".to_owned(),
            directory: "signal".to_owned(),
            cadence: Some("selected steps".to_owned()),
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
            max_chunk_bytes: 128,
            queue_bytes: 4_096,
            chunks,
        }],
    );
    metadata.status = status;
    metadata.validate(&run.metadata()).unwrap();
    fs::write(
        run.metadata(),
        serde_json::to_vec_pretty(&metadata).unwrap(),
    )
    .unwrap();
}

fn start_writer(run: &TempRun, queue_bytes: u64) -> StateWriter {
    StateWriter::start(
        WriterConfig::new(
            "signal",
            run.stream(),
            NonZeroU64::new(128).unwrap(),
            NonZeroU64::new(queue_bytes).unwrap(),
        )
        .unwrap(),
    )
    .unwrap()
}

fn valid_record(index: u64) -> EncodedRecord {
    EncodedRecord::new(
        TimePoint::new(index),
        format!(
            r#"{{"index":{index},"values":{{"values":[{index}.0],"label":"sample-{index}"}}}}"#
        )
        .into_bytes(),
    )
}

fn write_valid_run(run: &TempRun) {
    let writer = start_writer(run, 4_096);
    writer.submit(valid_record(2)).unwrap();
    writer.submit(valid_record(5)).unwrap();
    let summary = writer.finish().unwrap();
    metadata_with(run, RunStatus::Complete, summary.chunks().to_vec());
}

fn write_single_raw(run: &TempRun, index: u64, json: &[u8]) {
    let writer = start_writer(run, 4_096);
    writer
        .submit(EncodedRecord::new(TimePoint::new(index), json.to_vec()))
        .unwrap();
    let summary = writer.finish().unwrap();
    metadata_with(run, RunStatus::Complete, summary.chunks().to_vec());
}

#[test]
fn storage_failures_are_detected_with_context_and_without_partial_success() {
    assert!(matches!(
        WriterConfig::new(
            " ",
            "signal",
            NonZeroU64::new(1).unwrap(),
            NonZeroU64::new(1).unwrap()
        ),
        Err(StorageError::InvalidConfig { .. })
    ));

    let existing = TempRun::new("existing");
    fs::create_dir(existing.stream()).unwrap();
    let existing_config = WriterConfig::new(
        "signal",
        existing.stream(),
        NonZeroU64::new(128).unwrap(),
        NonZeroU64::new(4_096).unwrap(),
    )
    .unwrap();
    assert!(matches!(
        StateWriter::start(existing_config),
        Err(StorageError::OutputExists { path }) if path == existing.stream()
    ));

    let oversized = TempRun::new("oversized");
    let oversized_writer = start_writer(&oversized, 32);
    let oversized_record = valid_record(1);
    let oversized_bytes = oversized_record.len() as u64;
    assert!(oversized_bytes > 32);
    assert!(matches!(
        oversized_writer.submit(oversized_record),
        Err(StorageError::RecordTooLarge {
            bytes,
            limit: 32,
            ..
        }) if bytes == oversized_bytes
    ));
    assert_eq!(oversized_writer.finish().unwrap().records(), 0);

    let ordering = TempRun::new("ordering");
    let ordering_writer = start_writer(&ordering, 4_096);
    ordering_writer.submit(valid_record(5)).unwrap();
    assert!(matches!(
        ordering_writer.submit(valid_record(5)),
        Err(StorageError::OutOfOrderRecord {
            index: 5,
            previous: 5,
            ..
        })
    ));
    assert!(matches!(
        ordering_writer.submit(valid_record(4)),
        Err(StorageError::OutOfOrderRecord {
            index: 4,
            previous: 5,
            ..
        })
    ));
    assert_eq!(ordering_writer.finish().unwrap().records(), 1);

    let terminal = TempRun::new("terminal");
    let terminal_writer = start_writer(&terminal, 4_096);
    fs::remove_dir(terminal.stream()).unwrap();
    terminal_writer.submit(valid_record(1)).unwrap();
    assert!(matches!(
        terminal_writer.finish(),
        Err(StorageError::WriterTerminated { .. })
    ));
    println!(
        "[backpressure] oversized_rejected=true ordering_rejected=true terminal_propagated=true"
    );

    let mut decoder_config = Decoders::new();
    assert!(matches!(
        decoder_config.add::<String, _>("", StringDecoder),
        Err(StorageError::InvalidConfig { .. })
    ));
    decoder_config
        .add::<String, _>("label", StringDecoder)
        .unwrap();
    assert!(matches!(
        decoder_config.add::<String, _>("label", StringDecoder),
        Err(StorageError::DuplicateDecoder { field }) if field == "label"
    ));

    let encoding = TempRun::new("encoding");
    let spec = sample_spec(encoding.metadata());
    let encoder = JsonEncoder::new("signal", &spec, ["values", "label"]).unwrap();
    let mut state = spec.empty(TimePoint::new(3));
    assert!(matches!(
        encoder.encode(&state),
        Err(StorageError::StateAccess { index: 3, .. })
    ));
    assert!(state.set("values", RejectEncoding).unwrap().is_none());
    assert!(state.set("label", String::from("valid")).unwrap().is_none());
    let encode_error = encoder
        .encode(&state)
        .expect_err("payload serializer must preserve field context");
    assert!(matches!(
        &encode_error,
        StorageError::EncodeField {
            stream,
            index: 3,
            field,
            ..
        } if stream == "signal" && field == "values"
    ));
    assert_eq!(
        encode_error.source().unwrap().to_string(),
        "deliberate resilience failure"
    );

    let incomplete = TempRun::new("incomplete");
    fs::create_dir(incomplete.stream()).unwrap();
    metadata_with(&incomplete, RunStatus::Running, Vec::new());
    assert!(matches!(
        SeriesReader::open(&incomplete.0, default_decoders()),
        Err(StorageError::RunIncomplete { path }) if path == incomplete.metadata()
    ));

    let coverage = TempRun::new("coverage");
    write_valid_run(&coverage);
    let reader = SeriesReader::open(&coverage.0, Decoders::new()).unwrap();
    assert!(matches!(
        reader.read("absent"),
        Err(StorageError::UnknownStream { stream }) if stream == "absent"
    ));
    assert!(matches!(
        reader.read("signal"),
        Err(StorageError::MissingDecoder { field }) if field == "values"
    ));

    let wrong_type = TempRun::new("wrong-type");
    write_single_raw(
        &wrong_type,
        7,
        br#"{"index":7,"values":{"values":"not a vector","label":"valid"}}"#,
    );
    let reader = SeriesReader::open(&wrong_type.0, default_decoders()).unwrap();
    let decode_error = reader.read("signal").unwrap_err();
    assert!(matches!(
        &decode_error,
        StorageError::DecodeField {
            stream,
            index: 7,
            field,
            ..
        } if stream == "signal" && field == "values"
    ));
    assert!(decode_error.source().unwrap().is::<serde_json::Error>());
    println!("[decoder] missing=true wrong_type=true source_preserved=true");

    let malformed = TempRun::new("malformed");
    write_single_raw(&malformed, 1, b"{");
    let malformed_reader = SeriesReader::open(&malformed.0, default_decoders()).unwrap();
    assert!(matches!(
        malformed_reader.read("signal"),
        Err(StorageError::InvalidRecord { line: 1, .. })
    ));

    let missing_field = TempRun::new("missing-field");
    write_single_raw(
        &missing_field,
        1,
        br#"{"index":1,"values":{"values":[1.0]}}"#,
    );
    let missing_field_reader = SeriesReader::open(&missing_field.0, default_decoders()).unwrap();
    assert!(matches!(
        missing_field_reader.read("signal"),
        Err(StorageError::InvalidRecord { reason, .. }) if reason.contains("missing payload field `label`")
    ));

    let missing = TempRun::new("missing-chunk");
    write_valid_run(&missing);
    let missing_chunk = missing.stream().join("chunk-000000.jsonl");
    fs::remove_file(&missing_chunk).unwrap();
    let missing_reader = SeriesReader::open(&missing.0, default_decoders()).unwrap();
    assert!(matches!(
        missing_reader.read("signal"),
        Err(StorageError::MissingChunk { path }) if path == missing_chunk
    ));

    let size = TempRun::new("size");
    write_valid_run(&size);
    let size_chunk = size.stream().join("chunk-000000.jsonl");
    let mut size_bytes = fs::read(&size_chunk).unwrap();
    size_bytes.push(b' ');
    fs::write(&size_chunk, size_bytes).unwrap();
    let size_reader = SeriesReader::open(&size.0, default_decoders()).unwrap();
    assert!(matches!(
        size_reader.read("signal"),
        Err(StorageError::ChunkSizeMismatch { path, .. }) if path == size_chunk
    ));

    let checksum = TempRun::new("checksum");
    write_valid_run(&checksum);
    let checksum_chunk = checksum.stream().join("chunk-000000.jsonl");
    let mut checksum_bytes = fs::read(&checksum_chunk).unwrap();
    let position = checksum_bytes
        .iter()
        .rposition(|byte| *byte == b'5')
        .expect("fixture must contain a label digit");
    checksum_bytes[position] = b'6';
    fs::write(&checksum_chunk, checksum_bytes).unwrap();
    let checksum_reader = SeriesReader::open(&checksum.0, default_decoders()).unwrap();
    assert!(matches!(
        checksum_reader.read("signal"),
        Err(StorageError::ChecksumMismatch { path, .. }) if path == checksum_chunk
    ));

    println!("[integrity] missing=true size=true checksum=true record=true");
    println!(
        "[expected-error] families=configuration,writer,decoder,record,integrity context_verified=true"
    );
    println!("[result] storage_resilience=passed");
}
