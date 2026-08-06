//! Contract tests for `storage/error.rs`.
//!
//! Storage is not public yet, so this suite includes its staged error module
//! directly while supplying the crate's real SystemState and StateSeries types.
//! It verifies diagnostic context, source preservation, shared terminal errors,
//! non-exhaustive matching, and thread-safety without filesystem mutation.

use std::error::Error as _;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

#[path = "../../src/storage/error.rs"]
mod error;

use crate::system_state::StateError;
use crate::time_series::SeriesError;
use error::StorageError;

#[derive(Debug)]
struct DecoderFailure(&'static str);

impl std::fmt::Display for DecoderFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for DecoderFailure {}

#[test]
fn configuration_and_lifecycle_errors_retain_context() {
    assert_eq!(
        StorageError::OutputExists {
            path: "runs/existing".into()
        }
        .to_string(),
        "output path `runs/existing` already exists"
    );
    assert_eq!(
        StorageError::InvalidConfig {
            setting: "stream.path",
            reason: "must be relative".into(),
        }
        .to_string(),
        "invalid storage setting `stream.path`: must be relative"
    );
    assert_eq!(
        StorageError::DuplicateStream {
            stream: "signal".into()
        }
        .to_string(),
        "output stream `signal` is configured more than once"
    );
    assert_eq!(
        StorageError::UnknownStream {
            stream: "energy".into()
        }
        .to_string(),
        "run does not declare output stream `energy`"
    );
    assert_eq!(
        StorageError::StreamFinished {
            stream: "space".into()
        }
        .to_string(),
        "output stream `space` has already finished"
    );
    assert_eq!(
        StorageError::RunFinished.to_string(),
        "run output has already finished"
    );
}

#[test]
fn metadata_and_chunk_errors_distinguish_integrity_failures() {
    let metadata = PathBuf::from("run/metadata.json");
    assert_eq!(
        StorageError::UnsupportedVersion {
            path: metadata.clone(),
            found: 2,
            supported: 1,
        }
        .to_string(),
        "metadata file `run/metadata.json` uses format version 2, but this crate supports version 1"
    );
    assert_eq!(
        StorageError::InvalidMetadata {
            path: metadata.clone(),
            reason: "chunk ordinals are not contiguous".into(),
        }
        .to_string(),
        "invalid run metadata in `run/metadata.json`: chunk ordinals are not contiguous"
    );
    assert_eq!(
        StorageError::RunIncomplete { path: metadata }.to_string(),
        "run metadata `run/metadata.json` does not declare successful completion"
    );

    let chunk = PathBuf::from("run/signal/chunk-000007.jsonl");
    assert_eq!(
        StorageError::MissingChunk {
            path: chunk.clone()
        }
        .to_string(),
        "committed chunk `run/signal/chunk-000007.jsonl` is missing"
    );
    assert_eq!(
        StorageError::ChunkSizeMismatch {
            path: chunk.clone(),
            expected: 8_192,
            actual: 4_096,
        }
        .to_string(),
        "chunk `run/signal/chunk-000007.jsonl` has 4096 bytes, but metadata declares 8192"
    );
    assert_eq!(
        StorageError::ChecksumMismatch {
            path: chunk.clone(),
            expected: "abc123".into(),
            actual: "def456".into(),
        }
        .to_string(),
        "chunk `run/signal/chunk-000007.jsonl` checksum is `def456`, but metadata declares `abc123`"
    );
    assert_eq!(
        StorageError::InvalidRecord {
            path: chunk,
            line: 14,
            reason: "duplicate field key".into(),
        }
        .to_string(),
        "invalid record at line 14 of `run/signal/chunk-000007.jsonl`: duplicate field key"
    );
}

#[test]
fn state_and_encoding_errors_preserve_typed_sources() {
    let access = StorageError::StateAccess {
        stream: "signal".into(),
        index: 12,
        field: "population".into(),
        source: StateError::MissingValue {
            field: "population".into(),
        },
    };
    assert_eq!(
        access.to_string(),
        "cannot sample field `population` for stream `signal` at time index 12: state field `population` does not contain a payload"
    );
    assert!(matches!(
        access.source().and_then(|source| source.downcast_ref::<StateError>()),
        Some(StateError::MissingValue { field }) if field == "population"
    ));

    let encoding = StorageError::EncodeField {
        stream: "signal".into(),
        index: 12,
        field: "population".into(),
        source: serde_json::from_str::<serde_json::Value>("{").unwrap_err(),
    };
    assert_eq!(
        encoding.to_string(),
        "failed to encode field `population` for stream `signal` at time index 12"
    );
    assert!(encoding.source().unwrap().is::<serde_json::Error>());
}

#[test]
fn decoder_errors_preserve_registration_and_reconstruction_context() {
    assert_eq!(
        StorageError::DuplicateDecoder {
            field: "space".into()
        }
        .to_string(),
        "a payload decoder is already registered for field `space`"
    );
    assert_eq!(
        StorageError::MissingDecoder {
            field: "space".into()
        }
        .to_string(),
        "no payload decoder is registered for field `space`"
    );
    let decoding = StorageError::DecodeField {
        stream: "space".into(),
        index: 27,
        field: "space".into(),
        source: Box::new(DecoderFailure("tensor shape mismatch")),
    };
    assert_eq!(
        decoding.to_string(),
        "failed to decode field `space` for stream `space` at time index 27"
    );
    assert_eq!(
        decoding.source().unwrap().to_string(),
        "tensor shape mismatch"
    );
    assert!(decoding.source().unwrap().is::<DecoderFailure>());

    let invariant = StorageError::SeriesInvariant {
        stream: "signal".into(),
        index: 4,
        source: SeriesError::NonIncreasingTime {
            previous: 9,
            next: 4,
        },
    };
    assert_eq!(
        invariant.to_string(),
        "decoded state for stream `signal` at time index 4 cannot enter its series"
    );
    assert!(matches!(
        invariant
            .source()
            .and_then(|source| source.downcast_ref::<SeriesError>()),
        Some(SeriesError::NonIncreasingTime {
            previous: 9,
            next: 4
        })
    ));
}

#[test]
fn filesystem_json_and_accounting_errors_preserve_causes() {
    let io_error = StorageError::Io {
        operation: "commit chunk",
        path: "run/signal/.chunk.tmp".into(),
        source: io::Error::new(io::ErrorKind::PermissionDenied, "read-only filesystem"),
    };
    assert_eq!(
        io_error.to_string(),
        "failed to commit chunk at `run/signal/.chunk.tmp`"
    );
    assert_eq!(
        io_error.source().unwrap().to_string(),
        "read-only filesystem"
    );

    let json_error = StorageError::Json {
        operation: "parse metadata",
        path: "run/metadata.json".into(),
        source: serde_json::from_str::<serde_json::Value>("[").unwrap_err(),
    };
    assert_eq!(
        json_error.to_string(),
        "failed to parse metadata JSON at `run/metadata.json`"
    );
    assert!(json_error.source().unwrap().is::<serde_json::Error>());
    assert_eq!(
        StorageError::ByteCountOverflow {
            stream: "signal".into()
        }
        .to_string(),
        "encoded byte count overflowed while processing stream `signal`"
    );
    assert_eq!(
        StorageError::RecordTooLarge {
            stream: "signal".into(),
            bytes: 2_048,
            limit: 1_024,
        }
        .to_string(),
        "encoded record for stream `signal` has 2048 bytes, exceeding the queue limit of 1024"
    );
    assert_eq!(
        StorageError::OutOfOrderRecord {
            stream: "signal".into(),
            index: 8,
            previous: 8,
        }
        .to_string(),
        "record index 8 for stream `signal` does not follow previously accepted index 8"
    );
}

#[test]
fn queue_and_worker_errors_share_one_terminal_failure() {
    assert_eq!(
        StorageError::QueueDisconnected {
            stream: "space".into()
        }
        .to_string(),
        "writer queue for stream `space` is disconnected"
    );
    let terminal = Arc::new(StorageError::Io {
        operation: "sync chunk",
        path: "run/space/chunk-000003.jsonl".into(),
        source: io::Error::other("device unavailable"),
    });
    let observed = StorageError::WriterTerminated {
        stream: "space".into(),
        source: Arc::clone(&terminal),
    };
    assert_eq!(
        observed.to_string(),
        "writer for stream `space` terminated: failed to sync chunk at `run/space/chunk-000003.jsonl`"
    );
    assert!(matches!(
        &observed,
        StorageError::WriterTerminated { source, .. } if Arc::ptr_eq(source, &terminal)
    ));
    assert_eq!(observed.source().unwrap().to_string(), terminal.to_string());
    assert_eq!(Arc::strong_count(&terminal), 2);
    assert_eq!(
        StorageError::WriterPanicked {
            stream: "space".into()
        }
        .to_string(),
        "writer worker for stream `space` panicked"
    );
}

#[test]
fn storage_error_is_send_sync_and_non_exhaustive() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<StorageError>();

    #[allow(
        unreachable_patterns,
        reason = "models the fallback required across the public non-exhaustive boundary"
    )]
    fn category(error: &StorageError) -> &'static str {
        match error {
            StorageError::StateAccess { .. }
            | StorageError::EncodeField { .. }
            | StorageError::DecodeField { .. } => "payload",
            StorageError::Io { .. } | StorageError::Json { .. } => "mechanics",
            _ => "other-or-future",
        }
    }

    assert_eq!(
        category(&StorageError::UnknownStream {
            stream: "energy".into()
        }),
        "other-or-future"
    );
}
