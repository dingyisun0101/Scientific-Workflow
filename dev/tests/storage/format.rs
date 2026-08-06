//! Contract tests for `storage/format.rs`.
//!
//! The storage facade remains staged, so this suite includes the reviewed
//! production error and format modules directly. Tests operate entirely on
//! in-memory values and JSON documents: filesystem existence, byte-length
//! checks, checksums of real files, and atomic commits belong to reader and
//! writer tests.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

// This suite exercises the format layer, which intentionally uses only the
// metadata-validation subset of the shared storage error vocabulary.
#[allow(dead_code)]
#[path = "../../src/storage/error.rs"]
mod error;
#[path = "../../src/storage/format.rs"]
mod format;

use crate::system_state::TimePoint;
use error::StorageError;
use format::{
    ChunkMetadata, EncodedRecord, FORMAT_NAME, FORMAT_VERSION, FieldMetadata, PAYLOAD_ENCODING,
    RECORD_FRAMING, RunMetadata, RunStatus, StreamMetadata, TimeAxis, chunk_filename,
};

/// Stable provenance used by semantic metadata validation.
fn metadata_path() -> PathBuf {
    PathBuf::from("run/metadata.json")
}

/// Returns one field declaration without a Rust type or decoder tag.
fn field(name: &str) -> FieldMetadata {
    FieldMetadata {
        name: name.to_owned(),
        description: None,
    }
}

/// Returns a valid committed chunk descriptor.
fn chunk(ordinal: u64, first_index: u64, last_index: u64) -> ChunkMetadata {
    ChunkMetadata {
        ordinal,
        file: chunk_filename(ordinal),
        records: 2,
        bytes: 128,
        checksum: "sha256:abcdef0123456789".to_owned(),
        first_index,
        last_index,
    }
}

/// Returns one valid stream with no committed chunks.
fn stream(name: &str, directory: &str) -> StreamMetadata {
    StreamMetadata {
        name: name.to_owned(),
        directory: directory.to_owned(),
        cadence: Some("every 10 simulation steps".to_owned()),
        fields: vec![field("population")],
        max_chunk_bytes: 1_048_576,
        queue_bytes: 4_194_304,
        chunks: Vec::new(),
    }
}

/// Returns one valid time-axis declaration.
fn time_axis() -> TimeAxis {
    TimeAxis {
        index_name: "simulation_step".to_owned(),
        index_unit: Some("step".to_owned()),
        physical_name: Some("time".to_owned()),
        physical_unit: Some("s".to_owned()),
    }
}

/// Returns valid running metadata with signal and space streams.
fn valid_metadata() -> RunMetadata {
    let mut run = Map::new();
    run.insert("seed".to_owned(), json!(42));
    run.insert("temperature".to_owned(), json!(0.25));
    RunMetadata::running(
        time_axis(),
        run,
        vec![stream("signal", "signal"), stream("space", "space")],
    )
}

/// Extracts the semantic reason from an expected metadata failure.
fn invalid_reason(error: StorageError) -> String {
    match error {
        StorageError::InvalidMetadata { path, reason } => {
            assert_eq!(path, metadata_path());
            reason
        }
        other => panic!("expected InvalidMetadata, got {other:?}"),
    }
}

#[test]
fn running_metadata_round_trip_preserves_the_versioned_schema() {
    let metadata = valid_metadata();
    metadata
        .validate(&metadata_path())
        .expect("valid metadata must pass");
    assert_eq!(metadata.format, FORMAT_NAME);
    assert_eq!(metadata.version, FORMAT_VERSION);
    assert_eq!(metadata.records.encoding, PAYLOAD_ENCODING);
    assert_eq!(metadata.records.framing, RECORD_FRAMING);
    assert!(matches!(metadata.status, RunStatus::Running));
    assert_eq!(metadata.stream("signal").unwrap().directory, "signal");
    assert!(metadata.stream("missing").is_none());

    let json = serde_json::to_string(&metadata).expect("serialize metadata");
    let decoded: RunMetadata = serde_json::from_str(&json).expect("deserialize metadata");
    assert_eq!(decoded, metadata);
    decoded
        .validate(&metadata_path())
        .expect("round trip remains valid");

    let value: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["format"], FORMAT_NAME);
    assert_eq!(value["version"], FORMAT_VERSION);
    assert_eq!(value["status"], json!({"state": "running"}));
    assert_eq!(
        value["records"],
        json!({"encoding": "json", "framing": "json_lines"})
    );
    assert_eq!(value["streams"][0]["fields"][0]["name"], "population");
    assert!(value["streams"][0]["fields"][0].get("type").is_none());
}

