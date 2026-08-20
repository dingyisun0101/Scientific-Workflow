//! Cheap task selections with lazy nested-document materialization.

use std::fmt;
use std::iter::FusedIterator;
use std::sync::{Arc, OnceLock};

use serde::de::DeserializeOwned;
use serde_json::Value;

use super::super::error::ConfigurationError;
use super::super::parameter_key_tuple::ParameterKeyTuple;
use super::super::parameter_path::ParameterPath;
use super::super::parameter_tree::{lookup_path, reconstruct};
use super::ParameterSpaceInner;
use super::resolved;

pub struct TaskParameters {
    inner: Arc<ParameterSpaceInner>,
    ordinal: u64,
    resolved: OnceLock<Value>,
}

impl Clone for TaskParameters {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            ordinal: self.ordinal,
            resolved: OnceLock::new(),
        }
    }
}

impl TaskParameters {
    pub(super) fn new(inner: Arc<ParameterSpaceInner>, ordinal: u64) -> Self {
        Self {
            inner,
            ordinal,
            resolved: OnceLock::new(),
        }
    }

    pub fn task_ordinal(&self) -> u64 {
        self.ordinal
    }

    /// Borrows an exact leaf or lazily reconstructed nested subtree.
    pub fn value(&self, key: &str) -> Option<&Value> {
        let path = ParameterPath::parse(key)?;
        if let Some(&position) = self.inner.fixed_by_path.get(&path) {
            return Some(&self.inner.fixed[position].value);
        }
        if let Some(leaf) = self.inner.sweep.selected_leaf(self.ordinal, &path) {
            return Some(&leaf.value);
        }
        if !self
            .inner
            .sweep
            .selected_leaves(self.ordinal)
            .any(|leaf| path.is_ancestor_of(&leaf.path))
        {
            return lookup_path(&self.inner.fixed_document, &path);
        }
        lookup_path(self.resolved_document(), &path)
    }

    pub fn require_value(&self, key: &str) -> Result<&Value, ConfigurationError> {
        self.value(key)
            .ok_or_else(|| ConfigurationError::UnknownTaskParameter {
                task_ordinal: self.ordinal,
                key: key.to_owned(),
            })
    }

    pub fn decode_value<T>(&self, key: &str) -> Result<T, ConfigurationError>
    where
        T: DeserializeOwned,
    {
        let Some(path) = ParameterPath::parse(key) else {
            return Err(ConfigurationError::UnknownTaskParameter {
                task_ordinal: self.ordinal,
                key: key.to_owned(),
            });
        };
        if let Some(value) = self.exact_leaf(&path) {
            return T::deserialize(value).map_err(|source| {
                ConfigurationError::DecodeTaskParameter {
                    task_ordinal: self.ordinal,
                    key: key.to_owned(),
                    source,
                }
            });
        }
        let has_selected_sweep_descendant = self
            .inner
            .sweep
            .selected_leaves(self.ordinal)
            .any(|leaf| path.is_ancestor_of(&leaf.path));
        if !has_selected_sweep_descendant
            && let Some(value) = lookup_path(&self.inner.fixed_document, &path)
        {
            return T::deserialize(value).map_err(|source| {
                ConfigurationError::DecodeTaskParameter {
                    task_ordinal: self.ordinal,
                    key: key.to_owned(),
                    source,
                }
            });
        }
        let subtree = reconstruct(
            self.inner
                .fixed
                .iter()
                .chain(self.inner.sweep.selected_leaves(self.ordinal))
                .filter(|leaf| path.is_ancestor_of(&leaf.path)),
        );
        let Some(value) = lookup_path(&subtree, &path) else {
            return Err(ConfigurationError::UnknownTaskParameter {
                task_ordinal: self.ordinal,
                key: key.to_owned(),
            });
        };
        T::deserialize(value).map_err(|source| ConfigurationError::DecodeTaskParameter {
            task_ordinal: self.ordinal,
            key: key.to_owned(),
            source,
        })
    }

    pub fn decode_values<Values, Keys>(&self, keys: Keys) -> Result<Values, ConfigurationError>
    where
        Keys: ParameterKeyTuple<Values>,
    {
        keys.decode(self)
    }

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
                .selected_leaves(self.ordinal)
                .any(|leaf| leaf.path == path || path.is_ancestor_of(&leaf.path))
    }

    pub fn len(&self) -> usize {
        self.inner.fixed.len() + self.inner.sweep.selected_leaf_count(self.ordinal)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Iterates canonical terminal parameter identifiers.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.inner
            .fixed
            .iter()
            .map(|leaf| leaf.path.identifier())
            .chain(
                self.inner
                    .sweep
                    .selected_leaves(self.ordinal)
                    .map(|leaf| leaf.path.identifier()),
            )
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.inner
            .fixed
            .iter()
            .map(|leaf| (leaf.path.identifier(), &leaf.value))
            .chain(
                self.inner
                    .sweep
                    .selected_leaves(self.ordinal)
                    .map(|leaf| (leaf.path.identifier(), &leaf.value)),
            )
    }

    /// Serializes the complete rehydrated nested task document.
    pub fn to_json(&self) -> Result<String, ConfigurationError> {
        serde_json::to_string(self.resolved_document()).map_err(|source| {
            ConfigurationError::SerializeTaskParameters {
                task_ordinal: self.ordinal,
                source,
            }
        })
    }

    fn resolved_document(&self) -> &Value {
        self.resolved
            .get_or_init(|| resolved::document(&self.inner, self.ordinal))
    }

    pub(crate) fn resolved_object(&self) -> &serde_json::Map<String, Value> {
        self.resolved_document()
            .as_object()
            .expect("resolved task parameters always form a JSON object")
    }

    fn exact_leaf(&self, path: &ParameterPath) -> Option<&Value> {
        if let Some(&position) = self.inner.fixed_by_path.get(path) {
            return Some(&self.inner.fixed[position].value);
        }
        self.inner
            .sweep
            .selected_leaf(self.ordinal, path)
            .map(|leaf| &leaf.value)
    }
}

impl fmt::Debug for TaskParameters {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TaskParameters")
            .field("task_ordinal", &self.ordinal)
            .field("parameters", &self.len())
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct TaskParametersIter {
    inner: Arc<ParameterSpaceInner>,
    next: u64,
    end: u64,
}

impl TaskParametersIter {
    pub(super) fn new(inner: Arc<ParameterSpaceInner>, end: u64) -> Self {
        Self {
            inner,
            next: 0,
            end,
        }
    }
}

impl Iterator for TaskParametersIter {
    type Item = TaskParameters;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next == self.end {
            return None;
        }
        let ordinal = self.next;
        self.next += 1;
        Some(TaskParameters::new(Arc::clone(&self.inner), ordinal))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.end - self.next;
        match usize::try_from(remaining) {
            Ok(remaining) => (remaining, Some(remaining)),
            Err(_) => (usize::MAX, None),
        }
    }
}

impl FusedIterator for TaskParametersIter {}

impl fmt::Debug for TaskParametersIter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TaskParametersIter")
            .field("next", &self.next)
            .field("end", &self.end)
            .finish_non_exhaustive()
    }
}
