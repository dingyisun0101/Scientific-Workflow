//! Conventional immutable definition of one scientific project.
//!
//! [`ScientificProject`] is the normal entry point for the complete standard
//! project layout:
//!
//! ```text
//! project-root/
//! └── config/
//!     ├── fixed.json
//!     ├── sweep.json
//!     ├── paths.json
//!     └── state.json
//! ```
//!
//! It combines immutable task/path configuration with the shared state schema.
//! It does not create execution directories, construct model payloads, run
//! tasks, or configure output streams.

use std::fmt;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::configuration::{ConfigurationError, ParameterSpace, ProjectConfig, ProjectPaths};
use crate::system_state::{StateError, SystemStateSchema};

/// Standard filename of the system-state schema beneath `config/`.
const STATE_SCHEMA_FILE: &str = "state.json";

/// Failure while loading a complete conventional scientific project.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ScientificProjectError {
    /// Fixed, sweep, or path configuration was invalid.
    #[error(transparent)]
    Configuration(#[from] ConfigurationError),
    /// The mandatory state schema could not be loaded or validated.
    #[error(transparent)]
    State(#[from] StateError),
}

/// Immutable configuration and state schema for one scientific project.
///
/// Cloning shares the parsed parameter, path, and schema allocations; it does
/// not clone JSON values or scientific payloads.
#[derive(Clone)]
pub struct ScientificProject {
    configuration: ProjectConfig,
    state_schema: SystemStateSchema,
}

impl ScientificProject {
    /// Loads all four conventional JSON documents from `project_root/config`.
    pub fn load(project_root: impl Into<PathBuf>) -> Result<Self, ScientificProjectError> {
        let project_root = project_root.into();
        let configuration = ProjectConfig::load(&project_root)?;
        let state_schema = SystemStateSchema::load_json_template(
            configuration
                .configuration_directory()
                .join(STATE_SCHEMA_FILE),
        )?;
        Ok(Self {
            configuration,
            state_schema,
        })
    }

    /// Returns the project root exactly as supplied during loading.
    pub fn project_root(&self) -> &Path {
        self.configuration.project_root()
    }

    /// Returns the conventional `config/` directory.
    pub fn configuration_directory(&self) -> &Path {
        self.configuration.configuration_directory()
    }

    /// Borrows the fixed-and-swept task parameter space.
    pub fn parameters(&self) -> &ParameterSpace {
        self.configuration.parameters()
    }

    /// Borrows the named project path dictionary.
    pub fn paths(&self) -> &ProjectPaths {
        self.configuration.paths()
    }

    /// Borrows the shared system-state schema loaded from `config/state.json`.
    pub fn state_schema(&self) -> &SystemStateSchema {
        &self.state_schema
    }

    /// Borrows the lower-level three-file configuration facade.
    pub fn configuration(&self) -> &ProjectConfig {
        &self.configuration
    }

    /// Consumes the project and returns its configuration and state schema.
    pub fn into_parts(self) -> (ProjectConfig, SystemStateSchema) {
        (self.configuration, self.state_schema)
    }
}

impl fmt::Debug for ScientificProject {
    /// Formats bounded project facts without source JSON or payload data.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ScientificProject")
            .field("project_root", &self.project_root())
            .field("parameters", &self.parameters().parameter_count())
            .field("tasks", &self.parameters().task_count())
            .field("paths", &self.paths().len())
            .field("state_fields", &self.state_schema().len())
            .finish()
    }
}
