//! Shared immutable parameter-space storage and loading.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::super::error::ConfigurationError;
use super::super::parameter_path::ParameterPath;
use super::super::parameter_tree::{ParameterLeaf, flatten_root, paths_conflict, reconstruct};
use super::super::source::{parse_strict_json, read_source, require_object, validate_name};
use super::super::sweep::{SweepPlan, parse_sweep};
use super::task::{TaskParameters, TaskParametersIter};

const FIXED_FILE: &str = "fixed.json";
const SWEEP_FILE: &str = "sweep.json";

#[derive(Clone)]
pub struct ParameterSpace {
    pub(super) inner: Arc<ParameterSpaceInner>,
}

impl ParameterSpace {
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
        let task_count = sweep.task_count();
        Ok(Self {
            inner: Arc::new(ParameterSpaceInner {
                configuration_directory,
                fixed_source: fixed_source.into_boxed_slice(),
                sweep_source: sweep_source.into_boxed_slice(),
                fixed,
                fixed_document,
                fixed_by_path,
                sweep,
                task_count,
            }),
        })
    }

    pub fn configuration_directory(&self) -> &Path {
        &self.inner.configuration_directory
    }

    pub fn fixed_source_json(&self) -> &[u8] {
        &self.inner.fixed_source
    }

    pub fn sweep_source_json(&self) -> &[u8] {
        &self.inner.sweep_source
    }

    /// Number of terminal values contributed by `fixed.json`.
    pub fn fixed_parameter_count(&self) -> usize {
        self.inner.fixed.len()
    }

    /// Number of independently selectable axes or explicit-case fields.
    pub fn sweep_parameter_count(&self) -> usize {
        self.inner.sweep.selectable_paths().len()
    }

    /// Number of terminal values in the first resolved task.
    pub fn parameter_count(&self) -> usize {
        self.task(0).map_or(0, |task| task.len())
    }

    pub fn task_count(&self) -> u64 {
        self.inner.task_count
    }

    pub fn contains_parameter(&self, key: &str) -> bool {
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

    pub fn task(&self, ordinal: u64) -> Result<TaskParameters, ConfigurationError> {
        if ordinal >= self.task_count() {
            return Err(ConfigurationError::TaskOrdinalOutOfBounds {
                ordinal,
                task_count: self.task_count(),
            });
        }
        Ok(TaskParameters::new(Arc::clone(&self.inner), ordinal))
    }

    pub fn tasks(&self) -> TaskParametersIter {
        TaskParametersIter::new(Arc::clone(&self.inner), self.task_count())
    }
}

impl fmt::Debug for ParameterSpace {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ParameterSpace")
            .field("configuration_directory", &self.configuration_directory())
            .field("fixed_parameters", &self.fixed_parameter_count())
            .field("sweep_parameters", &self.sweep_parameter_count())
            .field("task_count", &self.task_count())
            .finish_non_exhaustive()
    }
}

pub(crate) struct ParameterSpaceInner {
    pub(super) configuration_directory: PathBuf,
    pub(super) fixed_source: Box<[u8]>,
    pub(super) sweep_source: Box<[u8]>,
    pub(super) fixed: Vec<ParameterLeaf>,
    pub(super) fixed_document: serde_json::Value,
    pub(super) fixed_by_path: HashMap<ParameterPath, usize>,
    pub(super) sweep: SweepPlan,
    pub(super) task_count: u64,
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
