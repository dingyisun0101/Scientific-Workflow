//! Immutable fixed and swept parameter definitions with deterministic task
//! expansion.
//!
//! [`ParameterSpace`] reads `fixed.json` and `sweep.json` from one standard
//! configuration directory. It validates both documents once and stores every
//! JSON value behind shared immutable ownership. [`TaskParameters`] identifies
//! one resolved task by ordinal and performs dictionary lookups directly into
//! that shared storage; creating or cloning a task does not clone parameter
//! values or allocate a merged JSON object.
//!
//! # Sweep formats
//!
//! Cartesian mode preserves declared axis order and changes the final axis
//! fastest:
//!
//! ```json
//! {
//!   "mode": "cartesian",
//!   "axes": [
//!     {"name": "temperature", "values": [280.0, 300.0]},
//!     {"name": "seed", "values": [1, 2]}
//!   ]
//! }
//! ```
//!
//! Explicit correlated cases use:
//!
//! ```json
//! {
//!   "mode": "cases",
//!   "cases": [
//!     {"temperature": 280.0, "physical_time_increment": 0.1},
//!     {"temperature": 300.0, "physical_time_increment": 0.05}
//!   ]
//! }
//! ```
//!
//! Fixed and swept keys must be disjoint. All exact JSON object keys are
//! duplicate-checked recursively, including objects nested inside parameter
//! values. An empty Cartesian axis list produces one fixed-only task; an empty
//! candidate list or empty explicit-case list is rejected.
//!
//! # Lookup and ownership
//!
//! Raw lookup returns `&serde_json::Value` without copying. Typed decoding is an
//! explicit conversion requested by the application and may allocate an owned
//! `String`, `Vec`, or domain value. Applications should decode their required
//! constants once before entering a numerical hot loop.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::iter::FusedIterator;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::de::{DeserializeOwned, MapAccess, SeqAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Number, Value};

use super::error::ConfigurationError;

const FIXED_FILE: &str = "fixed.json";
const SWEEP_FILE: &str = "sweep.json";

/// A validated immutable fixed-and-swept parameter definition.
///
/// Cloning this type clones only an [`Arc`]. Parsed source values, lookup
/// indexes, Cartesian strides, explicit cases, and exact source bytes remain in
/// one shared allocation.
#[derive(Clone)]
pub struct ParameterSpace {
    inner: Arc<ParameterSpaceInner>,
}

