//! Contract tests for the private `time_series/error.rs` implementation.
//!
//! The time-series facade is intentionally not connected while its component
//! files are reviewed one at a time. This test therefore includes the
//! production error module directly and supplies the same crate-root
//! `system_state::StateError` path that the final library module graph will
//! provide.
//!
//! The suite verifies that [`SeriesError`](error::SeriesError):
//!
//! - retains the exact context needed to diagnose state-series invariants;
//! - distinguishes registry, codec, format, filesystem, and writer failures;
//! - preserves lower-level sources for programmatic error traversal;
//! - transparently converts existing SystemState failures;
//! - remains transferable and shareable across a future writer thread.

use std::error::Error as _;
use std::io;
use std::path::PathBuf;

#[path = "../../src/system_state/error.rs"]
#[allow(dead_code)]
mod state_error;

/// Reproduces the production crate-root path imported by `error.rs` without
/// exposing any additional SystemState implementation to this isolated test.
mod system_state {
    pub use super::state_error::StateError;
}

#[path = "../../src/time_series/error.rs"]
mod error;

use error::SeriesError;
use system_state::StateError;

/// Minimal codec-specific error used to verify boxed source preservation.
#[derive(Debug)]
struct CodecFailure(&'static str);

impl std::fmt::Display for CodecFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for CodecFailure {}

#[test]
fn series_invariant_errors_retain_indices_and_clear_messages() {
    let mismatch = SeriesError::SpecMismatch { index: 41 };
    assert_eq!(
        mismatch.to_string(),
        "state at time index 41 does not share the series specification"
    );
    assert!(matches!(mismatch, SeriesError::SpecMismatch { index: 41 }));

    let ordering = SeriesError::NonIncreasingTime {
        previous: 41,
        next: 41,
    };
    assert_eq!(
        ordering.to_string(),
        "state time index 41 must be greater than the previous index 41"
    );
    assert!(matches!(
        ordering,
        SeriesError::NonIncreasingTime {
            previous: 41,
            next: 41
        }
    ));
}

#[test]
fn registry_errors_identify_stable_tags_and_concrete_types() {
    let missing = SeriesError::MissingCodec {
        type_tag: "example.tensor.f64.v1".to_owned(),
    };
    assert_eq!(
        missing.to_string(),
        "no payload codec is registered for type tag `example.tensor.f64.v1`"
    );

    let duplicate = SeriesError::DuplicateCodec {
        type_tag: "example.tensor.f64.v1".to_owned(),
    };
    assert_eq!(
        duplicate.to_string(),
        "a payload codec is already registered for type tag `example.tensor.f64.v1`"
    );

    let mismatch = SeriesError::CodecTypeMismatch {
        field: "position".to_owned(),
        type_tag: "example.tensor.f64.v1".to_owned(),
        expected: "example::Tensor<f64>",
        actual: "alloc::vec::Vec<f64>",
    };
    assert_eq!(
        mismatch.to_string(),
        "state field `position` with type tag `example.tensor.f64.v1` contains `alloc::vec::Vec<f64>`, but its codec expects `example::Tensor<f64>`"
    );
}

#[test]
fn payload_errors_preserve_user_codec_sources() {
    let encode = SeriesError::EncodePayload {
        field: "position".to_owned(),
        type_tag: "example.tensor.f64.v1".to_owned(),
        source: Box::new(CodecFailure("non-finite tensor value")),
    };
    assert_eq!(
        encode.to_string(),
        "failed to encode state field `position` with codec `example.tensor.f64.v1`"
    );
    assert_eq!(
        encode
            .source()
            .expect("codec source must be retained")
            .to_string(),
        "non-finite tensor value"
    );

    let decode = SeriesError::DecodePayload {
        field: "position".to_owned(),
        type_tag: "example.tensor.f64.v1".to_owned(),
        source: Box::new(CodecFailure("tensor shape does not match data")),
    };
    assert_eq!(
        decode.to_string(),
        "failed to decode state field `position` with codec `example.tensor.f64.v1`"
    );
    assert_eq!(
        decode
            .source()
            .expect("codec source must be retained")
            .to_string(),
        "tensor shape does not match data"
    );
}

#[test]
fn format_errors_distinguish_version_structure_presence_and_size() {
    let metadata_path = PathBuf::from("run/series.json");
    let chunk_path = PathBuf::from("run/chunks/000007.json");

    let version = SeriesError::UnsupportedVersion {
        path: metadata_path.clone(),
        found: 2,
        supported: 1,
    };
    assert_eq!(
        version.to_string(),
        "metadata file `run/series.json` uses format version 2, but this crate supports version 1"
    );

    let metadata = SeriesError::InvalidMetadata {
        path: metadata_path,
        reason: "chunk ranges overlap".to_owned(),
    };
    assert_eq!(
        metadata.to_string(),
        "invalid series metadata in `run/series.json`: chunk ranges overlap"
    );

    let chunk = SeriesError::InvalidChunk {
        path: chunk_path.clone(),
        reason: "state indices are unordered".to_owned(),
    };
    assert_eq!(
        chunk.to_string(),
        "invalid payload chunk `run/chunks/000007.json`: state indices are unordered"
    );

    let missing = SeriesError::MissingChunk {
        path: chunk_path.clone(),
    };
    assert_eq!(
        missing.to_string(),
        "payload chunk `run/chunks/000007.json` is missing"
    );

    let size = SeriesError::ChunkSizeMismatch {
        path: chunk_path,
        expected: 8192,
        actual: 4096,
    };
    assert_eq!(
        size.to_string(),
        "payload chunk `run/chunks/000007.json` has 4096 bytes, but its metadata declares 8192"
    );
}

#[test]
fn io_and_json_errors_retain_paths_and_underlying_sources() {
    let io_error = SeriesError::Io {
        operation: "rename temporary chunk",
        path: PathBuf::from("run/chunks/.000000.json.tmp"),
        source: io::Error::new(io::ErrorKind::PermissionDenied, "read-only filesystem"),
    };
    assert_eq!(
        io_error.to_string(),
        "failed to rename temporary chunk at `run/chunks/.000000.json.tmp`"
    );
    assert_eq!(
        io_error
            .source()
            .expect("filesystem source must be retained")
            .to_string(),
        "read-only filesystem"
    );

    let json_source = serde_json::from_str::<serde_json::Value>("{")
        .expect_err("the fixture must be malformed JSON");
    let json_error = SeriesError::Json {
        path: PathBuf::from("run/series.json"),
        source: json_source,
    };
    assert_eq!(
        json_error.to_string(),
        "failed to process JSON at `run/series.json`"
    );
    assert!(
        json_error
            .source()
            .expect("JSON source must be retained")
            .to_string()
            .contains("EOF while parsing an object")
    );
}

#[test]
fn state_errors_convert_transparently_and_preserve_their_source_chain() {
    let error = SeriesError::from(StateError::UnknownField {
        field: "energy".to_owned(),
    });

    assert_eq!(
        error.to_string(),
        "state template does not declare field `energy`"
    );
    assert!(matches!(
        error,
        SeriesError::State(StateError::UnknownField { ref field }) if field == "energy"
    ));
}

#[test]
fn writer_lifecycle_errors_are_distinct_and_thread_safe() {
    fn assert_send_sync<T: Send + Sync>() {}

    assert_send_sync::<SeriesError>();
    assert_eq!(
        SeriesError::WriterFinished.to_string(),
        "the series writer has already been finished"
    );
    assert_eq!(
        SeriesError::WriterFailed.to_string(),
        "the series writer is unavailable after an earlier commit failure"
    );
}
