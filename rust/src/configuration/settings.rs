//! Strict study-level replicate policy loaded from `study.json`.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;

use super::error::ConfigurationError;
use super::source::{invalid, parse_strict_json, read_source};

const STUDY_SETTINGS_FILE: &str = "study.json";

/// Validated, immutable study-level replicate settings.
///
/// This type owns only the library-defined `study.json` grammar. Scientific
/// parameters remain in `config/parameters.json`, and named paths remain in
/// `config/paths.json`.
#[derive(Clone)]
pub struct StudySettings {
    inner: Arc<StudySettingsInner>,
}

impl StudySettings {
    /// Loads and validates `study.json` directly beneath `study_root`.
    pub fn load(study_root: impl Into<PathBuf>) -> Result<Self, ConfigurationError> {
        let study_root = study_root.into();
        let source_path = study_root.join(STUDY_SETTINGS_FILE);
        let source = read_source(&source_path)?;
        let document = parse_strict_json(&source_path, &source)?.into_json();
        let raw: RawStudySettings = serde_json::from_value(document).map_err(|source| {
            ConfigurationError::InvalidConfigurationDocument {
                path: source_path.clone(),
                reason: source.to_string(),
            }
        })?;
        if raw.replicate_settings.replicates == 0 {
            return invalid(
                &source_path,
                "replicate_settings.replicates must be positive",
            );
        }

        Ok(Self {
            inner: Arc::new(StudySettingsInner {
                study_root,
                source_path,
                source: source.into_boxed_slice(),
                replicate_settings: ReplicateSettings {
                    replicates: raw.replicate_settings.replicates,
                    execution: raw.replicate_settings.execution,
                    failure_policy: raw.replicate_settings.failure_policy,
                    seed: raw.replicate_settings.seed,
                },
            }),
        })
    }

    /// Returns the study root supplied to [`Self::load`].
    pub fn study_root(&self) -> &Path {
        &self.inner.study_root
    }

    /// Returns the exact `study.json` source path.
    pub fn source_path(&self) -> &Path {
        &self.inner.source_path
    }

    /// Borrows the original validated source bytes without reserialization.
    pub fn source_json(&self) -> &[u8] {
        &self.inner.source
    }

    /// Returns the complete replicate policy.
    pub fn replicate_settings(&self) -> ReplicateSettings {
        self.inner.replicate_settings
    }
}

impl fmt::Debug for StudySettings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StudySettings")
            .field("study_root", &self.study_root())
            .field("source_path", &self.source_path())
            .field("replicate_settings", &self.replicate_settings())
            .finish_non_exhaustive()
    }
}

struct StudySettingsInner {
    study_root: PathBuf,
    source_path: PathBuf,
    source: Box<[u8]>,
    replicate_settings: ReplicateSettings,
}

/// Validated policy for executing one or more isolated study replicates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReplicateSettings {
    replicates: u64,
    execution: ReplicateExecutionMode,
    failure_policy: ReplicateFailurePolicy,
    seed: u64,
}

impl ReplicateSettings {
    /// Returns the positive number of replicate subprocesses.
    pub const fn replicates(self) -> u64 {
        self.replicates
    }

    /// Returns whether replicate subprocesses run sequentially or in parallel.
    pub const fn execution(self) -> ReplicateExecutionMode {
        self.execution
    }

    /// Returns the controller response to a failed replicate subprocess.
    pub const fn failure_policy(self) -> ReplicateFailurePolicy {
        self.failure_policy
    }

    /// Returns the study-level seed used for lazy per-replicate derivation.
    pub const fn seed(self) -> u64 {
        self.seed
    }
}

/// Process-level scheduling mode for study replicates.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ReplicateExecutionMode {
    /// Start and await one replicate subprocess at a time.
    Sequential,
    /// Start one subprocess for every replicate before awaiting completion.
    Parallel,
}

impl ReplicateExecutionMode {
    /// Returns the exact `study.json` spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sequential => "sequential",
            Self::Parallel => "parallel",
        }
    }
}

/// Controller response when a replicate subprocess fails.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ReplicateFailurePolicy {
    /// Stop launching sequential work or terminate active parallel children.
    FailFast,
    /// Allow every declared replicate subprocess to finish.
    FinishAll,
}

impl ReplicateFailurePolicy {
    /// Returns the exact `study.json` spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FailFast => "fail_fast",
            Self::FinishAll => "finish_all",
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStudySettings {
    replicate_settings: RawReplicateSettings,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawReplicateSettings {
    replicates: u64,
    execution: ReplicateExecutionMode,
    failure_policy: ReplicateFailurePolicy,
    seed: u64,
}
