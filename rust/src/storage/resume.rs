//! Metadata-first preparation of a cross-stream checkpoint rewind.

use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;

use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::error::StorageError;
use super::jsonl_format::{ChunkMetadata, RecordingMetadata};
use super::stored_state_series_reader::read_verified_chunk;

#[derive(Deserialize)]
struct RecordCoordinate {
    iteration: u64,
}

pub(crate) fn prepare_rewind_after_checkpoint(
    root: &Path,
    metadata_path: &Path,
    metadata: &mut RecordingMetadata,
    checkpoint_iteration: u64,
) -> Result<(), StorageError> {
    for stream in &mut metadata.streams {
        let directory = root.join(&stream.directory);
        let mut retained = Vec::with_capacity(stream.chunks.len());
        let chunks = std::mem::take(&mut stream.chunks);
        for mut chunk in chunks {
            let path = directory.join(&chunk.file);
            if chunk.last_iteration <= checkpoint_iteration {
                retained.push(chunk);
            } else if chunk.first_iteration > checkpoint_iteration {
                // Metadata omission is the deletion authority. Recovery
                // removes this now-extra sealed file after metadata commits.
            } else if let Some(bytes) =
                retained_prefix(metadata_path, &path, &chunk, checkpoint_iteration)?
            {
                stage_chunk_replacement(&path, &bytes)?;
                refresh_descriptor(&mut chunk, &bytes)?;
                retained.push(chunk);
            }
        }
        stream.chunks = retained;
    }
    metadata.validate(metadata_path)
}

fn retained_prefix(
    metadata_path: &Path,
    path: &Path,
    chunk: &ChunkMetadata,
    checkpoint_iteration: u64,
) -> Result<Option<Vec<u8>>, StorageError> {
    let bytes = read_verified_chunk(metadata_path, path, chunk)?;
    let mut retained_bytes = 0_usize;
    for (line_number, line) in bytes.split_inclusive(|byte| *byte == b'\n').enumerate() {
        if line.last() != Some(&b'\n') {
            return Err(StorageError::InvalidRecord {
                path: path.to_path_buf(),
                line: line_number as u64 + 1,
                reason: "sealed chunk record is not newline terminated".to_owned(),
            });
        }
        let record: RecordCoordinate =
            serde_json::from_slice(&line[..line.len() - 1]).map_err(|source| {
                StorageError::Json {
                    operation: "parse post-checkpoint record",
                    path: path.to_path_buf(),
                    source,
                }
            })?;
        if record.iteration > checkpoint_iteration {
            break;
        }
        retained_bytes += line.len();
    }
    if retained_bytes == 0 {
        Ok(None)
    } else {
        Ok(Some(bytes[..retained_bytes].to_vec()))
    }
}

fn stage_chunk_replacement(path: &Path, bytes: &[u8]) -> Result<(), StorageError> {
    let temporary = path.with_extension("jsonl.tmp");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|source| StorageError::Io {
            operation: "create staged checkpoint rewind",
            path: temporary.clone(),
            source,
        })?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|source| StorageError::Io {
            operation: "write staged checkpoint rewind",
            path: temporary.clone(),
            source,
        })?;
    sync_directory(
        path.parent()
            .expect("a recording chunk always has a stream directory"),
    )
}

fn refresh_descriptor(chunk: &mut ChunkMetadata, bytes: &[u8]) -> Result<(), StorageError> {
    let mut first = None;
    let mut last = None;
    let mut records = 0_u64;
    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let record: RecordCoordinate =
            serde_json::from_slice(line).map_err(|source| StorageError::Json {
                operation: "parse retained checkpoint prefix",
                path: Path::new(&chunk.file).to_path_buf(),
                source,
            })?;
        first.get_or_insert(record.iteration);
        last = Some(record.iteration);
        records = records
            .checked_add(1)
            .ok_or_else(|| StorageError::ByteCountOverflow {
                stream: chunk.file.clone(),
            })?;
    }
    let digest = Sha256::digest(bytes);
    let mut checksum = String::from("sha256:");
    for byte in digest {
        write!(&mut checksum, "{byte:02x}").expect("writing to a String cannot fail");
    }
    chunk.records = records;
    chunk.bytes = u64::try_from(bytes.len()).map_err(|_| StorageError::ByteCountOverflow {
        stream: chunk.file.clone(),
    })?;
    chunk.checksum = checksum;
    chunk.first_iteration = first.expect("a retained prefix contains a record");
    chunk.last_iteration = last.expect("a retained prefix contains a record");
    Ok(())
}

fn sync_directory(path: &Path) -> Result<(), StorageError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| StorageError::Io {
            operation: "synchronize checkpoint rewind",
            path: path.to_path_buf(),
            source,
        })
}
