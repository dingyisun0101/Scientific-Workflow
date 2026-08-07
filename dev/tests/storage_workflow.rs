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
        assert_eq!(records.first().unwrap()["index"], chunk["first_index"]);
        assert_eq!(records.last().unwrap()["index"], chunk["last_index"]);
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

    let workspace = TempWorkspace::new();
    let run_path = workspace.run();
    let spec = StateSpec::load(fixture_path()).expect("checked-in template must load");
    let clones = Arc::new(AtomicUsize::new(0));

    let signal = StreamConfig::new(
        "signal",
        ["activity", "population"],
        NonZeroU64::new(SIGNAL_CHUNK_BYTES).unwrap(),
        NonZeroU64::new(QUEUE_BYTES).unwrap(),
    )
    .directory("streams/signals")
    .cadence("every simulation step");
    let space = StreamConfig::new(
        "space",
        ["space"],
        NonZeroU64::new(SPACE_CHUNK_BYTES).unwrap(),
        NonZeroU64::new(QUEUE_BYTES).unwrap(),
    )
    .cadence("every two simulation steps");

    let mut annotations = Map::new();
    annotations.insert("seed".to_owned(), Value::from(42));
    annotations.insert("program".to_owned(), Value::from("public-api-demo"));
    let output = RunOutputBuilder::new(&run_path, &spec)
        .time_axis(
            TimeAxis::new("simulation_step")
                .index_unit("step")
                .physical_name("time")
                .physical_unit("s"),
        )
        .run_metadata(annotations)
        .stream(signal)
        .stream(space)
        .start()
        .expect("valid run must start");
    assert_eq!(output.root(), run_path);
    assert_eq!(output.streams().collect::<Vec<_>>(), ["signal", "space"]);
    assert!(run_path.join("metadata.json").is_file());

    let mut lattice = Tensor::<u64, Dense>::zeros(&[2, 2]);
    for (coordinate, value) in [([0, 0], 1), ([0, 1], 2), ([1, 0], 3), ([1, 1], 4)] {
        lattice.set(&coordinate, value);
    }
    let mut live = spec.empty(TimePoint::from_physical(0, 0.0).unwrap());
    live.set(
        "population",
        TrackedVec {
            values: vec![10.0, 20.0, 30.0],
            clones: Arc::clone(&clones),
        },
    )
    .unwrap();
    live.set("space", lattice).unwrap();
    live.set("activity", String::from("initial")).unwrap();

    for index in 0..4_u64 {
        output.sample("signal", &live).unwrap();
        if index % 2 == 0 {
            output.sample("space", &live).unwrap();
        }
        assert_eq!(clones.load(Ordering::SeqCst), 0);
        println!(
            "[sample] index={index} physical={:.2} signal=true space={}",
            live.time().physical().unwrap(),
            index % 2 == 0
        );

        if index < 3 {
            live.get_mut::<TrackedVec>("population").unwrap().values[0] += 1.0;
            let lattice = live.get_mut::<Tensor<u64, Dense>>("space").unwrap();
            lattice.set(&[0, 0], lattice.get(&[0, 0]) + 10);
            *live.get_mut::<String>("activity").unwrap() = format!("step-{index} 世界");
            live.advance(Some(0.25)).unwrap();
        }
    }
    assert!(matches!(
        output.sample("absent", &live),
        Err(StorageError::UnknownStream { stream }) if stream == "absent"
    ));
    output.finish().expect("all writers must drain and finish");
    assert_eq!(clones.load(Ordering::SeqCst), 0);

    let metadata_bytes = fs::read(run_path.join("metadata.json")).unwrap();
    let metadata: Value = serde_json::from_slice(&metadata_bytes).unwrap();
    assert_eq!(metadata["status"]["state"], "complete");
    assert_eq!(metadata["run"]["seed"], 42);
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
    assert_eq!(space_records, 2);
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

    assert_eq!(VecF64Decoder.decode("[1.25,-2.5]").unwrap(), [1.25, -2.5]);
    assert!(VecF64Decoder.decode("[]").unwrap().is_empty());
    assert_eq!(
        StringDecoder.decode(r#""hello 世界""#).unwrap(),
        "hello 世界"
    );
    assert!(StringDecoder.decode(r#""""#).unwrap().is_empty());
    let mut decoders = Decoders::with_capacity(3);
    assert!(decoders.is_empty());
    decoders.add("population", VecF64Decoder).unwrap();
    decoders.add("activity", StringDecoder).unwrap();
    decoders
        .add::<Tensor<u64, Dense>, _>("space", |raw: &str| serde_json::from_str(raw))
        .unwrap();
    assert_eq!(decoders.len(), 3);
    assert!(decoders.contains("space"));
    let mut decoder_keys = decoders.keys().collect::<Vec<_>>();
    decoder_keys.sort_unstable();
    assert_eq!(decoder_keys, ["activity", "population", "space"]);

    let reader = SeriesReader::open(&run_path, decoders).unwrap();
    assert_eq!(reader.root(), run_path);
    assert_eq!(reader.streams().collect::<Vec<_>>(), ["signal", "space"]);
    assert!(format!("{reader:?}").contains("SeriesReader"));
    let signal_series = reader.read("signal").unwrap();
    assert_eq!(signal_series.len(), 4);
    assert_eq!(signal_series.first().unwrap().time().index(), 0);
    assert_eq!(signal_series.last().unwrap().time().index(), 3);
    assert_eq!(
        signal_series
            .last()
            .unwrap()
            .get::<Vec<f64>>("population")
            .unwrap()[0],
        13.0
    );
    assert_eq!(
        signal_series
            .last()
            .unwrap()
            .get::<String>("activity")
            .unwrap(),
        "step-2 世界"
    );
    let all = reader.read_all().unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].0, "signal");
    assert_eq!(all[1].0, "space");
    assert_eq!(
        all[1]
            .1
            .last()
            .unwrap()
            .get::<Tensor<u64, Dense>>("space")
            .unwrap()
            .get(&[0, 0]),
        21
    );
    println!(
        "[readback] signal_states={} space_states={} typed_round_trip=true clone_calls={}",
        signal_series.len(),
        all[1].1.len(),
        clones.load(Ordering::SeqCst)
    );
    println!("[result] storage_workflow=passed");
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
