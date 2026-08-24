//! Shared immutable configuration-space storage and loading.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::super::error::ConfigurationError;
use super::super::parameter_path::ParameterPath;
use super::super::parameter_tree::{ParameterLeaf, flatten_root, paths_conflict, reconstruct};
use super::super::source::{parse_strict_json, read_source, require_object, validate_name};
use super::super::sweep::{SweepPlan, parse_sweep};
use super::resolved_configuration::{ConfigurationIter, ResolvedConfiguration};

const FIXED_FILE: &str = "fixed.json";
const SWEEP_FILE: &str = "sweep.json";

#[derive(Clone)]
pub struct ConfigurationSpace {
    pub(super) inner: Arc<ConfigurationSpaceInner>,
}

impl ConfigurationSpace {
    /// Loads and validates `fixed.json` and `sweep.json` from `directory`.
    pub fn load(configuration_directory: impl Into<PathBuf>) -> Result<Self, ConfigurationError> {
        let configuration_directory = configuration_directory.into();
        let fixed_path = configuration_directory.join(FIXED_FILE);
        let sweep_path = configuration_directory.join(SWEEP_FILE);
        let fixed_source = read_source(&fixed_path)?;
        let sweep_source = read_source(&sweep_path)?;
        let fixed_document = parse_strict_json(&fixed_path, &fixed_source)?;
        let sweep_document = parse_strict_json(&sweep_path, &sweep_source)?;
        let fixed_entries = require_object(
            &fixed_path,
            fixed_document,
            "fixed.json root must be an object",
        )?;
        for (name, _) in &fixed_entries {
            validate_name(&fixed_path, name, "fixed parameter")?;
        }
        let fixed = flatten_root(fixed_entries);
        let fixed_document = reconstruct(fixed.iter());
        let sweep = parse_sweep(&sweep_path, sweep_document)?;
        validate_disjoint(&fixed_path, &sweep_path, &fixed, &sweep)?;

        let fixed_by_path = fixed
            .iter()
            .enumerate()
            .map(|(index, leaf)| (leaf.path.clone(), index))
            .collect();
        let combination_count = sweep.combination_count();
        Ok(Self {
            inner: Arc::new(ConfigurationSpaceInner {
                configuration_directory,
                fixed_source: fixed_source.into_boxed_slice(),
                sweep_source: sweep_source.into_boxed_slice(),
                fixed,
                fixed_document,
                fixed_by_path,
                sweep,
                combination_count,
            }),
        })
    }

    /// Returns the configuration directory exactly as supplied during loading.
    pub fn directory(&self) -> &Path {
        &self.inner.configuration_directory
    }

    pub fn fixed_source_json(&self) -> &[u8] {
        &self.inner.fixed_source
    }

    pub fn sweep_source_json(&self) -> &[u8] {
        &self.inner.sweep_source
    }

    /// Returns the number of fixed terminal values.
    pub fn fixed_value_count(&self) -> usize {
        self.inner.fixed.len()
    }

    /// Returns the number of independently selected sweep paths.
    pub fn sweep_dimension_count(&self) -> usize {
        self.inner.sweep.selectable_paths().len()
    }

    pub fn combination_count(&self) -> u64 {
        self.inner.combination_count
    }

    /// Reports whether a fixed or swept value exists at or beneath `key`.
    pub fn contains(&self, key: &str) -> bool {
        let Some(path) = ParameterPath::parse(key) else {
            return false;
        };
        self.inner
            .fixed
            .iter()
            .any(|leaf| leaf.path == path || path.is_ancestor_of(&leaf.path))
            || self
                .inner
                .sweep
                .all_leaf_paths()
                .any(|candidate| candidate == &path || path.is_ancestor_of(candidate))
    }

    pub fn fixed_keys(&self) -> impl ExactSizeIterator<Item = &str> {
        self.inner.fixed.iter().map(|leaf| leaf.path.identifier())
    }

    pub fn sweep_keys(&self) -> impl ExactSizeIterator<Item = &str> {
        self.inner
            .sweep
            .selectable_paths()
            .iter()
            .map(ParameterPath::identifier)
    }

    pub fn combination(&self, ordinal: u64) -> Result<ResolvedConfiguration, ConfigurationError> {
        if ordinal >= self.combination_count() {
            return Err(ConfigurationError::CombinationOrdinalOutOfBounds {
                ordinal,
                combination_count: self.combination_count(),
            });
        }
        Ok(ResolvedConfiguration::new(Arc::clone(&self.inner), ordinal))
    }

    pub fn combinations(&self) -> ConfigurationIter {
        ConfigurationIter::new(Arc::clone(&self.inner), self.combination_count())
    }
}

impl fmt::Debug for ConfigurationSpace {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConfigurationSpace")
            .field("configuration_directory", &self.directory())
            .field("fixed_values", &self.fixed_value_count())
            .field("sweep_dimensions", &self.sweep_dimension_count())
            .field("combinations", &self.combination_count())
            .finish_non_exhaustive()
    }
}

pub(crate) struct ConfigurationSpaceInner {
    pub(super) configuration_directory: PathBuf,
    pub(super) fixed_source: Box<[u8]>,
    pub(super) sweep_source: Box<[u8]>,
    pub(super) fixed: Vec<ParameterLeaf>,
    pub(super) fixed_document: serde_json::Value,
    pub(super) fixed_by_path: HashMap<ParameterPath, usize>,
    pub(super) sweep: SweepPlan,
    pub(super) combination_count: u64,
}

fn validate_disjoint(
    fixed_path: &Path,
    sweep_path: &Path,
    fixed: &[ParameterLeaf],
    sweep: &SweepPlan,
) -> Result<(), ConfigurationError> {
    for fixed_leaf in fixed {
        if let Some(sweep_path_value) = sweep
            .all_leaf_paths()
            .find(|candidate| paths_conflict(&fixed_leaf.path, candidate))
        {
            return Err(ConfigurationError::FixedSweepKeyConflict {
                key: if fixed_leaf.path == *sweep_path_value {
                    fixed_leaf.path.identifier().to_owned()
                } else {
                    format!("{} <> {}", fixed_leaf.path, sweep_path_value)
                },
                fixed_path: fixed_path.to_path_buf(),
                sweep_path: sweep_path.to_path_buf(),
            });
        }
    }
    Ok(())
}
