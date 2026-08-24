//! Cheap resolved combinations with lazy nested-document materialization.

use std::fmt;
use std::iter::FusedIterator;
use std::sync::{Arc, OnceLock};

use serde::de::DeserializeOwned;
use serde_json::Value;

use super::super::error::ConfigurationError;
use super::super::parameter_key_tuple::ParameterKeyTuple;
use super::super::parameter_path::ParameterPath;
use super::super::parameter_tree::{lookup_path, reconstruct};
use super::PhaseConfigurationInner;
use super::reconstruction;

/// One immutable, lazily materialized phase configuration combination.
pub struct ResolvedConfiguration {
    inner: Arc<PhaseConfigurationInner>,
    ordinal: u64,
    resolved: OnceLock<Value>,
}

impl Clone for ResolvedConfiguration {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            ordinal: self.ordinal,
            resolved: OnceLock::new(),
        }
    }
}

impl ResolvedConfiguration {
    pub(super) fn new(inner: Arc<PhaseConfigurationInner>, ordinal: u64) -> Self {
        Self {
            inner,
            ordinal,
            resolved: OnceLock::new(),
        }
    }

    /// Returns the flattened ordinal within this phase configuration.
    pub fn ordinal(&self) -> u64 {
        self.ordinal
    }

    /// Returns the stable string key of the containing phase group.
    pub fn phase_group(&self) -> &str {
        &self.inner.phase_group
    }

    /// Returns the stable string key of the selected phase.
    pub fn phase(&self) -> &str {
        &self.inner.phase
    }

    /// Returns the selection ordinal contributed by the global scope.
    pub fn global_ordinal(&self) -> u64 {
        self.inner.scope_ordinal(self.ordinal, 0)
    }

    /// Returns the selection ordinal contributed by the group shared scope.
    pub fn group_ordinal(&self) -> u64 {
        self.inner.scope_ordinal(self.ordinal, 1)
    }

    /// Returns the selection ordinal contributed by the phase-local scope.
    pub fn phase_ordinal(&self) -> u64 {
        self.inner.scope_ordinal(self.ordinal, 2)
    }

    /// Borrows an exact leaf or lazily reconstructed nested subtree.
    pub fn value(&self, key: &str) -> Option<&Value> {
        let path = ParameterPath::parse(key)?;
        if let Some(leaf) = self.inner.fixed_leaf(&path) {
            return Some(&leaf.value);
        }
        if let Some(leaf) = self
            .inner
            .selected_leaves(self.ordinal)
            .find(|leaf| leaf.path == path)
        {
            return Some(&leaf.value);
        }
        lookup_path(self.resolved_document(), &path)
    }

    /// Borrows a required value or returns an ordinal-qualified error.
    pub fn require_value(&self, key: &str) -> Result<&Value, ConfigurationError> {
        self.value(key)
            .ok_or_else(|| ConfigurationError::UnknownConfigurationValue {
                ordinal: self.ordinal,
                key: key.to_owned(),
            })
    }

    /// Deserializes one exact leaf or reconstructed subtree into `T`.
    pub fn decode_value<T>(&self, key: &str) -> Result<T, ConfigurationError>
    where
        T: DeserializeOwned,
    {
        let Some(path) = ParameterPath::parse(key) else {
            return Err(ConfigurationError::UnknownConfigurationValue {
                ordinal: self.ordinal,
                key: key.to_owned(),
            });
        };
        if let Some(value) = self.exact_leaf(&path) {
            return T::deserialize(value).map_err(|source| {
                ConfigurationError::DecodeConfigurationValue {
                    ordinal: self.ordinal,
                    key: key.to_owned(),
                    source,
                }
            });
        }
        let subtree = reconstruct(
            self.inner
                .fixed_leaves()
                .chain(self.inner.selected_leaves(self.ordinal))
                .filter(|leaf| path.is_ancestor_of(&leaf.path)),
        );
        let Some(value) = lookup_path(&subtree, &path) else {
            return Err(ConfigurationError::UnknownConfigurationValue {
                ordinal: self.ordinal,
                key: key.to_owned(),
            });
        };
        T::deserialize(value).map_err(|source| ConfigurationError::DecodeConfigurationValue {
            ordinal: self.ordinal,
            key: key.to_owned(),
            source,
        })
    }

