//! Private inferred Runtime output directories.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::error::RuntimeError;

static EXECUTION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub(crate) fn create_execution(root: &Path) -> Result<PathBuf, RuntimeError> {
    fs::create_dir_all(root).map_err(|source| RuntimeError::OutputScope {
        path: root.to_path_buf(),
        source,
    })?;
    for _ in 0..1024 {
        let sequence = EXECUTION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let directory = root.join(format!("execution-{}-{sequence}", std::process::id()));
        match fs::create_dir(&directory) {
            Ok(()) => return Ok(directory),
            Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(RuntimeError::OutputScope {
                    path: directory,
                    source,
                });
            }
        }
    }
    Err(RuntimeError::OutputScope {
        path: root.to_path_buf(),
        source: std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not allocate a unique execution directory",
        ),
    })
}

pub(crate) fn create_replicate(root: &Path, index: u64) -> Result<PathBuf, RuntimeError> {
    let directory = root.join(format!("replicate-{index:06}"));
    fs::create_dir(&directory).map_err(|source| RuntimeError::OutputScope {
        path: directory.clone(),
        source,
    })?;
    Ok(directory)
}
