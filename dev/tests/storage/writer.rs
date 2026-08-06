//! Focused contract tests for `storage/writer.rs`.
//!
//! The storage facade remains staged, so this suite includes the reviewed
//! production modules directly. Every filesystem mutation is confined to a
//! uniquely named process-local temporary directory removed by [`TempRun`].

use std::fs;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

mod system_state {
    pub use scientific_workflow::system_state::*;
}

#[allow(unused_imports)]
mod time_series {
    pub use scientific_workflow::time_series::*;
}

#[allow(dead_code)]
#[path = "../../src/storage/error.rs"]
mod error;
#[allow(dead_code)]
#[path = "../../src/storage/format.rs"]
mod format;
#[allow(dead_code)]
#[path = "../../src/storage/writer.rs"]
mod writer;

use error::StorageError;
use format::EncodedRecord;
use system_state::TimePoint;
use writer::{MAX_OUTSTANDING_RECORDS, StateWriter, WriterConfig};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Owns one exact test directory and removes only that directory on drop.
struct TempRun {
    path: PathBuf,
}

impl TempRun {
    /// Creates a collision-resistant directory under the platform temp root.
    fn new(label: &str) -> Self {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "scientific-workflow-writer-{label}-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("the unique test root must be creatable");
        Self { path }
    }

    /// Returns an absent child path suitable for writer startup.
    fn stream(&self, name: &str) -> PathBuf {
        self.path.join(name)
    }
}

impl Drop for TempRun {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.path) {
            eprintln!(
                "failed to remove test directory {}: {error}",
                self.path.display()
            );
        }
    }
}

/// Constructs one complete compact record through the real framing type.
fn record(index: u64, payload: &str) -> EncodedRecord {
    EncodedRecord::new(
        TimePoint::new(index),
        format!(r#"{{"index":{index},"values":{{"sample":"{payload}"}}}}"#).into_bytes(),
    )
}

/// Independently renders digest bytes in the persisted lowercase notation.
fn lowercase_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Creates a writer with concise integer byte limits.
fn start_writer(
    directory: &Path,
    chunk_bytes: u64,
    queue_bytes: u64,
) -> Result<StateWriter, StorageError> {
    StateWriter::start(
        WriterConfig::new(
            "signal",
            directory,
            NonZeroU64::new(chunk_bytes).expect("test chunk limit must be nonzero"),
            NonZeroU64::new(queue_bytes).expect("test queue limit must be nonzero"),
        )
        .expect("static test configuration must be valid"),
    )
}

#[test]
fn configuration_exposes_only_byte_limits_and_the_record_limit_is_internal_policy() {
    let config = WriterConfig::new(
        "signal",
        "run/signal",
        NonZeroU64::new(4_096).unwrap(),
        NonZeroU64::new(8_192).unwrap(),
    )
    .expect("valid configuration must construct");

    assert_eq!(config.stream(), "signal");
    assert_eq!(config.directory(), Path::new("run/signal"));
    assert_eq!(config.max_chunk_bytes().get(), 4_096);
    assert_eq!(config.queue_bytes().get(), 8_192);
    assert_eq!(MAX_OUTSTANDING_RECORDS, 1_024);
    assert!(matches!(
        WriterConfig::new(
            "  ",
            "run/signal",
            NonZeroU64::new(1).unwrap(),
            NonZeroU64::new(1).unwrap()
        ),
        Err(StorageError::InvalidConfig {
            setting: "stream",
            ..
        })
    ));
}

#[test]
fn startup_refuses_to_replace_an_existing_stream_directory() {
    let run = TempRun::new("existing");
    let stream = run.stream("signal");
    fs::create_dir(&stream).expect("the conflicting directory must be created");

    assert!(matches!(
        start_writer(&stream, 1_024, 1_024),
        Err(StorageError::OutputExists { path }) if path == stream
    ));
}

#[test]
fn finishing_an_empty_writer_produces_no_chunk() {
    let run = TempRun::new("empty");
    let stream = run.stream("signal");
    let summary = start_writer(&stream, 1_024, 1_024)
        .expect("writer must start")
        .finish()
        .expect("empty writer must finish");

    assert_eq!(summary.stream(), "signal");
    assert_eq!(summary.records(), 0);
    assert_eq!(summary.bytes(), 0);
    assert!(summary.chunks().is_empty());
    assert_eq!(fs::read_dir(stream).unwrap().count(), 0);
}

#[test]
fn exact_byte_rollover_writes_indivisible_fifo_records_and_valid_checksums() {
    let run = TempRun::new("rollover");
    let stream = run.stream("signal");
    let first = record(2, "alpha");
    let second = record(4, "beta");
    let third = record(8, "gamma");
    let first_bytes = first.bytes().to_vec();
    let second_bytes = second.bytes().to_vec();
    let third_bytes = third.bytes().to_vec();
    let exact_first_chunk = (first.len() + second.len()) as u64;
    let writer = start_writer(&stream, exact_first_chunk, 16_384).expect("writer must start");

    writer.submit(first).expect("first record must be accepted");
    writer
        .submit(second)
        .expect("second record must be accepted");
    writer.submit(third).expect("third record must be accepted");
    let summary = writer.finish().expect("accepted records must commit");

    assert_eq!(summary.records(), 3);
    assert_eq!(summary.chunks().len(), 2);
    assert_eq!(summary.chunks()[0].first_index, 2);
    assert_eq!(summary.chunks()[0].last_index, 4);
    assert_eq!(summary.chunks()[1].first_index, 8);
    assert_eq!(summary.chunks()[1].last_index, 8);

    let mut expected_first = first_bytes;
    expected_first.extend(second_bytes);
    let first_file = fs::read(stream.join("chunk-000000.jsonl")).unwrap();
    let second_file = fs::read(stream.join("chunk-000001.jsonl")).unwrap();
    assert_eq!(first_file, expected_first);
    assert_eq!(second_file, third_bytes);
    assert_eq!(
        summary.bytes(),
        (first_file.len() + second_file.len()) as u64
    );
    assert_eq!(
        summary.chunks()[0].checksum,
        format!("sha256:{}", lowercase_hex(&Sha256::digest(&first_file)))
    );
    assert_eq!(
        summary.chunks()[1].checksum,
        format!("sha256:{}", lowercase_hex(&Sha256::digest(&second_file)))
    );
    assert!(!fs::read_dir(&stream).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")
    }));
}