impl ParameterSpace {
    /// Loads `fixed.json` and `sweep.json` from `configuration_directory`.
    ///
    /// The directory is the `config/` directory itself. The later
    /// `ProjectConfig` facade accepts a project root and applies that standard
    /// suffix automatically.
    ///
    /// # Errors
    ///
    /// Returns contextual IO or JSON errors, duplicate-key and document-shape
    /// errors, fixed/sweep collisions, empty sweep definitions, or checked task
    /// count overflow. Neither source file is modified.
    pub fn load(configuration_directory: impl Into<PathBuf>) -> Result<Self, ConfigurationError> {
        let configuration_directory = configuration_directory.into();
        let fixed_path = configuration_directory.join(FIXED_FILE);
        let sweep_path = configuration_directory.join(SWEEP_FILE);
        let fixed_source = read_source(&fixed_path)?;
        let sweep_source = read_source(&sweep_path)?;
        let fixed_document = parse_strict_json(&fixed_path, &fixed_source)?;
        let sweep_document = parse_strict_json(&sweep_path, &sweep_source)?;
        let fixed = parse_fixed(&fixed_path, fixed_document)?;
        let sweep = parse_sweep(&sweep_path, sweep_document)?;
        validate_disjoint(&fixed_path, &sweep_path, &fixed, &sweep)?;

        let fixed_by_name = fixed
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.name.clone(), index))
            .collect();
        let sweep_by_name = sweep
            .keys()
            .enumerate()
            .map(|(index, key)| (Box::<str>::from(key), index))
            .collect();
        let task_count = sweep.task_count();

        Ok(Self {
            inner: Arc::new(ParameterSpaceInner {
                configuration_directory,
                fixed_source: fixed_source.into_boxed_slice(),
                sweep_source: sweep_source.into_boxed_slice(),
                fixed,
                fixed_by_name,
                sweep,
                sweep_by_name,
                task_count,
            }),
        })
    }

    /// Returns the configuration directory exactly as supplied at load time.
    pub fn configuration_directory(&self) -> &Path {
        &self.inner.configuration_directory
    }

    /// Borrows the validated original bytes of `fixed.json` unchanged.
    ///
    /// These bytes include the source document's original whitespace, key
    /// presentation order, and number spelling.
    pub fn fixed_source_json(&self) -> &[u8] {
        &self.inner.fixed_source
    }

    /// Borrows the validated original bytes of `sweep.json` unchanged.
    pub fn sweep_source_json(&self) -> &[u8] {
        &self.inner.sweep_source
    }

    /// Returns the number of names supplied by `fixed.json`.
    pub fn fixed_parameter_count(&self) -> usize {
        self.inner.fixed.len()
    }

    /// Returns the number of names supplied by one resolved sweep selection.
    pub fn sweep_parameter_count(&self) -> usize {
        self.inner.sweep.key_count()
    }

    /// Returns the number of entries in every resolved task dictionary.
    pub fn parameter_count(&self) -> usize {
        self.fixed_parameter_count() + self.sweep_parameter_count()
    }

    /// Returns the exact checked number of deterministic task combinations.
    pub fn task_count(&self) -> u64 {
        self.inner.task_count
    }

    /// Reports whether either the fixed table or sweep definition declares an
    /// exact parameter key.
    pub fn contains_parameter(&self, key: &str) -> bool {
        self.inner.fixed_by_name.contains_key(key) || self.inner.sweep_by_name.contains_key(key)
    }

    /// Iterates fixed parameter names in their JSON declaration order.
    pub fn fixed_keys(&self) -> impl ExactSizeIterator<Item = &str> {
        self.inner.fixed.iter().map(|entry| entry.name.as_ref())
    }

    /// Iterates swept parameter names in declared axis or first-case order.
    pub fn sweep_keys(&self) -> impl ExactSizeIterator<Item = &str> {
        self.inner.sweep.keys()
    }

    /// Resolves one zero-based deterministic task ordinal.
    ///
    /// The returned dictionary shares this space's allocation and computes
    /// selected values lazily during lookup. No parameter value is cloned.
    pub fn task(&self, ordinal: u64) -> Result<TaskParameters, ConfigurationError> {
        if ordinal >= self.task_count() {
            return Err(ConfigurationError::TaskOrdinalOutOfBounds {
                ordinal,
                task_count: self.task_count(),
            });
        }
        Ok(TaskParameters {
            inner: Arc::clone(&self.inner),
            ordinal,
        })
    }

    /// Iterates every resolved task in increasing deterministic ordinal order.
    ///
    /// Iterator items cannot fail because the range is constructed from the
    /// already validated task count.
    pub fn tasks(&self) -> TaskParametersIter {
        TaskParametersIter {
            inner: Arc::clone(&self.inner),
            next: 0,
            end: self.task_count(),
        }
    }
}

impl fmt::Debug for ParameterSpace {
    /// Formats bounded structural facts without traversing parameter values.
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

/// One immutable dict-like fixed-plus-sweep parameter selection.
///
/// This type owns no JSON values. Cloning it increments one shared reference
/// count and copies one `u64` task ordinal.
#[derive(Clone)]
pub struct TaskParameters {
    inner: Arc<ParameterSpaceInner>,
    ordinal: u64,
}

impl TaskParameters {
    /// Returns this task's zero-based deterministic ordinal.
    pub fn task_ordinal(&self) -> u64 {
        self.ordinal
    }

    /// Borrows one fixed or selected sweep value by exact or dotted-nested JSON key.
    ///
    /// Missing keys return `None`. No value is cloned, decoded, or allocated.
    pub fn value(&self, key: &str) -> Option<&Value> {
        if let Some((root, path)) = split_nested_key(key) {
            if let Some(&position) = self.inner.fixed_by_name.get(root) {
                return lookup_json_path(&self.inner.fixed[position].value, path);
            }
            if let Some(&position) = self.inner.sweep_by_name.get(root) {
                return lookup_json_path(self.inner.sweep.value(self.ordinal, position), path);
            }
        }
        if let Some(&position) = self.inner.fixed_by_name.get(key) {
            return Some(&self.inner.fixed[position].value);
        }
        let &position = self.inner.sweep_by_name.get(key)?;
        Some(self.inner.sweep.value(self.ordinal, position))
    }

