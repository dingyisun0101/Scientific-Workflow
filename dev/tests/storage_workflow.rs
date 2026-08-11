//! Logged end-to-end persistence and reconstruction through the public API.
//!
//! Run with:
//!
//! ```text
//! cargo test --test storage_workflow -- --nocapture
//! ```

use std::fmt::Write as _;
use std::fs;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use physics_in_parallel::engines::soa::{AttrsCore, AttrsMeta, PhysObj};
use physics_in_parallel::math::{Dense, Tensor};
use scientific_workflow::prelude::*;
use serde::{Serialize, Serializer};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Owns one collision-resistant test workspace and its absent run child.
struct TempWorkspace {
    root: PathBuf,
}

impl TempWorkspace {
    fn new() -> Self {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "scientific-workflow-public-storage-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("unique workspace must be creatable");
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

/// JSON-array payload whose explicit clone operation is observable.
#[derive(Debug)]
struct TrackedVec {
    values: Vec<f64>,
    clones: Arc<AtomicUsize>,
}

impl Clone for TrackedVec {
    fn clone(&self) -> Self {
        self.clones.fetch_add(1, Ordering::SeqCst);
        Self {
            values: self.values.clone(),
            clones: Arc::clone(&self.clones),
        }
    }
}

impl Serialize for TrackedVec {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.values.serialize(serializer)
    }
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/state.json")
}

fn stream_metadata<'a>(metadata: &'a Value, name: &str) -> &'a Value {
    metadata["streams"]
        .as_array()
        .unwrap()
        .iter()
        .find(|stream| stream["name"] == name)
        .expect("configured stream must occur in metadata")
}

/// Verifies persisted chunk descriptors against actual immutable files.
fn verify_chunks(run: &Path, stream: &Value) -> (u64, u64) {
    let directory = stream["directory"].as_str().unwrap();
    let chunks = stream["chunks"].as_array().unwrap();
    let mut total_records = 0_u64;
    let mut total_bytes = 0_u64;

    for (ordinal, chunk) in chunks.iter().enumerate() {
        let file = chunk["file"].as_str().unwrap();
        assert_eq!(file, format!("chunk-{ordinal:06}.jsonl"));
        let path = run.join(directory).join(file);
        let bytes = fs::read(&path).expect("committed chunk must be readable");
        assert_eq!(bytes.len() as u64, chunk["bytes"].as_u64().unwrap());
        assert!(bytes.ends_with(b"\n"));
        let records = bytes
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty());
        let records = records
            .map(|line| serde_json::from_slice::<Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(records.len() as u64, chunk["records"].as_u64().unwrap());
        assert_eq!(
            records.first().unwrap()["iteration"],
            chunk["first_iteration"]
        );
        assert_eq!(
            records.last().unwrap()["iteration"],
            chunk["last_iteration"]
        );
        let checksum = sha256_checksum(&bytes);
        assert_eq!(checksum, chunk["checksum"].as_str().unwrap());
        total_records += records.len() as u64;
        total_bytes += bytes.len() as u64;
        println!(
            "[chunk] stream={} file={file} records={} bytes={} checksum_verified=true",
            stream["name"].as_str().unwrap(),
            records.len(),
            bytes.len()
        );
    }
    (total_records, total_bytes)
}

