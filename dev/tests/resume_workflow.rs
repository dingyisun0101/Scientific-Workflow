//! Logged integration tests for filename-based crash recovery and append.
//!
//! Run with:
//!
//! ```text
//! cargo test --test resume_workflow -- --nocapture
//! ```

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use physics_in_parallel::math::{Dense, Tensor};
use scientific_workflow::prelude::*;
use serde_json::Value;

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
    SystemStateWriter::builder(run, spec)
        .with_shared_stream_limits(
            NonZeroU64::new(max_chunk_bytes).unwrap(),
            NonZeroU64::new(1_000_000).unwrap(),
        )
        .add_periodic_state_stream(
            "checkpoint",
            ["population", "space", "activity"],
            NonZeroU64::new(1).unwrap(),
        )
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

fn state(spec: &SystemStateSchema, index: u64) -> SystemState {
    let mut lattice = Tensor::<u64, Dense>::zeros(&[2]);
    lattice.set(&[0], index + 10);
    lattice.set(&[1], index + 20);
    let mut state = spec.create_empty_state(
        SimulationTime::from_step_and_physical_time(index, index as f64 * 0.5).unwrap(),
    );
    state
        .insert_payload("population", vec![index as f64, index as f64 + 0.25])
        .unwrap();
    state.insert_payload("space", lattice).unwrap();
    state
        .insert_payload("activity", format!("step-{index}"))
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

#[test]
fn unprepared_tail_reconstructs_complete_state_and_continues_the_same_chunk() {
    let workspace = TempWorkspace::new("unprepared-resume");
    let run = workspace.run();
    let spec = SystemStateSchema::load_json_template(fixture_path()).unwrap();

    let mut output = builder(&run, &spec).create_new_recording().unwrap();
    output.observe_state(&state(&spec, 0)).unwrap();
    output.observe_state(&state(&spec, 1)).unwrap();
    output.complete_recording().unwrap();

    // Reproduce a crash before descriptor preparation: the real payload keeps
    // its open name, metadata contains no descriptor, and its final bytes end
    // in one incomplete record fragment.
    let mut manifest = metadata(&run);
    manifest["status"] = serde_json::json!({"state": "running"});
    manifest["streams"][0]
        .as_object_mut()
        .unwrap()
        .remove("chunks");
    write_metadata(&run, &manifest);
    let sealed = run.join("checkpoint/chunk-000000.jsonl");
    let open = run.join("checkpoint/chunk-000000.jsonl.tmp");
    fs::rename(&sealed, &open).unwrap();
    OpenOptions::new()
        .append(true)
        .open(&open)
        .unwrap()
        .write_all(br#"{"index":2,"physical":"#)
        .unwrap();

    let (mut output, mut resumed) = builder(&run, &spec)
        .continue_recording_from_latest_checkpoint("checkpoint", decoders())
        .unwrap();
    assert_eq!(resumed.simulation_time().step(), 1);
    assert_eq!(resumed.simulation_time().physical_time(), Some(0.5));
    assert_eq!(
        resumed.payload::<Vec<f64>>("population").unwrap(),
        &[1.0, 1.25]
    );
    assert_eq!(resumed.payload::<String>("activity").unwrap(), "step-1");
    assert_eq!(
        resumed
            .payload::<Tensor<u64, Dense>>("space")
            .unwrap()
            .get(&[1]),
        21
    );
    assert_eq!(
        resumed.populated_field_count(),
        resumed.declared_field_count()
    );
    println!(
        "[resume-state] index={} physical={:?} fields={} complete=true",
        resumed.simulation_time().step(),
        resumed.simulation_time().physical_time(),
        resumed.populated_field_count()
    );

    resumed = state(&spec, 2);
    output.observe_state(&resumed).unwrap();
    output.flush_stream_to_storage("checkpoint").unwrap();
    assert!(sealed.is_file());
    assert!(!open.exists());
    let running = metadata(&run);
    assert_eq!(running["status"]["state"], "running");
    assert_eq!(running["streams"][0]["chunks"][0]["records"], 3);
    println!(
        "[recovery] incomplete_tail_truncated=true continued_open_chunk=true records=3 durable_barrier=true"
    );
    output.complete_recording().unwrap();

    let reader = StoredStateSeriesReader::open_completed_recording(&run, decoders()).unwrap();
    let series = reader.read_stream_as_state_series("checkpoint").unwrap();
    assert_eq!(series.len(), 3);
    assert_eq!(series.last_state().unwrap().simulation_time().step(), 2);
    println!(
        "[result] unprepared_resume=passed final_states={}",
        series.len()
    );
}

#[test]
fn prepared_tail_finishes_rename_and_exclusive_lease_rejects_competitors() {
    let workspace = TempWorkspace::new("prepared-resume");
    let run = workspace.run();
    let spec = SystemStateSchema::load_json_template(fixture_path()).unwrap();

    let mut output = builder(&run, &spec).create_new_recording().unwrap();
    output.observe_state(&state(&spec, 4)).unwrap();
    output.complete_recording().unwrap();

    // Reproduce the narrow crash window after metadata preparation but before
    // the lifecycle rename. The descriptor remains authoritative.
    let mut manifest = metadata(&run);
    manifest["status"] = serde_json::json!({"state": "running"});
    write_metadata(&run, &manifest);
    let sealed = run.join("checkpoint/chunk-000000.jsonl");
    let open = run.join("checkpoint/chunk-000000.jsonl.tmp");
    fs::rename(&sealed, &open).unwrap();

    let (mut output, resumed) = builder(&run, &spec)
        .continue_recording_from_latest_checkpoint("checkpoint", decoders())
        .unwrap();
    assert_eq!(resumed.simulation_time().step(), 4);
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
            .map(|state| state.simulation_time().step())
            .collect::<Vec<_>>(),
        [4, 5]
    );
    println!(
        "[result] prepared_resume=passed final_states={}",
        series.len()
    );
}

#[test]
fn partial_stream_continues_output_but_cannot_construct_a_full_state() {
    let workspace = TempWorkspace::new("partial-resume");
    let run = workspace.run();
    let spec = SystemStateSchema::load_json_template(fixture_path()).unwrap();
    let partial_builder = || {
        SystemStateWriter::builder(&run, &spec).add_state_stream(StateStreamConfig::new(
            "signal",
            ["population"],
            NonZeroU64::new(1).unwrap(),
            NonZeroU64::new(1_000_000).unwrap(),
            NonZeroU64::new(1_000_000).unwrap(),
        ))
    };

    let mut output = partial_builder().create_new_recording().unwrap();
    output.observe_state(&state(&spec, 8)).unwrap();
    output.complete_recording().unwrap();
    let mut manifest = metadata(&run);
    manifest["status"] = serde_json::json!({"state": "running"});
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
fn several_sealed_chunks_are_trusted_before_recovering_only_the_open_tail() {
    let workspace = TempWorkspace::new("multi-chunk-resume");
    let run = workspace.run();
    let spec = SystemStateSchema::load_json_template(fixture_path()).unwrap();
    let recording_builder = || builder_with_chunk_limit(&run, &spec, 1);

    let mut output = recording_builder().create_new_recording().unwrap();
    for index in 0..3 {
        output.observe_state(&state(&spec, index)).unwrap();
    }
    output.complete_recording().unwrap();

    let mut manifest = metadata(&run);
    manifest["status"] = serde_json::json!({"state": "running"});
    let chunks = manifest["streams"][0]["chunks"].as_array_mut().unwrap();
    assert_eq!(chunks.len(), 3);
    chunks.pop();
    write_metadata(&run, &manifest);

    let stream_directory = run.join("checkpoint");
    let sealed_zero = stream_directory.join("chunk-000000.jsonl");
    let sealed_two = stream_directory.join("chunk-000002.jsonl");
    let open_two = stream_directory.join("chunk-000002.jsonl.tmp");
    fs::write(&sealed_zero, b"deliberately invalid sealed history\n").unwrap();
    fs::rename(&sealed_two, &open_two).unwrap();

    let (mut output, resumed) = recording_builder()
        .continue_recording_from_latest_checkpoint("checkpoint", decoders())
        .expect("resume must trust sealed history and decode only the open tail");
    assert_eq!(resumed.simulation_time().step(), 2);
    assert_eq!(resumed.payload::<String>("activity").unwrap(), "step-2");

    output.observe_state(&state(&spec, 3)).unwrap();
    output.complete_recording().unwrap();

    let completed = metadata(&run);
    let chunks = completed["streams"][0]["chunks"].as_array().unwrap();
    assert_eq!(chunks.len(), 4);
    assert_eq!(
        chunks
            .iter()
            .map(|chunk| chunk["ordinal"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        [0, 1, 2, 3]
    );
    assert!((0..4).all(|ordinal| {
        stream_directory
            .join(format!("chunk-{ordinal:06}.jsonl"))
            .is_file()
    }));
    assert!(!open_two.exists());
    println!(
        "[multi-chunk] sealed_history_trusted=true open_tail_scanned=true resumed_index=2 next_ordinal=3"
    );
}

#[test]
fn continuation_rejects_terminal_mismatched_and_empty_recordings() {
    let workspace = TempWorkspace::new("resume-rejections");
    let spec = SystemStateSchema::load_json_template(fixture_path()).unwrap();

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
            .with_time_axis_metadata(TimeAxisMetadata::new("iteration"))
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