#[test]
fn mutable_stream_lookup_supports_chunk_commit_bookkeeping() {
    let mut metadata = valid_metadata();
    metadata
        .stream_mut("signal")
        .expect("signal stream must exist")
        .chunks
        .push(chunk(0, 0, 4));
    assert!(metadata.stream_mut("missing").is_none());
    metadata
        .validate(&metadata_path())
        .expect("committed chunk must remain valid");
    assert_eq!(metadata.stream("signal").unwrap().chunks[0].ordinal, 0);
}

#[test]
fn format_version_encoding_and_unknown_properties_are_strict() {
    let mut wrong_name = valid_metadata();
    wrong_name.format = "another-format".to_owned();
    assert!(
        invalid_reason(wrong_name.validate(&metadata_path()).unwrap_err()).contains(FORMAT_NAME)
    );

    let mut wrong_version = valid_metadata();
    wrong_version.version = FORMAT_VERSION + 1;
    assert!(matches!(
        wrong_version.validate(&metadata_path()),
        Err(StorageError::UnsupportedVersion {
            found,
            supported: FORMAT_VERSION,
            ..
        }) if found == FORMAT_VERSION + 1
    ));

    let mut wrong_encoding = valid_metadata();
    wrong_encoding.records.encoding = "protobuf".to_owned();
    assert!(
        invalid_reason(wrong_encoding.validate(&metadata_path()).unwrap_err())
            .contains("record format")
    );

    let mut value = serde_json::to_value(valid_metadata()).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("unexpected".to_owned(), json!(true));
    assert!(serde_json::from_value::<RunMetadata>(value).is_err());
}

#[test]
fn lifecycle_and_time_axis_validation_reject_empty_or_inconsistent_labels() {
    let mut failed = valid_metadata();
    failed.status = RunStatus::Failed {
        message: "  ".to_owned(),
    };
    assert!(invalid_reason(failed.validate(&metadata_path()).unwrap_err()).contains("message"));

    let mut empty_index = valid_metadata();
    empty_index.time.index_name = " ".to_owned();
    assert!(
        invalid_reason(empty_index.validate(&metadata_path()).unwrap_err()).contains("index_name")
    );

    let mut empty_unit = valid_metadata();
    empty_unit.time.index_unit = Some(String::new());
    assert!(
        invalid_reason(empty_unit.validate(&metadata_path()).unwrap_err()).contains("index_unit")
    );

    let mut orphan_unit = valid_metadata();
    orphan_unit.time.physical_name = None;
    assert!(
        invalid_reason(orphan_unit.validate(&metadata_path()).unwrap_err())
            .contains("requires time.physical_name")
    );

    let mut empty_physical_name = valid_metadata();
    empty_physical_name.time.physical_name = Some(" ".to_owned());
    assert!(
        invalid_reason(empty_physical_name.validate(&metadata_path()).unwrap_err())
            .contains("physical_name")
    );
}

#[test]
fn stream_validation_rejects_duplicates_unsafe_paths_and_zero_limits() {
    let mut no_streams = valid_metadata();
    no_streams.streams.clear();
    assert!(
        invalid_reason(no_streams.validate(&metadata_path()).unwrap_err()).contains("at least one")
    );

    let mut duplicate_name = valid_metadata();
    duplicate_name.streams[1].name = "signal".to_owned();
    assert!(matches!(
        duplicate_name.validate(&metadata_path()),
        Err(StorageError::DuplicateStream { ref stream }) if stream == "signal"
    ));

    let mut duplicate_directory = valid_metadata();
    duplicate_directory.streams[1].directory = "signal".to_owned();
    assert!(
        invalid_reason(duplicate_directory.validate(&metadata_path()).unwrap_err())
            .contains("same output directory")
    );

    for unsafe_path in ["", "/absolute", "../escape", "signal/../space", "."] {
        let mut metadata = valid_metadata();
        metadata.streams[0].directory = unsafe_path.to_owned();
        assert!(
            invalid_reason(metadata.validate(&metadata_path()).unwrap_err())
                .contains("safe relative path")
        );
    }

    for select_limit in 0..2 {
        let mut metadata = valid_metadata();
        match select_limit {
            0 => metadata.streams[0].max_chunk_bytes = 0,
            _ => metadata.streams[0].queue_bytes = 0,
        }
        assert!(
            invalid_reason(metadata.validate(&metadata_path()).unwrap_err())
                .contains("zero storage limit")
        );
    }

    let mut empty_cadence = valid_metadata();
    empty_cadence.streams[0].cadence = Some(" ".to_owned());
    assert!(
        invalid_reason(empty_cadence.validate(&metadata_path()).unwrap_err())
            .contains("empty cadence")
    );
}