#[test]
fn complete_scientific_workflow_is_consistent_and_observable() {
    const SIGNAL_CHUNK_BYTES: u64 = 110;
    const SPACE_CHUNK_BYTES: u64 = 64;
    const QUEUE_BYTES: u64 = 16_384;

    let interval = SamplingInterval::iterations(2).expect("positive interval must be valid");
    assert_eq!(
        serde_json::to_value(interval).unwrap(),
        serde_json::json!({"iterations": 2})
    );
    assert_eq!(
        serde_json::from_value::<SamplingInterval>(serde_json::json!({"iterations": 2})).unwrap(),
        interval
    );
    assert_eq!(
        serde_json::from_value::<SamplingInterval>(serde_json::json!(2)).unwrap(),
        interval
    );
    assert!(SamplingInterval::iterations(0).is_none());
    assert!(
        serde_json::from_value::<SamplingInterval>(serde_json::json!({"iterations": 0})).is_err()
    );
    assert!(serde_json::from_value::<SamplingInterval>(serde_json::json!(0)).is_err());
    println!("[sampling-interval] coordinate=iterations interval=2 zero_rejected=true");

    let workspace = TempWorkspace::new();
    let run_path = workspace.run();
    let spec = SystemStateSchema::load_json_template(fixture_path())
        .expect("checked-in template must load");
    let clones = Arc::new(AtomicUsize::new(0));

    let signal = StateStreamConfig::new(
        "signal",
        ["activity", "population"],
        SamplingInterval::iterations(1).unwrap(),
        NonZeroU64::new(SIGNAL_CHUNK_BYTES).unwrap(),
        NonZeroU64::new(QUEUE_BYTES).unwrap(),
    )
    .with_relative_directory("streams/signals");
    let space = StateStreamConfig::new(
        "space",
        ["space"],
        SamplingInterval::iterations(2).unwrap(),
        NonZeroU64::new(SPACE_CHUNK_BYTES).unwrap(),
        NonZeroU64::new(QUEUE_BYTES).unwrap(),
    );

    let mut annotations = Map::new();
    annotations.insert("seed".to_owned(), Value::from(42));
    annotations.insert("program".to_owned(), Value::from("public-api-demo"));
    let mut output = SystemStateWriterBuilder::new(&run_path, &spec)
        .with_time_axis_metadata(
            TimeAxisMetadata::new("simulation_iteration")
                .with_iteration_unit("iteration")
                .with_physical_axis("physical_time", "s"),
        )
        .with_user_metadata(annotations)
        .add_state_stream(signal)
        .add_state_stream(space)
        .create_new_recording()
        .expect("valid run must start");
    assert_eq!(output.recording_directory(), run_path);
    assert_eq!(
        output.stream_names().collect::<Vec<_>>(),
        ["signal", "space"]
    );
    assert!(run_path.join("metadata.json").is_file());

    let mut lattice = Tensor::<u64, Dense>::zeros(&[2, 2]);
    for (coordinate, value) in [([0, 0], 1), ([0, 1], 2), ([1, 0], 3), ([1, 1], 4)] {
        lattice.set(&coordinate, value);
    }
    let mut live =
        spec.create_empty_state(SimulationTime::from_iteration_and_physical_time(0, 0.0).unwrap());
    live.insert_payload(
        "population",
        TrackedVec {
            values: vec![10.0, 20.0, 30.0],
            clones: Arc::clone(&clones),
        },
    )
    .unwrap();
    live.insert_payload("space", lattice).unwrap();
    live.insert_payload("activity", String::from("initial"))
        .unwrap();

    for iteration in 0..4_u64 {
        output.observe_state(&live).unwrap();
        assert_eq!(clones.load(Ordering::SeqCst), 0);
        println!(
            "[sample] iteration={iteration} physical_time={:.2} signal=true space={}",
            live.simulation_time().physical_time().unwrap(),
            iteration % 2 == 0
        );

        if iteration < 3 {
            live.payload_mut::<TrackedVec>("population").unwrap().values[0] += 1.0;
            let lattice = live.payload_mut::<Tensor<u64, Dense>>("space").unwrap();
            lattice.set(&[0, 0], lattice.get(&[0, 0]) + 10);
            *live.payload_mut::<String>("activity").unwrap() =
                format!("evolved-to-iteration-{} 世界", iteration + 1);
            live.advance_simulation_time(Some(0.25)).unwrap();
        }
    }
    let mut terminal_metadata = Map::new();
    terminal_metadata.insert("completed_step_count".to_owned(), Value::from(3));
    terminal_metadata.insert(
        "termination_reason".to_owned(),
        Value::from("requested_steps_completed"),
    );
    let completed = output
        .complete_recording_with_final_state_and_terminal_metadata(&live, terminal_metadata)
        .expect("all writers must drain and finish");
    assert_eq!(completed.directory(), run_path);
    assert!(completed.timing().created_at_utc().ends_with('Z'));
    assert!(completed.timing().finalized_at_utc().ends_with('Z'));
    assert_eq!(completed.timing().continuation_count(), 0);
    assert_eq!(
        completed.terminal_metadata()["completed_step_count"],
        Value::from(3)
    );
    assert_eq!(completed.stream_summaries().len(), 2);
    let signal_summary = completed.stream_summary("signal").unwrap();
    assert_eq!(signal_summary.name(), "signal");
    assert_eq!(signal_summary.record_count(), 4);
    assert!(signal_summary.chunk_count() >= 2);
    assert!(signal_summary.encoded_bytes() > 0);
    assert_eq!(signal_summary.first_iteration(), Some(0));
    assert_eq!(signal_summary.last_iteration(), Some(3));
    assert_eq!(clones.load(Ordering::SeqCst), 0);

    let metadata_bytes = fs::read(run_path.join("metadata.json")).unwrap();
    let metadata: Value = serde_json::from_slice(&metadata_bytes).unwrap();
    assert_eq!(metadata["status"]["state"], "complete");
    assert_eq!(metadata["version"], 4);
    assert!(
        metadata["timing"]["created_at_utc"]
            .as_str()
            .unwrap()
            .ends_with('Z')
    );
    assert!(
        metadata["timing"]["finalized_at_utc"]
            .as_str()
            .unwrap()
            .ends_with('Z')
    );
    assert_eq!(metadata["timing"]["continuation_count"], 0);
    assert_eq!(
        metadata["terminal_metadata"]["termination_reason"],
        "requested_steps_completed"
    );
    assert_eq!(metadata["time"]["iteration_name"], "simulation_iteration");
    assert_eq!(metadata["time"]["iteration_unit"], "iteration");
    assert!(metadata["time"].get("step_name").is_none());
    assert_eq!(metadata["user_metadata"]["seed"], 42);
    assert_eq!(
        serde_json::from_slice::<Value>(&serde_json::to_vec_pretty(&metadata).unwrap()).unwrap(),
        metadata
    );
    let signal_metadata = stream_metadata(&metadata, "signal");
    let space_metadata = stream_metadata(&metadata, "space");
    assert_eq!(signal_metadata["directory"], "streams/signals");
    assert_eq!(signal_metadata["fields"][0]["name"], "population");
    assert_eq!(signal_metadata["fields"][1]["name"], "activity");
    let (signal_records, signal_bytes) = verify_chunks(&run_path, signal_metadata);
    let (space_records, space_bytes) = verify_chunks(&run_path, space_metadata);
    assert_eq!(signal_records, 4);
    assert_eq!(space_records, 3);
    assert!(signal_metadata["chunks"].as_array().unwrap().len() >= 2);
    assert!(
        space_metadata["chunks"]
            .as_array()
            .unwrap()
            .iter()
            .all(|chunk| {
                chunk["records"] == 1 && chunk["bytes"].as_u64().unwrap() > SPACE_CHUNK_BYTES
            })
    );

    let files = walk_files(&run_path);
    assert_eq!(
        files
            .iter()
            .filter(|path| path.file_name().unwrap() == "metadata.json")
            .count(),
        1
    );
    assert!(
        files
            .iter()
            .all(|path| { !path.file_name().unwrap().to_string_lossy().contains(".tmp") })
    );
    println!("[durability] final_chunk_names=true temporary_files=false");
    println!(
        "[metadata] files={} bytes={} semantic_round_trip=true status=complete",
        files.len(),
        metadata_bytes.len()
    );
    println!(
        "[writer] signal_records={signal_records} signal_bytes={signal_bytes} space_records={space_records} space_bytes={space_bytes}"
    );

    assert_eq!(
        JsonVecF64Decoder
            .decode_json_payload("[1.25,-2.5]")
            .unwrap(),
        [1.25, -2.5]
    );
    assert!(
        JsonVecF64Decoder
            .decode_json_payload("[]")
            .unwrap()
            .is_empty()
    );
    // This value exposed a one-ULP discrepancy before the crate enabled
    // serde_json's `float_roundtrip` parser. Preserve it as an exact-bit
    // integration regression because scientific checkpoints may depend on
    // reproducible finite floating-point payloads.
    let sensitive_float = f64::from_bits(0xbfc1_5855_07ca_40c8);
    let encoded_sensitive_float = serde_json::to_string(&[sensitive_float]).unwrap();
    let decoded_sensitive_float = JsonVecF64Decoder
        .decode_json_payload(&encoded_sensitive_float)
        .unwrap()[0];
    assert_eq!(decoded_sensitive_float.to_bits(), sensitive_float.to_bits());
    println!("[float-round-trip] encoded={encoded_sensitive_float} exact_bits=true");
    assert_eq!(
        JsonStringDecoder
            .decode_json_payload(r#""hello 世界""#)
            .unwrap(),
        "hello 世界"
    );
    assert!(
        JsonStringDecoder
            .decode_json_payload(r#""""#)
            .unwrap()
            .is_empty()
    );
    let decoders = JsonPayloadDecoderRegistry::with_capacity(3);
    assert!(decoders.is_empty());
    let decoders = decoders
        .with_json_field::<Vec<f64>>("population")
        .unwrap()
        .with_json_field::<String>("activity")
        .unwrap()
        .with_json_field::<Tensor<u64, Dense>>("space")
        .unwrap();
    assert_eq!(decoders.len(), 3);
    assert!(decoders.has_decoder_for_field("space"));
    let mut decoder_keys = decoders.registered_field_names().collect::<Vec<_>>();
    decoder_keys.sort_unstable();
    assert_eq!(decoder_keys, ["activity", "population", "space"]);

    let reader = StoredStateSeriesReader::open_completed_recording(&run_path, decoders).unwrap();
    assert_eq!(reader.recording_directory(), run_path);
    assert_eq!(
        reader.stream_names().collect::<Vec<_>>(),
        ["signal", "space"]
    );
    assert!(format!("{reader:?}").contains("StoredStateSeriesReader"));
    assert_eq!(reader.format_version(), 4);
    assert_eq!(reader.user_metadata()["seed"], 42);
    assert_eq!(
        reader.terminal_metadata()["termination_reason"],
        "requested_steps_completed"
    );
    assert_eq!(reader.recording_timing(), completed.timing());
    assert_eq!(reader.stream_record_count("signal").unwrap(), 4);
    assert_eq!(
        reader.stream_encoded_bytes("signal").unwrap(),
        signal_summary.encoded_bytes()
    );
    let latest_signal = reader.read_latest_state_from_stream("signal").unwrap();
    assert_eq!(latest_signal.simulation_time().iteration(), 3);
    assert_eq!(
        latest_signal.payload::<Vec<f64>>("population").unwrap()[0],
        13.0
    );
    let signal_series = reader.read_stream_as_state_series("signal").unwrap();
    assert_eq!(signal_series.len(), 4);
    assert_eq!(
        signal_series
            .first_state()
            .unwrap()
            .simulation_time()
            .iteration(),
        0
    );
    assert_eq!(
        signal_series
            .last_state()
            .unwrap()
            .simulation_time()
            .iteration(),
        3
    );
    assert_eq!(
        signal_series
            .last_state()
            .unwrap()
            .payload::<Vec<f64>>("population")
            .unwrap()[0],
        13.0
    );
    assert_eq!(
        signal_series
            .last_state()
            .unwrap()
            .payload::<String>("activity")
            .unwrap(),
        "evolved-to-iteration-3 世界"
    );
    let all = reader.read_all_streams_as_state_series().unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].0, "signal");
    assert_eq!(all[1].0, "space");
    assert_eq!(
        all[1]
            .1
            .last_state()
            .unwrap()
            .payload::<Tensor<u64, Dense>>("space")
            .unwrap()
            .get(&[0, 0]),
        31
    );
    println!(
        "[readback] signal_states={} space_states={} typed_round_trip=true clone_calls={}",
        signal_series.len(),
        all[1].1.len(),
        clones.load(Ordering::SeqCst)
    );

    let project = ProjectConfig::load(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/configuration/cartesian_project"),
    )
    .unwrap();
    let task = project.parameters().task(0).unwrap();
    let task_metadata_run = workspace.root.join("task-metadata-run");
    SystemStateWriter::builder(&task_metadata_run, &spec)
        .with_task_parameters(&task)
        .with_shared_stream_limits(
            NonZeroU64::new(4_096).unwrap(),
            NonZeroU64::new(16_384).unwrap(),
        )
        .add_sampled_state_stream(
            "checkpoint",
            ["population", "space", "activity"],
            SamplingInterval::iterations(1).unwrap(),
        )
        .create_new_recording()
        .unwrap()
        .complete_recording()
        .unwrap();
    let task_metadata: Value =
        serde_json::from_slice(&fs::read(task_metadata_run.join("metadata.json")).unwrap())
            .unwrap();
    assert_eq!(task_metadata["user_metadata"]["task_ordinal"], 0);
    assert_eq!(task_metadata["user_metadata"]["temperature"], 280.0);
    assert_eq!(task_metadata["user_metadata"]["seed"], 7);
    assert_eq!(task_metadata["version"], 4);
    assert_eq!(
        task_metadata["streams"][0]["sampling_interval"],
        serde_json::json!({"iterations": 1})
    );
    println!("[task-metadata] task_ordinal=0 temperature=280 seed=7");
    println!("[result] storage_workflow=passed");
}

