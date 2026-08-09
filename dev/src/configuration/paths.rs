//! Immutable named project paths loaded from `config/paths.json`.
//!
//! [`ProjectPaths`] separates filesystem locations from scientific task
//! parameters. Every JSON value must be a string. The original string becomes
//! a [`PathBuf`] for direct inspection, while relative paths can be joined to
//! the configured project root through [`ProjectPaths::resolve_path`]. Loading
//! does not canonicalize paths, expand environment variables or `~`, inspect
//! target metadata, or require a target to exist.
//!
//! The complete validated source bytes are retained unchanged for the exact
//! three-file export performed by the later `ProjectConfig` facade. Parsed path
//! entries remain in JSON declaration order and are shared by every clone.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::error::ConfigurationError;
use super::parameters::{
    StrictValue, invalid, parse_strict_json, read_source, require_object, validate_name,
};

const CONFIGURATION_DIRECTORY: &str = "config";
const PATHS_FILE: &str = "paths.json";

/// A validated read-only dictionary of project-wide named filesystem paths.
///
/// Cloning this type clones only an [`Arc`]. The project root, exact source
/// bytes, declaration-ordered entries, and lookup index remain in one shared
/// immutable allocation.
#[derive(Clone)]
pub struct ProjectPaths {
    inner: Arc<ProjectPathsInner>,
}

impl ProjectPaths {
    /// Loads the standard `config/paths.json` beneath `project_root`.
    ///
    /// `project_root` is retained exactly as supplied. Relative configured paths
    /// are later joined to it lexically; neither the root nor configured values
    /// are canonicalized.
    ///
    /// # Errors
    ///
    /// Returns contextual file or JSON errors, recursive duplicate-key errors,
    /// or [`ConfigurationError::InvalidConfigurationDocument`] when the root is
    /// not an object or a key has a non-string or empty path value.
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
                raw: PathBuf::from(raw),
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

    /// Returns the project root exactly as supplied at load time.
    pub fn project_root(&self) -> &Path {
        &self.inner.project_root
    }

    /// Returns the derived `config/paths.json` source path.
    pub fn source_path(&self) -> &Path {
        &self.inner.source_path
    }

    /// Borrows the validated original bytes of `paths.json` unchanged.
    ///
    /// The slice preserves whitespace, declaration order, escaping, and every
    /// other byte-level source detail.
    pub fn source_json(&self) -> &[u8] {
        &self.inner.source
    }

    /// Returns the number of declared path names.
    pub fn len(&self) -> usize {
        self.inner.entries.len()
    }

    /// Reports whether `paths.json` declares no entries.
    pub fn is_empty(&self) -> bool {
        self.inner.entries.is_empty()
    }

    /// Reports whether an exact, case-sensitive path key is declared.
    pub fn contains(&self, key: &str) -> bool {
        self.inner.by_name.contains_key(key)
    }

    /// Borrows one configured path exactly as represented by its JSON string.
    ///
    /// Relative values remain relative. Missing keys return `None`; no
    /// filesystem operation or allocation occurs.
    pub fn path(&self, key: &str) -> Option<&Path> {
        let &position = self.inner.by_name.get(key)?;
        Some(&self.inner.entries[position].raw)
    }

    /// Borrows one required configured path or returns its exact missing key.
    pub fn require_path(&self, key: &str) -> Result<&Path, ConfigurationError> {
        self.path(key)
            .ok_or_else(|| ConfigurationError::UnknownProjectPath {
                key: key.to_owned(),
            })
    }

    /// Returns one path resolved lexically against the project root.
    ///
    /// Absolute configured paths are returned unchanged. Relative paths are
    /// joined with [`ProjectPaths::project_root`]. The result is not
    /// canonicalized, normalized, opened, or checked for existence.
    pub fn resolve_path(&self, key: &str) -> Result<PathBuf, ConfigurationError> {
        let path = self.require_path(key)?;
        if path.is_absolute() {
            Ok(path.to_path_buf())
        } else {
            Ok(self.project_root().join(path))
        }
    }

    /// Iterates exact path keys in JSON declaration order.
    pub fn keys(&self) -> impl ExactSizeIterator<Item = &str> {
        self.inner.entries.iter().map(|entry| entry.name.as_ref())
    }

    /// Iterates declaration-ordered exact keys and unresolved path values.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&str, &Path)> {
        self.inner
            .entries
            .iter()
            .map(|entry| (entry.name.as_ref(), entry.raw.as_path()))
    }
}

impl fmt::Debug for ProjectPaths {
    /// Formats only the project root, source path, and entry count.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProjectPaths")
            .field("project_root", &self.project_root())
            .field("source_path", &self.source_path())
            .field("entries", &self.len())
            .finish_non_exhaustive()
    }
}

/// Shared immutable allocation behind every `ProjectPaths` clone.
struct ProjectPathsInner {
    project_root: PathBuf,
    source_path: PathBuf,
    source: Box<[u8]>,
    entries: Vec<PathEntry>,
    by_name: HashMap<Box<str>, usize>,
}

/// One declaration-ordered exact name and unresolved path value.
struct PathEntry {
    name: Box<str>,
    raw: PathBuf,
}
