//! Unified staged integration target for storage and all earlier modules.
//!
//! Focused storage suites remain in `tests/storage/`. This Cargo-discovered
//! target runs those suites and adds one workflow spanning the public state and
//! series APIs, storage errors and format types, asynchronous writing, chunk
//! descriptors, the sole metadata document, and raw JSONL reconstruction.

#![allow(
    clippy::duplicate_mod,
    reason = "focused staged suites intentionally include the same private production modules"
)]

#[path = "storage/error.rs"]
mod error_tests;
#[path = "storage/format.rs"]
mod format_tests;
#[path = "storage/writer.rs"]
mod writer_tests;

use std::fs;
use std::num::NonZeroU64;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Map, Value, json};

use scientific_workflow::system_state::{StateSpec, TimePoint};
use scientific_workflow::time_series::StateSeries;

mod system_state {
    pub use scientific_workflow::system_state::*;
}

mod time_series {
    pub use scientific_workflow::time_series::*;
}

#[allow(dead_code)]
#[path = "../src/storage/error.rs"]
mod error;
#[allow(dead_code)]
#[path = "../src/storage/format.rs"]
mod format;
#[allow(dead_code)]
#[path = "../src/storage/writer.rs"]
mod writer;

use format::{
    EncodedRecord, FieldMetadata, RawRecord, RunMetadata, RunStatus, StreamMetadata, TimeAxis,
};
use writer::{StateWriter, WriterConfig};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Removes one precisely owned integration directory after the test.
struct TempRun(PathBuf);

impl TempRun {
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
                "failed to remove integration directory {}: {error}",
                self.0.display()
            );
        }
    }
}

/// Resolves the canonical state template independently of the process cwd.
fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/state.json")
}

#[test]
fn state_series_format_and_writer_form_one_consistent_storage_workflow() {
    let run = TempRun::new();
    let stream_directory = run.0.join("signal");
    let spec = StateSpec::load(fixture_path()).expect("the canonical state template must load");
    let mut series = StateSeries::with_capacity(spec.clone(), 3);

    for (index, physical, population) in [
        (0, 0.0, vec![2_u64, 3, 5]),
        (4, 0.5, vec![7_u64, 11]),
        (9, 1.0, vec![13_u64, 17, 19]),
    ] {
        let time = TimePoint::from_physical(index, physical).expect("physical time must be finite");
        let mut state = spec.empty(time);
        assert!(
            state
                .set("population", population)
                .expect("declared field must accept its payload")
                .is_none()
        );
        series
            .push(state)
            .expect("increasing canonical-layout states must append");
    }

    let encoded: Vec<EncodedRecord> = series
        .iter()
        .map(|state| {
            let population = state
                .get::<Vec<u64>>("population")
                .expect("the sampled payload must remain typed");
            let document = json!({
                "index": state.time().index(),
                "physical": state.time().physical(),
                "values": {"population": population},
            });
            EncodedRecord::new(
                state.time(),
                serde_json::to_vec(&document).expect("representative record must encode"),
            )
        })
        .collect();
    let first_two_bytes = (encoded[0].len() + encoded[1].len()) as u64;
    let writer = StateWriter::start(
        WriterConfig::new(
            "signal",
            &stream_directory,
            NonZeroU64::new(first_two_bytes).unwrap(),
            NonZeroU64::new(1_048_576).unwrap(),
        )
        .expect("writer configuration must be valid"),
    )
    .expect("writer must start");
    for record in encoded {
        writer.submit(record).expect("record must be admitted");
    }
    let summary = writer.finish().expect("all admitted records must commit");

    let fields = vec![FieldMetadata {
        name: "population".to_owned(),
        description: spec
            .get("population")
            .expect("field must be declared")
            .description()
            .map(str::to_owned),
    }];
    let stream = StreamMetadata {
        name: "signal".to_owned(),
        directory: "signal".to_owned(),
        cadence: Some("selected simulation indices".to_owned()),
        fields,
        max_chunk_bytes: first_two_bytes,
        queue_bytes: 1_048_576,
        chunks: summary.chunks().to_vec(),
    };
    let mut run_attributes = Map::new();
    run_attributes.insert("seed".to_owned(), Value::from(42));
    let mut metadata = RunMetadata::running(
        TimeAxis {
            index_name: "simulation_step".to_owned(),
            index_unit: Some("step".to_owned()),
            physical_name: Some("time".to_owned()),
            physical_unit: Some("s".to_owned()),
        },
        run_attributes,
        vec![stream],
    );
    metadata.status = RunStatus::Complete;
    let metadata_path = run.0.join("metadata.json");
    metadata
        .validate(&metadata_path)
        .expect("writer descriptors must satisfy metadata invariants");
    fs::write(
        &metadata_path,
        serde_json::to_vec_pretty(&metadata).expect("metadata must serialize"),
    )
    .expect("the sole metadata file must be writable");

    let restored: RunMetadata =
        serde_json::from_slice(&fs::read(&metadata_path).expect("metadata file must be readable"))
            .expect("metadata file must parse");
    restored
        .validate(&metadata_path)
        .expect("restored metadata must remain valid");
    assert_eq!(restored, metadata);
    assert_eq!(summary.records(), series.len() as u64);
    assert_eq!(summary.chunks().len(), 2);

    let raw_records: Vec<RawRecord> = restored.streams[0]
        .chunks
        .iter()
        .flat_map(|chunk| {
            let bytes = fs::read(stream_directory.join(&chunk.file))
                .expect("committed chunk must be readable");
            bytes
                .split(|byte| *byte == b'\n')
                .filter(|line| !line.is_empty())
                .map(|line| {
                    serde_json::from_slice::<RawRecord>(line)
                        .expect("every complete JSONL record must parse")
                })
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(
        raw_records
            .iter()
            .map(|record| record.time().index())
            .collect::<Vec<_>>(),
        vec![0, 4, 9]
    );
    assert_eq!(raw_records[1].values["population"], json!([7, 11]));
}
