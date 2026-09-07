//! Shared durability primitives with stable persistence error context.

use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::Path;

use super::PersistenceError;

pub(super) fn create_new_file(
    path: &Path,
    operation: &'static str,
) -> Result<File, PersistenceError> {
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|source| PersistenceError::Io {
            operation,
            path: path.to_path_buf(),
            source,
        })
}

pub(super) fn write_new(
    path: &Path,
    bytes: &[u8],
    operation: &'static str,
) -> Result<(), PersistenceError> {
    let mut file = create_new_file(path, operation)?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|source| PersistenceError::Io {
            operation,
            path: path.to_path_buf(),
            source,
        })
}

pub(super) fn sync_directory(path: &Path, operation: &'static str) -> Result<(), PersistenceError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| PersistenceError::Io {
            operation,
            path: path.to_path_buf(),
            source,
        })
}
