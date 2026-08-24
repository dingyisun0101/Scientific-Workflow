//! Errors produced while loading, expanding, and inspecting configuration.

use std::io;
use std::path::PathBuf;

use thiserror::Error;

/// A failure encountered while loading study settings, paths, or parameters.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ConfigurationError {
    /// A configuration source could not be read from the filesystem.
    #[error("failed to read configuration file `{path}`")]
    ReadConfigurationFile {
        /// Source path that could not be read.
        path: PathBuf,
        /// Underlying filesystem failure.
        #[source]
        source: io::Error,
    },

    /// A source file was not valid JSON.
    #[error("failed to parse configuration file `{path}`")]
    ParseConfigurationFile {
        /// Source path containing invalid JSON.
        path: PathBuf,
        /// Underlying JSON syntax failure.
        #[source]
        source: serde_json::Error,
    },

    /// Valid JSON did not conform to the configuration grammar.
    #[error("invalid configuration in `{path}`: {reason}")]
    InvalidConfigurationDocument {
        /// Rejected source path.
        path: PathBuf,
        /// Contextual grammar violation.
        reason: String,
    },

    /// A JSON object repeated a key that would otherwise be overwritten.
    #[error("configuration file `{path}` repeats key `{key}`")]
    DuplicateConfigurationKey {
        /// Source path containing the duplicate.
        path: PathBuf,
        /// Repeated JSON object key.
        key: String,
    },

    /// A requested group-qualified phase is not declared.
    #[error("study configuration has no phase `{phase_group}/{phase}`")]
    UnknownPhaseConfiguration {
        /// Requested phase-group key.
        phase_group: String,
        /// Requested phase key.
        phase: String,
    },

    /// The Cartesian product cannot be represented by a `u64` ordinal.
    #[error("configuration combination count overflows u64 while adding axis `{axis}`")]
    CombinationCountOverflow {
        /// Sweep axis or phase composition that caused the overflow.
        axis: String,
    },

    /// A requested flattened ordinal is outside its phase configuration.
    #[error(
        "combination ordinal {ordinal} is out of bounds for a configuration space containing {combination_count} combinations"
    )]
    CombinationOrdinalOutOfBounds {
        /// Requested zero-based ordinal.
        ordinal: u64,
        /// Number of valid combinations in the phase.
        combination_count: u64,
    },

    /// A resolved configuration does not contain a requested JSON Pointer.
    #[error("configuration ordinal {ordinal} does not contain value `{key}`")]
    UnknownConfigurationValue {
        /// Flattened combination ordinal being inspected.
        ordinal: u64,
        /// Missing canonical JSON Pointer.
        key: String,
    },

    /// A present configuration value could not be decoded as the requested type.
    #[error("failed to decode value `{key}` from configuration ordinal {ordinal}")]
    DecodeConfigurationValue {
        /// Flattened combination ordinal being decoded.
        ordinal: u64,
        /// Canonical JSON Pointer of the value.
        key: String,
        /// Underlying Serde conversion failure.
        #[source]
        source: serde_json::Error,
    },

    /// The named project-path table does not contain a requested key.
    #[error("project paths do not contain `{key}`")]
    UnknownProjectPath {
        /// Missing project-path key.
        key: String,
    },
}
