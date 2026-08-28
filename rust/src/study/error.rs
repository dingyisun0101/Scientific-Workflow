//! Study composition and preflight failures.

use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::config::advanced::ConfigError;
use crate::state::advanced::StateError;
use crate::task::advanced::ExecutionUnitCatalogError;

/// A failure while compiling complete declared intent into an immutable study.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum StudyError {
    /// Config could not load or validate project declarations.
    #[error("failed to load project declarations")]
    Config(#[from] ConfigError),

    /// State rejected one centrally parsed named state-schema document.
    #[error("failed to validate state schema `{state}` from `{path}`")]
    State {
        /// Semantic state key declared by the study manifest.
        state: String,
        /// Canonical source document path retained by Config.
        path: PathBuf,
        /// Original State semantic-validation failure.
        #[source]
        source: StateError,
    },

    /// Compiled execution-unit registrations are invalid or ambiguous.
    #[error("invalid compiled execution-unit registration: {reason}")]
    InvalidExecutionUnitRegistration {
        /// Stable explanation of the invalid or duplicate registration.
        reason: String,
    },

    /// A manifest task references no compiled execution unit.
    #[error("phase `{phase}` references unregistered execution unit `{execution_unit}`")]
    UnknownExecutionUnit {
        /// Phase containing the unresolved reference.
        phase: String,
        /// Stable execution-unit key requested by the manifest.
        execution_unit: String,
    },

    /// Typed constants or observation-plan binding failed during execution-unit preflight.
    #[error(
        "execution unit `{execution_unit}` failed preflight for resolved parameters {ordinal} in phase `{phase}`"
    )]
    ExecutionUnitPreflight {
        /// Phase containing the rejected invocation.
        phase: String,
        /// Stable compiled execution-unit key.
        execution_unit: String,
        /// Deterministic parameter expansion ordinal.
        ordinal: u64,
        /// Original config, observation, or execution-unit declaration error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },

    /// The total expanded task count cannot be represented by stable ordinals.
    #[error("the study contains more expanded task invocations than can be identified")]
    TaskIdentityOverflow,
}

impl From<ExecutionUnitCatalogError> for StudyError {
    fn from(source: ExecutionUnitCatalogError) -> Self {
        Self::InvalidExecutionUnitRegistration {
            reason: source.to_string(),
        }
    }
}

impl StudyError {
    pub(crate) fn state_schema(state: &str, path: &Path, source: StateError) -> Self {
        Self::State {
            state: state.to_owned(),
            path: path.to_path_buf(),
            source,
        }
    }

    pub(crate) fn execution_unit_preflight(
        phase: &str,
        execution_unit: &str,
        ordinal: u64,
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    ) -> Self {
        Self::ExecutionUnitPreflight {
            phase: phase.to_owned(),
            execution_unit: execution_unit.to_owned(),
            ordinal,
            source,
        }
    }
}
