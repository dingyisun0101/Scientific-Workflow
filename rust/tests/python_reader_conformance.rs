//! Cross-language storage conformance for Workflow's Rust and Python APIs.

use std::fs;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use scientific_workflow::prelude::basics::{
    JsonPayloadDecoderRegistry, SamplingInterval, SimulationTime, StateStreamConfig,
    StoredStateSeriesReader, SystemStateSchema, SystemStateWriterBuilder, TimeAxisMetadata,
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
    let stream = StateStreamConfig::new(
        "signal",
        ["population", "label"],
        SamplingInterval::iterations(1).unwrap(),
        Some((NonZeroU64::new(96).unwrap(), NonZeroU64::new(4096).unwrap())),
    )
    .with_relative_directory("streams/signal");
    let mut user_metadata = Map::new();
    user_metadata.insert("producer".to_owned(), Value::from("rust-public-writer"));
    let mut writer = SystemStateWriterBuilder::new(root, &schema)
        .with_time_axis_metadata(
            TimeAxisMetadata::new("iteration").with_physical_axis("physical_time", "s"),
        )
        .with_user_metadata(user_metadata)
        .add_state_stream(stream)
        .create_new_recording()
        .unwrap();

    let mut state = schema
        .create_empty_state(SimulationTime::from_iteration_and_physical_time(0, 0.0).unwrap());
    state
        .insert_payload("population", vec![sensitive, 1.25])
        .unwrap();
    state
        .insert_payload("label", String::from("rust → python 世界"))
        .unwrap();
    writer.observe_state(&state).unwrap();
    state.advance_simulation_time(Some(0.25)).unwrap();
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
fn rust_and_python_readers_share_one_format_v5_fixture() {
    let reader = StoredStateSeriesReader::open_completed_recording(fixture(), decoders()).unwrap();
    assert_eq!(reader.format_version(), 5);
    assert_eq!(reader.stream_names().collect::<Vec<_>>(), ["signal"]);
    assert_eq!(reader.stream_record_count("signal").unwrap(), 2);

    let series = reader.read_stream_as_state_series("signal").unwrap();
    assert_eq!(series.len(), 2);
    assert_eq!(series.state_at(0).unwrap().simulation_time().iteration(), 0);
    assert_eq!(series.state_at(1).unwrap().simulation_time().iteration(), 2);
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
fn rust_writes_python_reads_and_writes_then_rust_reads_exactly() {
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
    assert_eq!(first.simulation_time().iteration(), 0);
    assert_eq!(first.simulation_time().physical_time(), Some(0.0));
    assert_eq!(second.simulation_time().iteration(), 1);
    assert_eq!(second.simulation_time().physical_time(), Some(0.25));
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