    /// Deserializes a tuple of required paths without an intermediate struct.
    pub fn decode_values<Values, Keys>(&self, keys: Keys) -> Result<Values, ConfigurationError>
    where
        Keys: ParameterKeyTuple<Values>,
    {
        keys.decode(self)
    }

    /// Reports whether this resolved combination contains `key` or descendants.
    pub fn contains(&self, key: &str) -> bool {
        let Some(path) = ParameterPath::parse(key) else {
            return false;
        };
        self.inner
            .fixed_leaves()
            .any(|leaf| leaf.path == path || path.is_ancestor_of(&leaf.path))
            || self
                .inner
                .selected_leaves(self.ordinal)
                .any(|leaf| leaf.path == path || path.is_ancestor_of(&leaf.path))
    }

    /// Returns the number of resolved terminal values.
    pub fn len(&self) -> usize {
        self.inner.fixed_leaves().count() + self.inner.selected_leaves(self.ordinal).count()
    }

    /// Reports whether the resolved document contains no terminal values.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Iterates canonical terminal parameter identifiers.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.inner
            .fixed_leaves()
            .map(|leaf| leaf.path.identifier())
            .chain(
                self.inner
                    .selected_leaves(self.ordinal)
                    .map(|leaf| leaf.path.identifier()),
            )
    }

    /// Iterates resolved terminal keys and borrowed values in declaration order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.inner
            .fixed_leaves()
            .map(|leaf| (leaf.path.identifier(), &leaf.value))
            .chain(
                self.inner
                    .selected_leaves(self.ordinal)
                    .map(|leaf| (leaf.path.identifier(), &leaf.value)),
            )
    }

    /// Serializes the complete rehydrated nested configuration document.
    pub fn to_json(&self) -> String {
        self.resolved_document().to_string()
    }

    /// Clones the complete rehydrated nested configuration document.
    ///
    /// The document is cached after its first reconstruction. This method is
    /// intended for durable provenance records that must own their JSON value.
    pub fn to_json_value(&self) -> Value {
        self.resolved_document().clone()
    }

    fn resolved_document(&self) -> &Value {
        self.resolved
            .get_or_init(|| reconstruction::document(&self.inner, self.ordinal))
    }

    pub(crate) fn resolved_object(&self) -> &serde_json::Map<String, Value> {
        self.resolved_document()
            .as_object()
            .expect("resolved configurations always form a JSON object")
    }

    fn exact_leaf(&self, path: &ParameterPath) -> Option<&Value> {
        if let Some(leaf) = self.inner.fixed_leaf(path) {
            return Some(&leaf.value);
        }
        self.inner
            .selected_leaves(self.ordinal)
            .find(|leaf| &leaf.path == path)
            .map(|leaf| &leaf.value)
    }
}

impl fmt::Debug for ResolvedConfiguration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedConfiguration")
            .field("phase_group", &self.phase_group())
            .field("phase", &self.phase())
            .field("ordinal", &self.ordinal)
            .field("values", &self.len())
            .finish_non_exhaustive()
    }
}

/// Lazy deterministic iterator over all combinations of one phase.
#[derive(Clone)]
pub struct ConfigurationIter {
    inner: Arc<PhaseConfigurationInner>,
    next: u64,
    end: u64,
}

impl ConfigurationIter {
    pub(super) fn new(inner: Arc<PhaseConfigurationInner>, end: u64) -> Self {
        Self {
            inner,
            next: 0,
            end,
        }
    }
}

impl Iterator for ConfigurationIter {
    type Item = ResolvedConfiguration;

    fn next(&mut self) -> Option<Self::Item> {
        if self.next == self.end {
            return None;
        }
        let ordinal = self.next;
        self.next += 1;
        Some(ResolvedConfiguration::new(Arc::clone(&self.inner), ordinal))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = self.end - self.next;
        match usize::try_from(remaining) {
            Ok(remaining) => (remaining, Some(remaining)),
            Err(_) => (usize::MAX, None),
        }
    }
}

impl FusedIterator for ConfigurationIter {}

impl fmt::Debug for ConfigurationIter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConfigurationIter")
            .field("next", &self.next)
            .field("end", &self.end)
            .finish_non_exhaustive()
    }
}