    /// Borrows one required value or reports its task and exact missing key.
    pub fn require_value(&self, key: &str) -> Result<&Value, ConfigurationError> {
        self.value(key)
            .ok_or_else(|| ConfigurationError::UnknownTaskParameter {
                task_ordinal: self.ordinal,
                key: key.to_owned(),
            })
    }

    /// Decodes one required JSON value into the caller's concrete Rust type.
    ///
    /// Deserialization reads directly from the borrowed `serde_json::Value`;
    /// this method does not first clone the generic JSON tree. The returned
    /// concrete value is owned and may allocate according to `T`.
    pub fn decode_value<T>(&self, key: &str) -> Result<T, ConfigurationError>
    where
        T: DeserializeOwned,
    {
        let value = self.require_value(key)?;
        T::deserialize(value).map_err(|source| ConfigurationError::DecodeTaskParameter {
            task_ordinal: self.ordinal,
            key: key.to_owned(),
            source,
        })
    }

    /// Reports whether this resolved dictionary contains an exact or nested key.
    pub fn contains(&self, key: &str) -> bool {
        self.inner.fixed_by_name.contains_key(key)
            || self.inner.sweep_by_name.contains_key(key)
            || split_nested_key(key).is_some_and(|(root, path)| {
                self.inner.fixed_by_name.get(root).is_some_and(|&position| {
                    lookup_json_path(&self.inner.fixed[position].value, path).is_some()
                }) || self.inner.sweep_by_name.get(root).is_some_and(|&position| {
                    lookup_json_path(self.inner.sweep.value(self.ordinal, position), path).is_some()
                })
            })
    }

    /// Returns the fixed-plus-swept entry count.
    pub fn len(&self) -> usize {
        self.inner.fixed.len() + self.inner.sweep.key_count()
    }

    /// Reports whether the resolved task contains no parameter entries.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Iterates exact keys with fixed declarations first and swept declarations
    /// second, preserving source declaration order within each group.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.inner
            .fixed
            .iter()
            .map(|entry| entry.name.as_ref())
            .chain(self.inner.sweep.keys())
    }

    /// Iterates resolved key/value references in the same order as
    /// [`TaskParameters::keys`].
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.keys().map(|key| {
            (
                key,
                self.value(key)
                    .expect("a key yielded by a validated parameter space must resolve"),
            )
        })
    }

    /// Serializes the resolved fixed-plus-sweep dictionary as compact JSON.
    ///
    /// This is derived task data, not either original source document. Keys are
    /// emitted in [`TaskParameters::keys`] order and values are serialized by
    /// reference without constructing a merged `serde_json::Map`.
    pub fn to_json(&self) -> Result<String, ConfigurationError> {
        serde_json::to_string(&ResolvedTaskRef { task: self }).map_err(|source| {
            ConfigurationError::SerializeTaskParameters {
                task_ordinal: self.ordinal,
                source,
            }
        })
    }
}

/// Splits `a.b.c` into (`a`, `b.c`) for nested parameter lookup.
fn split_nested_key(key: &str) -> Option<(&str, &str)> {
    let (root, path) = key.split_once('.')?;
    if root.is_empty() || path.is_empty() {
        return None;
    }
    Some((root, path))
}

/// Looks up one nested JSON path inside an object value.
fn lookup_json_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in path.split('.') {
        match current {
            Value::Object(values) => current = values.get(segment)?,
            _ => return None,
        }
    }
    Some(current)
}

impl fmt::Debug for TaskParameters {
    /// Formats identity and key counts without exposing parameter values.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TaskParameters")
            .field("task_ordinal", &self.ordinal)
            .field("parameters", &self.len())
            .finish_non_exhaustive()
    }
}

/// Owning iterator over cheap resolved task dictionaries.
///
/// The iterator retains the shared parameter space independently of the
/// `ParameterSpace` handle that created it.
#[derive(Clone)]
pub struct TaskParametersIter {
    inner: Arc<ParameterSpaceInner>,
    next: u64,
    end: u64,
}

