use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::clock::utc_now_rfc3339;

use super::error::ExecutionScopeError;

static EXECUTION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// One created or reopened project-execution directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionScope {
    directory: PathBuf,
    created_at_utc: Option<String>,
}

impl ExecutionScope {
    /// Uses the recording root itself as the deterministic execution scope.
    ///
    /// This is intended for application-owned semantic task names where a
    /// repeated configuration must resolve to the same output path instead of
    /// receiving a timestamped execution wrapper.
    pub fn open_or_create(recording_root: impl AsRef<Path>) -> Result<Self, ExecutionScopeError> {
        let directory = recording_root.as_ref().to_path_buf();
        fs::create_dir_all(&directory).map_err(|source| ExecutionScopeError::Io {
            operation: "create deterministic",
            path: directory.clone(),
            source,
        })?;
        Ok(Self {
            directory,
            created_at_utc: None,
        })
    }

    /// Creates a uniquely named scope beneath `recording_root`.
    ///
    /// The readable UTC component is supplemented by process and sequence
    /// values. Exclusive directory creation remains the final collision check.
    pub fn create_generated(recording_root: impl AsRef<Path>) -> Result<Self, ExecutionScopeError> {
        let recording_root = recording_root.as_ref();
        fs::create_dir_all(recording_root).map_err(|source| ExecutionScopeError::Io {
            operation: "create recording root for",
            path: recording_root.to_path_buf(),
            source,
        })?;
        let created_at_utc =
            utc_now_rfc3339().map_err(|source| ExecutionScopeError::Timestamp { source })?;
        let compact_timestamp = compact_timestamp(&created_at_utc);
        for _ in 0..1024 {
            let sequence = EXECUTION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let name = format!(
                "execution-{compact_timestamp}-{}-{sequence}",
                std::process::id()
            );
            let directory = recording_root.join(name);
            match fs::create_dir(&directory) {
                Ok(()) => {
                    return Ok(Self {
                        directory,
                        created_at_utc: Some(created_at_utc),
                    });
                }
                Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(source) => {
                    return Err(ExecutionScopeError::Io {
                        operation: "create generated",
                        path: directory,
                        source,
                    });
                }
            }
        }
        Err(ExecutionScopeError::IdentityExhausted {
            root: recording_root.to_path_buf(),
        })
    }

    /// Creates one caller-named scope beneath `recording_root`.
    pub fn create_named(
        recording_root: impl AsRef<Path>,
        name: impl Into<String>,
    ) -> Result<Self, ExecutionScopeError> {
        let recording_root = recording_root.as_ref();
        let name = name.into();
        validate_name(&name)?;
        let created_at_utc =
            utc_now_rfc3339().map_err(|source| ExecutionScopeError::Timestamp { source })?;
        fs::create_dir_all(recording_root).map_err(|source| ExecutionScopeError::Io {
            operation: "create recording root for",
            path: recording_root.to_path_buf(),
            source,
        })?;
        let directory = recording_root.join(&name);
        fs::create_dir(&directory).map_err(|source| ExecutionScopeError::Io {
            operation: "create named",
            path: directory.clone(),
            source,
        })?;
        Ok(Self {
            directory,
            created_at_utc: Some(created_at_utc),
        })
    }

    /// Opens an existing execution scope without creating or modifying files.
    pub fn open_existing(directory: impl Into<PathBuf>) -> Result<Self, ExecutionScopeError> {
        let directory = directory.into();
        let metadata = fs::metadata(&directory).map_err(|source| ExecutionScopeError::Io {
            operation: "inspect existing",
            path: directory.clone(),
            source,
        })?;
        if !metadata.is_dir() {
            return Err(ExecutionScopeError::Io {
                operation: "open non-directory",
                path: directory.clone(),
                source: std::io::Error::new(
                    std::io::ErrorKind::NotADirectory,
                    "execution scope must be a directory",
                ),
            });
        }
        Ok(Self {
            directory,
            created_at_utc: None,
        })
    }

    /// Returns the scope directory.
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// Returns the automatically captured creation timestamp when this handle
    /// created the scope.
    ///
    /// Reopened legacy scopes return `None` because no auxiliary scope metadata
    /// is invented merely to recover a timestamp.
    pub fn created_at_utc(&self) -> Option<&str> {
        self.created_at_utc.as_deref()
    }

    /// Derives the absent recording path reserved for one deterministic task.
    ///
    /// The directory is deliberately not created: the recording writer must
    /// retain exclusive creation and overwrite protection.
    pub fn task_recording_directory(&self, task_ordinal: u64) -> PathBuf {
        self.directory.join(format!("task-{task_ordinal:06}"))
    }

    /// Derives the absent recording path reserved for one semantic task name.
    pub fn named_task_recording_directory(
        &self,
        name: &str,
    ) -> Result<PathBuf, ExecutionScopeError> {
        validate_name(name)?;
        Ok(self.directory.join(name))
    }
}

/// Requires a nonempty relative name containing exactly one normal component.
fn validate_name(name: &str) -> Result<(), ExecutionScopeError> {
    let path = Path::new(name);
    let mut components = path.components();
    let valid = !name.trim().is_empty()
        && matches!(components.next(), Some(Component::Normal(_)))
        && components.next().is_none();
    if valid {
        Ok(())
    } else {
        Err(ExecutionScopeError::InvalidName {
            name: name.to_owned(),
        })
    }
}

/// Removes RFC 3339 punctuation while retaining ordered UTC date/time digits.
fn compact_timestamp(timestamp: &str) -> String {
    timestamp
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect()
}
