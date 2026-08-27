use std::fs;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use scientific_workflow::prelude::basic::*;
use scientific_workflow::rng_record::*;
use scientific_workflow::storage::*;
use serde_json::{Map, Value, json};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TempWorkspace {
    root: PathBuf,
}

impl TempWorkspace {
    fn new() -> Self {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "scientific-workflow-rng-record-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&root).unwrap();
        Self { root }
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.root) {
            eprintln!("failed to clean {}: {error}", self.root.display());
        }
    }
}

fn state_schema() -> SystemStateSchema {
    SystemStateSchema::load_json_template(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/state.json"),
    )
    .unwrap()
}

fn record(key: &str) -> RngRecord {
    RngRecord::new(
        "simulation.noise",
        "chacha12+standard_normal",
        "rand_chacha-0.10+rand_distr-0.6",
        "u64_be_hex",
        key,
        Some(Map::from_iter([("lanes".to_owned(), json!(2))])),
    )
    .unwrap()
}

fn metadata(key: &str) -> Map<String, Value> {
    let mut metadata = Map::from_iter([("experiment".to_owned(), json!("test"))]);
    record(key).insert_into_metadata(&mut metadata).unwrap();
    metadata
}

#[test]
fn replicate_seeds_are_lazy_versioned_and_order_independent() {
    let replicate_zero = ReplicateSeedDeriver::new(1101, 0);
    let matrix = replicate_zero.derive("matrix").unwrap();
    let pairing = replicate_zero.derive("pairing").unwrap();
    let next_matrix = ReplicateSeedDeriver::new(1101, 1).derive("matrix").unwrap();

    assert_eq!(matrix.value(), 7_764_280_038_077_573_120);
    assert_eq!(pairing.value(), 16_304_155_092_128_863_366);
    assert_eq!(next_matrix.value(), 6_233_544_743_961_248_020);
    assert_eq!(replicate_zero.derive("matrix").unwrap(), matrix);
    assert_eq!(matrix.record().namespace(), "matrix");
    assert_eq!(matrix.record().parameters()["base_seed"], 1101);
    assert_eq!(matrix.record().parameters()["replicate_index"], 0);
    assert!(matches!(
        replicate_zero.derive(" "),
        Err(RngRecordError::EmptySeedNamespace)
    ));
}

fn writer_builder(
    run: &Path,
    schema: &SystemStateSchema,
    metadata: Map<String, Value>,
) -> SystemStateWriterBuilder {
    SystemStateWriter::builder(run.to_path_buf(), schema)
        .with_writer(Writer::streams([Stream::fields("signal", ["population"]).unwrap()]).unwrap())
        .with_user_metadata(metadata)
        .with_shared_stream_storage(StateStreamStorage::chunked(
            NonZeroU64::new(1_024).unwrap(),
            NonZeroU64::new(4_096).unwrap(),
        ))
}

#[test]
fn rng_records_validate_insert_and_round_trip_without_rng_behavior() {
    let mut metadata = metadata("000000000000002a");
    let stored = RngRecord::from_metadata(&metadata, "simulation.noise")
        .unwrap()
        .unwrap();
    assert_eq!(stored, record("000000000000002a"));
    assert_eq!(stored.method(), "chacha12+standard_normal");
    assert_eq!(stored.key_encoding(), "u64_be_hex");
    assert_eq!(stored.parameters()["lanes"], 2);
    assert!(matches!(
        stored.insert_into_metadata(&mut metadata),
        Err(RngRecordError::DuplicateNamespace { namespace })
            if namespace == "simulation.noise"
    ));
    assert!(matches!(
        RngRecord::new(" ", "method", "1", "hex", "00", None),
        Err(RngRecordError::EmptyField { field: "namespace" })
    ));
    assert!(
        RngRecord::from_metadata(&Map::new(), "missing")
            .unwrap()
            .is_none()
    );
}

#[test]
fn writer_persists_records_and_continuation_requires_exact_identity() {
    let workspace = TempWorkspace::new();
    let run = workspace.root.join("run");
    let schema = state_schema();
    let original = metadata("000000000000002a");
    drop(
        writer_builder(&run, &schema, original.clone())
            .create_new_recording()
            .unwrap(),
    );

    let document: Value =
        serde_json::from_slice(&fs::read(run.join("metadata.json")).unwrap()).unwrap();
    assert_eq!(
        document["user_metadata"][RNG_RECORDS_METADATA_KEY]["simulation.noise"]["key"],
        "000000000000002a"
    );
    assert!(matches!(
        writer_builder(&run, &schema, metadata("000000000000002b")).continue_existing_recording(),
        Err(StorageError::RecordingConfigurationMismatch { .. })
    ));

    writer_builder(&run, &schema, original)
        .continue_existing_recording()
        .unwrap()
        .mark_recording_failed("test finished")
        .unwrap();
}
