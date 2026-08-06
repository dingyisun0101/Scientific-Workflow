//! Unified executable workflow for every implemented crate layer.
//!
//! Focused suites remain under `tests/storage/` and are included in this Cargo
//! target. The final test is deliberately demonstrative: it loads the checked-
//! in template, evolves real `physics_in_parallel` tensors in one live state,
//! retains explicit analysis snapshots, samples two logical streams at
//! different cadences, encodes without cloning, writes bounded byte-targeted
//! chunks, commits the sole metadata document, and reconstructs typed analysis
//! series through per-key decoders and the all-in-one reader.
//!
//! Run with `--nocapture` to display its concise execution log:
//!
//! ```text
//! cargo test --test storage -- --nocapture
//! ```

#![allow(
    clippy::duplicate_mod,
    reason = "focused staged suites intentionally include the same private production modules"
)]

#[path = "storage/decoder.rs"]
mod decoder_tests;
#[path = "storage/encoder.rs"]
mod encoder_tests;
#[path = "storage/error.rs"]
mod error_tests;
#[path = "storage/format.rs"]
mod format_tests;
#[path = "storage/reader.rs"]
mod reader_tests;
#[path = "storage/writer.rs"]
mod writer_tests;

use std::fs;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use physics_in_parallel::math::{Dense, Tensor};
use serde_json::{Map, Value, json};

// Storage is not exported from lib.rs during staged review. Compiling the real
// source modules here preserves crate-private visibility between SystemState
// and JsonEncoder without broadening the public API merely for a test.
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

use storage::decoder::Decoders;
use storage::encoder::JsonEncoder;
use storage::format::{FieldMetadata, RunMetadata, RunStatus, StreamMetadata, TimeAxis};
use storage::reader::SeriesReader;
use storage::writer::{StateWriter, WriterConfig, WriterSummary};
use system_state::{StateSpec, TimePoint};
use time_series::StateSeries;

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Removes one precisely owned integration directory after the test.
struct TempRun(PathBuf);

impl TempRun {
    /// Creates a collision-resistant run root beneath the platform temp root.
    fn new() -> Self {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "scientific-workflow-storage-integration-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("the unique integration root must be created");
        Self(path)
    }
}

impl Drop for TempRun {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0) {
            eprintln!(
                "[cleanup] failed to remove integration directory {}: {error}",
                self.0.display()
            );
        }
    }
}

/// Resolves the canonical state template independently of the process cwd.
fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/state.json")
}

/// Builds persisted field declarations directly from validated state metadata.
fn stream_fields(encoder: &JsonEncoder) -> Vec<FieldMetadata> {
    encoder
        .fields()
        .map(|name| {
            let field = encoder
                .spec()
                .get(name)
                .expect("encoder fields were validated against this specification");
            FieldMetadata {
                name: name.to_owned(),
                description: field.description().map(str::to_owned),
            }
        })
        .collect()
}

/// Writes the authoritative metadata file and returns its exact byte length.
fn write_metadata(path: &Path, metadata: &RunMetadata) -> u64 {
    metadata
        .validate(path)
        .expect("metadata must be valid before every commit");
    let bytes = serde_json::to_vec_pretty(metadata).expect("metadata must serialize");
    fs::write(path, &bytes).expect("metadata commit must be writable");
    bytes.len() as u64
}

