//! Study composition and preflight failures.

use thiserror::Error;

use crate::config::advanced::ConfigError;
use crate::state::advanced::StateError;
use crate::task::advanced::ModelCatalogError;

/// A failure while compiling complete declared intent into an immutable study.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum StudyError {
    /// Config could not load or validate project declarations.
    #[error("failed to load project declarations")]
    Config(#[from] ConfigError),

    /// State rejected the centrally parsed state-schema document.
    #[error("failed to validate the project state schema")]
    State(#[from] StateError),

    /// Compiled model registrations are invalid or ambiguous.
    #[error("invalid compiled model registration: {reason}")]
    InvalidModelRegistration {
        /// Stable explanation of the invalid or duplicate registration.
        reason: String,
    },

    /// A manifest task references no compiled model.
    #[error("phase `{phase}` references unregistered model `{model}`")]
    UnknownModel {
        /// Phase containing the unresolved reference.
        phase: String,
        /// Stable model key requested by the manifest.
        model: String,
    },

    /// Typed constants or observation-plan binding failed during model preflight.
    #[error(
        "model `{model}` failed preflight for resolved parameters {ordinal} in phase `{phase}`"
    )]
    ModelPreflight {
        /// Phase containing the rejected invocation.
        phase: String,
        /// Stable compiled model key.
        model: String,
        /// Deterministic parameter expansion ordinal.
        ordinal: u64,
        /// Original config, observation, or model-declaration error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },

    /// The total expanded task count cannot be represented by stable ordinals.
    #[error("the study contains more expanded task invocations than can be identified")]
    TaskIdentityOverflow,
}

impl From<ModelCatalogError> for StudyError {
    fn from(source: ModelCatalogError) -> Self {
        Self::InvalidModelRegistration {
            reason: source.to_string(),
        }
    }
}

impl StudyError {
    pub(crate) fn model_preflight(
        phase: &str,
        model: &str,
        ordinal: u64,
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    ) -> Self {
        Self::ModelPreflight {
            phase: phase.to_owned(),
            model: model.to_owned(),
            ordinal,
            source,
        }
    }
}
