//! Durable task-owned input snapshots, shared by execution and supported readers.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::PersistenceError;
use super::fs::{sync_directory, write_new};

const FORMAT: &str = "scientific-workflow-inputs.v1";
const MANIFEST: &str = "workflow-inputs.json";
const CONFIG: &str = "workflow-config.json";
const DEPENDENCIES: &str = "workflow-dependencies.json";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InputReference {
    path: PathBuf,
    checksum: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TaskInputReferences {
    config: InputReference,
    dependencies: InputReference,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct InputManifest {
    format: String,
    config: InputReference,
    dependencies: InputReference,
}

pub(crate) struct TaskInputSnapshots {
    directory: PathBuf,
    references: TaskInputReferences,
}

impl TaskInputSnapshots {
    pub(crate) fn create(
        directory: &Path,
        config: &[u8],
        dependencies: &[u8],
    ) -> Result<Self, PersistenceError> {
        let references = TaskInputReferences {
            config: InputReference::new(Path::new(CONFIG), config),
            dependencies: InputReference::new(Path::new(DEPENDENCIES), dependencies),
        };
        write_new(
            &directory.join(CONFIG),
            config,
            "write task configuration snapshot",
        )?;
        write_new(
            &directory.join(DEPENDENCIES),
            dependencies,
            "write task dependency snapshot",
        )?;
        let manifest = InputManifest {
            format: FORMAT.into(),
            config: references.config.clone(),
            dependencies: references.dependencies.clone(),
        };
        let path = directory.join(MANIFEST);
        let bytes =
            serde_json::to_vec_pretty(&manifest).map_err(|source| PersistenceError::Json {
                operation: "serialize task input checksum manifest",
                path: path.clone(),
                source,
            })?;
        write_new(&path, &bytes, "write task input checksum manifest")?;
        sync_directory(directory, "synchronize task input snapshots")?;
        Ok(Self {
            directory: directory.to_path_buf(),
            references,
        })
    }

    pub(crate) fn open(directory: &Path) -> Result<Self, PersistenceError> {
        let references = read_references(directory)?;
        let snapshots = Self {
            directory: directory.to_path_buf(),
            references,
        };
        snapshots.validate()?;
        Ok(snapshots)
    }

    pub(crate) fn references(&self) -> &TaskInputReferences {
        &self.references
    }

    pub(crate) fn validate(&self) -> Result<(), PersistenceError> {
        self.read_verified().map(|_| ())
    }

    pub(crate) fn read_verified(&self) -> Result<[Vec<u8>; 2], PersistenceError> {
        if read_references(&self.directory)? != self.references {
            return Err(invalid(
                &self.directory.join(MANIFEST),
                "task input checksum references changed",
            ));
        }
        self.validate_authority()?;
        Ok([
            self.references
                .config
                .read(&self.directory, Path::new(CONFIG))?,
            self.references
                .dependencies
                .read(&self.directory, Path::new(DEPENDENCIES))?,
        ])
    }

    pub(crate) fn verify_references(
        directory: &Path,
        references: &TaskInputReferences,
    ) -> Result<[Vec<u8>; 2], PersistenceError> {
        Self {
            directory: directory.to_path_buf(),
            references: references.clone(),
        }
        .read_verified()
    }

    fn validate_authority(&self) -> Result<(), PersistenceError> {
        for (name, format) in [
            ("program.json", "scientific-workflow-program-v2"),
            ("workflow-result.json", "scientific-workflow-task-result.v2"),
        ] {
            let path = self.directory.join(name);
            if !path.exists() {
                continue;
            }
            let value: serde_json::Value =
                serde_json::from_slice(&read(&path)?).map_err(|source| PersistenceError::Json {
                    operation: "read task input checksum authority",
                    path: path.clone(),
                    source,
                })?;
            if value["format"] != format {
                return Err(invalid(
                    &path,
                    "unsupported task input checksum authority format",
                ));
            }
            let references: TaskInputReferences = serde_json::from_value(value["inputs"].clone())
                .map_err(|source| PersistenceError::Json {
                operation: "read task input checksum references",
                path: path.clone(),
                source,
            })?;
            if references != self.references {
                return Err(invalid(
                    &path,
                    "task input checksum authority disagrees with stored references",
                ));
            }
        }
        Ok(())
    }
}

impl InputReference {
    fn new(path: &Path, bytes: &[u8]) -> Self {
        Self {
            path: path.to_path_buf(),
            checksum: checksum(bytes),
        }
    }

    fn read(&self, directory: &Path, expected: &Path) -> Result<Vec<u8>, PersistenceError> {
        if self.path.as_os_str() != expected.as_os_str() {
            return Err(invalid(
                &directory.join(MANIFEST),
                "task snapshot path is not canonical",
            ));
        }
        let digest = self.checksum.strip_prefix("sha256:").unwrap_or("");
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(invalid(
                &directory.join(MANIFEST),
                "invalid task snapshot SHA-256 checksum",
            ));
        }
        let path = directory.join(&self.path);
        let bytes = read(&path)?;
        if checksum(&bytes) != self.checksum {
            return Err(invalid(&path, "immutable task input snapshot was modified"));
        }
        Ok(bytes)
    }
}

fn read_references(directory: &Path) -> Result<TaskInputReferences, PersistenceError> {
    let path = directory.join(MANIFEST);
    let manifest: InputManifest =
        serde_json::from_slice(&read(&path)?).map_err(|source| PersistenceError::Json {
            operation: "read task input checksum manifest",
            path: path.clone(),
            source,
        })?;
    if manifest.format != FORMAT {
        return Err(invalid(
            &path,
            "unsupported task input checksum manifest format",
        ));
    }
    Ok(TaskInputReferences {
        config: manifest.config,
        dependencies: manifest.dependencies,
    })
}

fn read(path: &Path) -> Result<Vec<u8>, PersistenceError> {
    fs::read(path).map_err(|source| PersistenceError::Io {
        operation: "verify immutable task input snapshot",
        path: path.to_path_buf(),
        source,
    })
}

fn checksum(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut checksum = String::from("sha256:");
    for byte in Sha256::digest(bytes) {
        write!(checksum, "{byte:02x}").expect("writing to a String cannot fail");
    }
    checksum
}

fn invalid(path: &Path, reason: &str) -> PersistenceError {
    PersistenceError::InvalidMetadata {
        path: path.to_path_buf(),
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_checksums_reject_either_modified_input_and_changed_references() {
        let root =
            std::env::temp_dir().join(format!("workflow-input-checksums-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir(&root).unwrap();
        let snapshots = TaskInputSnapshots::create(&root, b"{}", b"[]").unwrap();
        assert!(TaskInputSnapshots::open(&root).is_ok());
        for (name, original) in [(CONFIG, b"{}".as_slice()), (DEPENDENCIES, b"[]".as_slice())] {
            fs::write(root.join(name), b" \n").unwrap();
            assert!(snapshots.validate().is_err());
            assert!(TaskInputSnapshots::open(&root).is_err());
            fs::write(root.join(name), original).unwrap();
        }
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(root.join(MANIFEST)).unwrap()).unwrap();
        manifest["config"]["path"] = "./workflow-config.json".into();
        fs::write(root.join(MANIFEST), serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(snapshots.validate().is_err());
        assert!(TaskInputSnapshots::open(&root).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
