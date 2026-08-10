//! Coordinated loading and exact export of standard project configuration.
//!
//! [`ProjectConfig`] is the normal entry point for the on-disk layout:
//!
//! ```text
//! project-root/
//! └── config/
//!     ├── fixed.json
//!     ├── sweep.json
//!     └── paths.json
//! ```
//!
//! Loading delegates scientific parameter expansion to [`ParameterSpace`] and
//! path semantics to [`ProjectPaths`]. The facade then combines their shared
//! handles into complete [`TaskConfig`] values for task execution. Exact export
//! writes the original validated source bytes, not a reserialized representation.
//!
//! # Export publication
//!
//! [`ProjectConfig::write_source_config`] never overwrites an existing
//! `config/` entry. It exclusively creates that directory, exclusively creates
//! and synchronizes all three files, syncs the configuration directory, and
//! finally syncs the project root. A failed operation may retain the newly
//! created partial directory as diagnostic evidence, but it never replaces
//! existing configuration.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::iter::FusedIterator;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::error::ConfigurationError;
use super::parameters::{ParameterSpace, TaskParameters, TaskParametersIter};
use super::paths::ProjectPaths;

const CONFIGURATION_DIRECTORY: &str = "config";
const FIXED_FILE: &str = "fixed.json";
const SWEEP_FILE: &str = "sweep.json";
const PATHS_FILE: &str = "paths.json";

/// Complete validated configuration for one scientific project.
///
/// This facade keeps fixed/sweep expansion and path storage internally distinct
/// while exposing complete task handles loaded from the same standard root.
/// Cloning it is lightweight because the component values share their parsed
/// allocations through [`std::sync::Arc`].
#[derive(Clone)]
pub struct ProjectConfig {
    project_root: PathBuf,
    parameters: ParameterSpace,
    paths: ProjectPaths,
}

impl ProjectConfig {
    /// Loads all three standard JSON files beneath `project_root/config`.
    ///
    /// Loading is read-only. The supplied project root is retained without
    /// canonicalization, and no configured path target needs to exist.
    ///
    /// # Errors
    ///
    /// Returns the precise [`ConfigurationError`] produced by fixed/sweep
    /// loading or path loading. A caller never receives a partially validated
    /// `ProjectConfig`.
    pub fn load(project_root: impl Into<PathBuf>) -> Result<Self, ConfigurationError> {
        let project_root = project_root.into();
        let configuration_directory = project_root.join(CONFIGURATION_DIRECTORY);
        let parameters = ParameterSpace::load(&configuration_directory)?;
        let paths = ProjectPaths::load(&project_root)?;
        Ok(Self {
            project_root,
            parameters,
            paths,
        })
    }

    /// Returns the project root exactly as supplied at load time.
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    /// Returns the standard `config/` directory derived from the project root.
    pub fn configuration_directory(&self) -> &Path {
        self.parameters.configuration_directory()
    }

    /// Borrows the validated fixed-and-swept parameter space.
    pub fn parameters(&self) -> &ParameterSpace {
        &self.parameters
    }

    /// Borrows the validated named project-path dictionary.
    pub fn paths(&self) -> &ProjectPaths {
        &self.paths
    }

    /// Returns the checked number of complete task configurations.
    pub fn task_count(&self) -> u64 {
        self.parameters.task_count()
    }

    /// Resolves one complete task configuration by deterministic ordinal.
    ///
    /// The returned handle shares all parsed fixed, sweep, and path storage.
    /// No merged parameter map or path table is allocated.
    pub fn task_config(&self, ordinal: u64) -> Result<TaskConfig, ConfigurationError> {
        Ok(TaskConfig {
            parameters: self.parameters.task(ordinal)?,
            paths: self.paths.clone(),
        })
    }

    /// Lazily iterates every complete task configuration.
    ///
    /// Cartesian sweeps yield their full product in canonical task order, with
    /// the final axis changing fastest. Explicit-case sweeps yield exactly the
    /// declared cases. Iterator items are cheap owned handles suitable for
    /// moving into scoped work or dispatcher queues.
    pub fn task_configs(&self) -> TaskConfigIter {
        TaskConfigIter {
            parameters: self.parameters.tasks(),
            paths: self.paths.clone(),
        }
    }

