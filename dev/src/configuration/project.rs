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
//! path semantics to [`ProjectPaths`]. Exact export writes the original
//! validated source bytes, not a reserialized representation.
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
use std::path::{Path, PathBuf};

use super::error::ConfigurationError;
use super::parameters::ParameterSpace;
use super::paths::ProjectPaths;

const CONFIGURATION_DIRECTORY: &str = "config";
const FIXED_FILE: &str = "fixed.json";
const SWEEP_FILE: &str = "sweep.json";
const PATHS_FILE: &str = "paths.json";

/// Complete validated configuration for one scientific project.
///
/// This facade keeps fixed/sweep expansion and path resolution distinct while
/// guaranteeing that both were loaded from the same standard project root.
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