impl Iterator for TaskParametersIter {
    type Item = TaskParameters;

    /// Produces the next increasing task ordinal without parameter-value
    /// allocation.
    fn next(&mut self) -> Option<Self::Item> {
        if self.next == self.end {
            return None;
        }
        let ordinal = self.next;
        self.next += 1;
        Some(TaskParameters {
            inner: Arc::clone(&self.inner),
            ordinal,
        })
    }

    /// Reports an exact upper bound whenever the remaining `u64` count fits in
    /// the platform's `usize`.
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
    /// Formats only the remaining ordinal range.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TaskParametersIter")
            .field("next", &self.next)
            .field("end", &self.end)
            .finish_non_exhaustive()
    }
}

/// Shared immutable allocation behind spaces, task views, and task iterators.
struct ParameterSpaceInner {
    configuration_directory: PathBuf,
    fixed_source: Box<[u8]>,
    sweep_source: Box<[u8]>,
    fixed: Vec<NamedValue>,
    fixed_by_name: HashMap<Box<str>, usize>,
    sweep: SweepPlan,
    sweep_by_name: HashMap<Box<str>, usize>,
    task_count: u64,
}

/// One fixed parameter retained in source declaration order.
struct NamedValue {
    name: Box<str>,
    value: Value,
}

/// Validated storage for either supported sweep expansion mode.
enum SweepPlan {
    Cartesian {
        axes: Vec<SweepAxis>,
        task_count: u64,
    },
    Cases {
        keys: Vec<Box<str>>,
        cases: Vec<Vec<Value>>,
    },
}

impl SweepPlan {
    /// Iterates resolved sweep keys in their authoritative declaration order.
    fn keys(&self) -> impl ExactSizeIterator<Item = &str> {
        let keys: &[Box<str>] = match self {
            Self::Cartesian { axes, .. } => {
                return SweepKeys::Axes(axes.iter()).map(SweepKey::into_str);
            }
            Self::Cases { keys, .. } => keys,
        };
        SweepKeys::Cases(keys.iter()).map(SweepKey::into_str)
    }

    /// Returns the number of sweep entries in each resolved task.
    fn key_count(&self) -> usize {
        match self {
            Self::Cartesian { axes, .. } => axes.len(),
            Self::Cases { keys, .. } => keys.len(),
        }
    }

    /// Returns the validated total number of generated tasks.
    fn task_count(&self) -> u64 {
        match self {
            Self::Cartesian { task_count, .. } => *task_count,
            Self::Cases { cases, .. } => {
                u64::try_from(cases.len()).expect("validated explicit case count must fit in u64")
            }
        }
    }

    /// Borrows one selected value for an already validated task and key
    /// position.
    fn value(&self, task_ordinal: u64, key_position: usize) -> &Value {
        match self {
            Self::Cartesian { axes, .. } => {
                let axis = &axes[key_position];
                let axis_length = u64::try_from(axis.values.len())
                    .expect("validated Cartesian axis length must fit in u64");
                let selected = (task_ordinal / axis.stride) % axis_length;
                &axis.values[usize::try_from(selected)
                    .expect("selected Cartesian index originated from a usize length")]
            }
            Self::Cases { cases, .. } => {
                &cases[usize::try_from(task_ordinal)
                    .expect("validated explicit task ordinal originated from a usize count")]
                    [key_position]
            }
        }
    }
}

/// One Cartesian axis with a precomputed mixed-radix stride.
struct SweepAxis {
    name: Box<str>,
    values: Vec<Value>,
    stride: u64,
}

/// Common key reference used to return one concrete iterator from both sweep
/// plan variants.
enum SweepKey<'a> {
    Axis(&'a SweepAxis),
    Case(&'a str),
}

impl<'a> SweepKey<'a> {
    fn into_str(self) -> &'a str {
        match self {
            Self::Axis(axis) => &axis.name,
            Self::Case(key) => key,
        }
    }
}

/// Variant-erased exact-size iterator over sweep keys.
enum SweepKeys<'a> {
    Axes(std::slice::Iter<'a, SweepAxis>),
    Cases(std::slice::Iter<'a, Box<str>>),
}

impl<'a> Iterator for SweepKeys<'a> {
    type Item = SweepKey<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Axes(iter) => iter.next().map(SweepKey::Axis),
            Self::Cases(iter) => iter.next().map(|key| SweepKey::Case(key)),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            Self::Axes(iter) => iter.size_hint(),
            Self::Cases(iter) => iter.size_hint(),
        }
    }
}