    /// Lazily yields every task whose selected sweep value exactly matches
    /// `value`.
    ///
    /// Other sweep dimensions remain unconstrained, so a Cartesian project can
    /// yield several configurations. Selection is restricted to sweep keys;
    /// fixed constants and paths do not define task identity. `value` is
    /// converted to JSON once and compared with exact JSON equality.
    pub fn task_configs_matching<V>(
        &self,
        key: impl Into<String>,
        value: V,
    ) -> Result<MatchingTaskConfigIter, ConfigurationError>
    where
        V: Serialize,
    {
        let key = key.into();
        if !self
            .parameters
            .sweep_keys()
            .any(|candidate| candidate == key)
        {
            return Err(ConfigurationError::UnknownSweepParameter { key });
        }
        let value = serde_json::to_value(value).map_err(|source| {
            ConfigurationError::EncodeTaskSelection {
                key: key.clone(),
                source,
            }
        })?;
        Ok(MatchingTaskConfigIter {
            tasks: self.task_configs(),
            key: key.into_boxed_str(),
            value,
        })
    }

    /// Returns the only task matching one exact sweep key/value pair.
    ///
    /// No match and multiple matches are distinct errors. In a multidimensional
    /// Cartesian sweep, callers should normally use
    /// [`ProjectConfig::task_configs_matching`] unless the selected key is
    /// known to identify one task uniquely.
    pub fn unique_task_config_matching<V>(
        &self,
        key: impl Into<String>,
        value: V,
    ) -> Result<TaskConfig, ConfigurationError>
    where
        V: Serialize,
    {
        let key = key.into();
        let mut matches = self.task_configs_matching(key.clone(), value)?;
        let task = matches
            .next()
            .ok_or_else(|| ConfigurationError::NoMatchingTaskConfiguration { key: key.clone() })?;
        if matches.next().is_some() {
            return Err(ConfigurationError::AmbiguousTaskConfiguration { key });
        }
        Ok(task)
    }

    /// Consumes the facade and returns its parameter and path components.
    ///
    /// Both returned handles retain their shared source allocations. No source
    /// bytes, parsed JSON values, path values, or task parameters are cloned.
    pub fn into_parts(self) -> (ParameterSpace, ProjectPaths) {
        (self.parameters, self.paths)
    }

    /// Writes an exact non-overwriting copy beneath `destination_project_root`.
    ///
    /// The destination root is created when absent. Publication refuses an
    /// existing `config/` path. All three files are created exclusively from
    /// the original validated byte slices and synchronized before directory
    /// publication is considered durable.
    ///
    /// # Errors
    ///
    /// Any root creation, exclusive configuration/file creation, write, or
    /// sync failure is returned as
    /// [`ConfigurationError::WriteConfigurationFile`] with the exact path at
    /// which it occurred. Existing destination data is never overwritten.
    pub fn write_source_config(
        &self,
        destination_project_root: impl AsRef<Path>,
    ) -> Result<(), ConfigurationError> {
        let destination_project_root = destination_project_root.as_ref();
        create_destination_root(destination_project_root)?;
        let destination = destination_project_root.join(CONFIGURATION_DIRECTORY);
        create_configuration_directory(&destination)?;

        write_source_file(
            &destination.join(FIXED_FILE),
            self.parameters.fixed_source_json(),
        )?;
        write_source_file(
            &destination.join(SWEEP_FILE),
            self.parameters.sweep_source_json(),
        )?;
        write_source_file(&destination.join(PATHS_FILE), self.paths.source_json())?;
        sync_directory(&destination)?;
        sync_directory(destination_project_root)
    }
}

/// One complete immutable task configuration.
///
/// This is an owned handle rather than an owned copy of configuration data.
/// [`TaskParameters`] shares fixed/sweep storage and [`ProjectPaths`] shares
/// path storage through independent [`std::sync::Arc`] allocations, making a
/// `TaskConfig` cheap to move into worker queues and safe to retain after the
/// originating [`ProjectConfig`] is dropped.
#[derive(Clone)]
pub struct TaskConfig {
    parameters: TaskParameters,
    paths: ProjectPaths,
}

impl TaskConfig {
    /// Returns the stable zero-based task ordinal.
    pub fn task_ordinal(&self) -> u64 {
        self.parameters.task_ordinal()
    }

    /// Borrows the fixed-plus-selected-sweep dictionary.
    pub fn parameters(&self) -> &TaskParameters {
        &self.parameters
    }

    /// Borrows the shared project path dictionary.
    pub fn paths(&self) -> &ProjectPaths {
        &self.paths
    }

