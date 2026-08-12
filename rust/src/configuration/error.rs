//! Errors produced while loading, expanding, inspecting, and exporting project
//! configuration.
//!
//! This module defines the complete public failure vocabulary for the standard
//! `config/fixed.json`, `config/sweep.json`, and `config/paths.json` workflow.
//! Errors retain owned paths, task indices, and exact JSON keys so callers may
//! report them after the originating [`ParameterSpace`](super::ParameterSpace)
//! or [`ProjectConfig`](super::ProjectConfig) has been dropped.
//!
//! # Error boundaries
//!
//! Filesystem and JSON mechanics preserve their original errors through
//! [`std::error::Error::source`]. Semantic failures—such as a fixed/sweep key
//! collision or an out-of-range task ordinal—carry their complete context
//! directly because no lower-level error produced them.
//!
//! Configuration errors never contain a resolved task dictionary or scientific
//! payload. In particular, a typed parameter-decoding failure retains the task
//! index and parameter name but not the potentially large JSON value.

use std::io;
use std::path::PathBuf;

use thiserror::Error;

/// A failure encountered while loading or using standardized project
/// configuration.
///
/// Variants are grouped conceptually by source-file IO, source-document
/// validation, task-space expansion, resolved task access, and exact source
/// export. The enum is non-exhaustive so later configuration formats can add
/// precise diagnostics without forcing downstream exhaustive matches.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ConfigurationError {
    /// One of the three standard JSON files could not be read.
    #[error("failed to read project configuration file `{path}`")]
    ReadConfigurationFile {
        /// Exact source path selected by the standard project layout.
        path: PathBuf,
        /// Underlying filesystem failure.
        #[source]
        source: io::Error,
    },

    /// A readable configuration file did not contain valid JSON in its
    /// required document shape.
    ///
    /// Duplicate object keys are detected during deserialization and reported
    /// through this variant rather than silently retaining the final value.
    #[error("failed to parse project configuration file `{path}`")]
    ParseConfigurationFile {
        /// Source document containing malformed or structurally invalid JSON.
        path: PathBuf,
        /// Underlying JSON syntax or data-model failure.
        #[source]
        source: serde_json::Error,
    },

    /// A syntactically valid source document violated a configuration
    /// invariant.
    ///
    /// Examples include an empty parameter name, a Cartesian axis without
    /// candidates, inconsistent explicit-case key sets, or a non-string path
    /// value. `reason` is intended for diagnostics; callers that need stable
    /// programmatic distinctions should match one of the dedicated variants
    /// below where available.
    #[error("invalid project configuration in `{path}`: {reason}")]
    InvalidConfigurationDocument {
        /// Configuration file whose semantic content was rejected.
        path: PathBuf,
        /// Concise description of the violated invariant.
        reason: String,
    },

    /// One JSON object repeated an exact key.
    ///
    /// JSON parsers often retain only the last duplicate entry. Scientific
    /// configuration rejects that ambiguity before constructing a parameter
    /// space or path table.
    #[error("project configuration file `{path}` repeats key `{key}`")]
    DuplicateConfigurationKey {
        /// Source document containing the duplicate declaration.
        path: PathBuf,
        /// Exact, unnormalized JSON key that appeared more than once.
        key: String,
    },

    /// A parameter was declared as both fixed and swept.
    ///
    /// Fixed values are never defaults or override targets. Keeping the two key
    /// sets disjoint makes every resolved lookup unambiguous.
    #[error(
        "parameter `{key}` appears in both fixed configuration `{fixed_path}` and sweep configuration `{sweep_path}`"
    )]
    FixedSweepKeyConflict {
        /// Exact colliding parameter name.
        key: String,
        /// Standard fixed-parameter source path.
        fixed_path: PathBuf,
        /// Standard sweep-definition source path.
        sweep_path: PathBuf,
    },

    /// Multiplying Cartesian axis lengths exceeded the supported `u64` task
    /// count.
    #[error("parameter sweep task count overflows u64 while adding axis `{axis}`")]
    TaskCountOverflow {
        /// Axis whose candidate count caused the checked product to overflow.
        axis: String,
    },

    /// Indexed task lookup addressed an ordinal outside the generated space.
    #[error(
        "task ordinal {ordinal} is out of bounds for a parameter space containing {task_count} tasks"
    )]
    TaskOrdinalOutOfBounds {
        /// Requested zero-based task ordinal.
        ordinal: u64,
        /// Total number of deterministic task combinations.
        task_count: u64,
    },

    /// Task selection named a fixed or absent key instead of a sweep key.
    #[error("task selection key `{key}` is not declared by sweep.json")]
    UnknownSweepParameter {
        /// Exact, case-sensitive selection key supplied by the caller.
        key: String,
    },

    /// A caller-provided typed selector could not be represented as JSON.
    #[error("failed to encode task selection value for sweep parameter `{key}`")]
    EncodeTaskSelection {
        /// Exact sweep key whose target value was being encoded.
        key: String,
        /// Underlying Serde JSON conversion failure.
        #[source]
        source: serde_json::Error,
    },

    /// No generated task has the requested exact sweep value.
    #[error("no task configuration matches sweep parameter `{key}`")]
    NoMatchingTaskConfiguration {
        /// Exact sweep key used for selection.
        key: String,
    },

    /// One key/value selector matched more than one generated task.
    #[error("more than one task configuration matches sweep parameter `{key}`")]
    AmbiguousTaskConfiguration {
        /// Exact sweep key that was insufficient to identify one task.
        key: String,
    },

    /// A resolved task dictionary does not contain the requested exact key.
    #[error("task ordinal {task_ordinal} does not contain parameter `{key}`")]
    UnknownTaskParameter {
        /// Resolved task from which the parameter was requested.
        task_ordinal: u64,
        /// Exact, case-sensitive lookup key supplied by the caller.
        key: String,
    },

    /// A present JSON value could not be decoded into the caller's requested
    /// Rust type.
    #[error("failed to decode parameter `{key}` from task ordinal {task_ordinal}")]
    DecodeTaskParameter {
        /// Resolved task containing the source value.
        task_ordinal: u64,
        /// Exact parameter key whose value was decoded.
        key: String,
        /// Underlying Serde JSON type or data-model failure.
        #[source]
        source: serde_json::Error,
    },

    /// A resolved task dictionary could not be serialized as JSON.
    #[error("failed to serialize resolved parameters for task ordinal {task_ordinal}")]
    SerializeTaskParameters {
        /// Resolved task whose logical fixed/sweep union was being serialized.
        task_ordinal: u64,
        /// Underlying JSON serialization failure.
        #[source]
        source: serde_json::Error,
    },

    /// A project path lookup addressed an undeclared exact key.
    #[error("project paths do not contain key `{key}`")]
    UnknownProjectPath {
        /// Exact, case-sensitive path name supplied by the caller.
        key: String,
    },

    /// Exact source configuration could not be written to its destination.
    #[error("failed to write project configuration file `{path}`")]
    WriteConfigurationFile {
        /// Destination file being created, written, synchronized, or renamed.
        path: PathBuf,
        /// Underlying filesystem failure.
        #[source]
        source: io::Error,
    },
}
