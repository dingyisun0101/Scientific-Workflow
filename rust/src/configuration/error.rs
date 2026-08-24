//! Errors produced while loading, expanding, and inspecting configuration.

use std::io;
use std::path::PathBuf;

use thiserror::Error;

/// A failure encountered while resolving `fixed.json` and `sweep.json`.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ConfigurationError {
    #[error("failed to read configuration file `{path}`")]
    ReadConfigurationFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },

    #[error("failed to parse configuration file `{path}`")]
    ParseConfigurationFile {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("invalid configuration in `{path}`: {reason}")]
    InvalidConfigurationDocument { path: PathBuf, reason: String },

    #[error("configuration file `{path}` repeats key `{key}`")]
    DuplicateConfigurationKey { path: PathBuf, key: String },

    #[error(
        "parameter `{key}` appears in both fixed configuration `{fixed_path}` and sweep configuration `{sweep_path}`"
    )]
    FixedSweepKeyConflict {
        key: String,
        fixed_path: PathBuf,
        sweep_path: PathBuf,
    },

    #[error("configuration combination count overflows u64 while adding axis `{axis}`")]
    CombinationCountOverflow { axis: String },

    #[error(
        "combination ordinal {ordinal} is out of bounds for a configuration space containing {combination_count} combinations"
    )]
    CombinationOrdinalOutOfBounds {
        ordinal: u64,
        combination_count: u64,
    },

    #[error("configuration ordinal {ordinal} does not contain value `{key}`")]
    UnknownConfigurationValue { ordinal: u64, key: String },

    #[error("failed to decode value `{key}` from configuration ordinal {ordinal}")]
    DecodeConfigurationValue {
        ordinal: u64,
        key: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("failed to serialize resolved configuration ordinal {ordinal}")]
    SerializeResolvedConfiguration {
        ordinal: u64,
        #[source]
        source: serde_json::Error,
    },
}
