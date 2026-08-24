//! Strict, immutable named paths loaded from `config/paths.json`.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Map, Value};

use super::error::ConfigurationError;
use super::source::{
    StrictValue, invalid, parse_strict_json, read_source, require_object, validate_name,
};

const CONFIGURATION_DIRECTORY: &str = "config";
const PATHS_FILE: &str = "paths.json";

/// A validated read-only dictionary of project-wide named filesystem paths.
///
/// Loading preserves declaration order and exact source bytes. It does not
/// canonicalize paths, inspect targets, or require targets to exist.
#[derive(Clone)]
pub struct ProjectPaths {
    inner: Arc<ProjectPathsInner>,
}

impl ProjectPaths {
    /// Loads the standard `config/paths.json` beneath `project_root`.
    pub fn load(project_root: impl Into<PathBuf>) -> Result<Self, ConfigurationError> {
        let project_root = project_root.into();
        let source_path = project_root.join(CONFIGURATION_DIRECTORY).join(PATHS_FILE);
        let source = read_source(&source_path)?;
        let document = parse_strict_json(&source_path, &source)?;
        let fields = require_object(&source_path, document, "paths.json root must be an object")?;
        let mut entries = Vec::with_capacity(fields.len());
        let mut by_name = HashMap::with_capacity(fields.len());
        for (position, (name, value)) in fields.into_iter().enumerate() {
            validate_name(&source_path, &name, "project path")?;
            let StrictValue::String(raw) = value else {
                return invalid(
                    &source_path,
                    format!("project path `{name}` must be a JSON string"),
                );
            };
            if raw.trim().is_empty() {
                return invalid(
                    &source_path,
                    format!("project path `{name}` must not be empty or whitespace-only"),
                );
            }
            by_name.insert(name.clone().into_boxed_str(), position);
            entries.push(PathEntry {
                name: name.into_boxed_str(),
                source: raw.into_boxed_str(),
            });
        }

        Ok(Self {
            inner: Arc::new(ProjectPathsInner {
                project_root,
                source_path,
                source: source.into_boxed_slice(),
                entries,
                by_name,
            }),
        })
    }

    /// Returns the project root supplied to [`Self::load`].
    pub fn project_root(&self) -> &Path {
        &self.inner.project_root
    }

    /// Returns the exact `paths.json` source path.
    pub fn source_path(&self) -> &Path {
        &self.inner.source_path
    }

    /// Borrows the original validated source bytes without reserialization.
    pub fn source_json(&self) -> &[u8] {
        &self.inner.source
    }

    /// Returns the number of declared named paths.
    pub fn len(&self) -> usize {
        self.inner.entries.len()
    }

    /// Reports whether the path table contains no entries.
    pub fn is_empty(&self) -> bool {
        self.inner.entries.is_empty()
    }

    /// Reports whether `key` is declared.
    pub fn contains(&self, key: &str) -> bool {
        self.inner.by_name.contains_key(key)
    }

    /// Borrows the declared path without resolving it against the project root.
    pub fn path(&self, key: &str) -> Option<&Path> {
        let &position = self.inner.by_name.get(key)?;
        Some(Path::new(self.inner.entries[position].source.as_ref()))
    }

    /// Borrows a required declared path or returns an unknown-path error.
    pub fn require_path(&self, key: &str) -> Result<&Path, ConfigurationError> {
        self.path(key)
            .ok_or_else(|| ConfigurationError::UnknownProjectPath {
                key: key.to_owned(),
            })
    }

    /// Returns an absolute declaration unchanged or joins a relative one to the project root.
    pub fn resolve_path(&self, key: &str) -> Result<PathBuf, ConfigurationError> {
        let path = self.require_path(key)?;
        if path.is_absolute() {
            Ok(path.to_path_buf())
        } else {
            Ok(self.project_root().join(path))
        }
    }

    /// Iterates names in source declaration order.
    pub fn keys(&self) -> impl ExactSizeIterator<Item = &str> {
        self.inner.entries.iter().map(|entry| entry.name.as_ref())
    }

    /// Iterates names and unresolved paths in source declaration order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&str, &Path)> {
        self.inner
            .entries
            .iter()
            .map(|entry| (entry.name.as_ref(), Path::new(entry.source.as_ref())))
    }

    /// Returns a deterministic JSON object suitable for task provenance.
    pub fn to_json_value(&self) -> Value {
        Value::Object(
            self.inner
                .entries
                .iter()
                .map(|entry| {
                    (
                        entry.name.to_string(),
                        Value::String(entry.source.to_string()),
                    )
                })
                .collect::<Map<_, _>>(),
        )
    }
}

impl fmt::Debug for ProjectPaths {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProjectPaths")
            .field("project_root", &self.project_root())
            .field("source_path", &self.source_path())
            .field("entries", &self.len())
            .finish_non_exhaustive()
    }
}

struct ProjectPathsInner {
    project_root: PathBuf,
    source_path: PathBuf,
    source: Box<[u8]>,
    entries: Vec<PathEntry>,
    by_name: HashMap<Box<str>, usize>,
}

struct PathEntry {
    name: Box<str>,
    source: Box<str>,
}