    /// Borrows one fixed or selected sweep value by exact key.
    pub fn value(&self, key: &str) -> Option<&Value> {
        self.parameters.value(key)
    }

    /// Borrows one required fixed or selected sweep value.
    pub fn require_value(&self, key: &str) -> Result<&Value, ConfigurationError> {
        self.parameters.require_value(key)
    }

    /// Decodes one required parameter into the requested concrete Rust type.
    pub fn decode_value<T>(&self, key: &str) -> Result<T, ConfigurationError>
    where
        T: DeserializeOwned,
    {
        self.parameters.decode_value(key)
    }

    /// Resolves one named path lexically against the project root.
    pub fn resolve_path(&self, key: &str) -> Result<PathBuf, ConfigurationError> {
        self.paths.resolve_path(key)
    }
}

impl fmt::Debug for TaskConfig {
    /// Formats task identity and bounded dictionary counts without values.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TaskConfig")
            .field("task_ordinal", &self.task_ordinal())
            .field("parameters", &self.parameters.len())
            .field("paths", &self.paths.len())
            .finish_non_exhaustive()
    }
}

/// Owning lazy iterator over every complete task configuration.
#[derive(Clone)]
pub struct TaskConfigIter {
    parameters: TaskParametersIter,
    paths: ProjectPaths,
}

impl Iterator for TaskConfigIter {
    type Item = TaskConfig;

    fn next(&mut self) -> Option<Self::Item> {
        self.parameters.next().map(|parameters| TaskConfig {
            parameters,
            paths: self.paths.clone(),
        })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.parameters.size_hint()
    }
}

impl FusedIterator for TaskConfigIter {}

impl fmt::Debug for TaskConfigIter {
    /// Formats only the underlying ordinal range and shared path count.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TaskConfigIter")
            .field("parameters", &self.parameters)
            .field("paths", &self.paths.len())
            .finish_non_exhaustive()
    }
}

/// Lazy exact-JSON filter over complete task configurations.
pub struct MatchingTaskConfigIter {
    tasks: TaskConfigIter,
    key: Box<str>,
    value: Value,
}

impl Iterator for MatchingTaskConfigIter {
    type Item = TaskConfig;

    fn next(&mut self) -> Option<Self::Item> {
        self.tasks
            .find(|task| task.value(&self.key) == Some(&self.value))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, self.tasks.size_hint().1)
    }
}

impl FusedIterator for MatchingTaskConfigIter {}

impl fmt::Debug for MatchingTaskConfigIter {
    /// Formats the selector key without exposing its potentially large value.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MatchingTaskConfigIter")
            .field("key", &self.key)
            .field("tasks", &self.tasks)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for ProjectConfig {
    /// Formats only bounded roots and component counts.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProjectConfig")
            .field("project_root", &self.project_root())
            .field("parameters", &self.parameters.parameter_count())
            .field("tasks", &self.parameters.task_count())
            .field("paths", &self.paths.len())
            .finish_non_exhaustive()
    }
}

/// Creates a missing destination root or verifies that an existing entry is a
/// directory.
fn create_destination_root(path: &Path) -> Result<(), ConfigurationError> {
    match fs::create_dir_all(path) {
        Ok(()) => Ok(()),
        Err(source) => Err(write_error(path.to_path_buf(), source)),
    }
}

/// Exclusively creates the standard destination directory, closing the
/// check/create race without platform-specific rename semantics.
fn create_configuration_directory(path: &Path) -> Result<(), ConfigurationError> {
    fs::create_dir(path).map_err(|source| write_error(path.to_path_buf(), source))
}

/// Exclusively creates, writes, and synchronizes one exact source file.
fn write_source_file(path: &Path, source_bytes: &[u8]) -> Result<(), ConfigurationError> {
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| write_error(path.to_path_buf(), source))?;
    output
        .write_all(source_bytes)
        .map_err(|source| write_error(path.to_path_buf(), source))?;
    output
        .sync_all()
        .map_err(|source| write_error(path.to_path_buf(), source))
}

/// Synchronizes directory-entry changes at one publication boundary.
fn sync_directory(path: &Path) -> Result<(), ConfigurationError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| write_error(path.to_path_buf(), source))
}

/// Constructs the shared exact-export IO variant.
fn write_error(path: PathBuf, source: io::Error) -> ConfigurationError {
    ConfigurationError::WriteConfigurationFile { path, source }
}
