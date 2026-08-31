//! Cross-language persistence conformance for Workflow's Rust and Python APIs.

use std::fs;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use super::local::{
    JsonPayloadDecoderRegistry, StateStreamStorage, StoredStateSeriesReader, SystemStateWriter,
};
use crate::observation::BoundObservationPlan;
use scientific_workflow::prelude::{
    ObservationPlan, ObservationStream, StateTime, SystemStateSchema,
};
use serde_json::{Map, Value};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TempWorkspace {
    root: PathBuf,
}

impl TempWorkspace {
    fn new() -> Self {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "scientific-workflow-python-roundtrip-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("unique round-trip workspace must be creatable");
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

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../python/tests/fixtures/complete")
}

fn invalid_metadata_cases() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../python/tests/fixtures/invalid_metadata_cases.json")
}

fn protocol_file(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../protocol")
        .join(name)
}

fn repository_python_support_is_available() -> bool {
    fixture().is_dir()
        && invalid_metadata_cases().is_file()
        && PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../python/src/scientific_workflow_reader")
            .is_dir()
}

fn decoders() -> JsonPayloadDecoderRegistry {
    JsonPayloadDecoderRegistry::with_capacity(2)
        .with_json_field::<Vec<f64>>("population")
        .unwrap()
        .with_json_field::<String>("label")
        .unwrap()
}

fn python_executable() -> String {
    std::env::var("PYTHON").unwrap_or_else(|_| "python3".to_owned())
}

fn write_rust_recording(root: &Path, schema_path: &Path, sensitive: f64) {
    fs::write(
        schema_path,
        r#"{
            "fields": [
                {"name": "population", "description": "Exact float payload"},
                {"name": "label", "description": "Unicode round-trip label"}
            ]
        }"#,
    )
    .unwrap();
    let schema = SystemStateSchema::load_json_template(schema_path).unwrap();
    let observation_plan =
        ObservationPlan::streams([
            ObservationStream::fields("signal", ["population", "label"]).unwrap()
        ])
        .unwrap()
        .with_physical_time_unit("s")
        .unwrap();
    let mut user_metadata = Map::new();
    user_metadata.insert("producer".to_owned(), Value::from("rust-public-writer"));
    let mut state =
        schema.create_empty_state(StateTime::from_iteration_and_physical_time(0, 0.0).unwrap());
    let descriptor = BoundObservationPlan::bind(observation_plan, &schema).unwrap();
    let mut writer = SystemStateWriter::create(
        root.to_path_buf(),
        descriptor,
        user_metadata,
        StateStreamStorage::chunked(NonZeroU64::new(96).unwrap(), NonZeroU64::new(4096).unwrap()),
    )
    .unwrap();

    state
        .insert_payload("population", vec![sensitive, 1.25])
        .unwrap();
    state
        .insert_payload("label", String::from("rust → python 世界"))
        .unwrap();
    writer.observe_state(&state).unwrap();
    state.advance_time(Some(0.25)).unwrap();
    state.payload_mut::<Vec<f64>>("population").unwrap()[1] = -2.5;
    *state.payload_mut::<String>("label").unwrap() = String::from("python → rust λ");
    writer.observe_state(&state).unwrap();

    let mut terminal = Map::new();
    terminal.insert(
        "termination_reason".to_owned(),
        Value::from("rust_roundtrip_ready"),
    );
    writer
        .complete_recording_with_terminal_metadata(terminal)
        .unwrap();
}

#[test]
fn rust_and_python_readers_share_one_format_v7_fixture() {
    if !repository_python_support_is_available() {
        return;
    }
    let reader = StoredStateSeriesReader::open_completed_recording(&fixture(), decoders()).unwrap();
    assert_eq!(reader.format_version(), 7);
    assert_eq!(reader.stream_names().collect::<Vec<_>>(), ["signal"]);
    assert_eq!(reader.stream_record_count("signal").unwrap(), 2);

    let series = reader.read_stream_as_state_series("signal").unwrap();
    assert_eq!(series.len(), 2);
    assert_eq!(series.state_at(0).unwrap().time().iteration(), 0);
    assert_eq!(series.state_at(1).unwrap().time().iteration(), 2);
    assert_eq!(
        series
            .state_at(1)
            .unwrap()
            .payload::<Vec<f64>>("population")
            .unwrap(),
        &[1.0, 2.0]
    );
    assert_eq!(
        series
            .state_at(1)
            .unwrap()
            .payload::<String>("label")
            .unwrap(),
        "later"
    );
}