#[test]
fn heterogeneous_pip_payload_round_trips_through_the_generic_json_contract() {
    let workspace = TempWorkspace::new();
    let template = workspace.root.join("phys-obj-state.json");
    fs::write(
        &template,
        br#"{"fields":[{"name":"particles","description":"typed particle columns"}]}"#,
    )
    .unwrap();
    let spec = SystemStateSchema::load_json_template(&template).unwrap();

    let mut attributes = AttrsCore::empty();
    attributes.allocate::<f64>("position", 2, 2).unwrap();
    attributes.allocate::<i64>("species", 1, 2).unwrap();
    attributes
        .set_vector_of("position", 1, &[1.25_f64, -2.5])
        .unwrap();
    attributes.set_vector_of("species", 0, &[7_i64]).unwrap();
    let particles = PhysObj::new(AttrsMeta::new(3, "particles", "mixed"), attributes);
    let mut state = spec.create_empty_state(SimulationTime::from_iteration(0));
    state.insert_payload("particles", particles).unwrap();

    let run = workspace.root.join("phys-obj-run");
    let writer = SystemStateWriter::builder(&run, &spec)
        .with_shared_stream_limits(
            NonZeroU64::new(16_384).unwrap(),
            NonZeroU64::new(65_536).unwrap(),
        )
        .add_sampled_state_stream(
            "checkpoint",
            ["particles"],
            SamplingInterval::iterations(1).unwrap(),
        )
        .create_new_recording()
        .unwrap();
    writer.complete_recording_with_final_state(&state).unwrap();

    let decoders = JsonPayloadDecoderRegistry::new()
        .with_json_field::<PhysObj>("particles")
        .unwrap();
    let series = StoredStateSeriesReader::open_completed_recording(&run, decoders)
        .unwrap()
        .read_stream_as_state_series("checkpoint")
        .unwrap();
    let decoded = series
        .last_state()
        .unwrap()
        .payload::<PhysObj>("particles")
        .unwrap();
    assert_eq!(decoded.meta.label, "particles");
    assert_eq!(
        decoded.core.vector_of::<f64>("position", 1).unwrap(),
        [1.25, -2.5]
    );
    assert_eq!(
        decoded.core.vector_of::<i64>("species", 0).unwrap(),
        [7_i64]
    );
    println!("[pip-payload] phys_obj=true mixed_types=true generic_decoder=true");
}

/// Returns every regular file recursively for sidecar/temp-file assertions.
fn walk_files(root: &Path) -> Vec<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

fn sha256_checksum(bytes: &[u8]) -> String {
    let mut checksum = String::from("sha256:");
    for byte in Sha256::digest(bytes) {
        write!(&mut checksum, "{byte:02x}").unwrap();
    }
    checksum
}
