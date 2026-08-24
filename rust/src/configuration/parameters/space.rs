//! Study-wide loading and workload-scoped configuration spaces.

use std::collections::HashMap;
use std::fmt;
use std::iter::FusedIterator;
use std::path::{Path, PathBuf};
use std::slice;
use std::sync::Arc;

use super::super::error::ConfigurationError;
use super::super::parameter_path::ParameterPath;
use super::super::parameter_tree::{ParameterLeaf, paths_conflict};
use super::super::source::{
    StrictValue, parse_strict_json, read_source, require_object, validate_name,
};
use super::super::sweep::{ParsedScope, SweepPlan, parse_scope};
use super::resolved_configuration::{ConfigurationIter, ResolvedConfiguration};

const PARAMETERS_FILE: &str = "parameters.json";
const CONFIGURATION_DIRECTORY: &str = "config";

/// One validated study-wide parameter registry.
///
/// Call [`StudyConfiguration::workload`] to obtain the only iterable space. A
/// workload automatically includes the global and containing component scopes.
#[derive(Clone)]
pub struct StudyConfiguration {
    inner: Arc<StudyConfigurationInner>,
}

/// The lazily expanded parameter space for one component-qualified workload.
#[derive(Clone)]
pub struct WorkloadConfiguration {
    pub(super) inner: Arc<WorkloadConfigurationInner>,
}

impl StudyConfiguration {
    /// Loads `config/parameters.json` beneath `study_root`.
    pub fn load(study_root: impl Into<PathBuf>) -> Result<Self, ConfigurationError> {
        let study_root = study_root.into();
        let configuration_directory = Arc::new(study_root.join(CONFIGURATION_DIRECTORY));
        let source_path = configuration_directory.join(PARAMETERS_FILE);
        let source = read_source(&source_path)?;
        let document = parse_strict_json(&source_path, &source)?;
        let mut root = require_object(
            &source_path,
            document,
            "parameters.json root must be an object",
        )?;
        let global = take_required(&source_path, &mut root, "global")?;
        let components = take_required(&source_path, &mut root, "components")?;
        reject_remaining(&source_path, &root, "parameters.json")?;

        let global = Arc::new(ScopeConfiguration::new(parse_scope(
            &source_path,
            require_object(&source_path, global, "`global` must be an object")?,
            "global scope",
        )?)?);
        let components =
            require_object(&source_path, components, "`components` must be an object")?;
        if components.is_empty() {
            return super::super::source::invalid(
                &source_path,
                "`components` must contain at least one component",
            );
        }

        let mut workloads = HashMap::with_capacity(components.len());
        for (component_key, component) in components {
            validate_name(&source_path, &component_key, "component key")?;
            let mut component = require_object(
                &source_path,
                component,
                format!("component `{component_key}` must be an object"),
            )?;
            let shared = take_required(&source_path, &mut component, "shared")?;
            let workload = take_required(&source_path, &mut component, "workloads")?;
            reject_remaining(
                &source_path,
                &component,
                &format!("component `{component_key}`"),
            )?;
            let shared = Arc::new(ScopeConfiguration::new(parse_scope(
                &source_path,
                require_object(
                    &source_path,
                    shared,
                    format!("component `{component_key}` field `shared` must be an object"),
                )?,
                &format!("component `{component_key}` shared scope"),
            )?)?);
            let workload = require_object(
                &source_path,
                workload,
                format!("component `{component_key}` field `workloads` must be an object"),
            )?;
            if workload.is_empty() {
                return super::super::source::invalid(
                    &source_path,
                    format!("component `{component_key}` must contain at least one workload"),
                );
            }
            let mut component_workloads = HashMap::with_capacity(workload.len());
            for (workload_key, values) in workload {
                validate_name(&source_path, &workload_key, "workload key")?;
                let local = Arc::new(ScopeConfiguration::new(parse_scope(
                    &source_path,
                    require_object(
                        &source_path,
                        values,
                        format!("workload `{component_key}/{workload_key}` must be an object"),
                    )?,
                    &format!("workload `{component_key}/{workload_key}`"),
                )?)?);
                let space = compose_space(
                    Arc::clone(&configuration_directory),
                    &source_path,
                    &component_key,
                    &workload_key,
                    [Arc::clone(&global), Arc::clone(&shared), local],
                )?;
                component_workloads.insert(workload_key.into_boxed_str(), Arc::new(space));
            }
            workloads.insert(component_key.into_boxed_str(), component_workloads);
        }
        Ok(Self {
            inner: Arc::new(StudyConfigurationInner {
                study_root,
                configuration_directory,
                source_path,
                source: source.into_boxed_slice(),
                workloads,
            }),
        })
    }

    /// Returns the study root supplied to [`Self::load`].
    pub fn study_root(&self) -> &Path {
        &self.inner.study_root
    }

    /// Returns the conventional `config` directory beneath the study root.
    pub fn configuration_directory(&self) -> &Path {
        self.inner.configuration_directory.as_path()
    }

    /// Returns the exact `parameters.json` source path.
    pub fn source_path(&self) -> &Path {
        &self.inner.source_path
    }