impl ExactSizeIterator for SweepKeys<'_> {}
impl FusedIterator for SweepKeys<'_> {}

/// Borrowed serializer that emits a resolved task without cloning its values.
struct ResolvedTaskRef<'a> {
    task: &'a TaskParameters,
}

impl Serialize for ResolvedTaskRef<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(self.task.len()))?;
        for (key, value) in self.task.iter() {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

/// Duplicate-preserving JSON syntax tree used only during strict validation.
///
/// `serde_json::Value` stores objects as maps and would discard repeated keys.
/// Retaining ordered pairs until validation allows the loader to reject every
/// duplicate before converting accepted payloads to the ordinary public JSON
/// value type.
pub(super) enum StrictValue {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<Self>),
    Object(Vec<(String, Self)>),
}

impl StrictValue {
    /// Converts a duplicate-validated syntax tree to `serde_json::Value`.
    fn into_json(self) -> Value {
        match self {
            Self::Null => Value::Null,
            Self::Bool(value) => Value::Bool(value),
            Self::Number(value) => Value::Number(value),
            Self::String(value) => Value::String(value),
            Self::Array(values) => {
                Value::Array(values.into_iter().map(StrictValue::into_json).collect())
            }
            Self::Object(entries) => Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, value.into_json()))
                    .collect(),
            ),
        }
    }

    /// Finds the first repeated exact key at any object depth.
    fn duplicate_key(&self) -> Option<&str> {
        match self {
            Self::Array(values) => values.iter().find_map(StrictValue::duplicate_key),
            Self::Object(entries) => {
                let mut seen = HashSet::with_capacity(entries.len());
                for (key, value) in entries {
                    if !seen.insert(key.as_str()) {
                        return Some(key);
                    }
                    if let Some(duplicate) = value.duplicate_key() {
                        return Some(duplicate);
                    }
                }
                None
            }
            Self::Null | Self::Bool(_) | Self::Number(_) | Self::String(_) => None,
        }
    }
}

impl<'de> Deserialize<'de> for StrictValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(StrictValueVisitor)
    }
}

/// Serde visitor that preserves object entries instead of collecting a map.
struct StrictValueVisitor;

