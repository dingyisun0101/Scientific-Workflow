//! Immutable central project-configuration graph.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Map, Value};

use super::document::read_json;
use super::error::ConfigError;

const STUDY_MANIFEST: &str = "study.json";
const CONFIG_DIRECTORY: &str = "config";

/// One immutable snapshot of every authored project JSON document.
#[derive(Clone)]
pub(crate) struct Config {
    inner: Arc<ConfigInner>,
}

struct ConfigInner {
    project_root: PathBuf,
    config_root: PathBuf,
    study_path: PathBuf,
    study: Value,
    documents: BTreeMap<PathBuf, ConfigDocument>,
    snapshot_json: Box<[u8]>,
}

struct ConfigDocument {
    path: PathBuf,
    value: Value,
}

impl Config {
    /// Loads the reserved study manifest and every JSON document beneath
    /// `<project-root>/config` exactly once.
    pub(crate) fn load(project_root: &Path) -> Result<Self, ConfigError> {
        let project_root = canonicalize(project_root)?;
        let config_root = canonicalize(&project_root.join(CONFIG_DIRECTORY))?;
        let study_path = project_root.join(STUDY_MANIFEST);
        let study = read_json(&study_path)?;

        let mut paths = Vec::new();
        discover_json(&config_root, &mut paths)?;
        paths.sort();

        let mut documents = BTreeMap::new();
        for path in paths {
            let canonical = canonicalize(&path)?;
            ensure_contained(&config_root, &canonical)?;
            let relative = canonical
                .strip_prefix(&config_root)
                .expect("contained config document has a relative path")
                .to_path_buf();
            let value = read_json(&canonical)?;
            documents.insert(
                relative,
                ConfigDocument {
                    path: canonical,
                    value,
                },
            );
        }

        let snapshot_json = snapshot_json(&study, &documents);
        Ok(Self {
            inner: Arc::new(ConfigInner {
                project_root,
                config_root,
                study_path,
                study,
                documents,
                snapshot_json,
            }),
        })
    }

    pub(crate) fn project_root(&self) -> &Path {
        &self.inner.project_root
    }

    pub(crate) fn config_root(&self) -> &Path {
        &self.inner.config_root
    }

    pub(crate) fn study_path(&self) -> &Path {
        &self.inner.study_path
    }

    pub(crate) fn study_value(&self) -> &Value {
        &self.inner.study
    }

    /// Retrieves one centrally parsed config document by its path relative to
    /// the config directory.
    pub(crate) fn document(&self, relative: &Path) -> Option<(&Path, &Value)> {
        self.inner
            .documents
            .get(relative)
            .map(|document| (document.path.as_path(), &document.value))
    }

    /// Returns the deterministic language-neutral snapshot supplied to
    /// program tasks.
    pub(crate) fn snapshot_json(&self) -> &[u8] {
        &self.inner.snapshot_json
    }
}

impl std::fmt::Debug for Config {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Config")
            .field("project_root", &self.project_root())
            .field("config_root", &self.config_root())
            .field("documents", &self.inner.documents.keys())
            .finish()
    }
}

fn discover_json(directory: &Path, paths: &mut Vec<PathBuf>) -> Result<(), ConfigError> {
    let entries = std::fs::read_dir(directory).map_err(|source| ConfigError::Read {
        path: directory.to_path_buf(),
        source,
    })?;
    let mut entries =
        entries
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| ConfigError::Read {
                path: directory.to_path_buf(),
                source,
            })?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        let file_type = entry.file_type().map_err(|source| ConfigError::Read {
            path: entry.path(),
            source,
        })?;
        if file_type.is_dir() {
            discover_json(&entry.path(), paths)?;
        } else if file_type.is_file()
            && entry.path().extension().and_then(std::ffi::OsStr::to_str) == Some("json")
        {
            paths.push(entry.path());
        }
    }
    Ok(())
}

fn snapshot_json(study: &Value, documents: &BTreeMap<PathBuf, ConfigDocument>) -> Box<[u8]> {
    let mut config = Map::new();
    for (relative, document) in documents {
        config.insert(
            relative
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/"),
            document.value.clone(),
        );
    }
    let snapshot = Value::Object(Map::from_iter([
        ("study".to_owned(), study.clone()),
        ("config".to_owned(), Value::Object(config)),
    ]));
    serde_json::to_vec_pretty(&snapshot)
        .expect("serializing centrally parsed JSON cannot fail")
        .into_boxed_slice()
}

pub(crate) fn canonicalize(path: &Path) -> Result<PathBuf, ConfigError> {
    std::fs::canonicalize(path).map_err(|source| ConfigError::Read {
        path: path.to_path_buf(),
        source,
    })
}

pub(crate) fn ensure_contained(root: &Path, path: &Path) -> Result<(), ConfigError> {
    if path.starts_with(root) {
        Ok(())
    } else {
        Err(ConfigError::PathOutsideConfig {
            path: path.to_path_buf(),
            config_root: root.to_path_buf(),
        })
    }
}
