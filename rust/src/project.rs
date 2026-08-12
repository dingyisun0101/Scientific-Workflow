//! Conventional immutable definition of one scientific project.
//!
//! [`ScientificProject`] combines the standard three-file task configuration
//! with one state schema. The schema may belong to the project:
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
//! It combines immutable task/path configuration with the shared state schema
//! and delegates complete lazy task generation through [`ScientificProject::task_configs`].
//! It does not create execution directories, construct model payloads, run
//! tasks, or configure output streams. Fixed-model crates instead call
//! [`ScientificProject::load_with_state_schema`] and supply their canonical
//! schema, so each individual project needs only the other three files.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::Serialize;
use thiserror::Error;

use crate::configuration::{
    ConfigurationError, MatchingTaskConfigIter, ParameterSpace, ProjectConfig, ProjectPaths,
    TaskConfig, TaskConfigIter,
};
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
    /// A project-owned state schema could not be loaded or validated.
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

    /// Loads task and path configuration with a model-owned state schema.
    ///
    /// This form reads only `config/fixed.json`, `config/sweep.json`, and
    /// `config/paths.json`. It is intended for scientific crates whose public
    /// model fixes one canonical state contract and therefore rejects
    /// project-specific schema changes. The supplied schema is already
    /// validated by its type and is retained without reparsing or cloning its
    /// field allocation.
    pub fn load_with_state_schema(
        project_root: impl Into<PathBuf>,
        state_schema: SystemStateSchema,
    ) -> Result<Self, ScientificProjectError> {
        let configuration = ProjectConfig::load(project_root.into())?;
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

    /// Resolves one named project path against the project root.
    pub fn resolve_path(&self, key: &str) -> Result<PathBuf, ConfigurationError> {
        self.configuration.paths().resolve_path(key)
    }

    /// Returns the checked number of complete task configurations.
    pub fn task_count(&self) -> u64 {
        self.configuration.task_count()
    }

    /// Resolves one complete task configuration by deterministic ordinal.
    pub fn task_config(&self, ordinal: u64) -> Result<TaskConfig, ConfigurationError> {
        self.configuration.task_config(ordinal)
    }

    /// Lazily iterates every complete fixed/sweep/path task configuration.
    pub fn task_configs(&self) -> TaskConfigIter {
        self.configuration.task_configs()
    }

    /// Lazily iterates every task matching one exact sweep key/value pair.
    pub fn task_configs_matching<V>(
        &self,
        key: impl Into<String>,
        value: V,
    ) -> Result<MatchingTaskConfigIter, ConfigurationError>
    where
        V: Serialize,
    {
        self.configuration.task_configs_matching(key, value)
    }

    /// Returns the unique task matching one exact sweep key/value pair.
    pub fn unique_task_config_matching<V>(
        &self,
        key: impl Into<String>,
        value: V,
    ) -> Result<TaskConfig, ConfigurationError>
    where
        V: Serialize,
    {
        self.configuration.unique_task_config_matching(key, value)
    }

    /// Borrows the shared project-owned or model-owned system-state schema.
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
            .field("tasks", &self.task_count())
            .field("paths", &self.paths().len())
            .field("state_fields", &self.state_schema().len())
            .finish()
    }
}