    /// Borrows the original validated source bytes without reserialization.
    pub fn source_json(&self) -> &[u8] {
        &self.inner.source
    }

    /// Returns one exact component-qualified workload configuration.
    pub fn workload(
        &self,
        component: &str,
        workload: &str,
    ) -> Result<WorkloadConfiguration, ConfigurationError> {
        let inner = self
            .inner
            .workloads
            .get(component)
            .and_then(|workloads| workloads.get(workload))
            .map(Arc::clone)
            .ok_or_else(|| ConfigurationError::UnknownWorkloadConfiguration {
                component: component.to_owned(),
                workload: workload.to_owned(),
            })?;
        Ok(WorkloadConfiguration { inner })
    }
}

impl WorkloadConfiguration {
    /// Returns the directory containing the study-wide parameter source.
    pub fn configuration_directory(&self) -> &Path {
        self.inner.configuration_directory.as_path()
    }

    /// Returns this workload's stable containing component key.
    pub fn component(&self) -> &str {
        &self.inner.component
    }

    /// Returns this workload's stable string key.
    pub fn workload(&self) -> &str {
        &self.inner.workload
    }

    /// Returns the complete `global × shared × workload` combination count.
    pub fn combination_count(&self) -> u64 {
        self.inner.combination_count
    }

    /// Returns the number of ordinary terminal leaves in the merged view.
    pub fn fixed_value_count(&self) -> usize {
        self.inner.fixed_leaves().count()
    }

    /// Returns the number of terminal paths selected by sweep dimensions.
    pub fn swept_value_count(&self) -> usize {
        self.inner
            .scopes
            .iter()
            .map(|scope| scope.sweep.selectable_paths().len())
            .sum()
    }

    /// Reports whether a fixed or selectable value exists at or below `key`.
    pub fn contains(&self, key: &str) -> bool {
        let Some(path) = ParameterPath::parse(key) else {
            return false;
        };
        self.inner
            .fixed_leaves()
            .any(|leaf| leaf.path == path || path.is_ancestor_of(&leaf.path))
            || self
                .inner
                .selectable_paths()
                .any(|candidate| candidate == &path || path.is_ancestor_of(candidate))
    }

    /// Iterates ordinary terminal JSON Pointer keys in declaration order.
    pub fn fixed_keys(&self) -> impl ExactSizeIterator<Item = &str> {
        self.inner.fixed_leaves().map(|leaf| leaf.path.identifier())
    }

    /// Iterates selectable terminal JSON Pointer keys in declaration order.
    pub fn sweep_keys(&self) -> impl ExactSizeIterator<Item = &str> {
        self.inner.selectable_paths().map(ParameterPath::identifier)
    }

    /// Resolves one flattened workload ordinal with bounds checking.
    pub fn combination(&self, ordinal: u64) -> Result<ResolvedConfiguration, ConfigurationError> {
        if ordinal >= self.combination_count() {
            return Err(ConfigurationError::CombinationOrdinalOutOfBounds {
                ordinal,
                combination_count: self.combination_count(),
            });
        }
        Ok(ResolvedConfiguration::new(Arc::clone(&self.inner), ordinal))
    }

    /// Lazily iterates every resolved combination in deterministic order.
    pub fn combinations(&self) -> ConfigurationIter {
        ConfigurationIter::new(Arc::clone(&self.inner), self.combination_count())
    }
}

impl fmt::Debug for StudyConfiguration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StudyConfiguration")
            .field("study_root", &self.study_root())
            .field(
                "workloads",
                &self
                    .inner
                    .workloads
                    .values()
                    .map(HashMap::len)
                    .sum::<usize>(),
            )
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for WorkloadConfiguration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorkloadConfiguration")
            .field("component", &self.component())
            .field("workload", &self.workload())
            .field("combinations", &self.combination_count())
            .finish_non_exhaustive()
    }
}

struct StudyConfigurationInner {
    study_root: PathBuf,
    configuration_directory: Arc<PathBuf>,
    source_path: PathBuf,
    source: Box<[u8]>,
    workloads: HashMap<Box<str>, HashMap<Box<str>, Arc<WorkloadConfigurationInner>>>,
}

pub(crate) struct WorkloadConfigurationInner {
    pub(super) configuration_directory: Arc<PathBuf>,
    pub(super) component: Box<str>,
    pub(super) workload: Box<str>,
    scopes: [Arc<ScopeConfiguration>; 3],
    pub(super) combination_count: u64,
}

fn compose_space(
    configuration_directory: Arc<PathBuf>,
    source_path: &Path,
    component: &str,
    workload: &str,
    scopes: [Arc<ScopeConfiguration>; 3],
) -> Result<WorkloadConfigurationInner, ConfigurationError> {
    validate_scopes_disjoint(source_path, &scopes)?;
    let combination_count = scopes.iter().try_fold(1_u64, |count, scope| {
        count.checked_mul(scope.combination_count()).ok_or_else(|| {
            ConfigurationError::CombinationCountOverflow {
                axis: format!("workload `{component}/{workload}`"),
            }
        })
    })?;
    Ok(WorkloadConfigurationInner {
        configuration_directory,
        component: component.to_owned().into_boxed_str(),
        workload: workload.to_owned().into_boxed_str(),
        scopes,
        combination_count,
    })
}

