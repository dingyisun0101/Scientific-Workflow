//! Immutable central project-configuration graph.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Map, Value};

use super::document::read_json;
use super::error::ConfigError;

const STUDY_MANIFEST: &str = "study.json";
const WORKFLOW_CONFIG_DIRECTORY: &str = "wf_configs";

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
}

/// Clone-cheap immutable language-neutral configuration snapshot for programs.
#[derive(Clone)]
pub(crate) struct ConfigSnapshot {
    bytes: Arc<[u8]>,
    parameters: Arc<Value>,
}

impl ConfigSnapshot {
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn parameters(&self) -> &Value {
        &self.parameters
    }
}

impl std::fmt::Debug for ConfigSnapshot {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ConfigSnapshot")
            .field("bytes", &self.bytes.len())
            .finish()
    }
}

struct ConfigDocument {
    path: PathBuf,
    value: Value,
}

impl Config {
    /// Loads the reserved study manifest and every other JSON document beneath
    /// `<project-root>/wf_configs` exactly once.
    pub(crate) fn load(project_root: &Path) -> Result<Self, ConfigError> {
        let project_root = canonicalize(project_root)?;
        ensure_utf8(&project_root, "project root")?;
        let config_root = canonicalize(&project_root.join(WORKFLOW_CONFIG_DIRECTORY))?;
        let study_path = canonicalize(&config_root.join(STUDY_MANIFEST))?;
        ensure_contained(&config_root, &study_path)?;
        let study = read_json(&study_path)?;

        let mut paths = Vec::new();
        discover_json(&config_root, &mut paths)?;
        paths.sort();

        let mut documents = BTreeMap::new();
        for path in paths {
            let canonical = canonicalize(&path)?;
            ensure_contained(&config_root, &canonical)?;
            ensure_utf8(&canonical, "configuration document")?;
            if canonical == study_path {
                continue;
            }
            let relative = path
                .strip_prefix(&config_root)
                .expect("contained config document has a relative path")
                .to_path_buf();
            if relative.to_str().is_none() {
                return Err(ConfigError::invalid(
                    &path,
                    "/",
                    "a config document path relative to `wf_configs/` must be valid UTF-8",
                ));
            }
            let value = read_json(&canonical)?;
            documents.insert(
                relative,
                ConfigDocument {
                    path: canonical,
                    value,
                },
            );
        }

        Ok(Self {
            inner: Arc::new(ConfigInner {
                project_root,
                config_root,
                study_path,
                study,
                documents,
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
    /// the `wf_configs` directory.
    pub(crate) fn document(&self, relative: &Path) -> Option<(&Path, &Value)> {
        self.inner
            .documents
            .get(relative)
            .map(|document| (document.path.as_path(), &document.value))
    }

    /// Resolves an authored project path to one already captured JSON document.
    pub(crate) fn project_document(
        &self,
        authored: &Path,
    ) -> Result<Option<(&Path, &Value)>, ConfigError> {
        let path = if authored.is_absolute() {
            authored.to_path_buf()
        } else {
            self.project_root().join(authored)
        };
        let canonical = canonicalize(&path)?;
        ensure_contained(self.config_root(), &canonical)?;
        Ok(self
            .inner
            .documents
            .values()
            .find(|document| document.path == canonical)
            .map(|document| (document.path.as_path(), &document.value)))
    }

    /// Builds a language-neutral snapshot with one resolved parameters document.
    pub(crate) fn snapshot_with_parameters(&self, parameters: &Value) -> ConfigSnapshot {
        ConfigSnapshot {
            bytes: Arc::from(snapshot_json_with_parameters(
                &self.inner.study,
                &self.inner.documents,
                parameters,
            )),
            parameters: Arc::new(parameters.clone()),
        }
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
        } else if (file_type.is_file() || file_type.is_symlink())
            && entry.path().extension().and_then(std::ffi::OsStr::to_str) == Some("json")
        {
            paths.push(entry.path());
        }
    }
    Ok(())
}

fn snapshot_json_with_parameters(
    study: &Value,
    documents: &BTreeMap<PathBuf, ConfigDocument>,
    parameters: &Value,
) -> Box<[u8]> {
    snapshot_json_inner(study, documents, parameters)
}

fn snapshot_json_inner(
    study: &Value,
    documents: &BTreeMap<PathBuf, ConfigDocument>,
    resolved_parameters: &Value,
) -> Box<[u8]> {
    let mut config = Map::new();
    for (relative, document) in documents {
        let value = if relative == Path::new("parameters.json") {
            resolved_parameters
        } else {
            &document.value
        };
        config.insert(
            relative
                .to_str()
                .expect("config paths were validated as UTF-8")
                .replace(std::path::MAIN_SEPARATOR, "/"),
            value.clone(),
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

/// Requires exact UTF-8 for paths that will later be represented in JSON.
pub(crate) fn ensure_utf8(path: &Path, context: &'static str) -> Result<(), ConfigError> {
    if path.to_str().is_some() {
        Ok(())
    } else {
        Err(ConfigError::NonUtf8Path {
            path: path.to_path_buf(),
            context,
        })
    }
}
