//! Configuration-owned project loading and parameter resolution failures.

use std::io;
use std::path::PathBuf;

use thiserror::Error;

/// A failure while compiling project documents into a specification.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ConfigError {
    /// A required project document could not be read.
    #[error("failed to read project document `{path}`")]
    Read {
        /// Document or directory path involved in the failed operation.
        path: PathBuf,
        /// Underlying filesystem failure.
        #[source]
        source: io::Error,
    },

    /// A project document was not valid JSON.
    #[error("failed to parse project document `{path}`")]
    Parse {
        /// Source document containing invalid JSON.
        path: PathBuf,
        /// Underlying JSON syntax failure.
        #[source]
        source: serde_json::Error,
    },

    /// A JSON object repeated a key.
    #[error("project document `{path}` repeats key `{key}`")]
    DuplicateKey {
        /// Source document containing the duplicate.
        path: PathBuf,
        /// Repeated object key.
        key: String,
    },

    /// Valid JSON did not satisfy its document grammar.
    #[error("invalid project document `{path}` at `{pointer}`: {reason}")]
    InvalidDocument {
        /// Rejected source document.
        path: PathBuf,
        /// Nearest meaningful JSON Pointer, or `/` for the root.
        pointer: String,
        /// Contextual grammar violation.
        reason: String,
    },

    /// A discovered document escaped the project configuration directory.
    #[error("document path `{path}` is outside configuration root `{config_root}`")]
    PathOutsideConfig {
        /// Rejected authored or resolved path.
        path: PathBuf,
        /// Canonical project configuration root.
        config_root: PathBuf,
    },

    /// A path that must cross a language-neutral JSON boundary is not UTF-8.
    #[error("{context} path `{path}` must be valid UTF-8")]
    NonUtf8Path {
        /// Rejected canonical filesystem path.
        path: PathBuf,
        /// Stable role the path would have served.
        context: &'static str,
    },

    /// A program or Python task did not resolve to a valid executable, script,
    /// or environment.
    #[error("invalid program task path `{path}`: {reason}")]
    InvalidProgram {
        /// Authored program path or command name.
        path: PathBuf,
        /// Stable resolution or validation failure.
        reason: String,
    },

    /// A phase dependency does not name a declared phase.
    #[error("phase `{phase}` depends on unknown phase `{dependency}`")]
    UnknownDependency {
        /// Referring phase.
        phase: String,
        /// Missing dependency.
        dependency: String,
    },

    /// A model task selects no state schema declared by the study.
    #[error("model `{model}` in phase `{phase}` selects unknown state `{state}`")]
    UnknownState {
        /// Containing phase key.
        phase: String,
        /// Compiled model key.
        model: String,
        /// Missing state-schema key.
        state: String,
    },

    /// A model-parameter expansion cannot be represented safely.
    #[error("model parameter expansion in `{path}` exceeds supported combination counts")]
    ExpansionOverflow {
        /// Parameters document whose selection product overflowed.
        path: PathBuf,
    },

    /// One resolved parameter combination could not decode as model constants.
    #[error(
        "failed to decode resolved parameter combination {ordinal} for model `{model}` from `{path}`"
    )]
    DecodeModelConstants {
        /// Compiled model key from the study manifest.
        model: String,
        /// Central parameters document path.
        path: PathBuf,
        /// Zero-based deterministic combination ordinal.
        ordinal: u64,
        /// Underlying Serde conversion failure.
        #[source]
        source: serde_json::Error,
    },
}

impl ConfigError {
    pub(crate) fn invalid(
        path: &std::path::Path,
        pointer: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self::InvalidDocument {
            path: path.to_path_buf(),
            pointer: pointer.into(),
            reason: reason.into(),
        }
    }
}