#[test]
fn field_schema_validation_preserves_order_and_rejects_bad_declarations() {
    let mut metadata = valid_metadata();
    metadata.streams[0].fields = vec![
        FieldMetadata {
            name: "population".to_owned(),
            description: Some("Population at each site".to_owned()),
        },
        field("activity"),
    ];
    metadata
        .validate(&metadata_path())
        .expect("ordered unique fields are valid");
    assert_eq!(metadata.streams[0].fields[0].name, "population");
    assert_eq!(metadata.streams[0].fields[1].name, "activity");

    let mut duplicate = valid_metadata();
    duplicate.streams[0].fields = vec![field("population"), field("population")];
    assert!(
        invalid_reason(duplicate.validate(&metadata_path()).unwrap_err())
            .contains("duplicate field")
    );

    let mut empty_name = valid_metadata();
    empty_name.streams[0].fields = vec![field(" ")];
    assert!(
        invalid_reason(empty_name.validate(&metadata_path()).unwrap_err())
            .contains("empty field name")
    );

    let mut empty_description = valid_metadata();
    empty_description.streams[0].fields[0].description = Some(" ".to_owned());
    assert!(
        invalid_reason(empty_description.validate(&metadata_path()).unwrap_err())
            .contains("empty description")
    );
}

#[test]
fn chunk_validation_enforces_names_ordinals_ranges_and_checksums() {
    let mut metadata = valid_metadata();
    metadata.streams[0].chunks = vec![chunk(0, 0, 4), chunk(1, 8, 12)];
    metadata
        .validate(&metadata_path())
        .expect("ordered chunks with gaps are valid");

    let mut bad_ordinal = metadata.clone();
    bad_ordinal.streams[0].chunks[1].ordinal = 3;
    assert!(
        invalid_reason(bad_ordinal.validate(&metadata_path()).unwrap_err())
            .contains("expected chunk ordinal 1")
    );

    let mut bad_name = metadata.clone();
    bad_name.streams[0].chunks[0].file = "chunk-0.jsonl".to_owned();
    assert!(
        invalid_reason(bad_name.validate(&metadata_path()).unwrap_err())
            .contains("filename must be")
    );

    for empty_member in 0..2 {
        let mut empty = metadata.clone();
        if empty_member == 0 {
            empty.streams[0].chunks[0].records = 0;
        } else {
            empty.streams[0].chunks[0].bytes = 0;
        }
        assert!(invalid_reason(empty.validate(&metadata_path()).unwrap_err()).contains("empty"));
    }

    let mut reversed = metadata.clone();
    reversed.streams[0].chunks[0].first_index = 5;
    assert!(invalid_reason(reversed.validate(&metadata_path()).unwrap_err()).contains("reversed"));

    for checksum in ["", "abcdef", "SHA256:abcdef", "sha256:ABCDEF", "sha256:xyz"] {
        let mut invalid = metadata.clone();
        invalid.streams[0].chunks[0].checksum = checksum.to_owned();
        assert!(
            invalid_reason(invalid.validate(&metadata_path()).unwrap_err())
                .contains("invalid checksum")
        );
    }

    let mut overlap = metadata;
    overlap.streams[0].chunks[1].first_index = 4;
    assert!(invalid_reason(overlap.validate(&metadata_path()).unwrap_err()).contains("not after"));
}

#[test]
fn encoded_record_owns_exactly_one_framed_line_without_debugging_payload() {
    let time = TimePoint::from_physical(7, 1.5).unwrap();
    let json = br#"{"index":7,"physical":1.5,"values":{"secret":987654321}}"#.to_vec();
    let original_capacity = json.capacity();
    let record = EncodedRecord::new(time, json);

    assert_eq!(record.time(), time);
    assert_eq!(record.bytes().last(), Some(&b'\n'));
    assert_eq!(record.len(), record.bytes().len());
    assert_eq!(
        record.bytes().iter().filter(|&&byte| byte == b'\n').count(),
        1
    );
    let debug = format!("{record:?}");
    assert!(debug.contains("EncodedRecord"));
    assert!(debug.contains(&format!("bytes: {}", record.len())));
    assert!(!debug.contains("987654321"));
    assert!(!debug.contains("secret"));

    let bytes = record.into_bytes();
    assert!(bytes.capacity() >= original_capacity);
    assert!(bytes.ends_with(b"\n"));
}

#[test]
fn deterministic_chunk_names_expand_beyond_the_minimum_width() {
    assert_eq!(chunk_filename(0), "chunk-000000.jsonl");
    assert_eq!(chunk_filename(42), "chunk-000042.jsonl");
    assert_eq!(chunk_filename(999_999), "chunk-999999.jsonl");
    assert_eq!(chunk_filename(1_000_000), "chunk-1000000.jsonl");
}

#[test]
fn validation_is_path_provenant_and_does_not_access_that_path() {
    let nonexistent = Path::new("this/path/does/not/exist/metadata.json");
    valid_metadata()
        .validate(nonexistent)
        .expect("validation must remain independent of filesystem existence");
}