/// Reads every complete record named by one stream's committed inventory.
fn read_records(root: &Path, stream: &StreamMetadata) -> Vec<Value> {
    stream
        .chunks
        .iter()
        .flat_map(|chunk| {
            let path = root.join(&stream.directory).join(&chunk.file);
            let bytes = fs::read(&path).expect("committed chunk must be readable");
            assert_eq!(bytes.len() as u64, chunk.bytes);
            bytes
                .split(|byte| *byte == b'\n')
                .filter(|line| !line.is_empty())
                .map(|line| {
                    serde_json::from_slice::<Value>(line)
                        .expect("every complete JSONL record must parse")
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Prints stable, bounded completion facts without dumping scientific payloads.
fn log_summary(summary: &WriterSummary) {
    println!(
        "[writer:{}] records={} chunks={} bytes={}",
        summary.stream(),
        summary.records(),
        summary.chunks().len(),
        summary.bytes()
    );
    for chunk in summary.chunks() {
        let checksum_prefix = &chunk.checksum[..chunk.checksum.len().min(23)];
        println!(
            "[chunk:{}] file={} records={} bytes={} indices={}..={} checksum={}...",
            summary.stream(),
            chunk.file,
            chunk.records,
            chunk.bytes,
            chunk.first_index,
            chunk.last_index,
            checksum_prefix
        );
    }
}

#[test]
fn complete_scientific_workflow_is_consistent_and_observable() {
    const SIGNAL_CHUNK_BYTES: u64 = 256;
    const SPACE_CHUNK_BYTES: u64 = 128;
    const QUEUE_BYTES: u64 = 1_048_576;

    let run = TempRun::new();
    let metadata_path = run.0.join("metadata.json");
    let template_path = fixture_path();
    let spec = StateSpec::load(&template_path).expect("the canonical template must load");
    let template_json: Value = serde_json::from_str(&spec.to_json().unwrap()).unwrap();
    let fixture_json: Value = serde_json::from_slice(&fs::read(&template_path).unwrap()).unwrap();
    assert_eq!(template_json, fixture_json);
    println!(
        "[setup] template={} fields={:?}",
        template_path.display(),
        spec.fields()
            .iter()
            .map(|field| field.name())
            .collect::<Vec<_>>()
    );

    let signal_encoder = JsonEncoder::new("signal", &spec, ["activity", "population"]).unwrap();
    let space_encoder = JsonEncoder::new("space", &spec, ["space"]).unwrap();
    assert_eq!(
        signal_encoder.fields().collect::<Vec<_>>(),
        vec!["population", "activity"]
    );
    println!(
        "[setup] streams=signal{:?}, space{:?}; queue_bytes={QUEUE_BYTES}",
        signal_encoder.fields().collect::<Vec<_>>(),
        space_encoder.fields().collect::<Vec<_>>()
    );

    let mut population = Tensor::<u64, Dense>::zeros(&[3]);
    population.set(&[0], 10);
    population.set(&[1], 20);
    population.set(&[2], 30);
    let mut space = Tensor::<u64, Dense>::zeros(&[2, 2]);
    space.set(&[0, 0], 1);
    space.set(&[0, 1], 2);
    space.set(&[1, 0], 3);
    space.set(&[1, 1], 4);
    let mut activity = Tensor::<u8, Dense>::zeros(&[3]);
    activity.set(&[0], 1);
    activity.set(&[1], 0);
    activity.set(&[2], 1);

    let mut live = spec.empty(TimePoint::from_physical(0, 0.0).unwrap());
    assert!(live.set("population", population).unwrap().is_none());
    assert!(live.set("space", space).unwrap().is_none());
    assert!(live.set("activity", activity).unwrap().is_none());
    assert_eq!(live.loaded(), spec.len());
    assert!(live.spec().shares_layout(&spec));
    let blank = live.empty(TimePoint::new(999));
    assert!(blank.is_blank());
    assert!(blank.spec().shares_layout(&spec));
    println!(
        "[state] initialized index={} physical={:?} loaded={}/{}",
        live.time().index(),
        live.time().physical(),
        live.loaded(),
        live.len()
    );

    let signal_metadata = StreamMetadata {
        name: "signal".to_owned(),
        directory: "signal".to_owned(),
        cadence: Some("every simulation step".to_owned()),
        fields: stream_fields(&signal_encoder),
        max_chunk_bytes: SIGNAL_CHUNK_BYTES,
        queue_bytes: QUEUE_BYTES,
        chunks: Vec::new(),
    };
    let space_metadata = StreamMetadata {
        name: "space".to_owned(),
        directory: "space".to_owned(),
        cadence: Some("every two simulation steps".to_owned()),
        fields: stream_fields(&space_encoder),
        max_chunk_bytes: SPACE_CHUNK_BYTES,
        queue_bytes: QUEUE_BYTES,
        chunks: Vec::new(),
    };
    let mut run_attributes = Map::new();
    run_attributes.insert("seed".to_owned(), Value::from(42));
    run_attributes.insert("program".to_owned(), Value::from("integration-demo"));
    let mut metadata = RunMetadata::running(
        TimeAxis {
            index_name: "simulation_step".to_owned(),
            index_unit: Some("step".to_owned()),
            physical_name: Some("time".to_owned()),
            physical_unit: Some("s".to_owned()),
        },
        run_attributes,
        vec![signal_metadata, space_metadata],
    );
    let initial_metadata_bytes = write_metadata(&metadata_path, &metadata);
    println!(
        "[metadata] initialized path={} bytes={} status=running streams={}",
        metadata_path.display(),
        initial_metadata_bytes,
        metadata.streams.len()
    );

    let signal_writer = StateWriter::start(
        WriterConfig::new(
            "signal",
            run.0.join("signal"),
            NonZeroU64::new(SIGNAL_CHUNK_BYTES).unwrap(),
            NonZeroU64::new(QUEUE_BYTES).unwrap(),
        )
        .unwrap(),
    )
    .expect("signal writer must start");
    let space_writer = StateWriter::start(
        WriterConfig::new(
            "space",
            run.0.join("space"),
            NonZeroU64::new(SPACE_CHUNK_BYTES).unwrap(),
            NonZeroU64::new(QUEUE_BYTES).unwrap(),
        )
        .unwrap(),
    )
    .expect("space writer must start");

    let mut analysis = StateSeries::with_capacity(spec.clone(), 4);
    for sample in 0..4_u64 {
        assert_eq!(live.time().index(), sample);
        let signal_record = signal_encoder
            .encode(&live)
            .expect("signal payloads must encode by borrow");
        let signal_bytes = signal_record.len();
        signal_writer
            .submit(signal_record)
            .expect("signal record must be admitted");

        let mut space_bytes = None;
        if sample % 2 == 0 {
            let record = space_encoder
                .encode(&live)
                .expect("space payload must encode by borrow");
            space_bytes = Some(record.len());
            space_writer
                .submit(record)
                .expect("space record must be admitted");
        }

        // Deep cloning is explicit and used only because this test also builds
        // the analysis-oriented in-memory series. The hot persistence path
        // above encoded the live state directly without this clone.
        analysis
            .push(live.clone())
            .expect("increasing snapshots with shared layout must append");
        println!(
            "[sample] index={} physical={:.2} signal_bytes={} space_bytes={}",
            live.time().index(),
            live.time().physical().unwrap(),
            signal_bytes,
            space_bytes.map_or_else(|| "skipped".to_owned(), |bytes| bytes.to_string())
        );

        if sample < 3 {
            let population = live
                .get_mut::<Tensor<u64, Dense>>("population")
                .expect("population tensor must remain mutable");
            population.set(&[0], population.get(&[0]) + 1);
            let space = live
                .get_mut::<Tensor<u64, Dense>>("space")
                .expect("space tensor must remain mutable");
            space.set(&[0, 0], space.get(&[0, 0]) + 10);
            let activity = live
                .get_mut::<Tensor<u8, Dense>>("activity")
                .expect("activity tensor must remain mutable");
            activity.set(&[1], (sample as u8 + 1) % 2);
            live.advance(Some(0.25))
                .expect("simulation and physical time must advance");
        }
    }

    let signal_summary = signal_writer.finish().expect("signal output must finish");
    let space_summary = space_writer.finish().expect("space output must finish");
    log_summary(&signal_summary);
    log_summary(&space_summary);
    assert_eq!(signal_summary.records(), 4);
    assert_eq!(space_summary.records(), 2);
    assert!(signal_summary.chunks().len() >= 2);
    assert!(space_summary.chunks().len() >= 2);

    metadata
        .stream_mut("signal")
        .unwrap()
        .chunks
        .clone_from(&signal_summary.chunks().to_vec());
    metadata
        .stream_mut("space")
        .unwrap()
        .chunks
        .clone_from(&space_summary.chunks().to_vec());
    metadata.status = RunStatus::Complete;
    let final_metadata_bytes = write_metadata(&metadata_path, &metadata);
    println!(
        "[metadata] committed bytes={} status=complete signal_chunks={} space_chunks={}",
        final_metadata_bytes,
        signal_summary.chunks().len(),
        space_summary.chunks().len()
    );

    let restored: RunMetadata =
        serde_json::from_slice(&fs::read(&metadata_path).expect("metadata must be readable"))
            .expect("metadata must parse");
    restored
        .validate(&metadata_path)
        .expect("restored metadata must remain valid");
    assert_eq!(restored, metadata);
    assert_eq!(restored.stream("signal").unwrap().queue_bytes, QUEUE_BYTES);

    let signal_records = read_records(&run.0, restored.stream("signal").unwrap());
    let space_records = read_records(&run.0, restored.stream("space").unwrap());
    assert_eq!(
        signal_records
            .iter()
            .map(|record| record["index"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![0, 1, 2, 3]
    );
    assert_eq!(
        space_records
            .iter()
            .map(|record| record["index"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![0, 2]
    );
    assert_eq!(
        signal_records[0]["values"]["population"]["data"],
        json!([10, 20, 30])
    );
    assert_eq!(
        signal_records[3]["values"]["population"]["data"],
        json!([13, 20, 30])
    );
    assert_eq!(
        space_records[1]["values"]["space"]["data"],
        json!([21, 2, 3, 4])
    );

    assert_eq!(analysis.len(), 4);
    assert_eq!(analysis.view().first().unwrap().time().index(), 0);
    assert_eq!(analysis.view().last().unwrap().time().index(), 3);
    assert_eq!(
        analysis
            .first()
            .unwrap()
            .get::<Tensor<u64, Dense>>("population")
            .unwrap()
            .get(&[0]),
        10
    );
    analysis
        .field_mut::<Tensor<u64, Dense>>(0, "population")
        .unwrap()
        .set(&[0], 999);
    assert_eq!(
        analysis
            .first()
            .unwrap()
            .get::<Tensor<u64, Dense>>("population")
            .unwrap()
            .get(&[0]),
        999
    );
    assert_eq!(signal_records[0]["values"]["population"]["data"][0], 10);

    let mut decoders = Decoders::with_capacity(3);
    decoders
        .add::<Tensor<u64, Dense>, _>("population", |raw: &str| {
            serde_json::from_str::<Tensor<u64, Dense>>(raw)
        })
        .unwrap();
    decoders
        .add::<Tensor<u64, Dense>, _>("space", |raw: &str| {
            serde_json::from_str::<Tensor<u64, Dense>>(raw)
        })
        .unwrap();
    decoders
        .add::<Tensor<u8, Dense>, _>("activity", |raw: &str| {
            serde_json::from_str::<Tensor<u8, Dense>>(raw)
        })
        .unwrap();
    let reader = SeriesReader::open(&run.0, decoders).expect("completed run must open");
    let decoded_signal = reader
        .read("signal")
        .expect("custom tensor decoders must reconstruct signal");
    let decoded_space = reader
        .read("space")
        .expect("custom tensor decoder must reconstruct space");
    assert_eq!(decoded_signal.len(), 4);
    assert_eq!(decoded_space.len(), 2);
    assert_eq!(
        decoded_signal
            .last()
            .unwrap()
            .get::<Tensor<u64, Dense>>("population")
            .unwrap()
            .get(&[0]),
        13
    );
    assert_eq!(
        decoded_space
            .last()
            .unwrap()
            .get::<Tensor<u64, Dense>>("space")
            .unwrap()
            .get(&[0, 0]),
        21
    );

    println!(
        "[readback] signal_indices={:?} space_indices={:?} analysis_states={} decoded_signal={} decoded_space={}",
        signal_records
            .iter()
            .map(|record| record["index"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        space_records
            .iter()
            .map(|record| record["index"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        analysis.len(),
        decoded_signal.len(),
        decoded_space.len()
    );
    println!(
        "[result] forward workflow verified; output={} (removed after test)",
        run.0.display()
    );
}