struct ScopeConfiguration {
    fixed: Vec<ParameterLeaf>,
    fixed_by_path: HashMap<ParameterPath, usize>,
    sweep: SweepPlan,
}

impl ScopeConfiguration {
    fn new(scope: ParsedScope) -> Result<Self, ConfigurationError> {
        let fixed_by_path = scope
            .fixed
            .iter()
            .enumerate()
            .map(|(index, leaf)| (leaf.path.clone(), index))
            .collect();
        Ok(Self {
            fixed: scope.fixed,
            fixed_by_path,
            sweep: SweepPlan::new(scope.dimensions)?,
        })
    }

    const fn combination_count(&self) -> u64 {
        self.sweep.combination_count()
    }
}

impl WorkloadConfigurationInner {
    pub(super) fn fixed_leaves(&self) -> ScopeSliceIter<'_, ParameterLeaf> {
        ScopeSliceIter::new([
            &self.scopes[0].fixed,
            &self.scopes[1].fixed,
            &self.scopes[2].fixed,
        ])
    }

    pub(super) fn selected_leaves(&self, ordinal: u64) -> impl Iterator<Item = &ParameterLeaf> {
        self.scopes[0]
            .sweep
            .selected_leaves(self.scope_ordinal(ordinal, 0))
            .chain(
                self.scopes[1]
                    .sweep
                    .selected_leaves(self.scope_ordinal(ordinal, 1)),
            )
            .chain(
                self.scopes[2]
                    .sweep
                    .selected_leaves(self.scope_ordinal(ordinal, 2)),
            )
    }

    pub(super) fn selectable_paths(&self) -> ScopeSliceIter<'_, ParameterPath> {
        ScopeSliceIter::new([
            self.scopes[0].sweep.selectable_paths(),
            self.scopes[1].sweep.selectable_paths(),
            self.scopes[2].sweep.selectable_paths(),
        ])
    }

    pub(super) fn fixed_leaf(&self, path: &ParameterPath) -> Option<&ParameterLeaf> {
        self.scopes.iter().find_map(|scope| {
            scope
                .fixed_by_path
                .get(path)
                .map(|&position| &scope.fixed[position])
        })
    }

    pub(super) fn scope_ordinal(&self, ordinal: u64, scope: usize) -> u64 {
        let following = self.scopes[(scope + 1)..]
            .iter()
            .map(|scope| scope.combination_count())
            .product::<u64>();
        (ordinal / following) % self.scopes[scope].combination_count()
    }
}

fn validate_scopes_disjoint(
    source_path: &Path,
    scopes: &[Arc<ScopeConfiguration>; 3],
) -> Result<(), ConfigurationError> {
    let mut paths: Vec<&ParameterPath> = Vec::new();
    for path in scopes.iter().flat_map(|scope| {
        scope
            .fixed
            .iter()
            .map(|leaf| &leaf.path)
            .chain(scope.sweep.selectable_paths())
    }) {
        if let Some(previous) = paths.iter().find(|previous| paths_conflict(previous, path)) {
            return super::super::source::invalid(
                source_path,
                format!("parameter paths `{previous}` and `{path}` overlap"),
            );
        }
        paths.push(path);
    }
    Ok(())
}

pub(super) struct ScopeSliceIter<'a, T> {
    scopes: [slice::Iter<'a, T>; 3],
    current: usize,
    remaining: usize,
}

impl<'a, T> ScopeSliceIter<'a, T> {
    fn new(scopes: [&'a [T]; 3]) -> Self {
        Self {
            remaining: scopes.iter().map(|scope| scope.len()).sum(),
            scopes: scopes.map(<[T]>::iter),
            current: 0,
        }
    }
}

impl<'a, T> Iterator for ScopeSliceIter<'a, T> {
    type Item = &'a T;

    fn next(&mut self) -> Option<Self::Item> {
        while self.current < self.scopes.len() {
            if let Some(value) = self.scopes[self.current].next() {
                self.remaining -= 1;
                return Some(value);
            }
            self.current += 1;
        }
        None
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl<T> ExactSizeIterator for ScopeSliceIter<'_, T> {}
impl<T> FusedIterator for ScopeSliceIter<'_, T> {}

fn take_required(
    path: &Path,
    entries: &mut Vec<(String, StrictValue)>,
    name: &str,
) -> Result<StrictValue, ConfigurationError> {
    let Some(position) = entries.iter().position(|(key, _)| key == name) else {
        return super::super::source::invalid(path, format!("required field `{name}` is missing"));
    };
    Ok(entries.remove(position).1)
}

fn reject_remaining(
    path: &Path,
    entries: &[(String, StrictValue)],
    context: &str,
) -> Result<(), ConfigurationError> {
    if let Some((name, _)) = entries.first() {
        return super::super::source::invalid(
            path,
            format!("{context} contains unknown field `{name}`"),
        );
    }
    Ok(())
}