impl<'de> Visitor<'de> for StrictValueVisitor {
    type Value = StrictValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("any valid JSON value")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(StrictValue::Null)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(StrictValue::Null)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        StrictValue::deserialize(deserializer)
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(StrictValue::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(StrictValue::Number(Number::from(value)))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(StrictValue::Number(Number::from(value)))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Number::from_f64(value)
            .map(StrictValue::Number)
            .ok_or_else(|| E::custom("JSON numbers must be finite"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(StrictValue::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(StrictValue::String(value))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::with_capacity(sequence.size_hint().unwrap_or(0));
        while let Some(value) = sequence.next_element()? {
            values.push(value);
        }
        Ok(StrictValue::Array(values))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut entries = Vec::with_capacity(map.size_hint().unwrap_or(0));
        while let Some((key, value)) = map.next_entry()? {
            entries.push((key, value));
        }
        Ok(StrictValue::Object(entries))
    }
}

/// Reads one source without normalizing bytes needed for exact re-export.
pub(super) fn read_source(path: &Path) -> Result<Vec<u8>, ConfigurationError> {
    fs::read(path).map_err(|source| ConfigurationError::ReadConfigurationFile {
        path: path.to_path_buf(),
        source,
    })
}

/// Parses JSON while preserving duplicates for a distinct semantic error.
pub(super) fn parse_strict_json(
    path: &Path,
    source: &[u8],
) -> Result<StrictValue, ConfigurationError> {
    let value: StrictValue = serde_json::from_slice(source).map_err(|source| {
        ConfigurationError::ParseConfigurationFile {
            path: path.to_path_buf(),
            source,
        }
    })?;
    if let Some(key) = value.duplicate_key() {
        return Err(ConfigurationError::DuplicateConfigurationKey {
            path: path.to_path_buf(),
            key: key.to_owned(),
        });
    }
    Ok(value)
}

/// Validates and converts the fixed root object in declaration order.
fn parse_fixed(path: &Path, document: StrictValue) -> Result<Vec<NamedValue>, ConfigurationError> {
    let entries = require_object(path, document, "fixed.json root must be an object")?;
    entries
        .into_iter()
        .map(|(name, value)| {
            validate_name(path, &name, "fixed parameter")?;
            Ok(NamedValue {
                name: name.into_boxed_str(),
                value: value.into_json(),
            })
        })
        .collect()
}

/// Dispatches the tagged sweep document to its mode-specific validator.
fn parse_sweep(path: &Path, document: StrictValue) -> Result<SweepPlan, ConfigurationError> {
    let mut root = require_object(path, document, "sweep.json root must be an object")?;
    let mode = take_required(path, &mut root, "mode")?;
    let StrictValue::String(mode) = mode else {
        return invalid(path, "sweep field `mode` must be a string");
    };
    match mode.as_str() {
        "cartesian" => {
            let axes = take_required(path, &mut root, "axes")?;
            reject_remaining(path, &root, "cartesian sweep")?;
            parse_cartesian(path, axes)
        }
        "cases" => {
            let cases = take_required(path, &mut root, "cases")?;
            reject_remaining(path, &root, "explicit-case sweep")?;
            parse_cases(path, cases)
        }
        _ => invalid(
            path,
            format!("unsupported sweep mode `{mode}`; expected `cartesian` or `cases`"),
        ),
    }
}

/// Validates ordered axes and precomputes mixed-radix strides.
fn parse_cartesian(path: &Path, axes: StrictValue) -> Result<SweepPlan, ConfigurationError> {
    let StrictValue::Array(axis_documents) = axes else {
        return invalid(path, "cartesian sweep field `axes` must be an array");
    };
    let mut parsed = Vec::with_capacity(axis_documents.len());
    let mut names = HashSet::with_capacity(axis_documents.len());
    for (position, document) in axis_documents.into_iter().enumerate() {
        let mut axis = require_object(
            path,
            document,
            format!("cartesian axis at position {position} must be an object"),
        )?;
        let name = take_required(path, &mut axis, "name")?;
        let StrictValue::String(name) = name else {
            return invalid(
                path,
                format!("cartesian axis at position {position} has a non-string `name`"),
            );
        };
        validate_name(path, &name, "sweep axis")?;
        if !names.insert(name.clone()) {
            return invalid(
                path,
                format!("sweep axis name `{name}` is declared more than once"),
            );
        }
        let values = take_required(path, &mut axis, "values")?;
        let StrictValue::Array(values) = values else {
            return invalid(
                path,
                format!("cartesian axis `{name}` field `values` must be an array"),
            );
        };
        if values.is_empty() {
            return invalid(path, format!("cartesian axis `{name}` has no candidates"));
        }
        reject_remaining(path, &axis, &format!("cartesian axis `{name}`"))?;
        parsed.push(SweepAxis {
            name: name.into_boxed_str(),
            values: values.into_iter().map(StrictValue::into_json).collect(),
            stride: 0,
        });
    }

    let mut task_count = 1_u64;
    for axis in parsed.iter_mut().rev() {
        axis.stride = task_count;
        let length = u64::try_from(axis.values.len()).map_err(|_| {
            ConfigurationError::TaskCountOverflow {
                axis: axis.name.to_string(),
            }
        })?;
        task_count = task_count.checked_mul(length).ok_or_else(|| {
            ConfigurationError::TaskCountOverflow {
                axis: axis.name.to_string(),
            }
        })?;
    }
    Ok(SweepPlan::Cartesian {
        axes: parsed,
        task_count,
    })
}

/// Validates correlated cases and normalizes their values to first-case order.
fn parse_cases(path: &Path, cases: StrictValue) -> Result<SweepPlan, ConfigurationError> {
    let StrictValue::Array(case_documents) = cases else {
        return invalid(path, "explicit sweep field `cases` must be an array");
    };
    if case_documents.is_empty() {
        return invalid(path, "explicit sweep must contain at least one case");
    }

    let mut case_iter = case_documents.into_iter().enumerate();
    let (_, first) = case_iter
        .next()
        .expect("an explicitly non-empty case array has a first item");
    let first = require_object(path, first, "explicit case at position 0 must be an object")?;
    if first.is_empty() {
        return invalid(
            path,
            "explicit sweep cases must contain at least one parameter",
        );
    }
    let mut keys = Vec::with_capacity(first.len());
    let mut first_values = Vec::with_capacity(first.len());
    for (name, value) in first {
        validate_name(path, &name, "explicit-case parameter")?;
        keys.push(name.into_boxed_str());
        first_values.push(value.into_json());
    }

    let expected = keys.iter().map(AsRef::as_ref).collect::<HashSet<&str>>();
    let mut parsed_cases = Vec::with_capacity(case_iter.size_hint().0 + 1);
    parsed_cases.push(first_values);
    for (position, document) in case_iter {
        let entries = require_object(
            path,
            document,
            format!("explicit case at position {position} must be an object"),
        )?;
        let actual = entries
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<HashSet<_>>();
        if actual != expected {
            return invalid(
                path,
                format!(
                    "explicit case at position {position} does not contain the same key set as case 0"
                ),
            );
        }
        let mut by_name = entries.into_iter().collect::<HashMap<_, _>>();
        parsed_cases.push(
            keys.iter()
                .map(|key| {
                    by_name
                        .remove(key.as_ref())
                        .expect("validated explicit case contains every first-case key")
                        .into_json()
                })
                .collect(),
        );
    }
    let _ =
        u64::try_from(parsed_cases.len()).map_err(|_| ConfigurationError::TaskCountOverflow {
            axis: "explicit cases".to_owned(),
        })?;
    Ok(SweepPlan::Cases {
        keys,
        cases: parsed_cases,
    })
}

/// Rejects every fixed key that is also selected by the sweep.
fn validate_disjoint(
    fixed_path: &Path,
    sweep_path: &Path,
    fixed: &[NamedValue],
    sweep: &SweepPlan,
) -> Result<(), ConfigurationError> {
    let fixed_names = fixed
        .iter()
        .map(|entry| entry.name.as_ref())
        .collect::<HashSet<&str>>();
    if let Some(key) = sweep.keys().find(|key| fixed_names.contains(key)) {
        return Err(ConfigurationError::FixedSweepKeyConflict {
            key: key.to_owned(),
            fixed_path: fixed_path.to_path_buf(),
            sweep_path: sweep_path.to_path_buf(),
        });
    }
    Ok(())
}

/// Extracts an object or constructs one contextual semantic error.
pub(super) fn require_object(
    path: &Path,
    value: StrictValue,
    reason: impl Into<String>,
) -> Result<Vec<(String, StrictValue)>, ConfigurationError> {
    match value {
        StrictValue::Object(entries) => Ok(entries),
        _ => invalid(path, reason),
    }
}

/// Removes one required exact field from an already duplicate-checked object.
fn take_required(
    path: &Path,
    entries: &mut Vec<(String, StrictValue)>,
    name: &str,
) -> Result<StrictValue, ConfigurationError> {
    let Some(position) = entries.iter().position(|(key, _)| key == name) else {
        return invalid(path, format!("required field `{name}` is missing"));
    };
    Ok(entries.remove(position).1)
}

/// Rejects unsupported fields after all required fields have been consumed.
fn reject_remaining(
    path: &Path,
    entries: &[(String, StrictValue)],
    context: &str,
) -> Result<(), ConfigurationError> {
    if let Some((name, _)) = entries.first() {
        return invalid(path, format!("{context} contains unknown field `{name}`"));
    }
    Ok(())
}

/// Enforces non-empty human-visible exact lookup names without normalization.
pub(super) fn validate_name(path: &Path, name: &str, kind: &str) -> Result<(), ConfigurationError> {
    if name.trim().is_empty() {
        return invalid(
            path,
            format!("{kind} name must not be empty or whitespace-only"),
        );
    }
    Ok(())
}

/// Constructs a semantic document error while preserving its source path.
pub(super) fn invalid<T>(path: &Path, reason: impl Into<String>) -> Result<T, ConfigurationError> {
    Err(ConfigurationError::InvalidConfigurationDocument {
        path: path.to_path_buf(),
        reason: reason.into(),
    })
}