#[test]
fn a_record_larger_than_the_chunk_target_occupies_one_oversized_chunk() {
    let run = TempRun::new("oversized-chunk");
    let stream = run.stream("signal");
    let oversized = record(10, &"x".repeat(256));
    let exact_bytes = oversized.len() as u64;
    let writer = start_writer(&stream, 32, exact_bytes).expect("writer must start");

    writer
        .submit(oversized)
        .expect("queue budget, not chunk target, controls admission");
    let summary = writer.finish().expect("oversized chunk must commit");

    assert_eq!(summary.chunks().len(), 1);
    assert_eq!(summary.chunks()[0].records, 1);
    assert_eq!(summary.chunks()[0].bytes, exact_bytes);
    assert!(summary.chunks()[0].bytes > 32);
}

#[test]
fn a_record_larger_than_the_strict_byte_budget_fails_without_waiting() {
    let run = TempRun::new("queue-limit");
    let stream = run.stream("signal");
    let rejected = record(1, &"x".repeat(64));
    let bytes = rejected.len() as u64;
    let writer = start_writer(&stream, 1_024, bytes - 1).expect("writer must start");

    assert!(matches!(
        writer.submit(rejected),
        Err(StorageError::RecordTooLarge {
            bytes: actual,
            limit,
            ..
        }) if actual == bytes && limit == bytes - 1
    ));
    assert_eq!(writer.finish().unwrap().records(), 0);
}

#[test]
fn nonincreasing_indices_are_rejected_before_entering_the_fifo() {
    let run = TempRun::new("ordering");
    let stream = run.stream("signal");
    let writer = start_writer(&stream, 1_024, 1_024).expect("writer must start");

    writer.submit(record(5, "accepted")).unwrap();
    assert!(matches!(
        writer.submit(record(5, "duplicate")),
        Err(StorageError::OutOfOrderRecord {
            index: 5,
            previous: 5,
            ..
        })
    ));
    writer.submit(record(6, "accepted")).unwrap();
    assert_eq!(writer.finish().unwrap().records(), 2);
}

#[test]
fn worker_failure_is_shared_as_a_terminal_writer_error() {
    let run = TempRun::new("terminal");
    let stream = run.stream("signal");
    let writer = start_writer(&stream, 1_024, 1_024).expect("writer must start");
    fs::remove_dir(&stream).expect("the empty writer directory must be removable");

    writer.submit(record(1, "cannot-open-chunk")).unwrap();
    let error = writer
        .finish()
        .expect_err("worker IO failure must reach finish");
    assert!(matches!(error, StorageError::WriterTerminated { .. }));
}