#[test]
fn protocol_manifests_match_the_rust_package_and_reader() {
    if !protocol_file("compatibility.json").is_file()
        || !protocol_file("recording-v7.schema.json").is_file()
    {
        return;
    }
    let compatibility: Value =
        serde_json::from_slice(&fs::read(protocol_file("compatibility.json")).unwrap()).unwrap();
    let schema: Value =
        serde_json::from_slice(&fs::read(protocol_file("recording-v7.schema.json")).unwrap())
            .unwrap();

    assert_eq!(
        compatibility["recording"]["format"],
        "scientific-workflow-jsonl"
    );
    assert_eq!(compatibility["recording"]["version"], 7);
    assert_eq!(
        schema["properties"]["format"]["const"],
        compatibility["recording"]["format"]
    );
    assert_eq!(
        schema["properties"]["version"]["const"],
        compatibility["recording"]["version"]
    );
    assert_eq!(
        compatibility["implementations"]["rust"]["package"],
        env!("CARGO_PKG_NAME")
    );
    assert_eq!(
        compatibility["implementations"]["rust"]["version"],
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(
        compatibility["implementations"]["rust"]["writes"],
        serde_json::json!([7])
    );
    assert_eq!(
        compatibility["implementations"]["rust"]["reads"],
        serde_json::json!([7])
    );

    let reader = StoredStateSeriesReader::open_completed_recording(&fixture(), decoders()).unwrap();
    assert_eq!(
        Value::from(reader.format_version()),
        compatibility["recording"]["version"]
    );
}

#[test]
fn rust_writes_python_reads_and_writes_then_rust_reads_exactly() {
    if !repository_python_support_is_available() {
        return;
    }
    let workspace = TempWorkspace::new();
    let rust_recording = workspace.root.join("rust-recording");
    let python_recording = workspace.root.join("python-recording");
    let schema_path = workspace.root.join("schema.json");
    let sensitive = f64::from_bits(0xbfc1_5855_07ca_40c8);
    write_rust_recording(&rust_recording, &schema_path, sensitive);
    let rust_metadata: Value =
        serde_json::from_slice(&fs::read(rust_recording.join("metadata.json")).unwrap()).unwrap();
    assert_eq!(
        rust_metadata["streams"][0]["chunks"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let repository = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let script = repository.join("python/tests/roundtrip_bridge.py");
    let python_path = repository.join("python/src");
    let output = Command::new(python_executable())
        .arg(script)
        .arg(&rust_recording)
        .arg(&python_recording)
        .env("PYTHONPATH", python_path)
        .output()
        .expect("Python 3.10 or newer is required for cross-language conformance");
    assert!(
        output.status.success(),
        "Python round-trip bridge failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let reader =
        StoredStateSeriesReader::open_completed_recording(&python_recording, decoders()).unwrap();
    assert_eq!(
        reader.user_metadata()["producer"],
        "python-roundtrip-bridge"
    );
    assert_eq!(reader.user_metadata()["rust_origin"], "rust-public-writer");
    assert_eq!(
        reader.terminal_metadata()["termination_reason"],
        "python_roundtrip_complete"
    );
    assert_eq!(reader.stream_record_count("signal").unwrap(), 2);
    let python_metadata: Value =
        serde_json::from_slice(&fs::read(python_recording.join("metadata.json")).unwrap()).unwrap();
    assert_eq!(
        python_metadata["streams"][0]["chunks"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let series = reader.read_stream_as_state_series("signal").unwrap();
    assert_eq!(series.len(), 2);
    let first = series.state_at(0).unwrap();
    let second = series.state_at(1).unwrap();
    assert_eq!(first.time().iteration(), 0);
    assert_eq!(first.time().physical_time(), Some(0.0));
    assert_eq!(second.time().iteration(), 1);
    assert_eq!(second.time().physical_time(), Some(0.25));
    assert_eq!(
        first.payload::<Vec<f64>>("population").unwrap()[0].to_bits(),
        sensitive.to_bits()
    );
    assert_eq!(
        first.payload::<String>("label").unwrap(),
        "rust → python 世界"
    );
    assert_eq!(
        second.payload::<String>("label").unwrap(),
        "python → rust λ"
    );
}

#[test]
fn rust_and_python_share_invalid_metadata_rejections() {
    if !repository_python_support_is_available() {
        return;
    }
    let workspace = TempWorkspace::new();
    let cases: Value =
        serde_json::from_slice(&fs::read(invalid_metadata_cases()).unwrap()).unwrap();
    for (index, case) in cases.as_array().unwrap().iter().enumerate() {
        let recording = workspace.root.join(format!("invalid-{index}"));
        fs::create_dir_all(recording.join("streams/signal")).unwrap();
        fs::copy(
            fixture().join("streams/signal/chunk-000000.jsonl"),
            recording.join("streams/signal/chunk-000000.jsonl"),
        )
        .unwrap();
        let mut metadata: Value =
            serde_json::from_slice(&fs::read(fixture().join("metadata.json")).unwrap()).unwrap();
        let pointer = case["pointer"].as_str().unwrap();
        *metadata.pointer_mut(pointer).unwrap() = case["value"].clone();
        fs::write(
            recording.join("metadata.json"),
            serde_json::to_vec(&metadata).unwrap(),
        )
        .unwrap();

        let outcome = StoredStateSeriesReader::open_completed_recording(&recording, decoders())
            .and_then(|reader| reader.read_stream_as_state_series("signal"));
        assert!(
            outcome.is_err(),
            "Rust accepted shared invalid metadata case `{}`",
            case["name"].as_str().unwrap()
        );
    }
}
