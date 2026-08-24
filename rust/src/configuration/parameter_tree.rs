//! Flattening and reconstruction of nested parameter documents.

use serde_json::{Map, Value};

use super::parameter_path::ParameterPath;
use super::source::StrictValue;

/// One uniquely identified terminal JSON value.
#[derive(Clone)]
pub(super) struct ParameterLeaf {
    pub(super) path: ParameterPath,
    pub(super) value: Value,
}

impl ParameterLeaf {
    pub(super) fn new(path: ParameterPath, value: Value) -> Self {
        Self { path, value }
    }
}

/// Flattens a root object while preserving depth-first declaration order.
pub(super) fn flatten_root(entries: Vec<(String, StrictValue)>) -> Vec<ParameterLeaf> {
    let mut leaves = Vec::new();
    for (key, value) in entries {
        flatten_at(
            ParameterPath::root(key.into_boxed_str()),
            value,
            &mut leaves,
        );
    }
    leaves
}

/// Flattens one candidate below an already declared sweep-axis path.
pub(super) fn flatten_candidate(prefix: &ParameterPath, value: StrictValue) -> Vec<ParameterLeaf> {
    let mut leaves = Vec::new();
    flatten_at(prefix.clone(), value, &mut leaves);
    leaves
}

fn flatten_at(path: ParameterPath, value: StrictValue, leaves: &mut Vec<ParameterLeaf>) {
    match value {
        StrictValue::Object(entries) if !entries.is_empty() => {
            for (key, value) in entries {
                flatten_at(path.appended(key.into_boxed_str()), value, leaves);
            }
        }
        value => leaves.push(ParameterLeaf::new(path, value.into_json())),
    }
}

/// Builds one ordinary nested JSON document from ordered, disjoint leaves.
pub(super) fn reconstruct<'a>(leaves: impl IntoIterator<Item = &'a ParameterLeaf>) -> Value {
    let mut root = Map::new();
    for leaf in leaves {
        insert_leaf(&mut root, &leaf.path, leaf.value.clone());
    }
    Value::Object(root)
}

fn insert_leaf(root: &mut Map<String, Value>, path: &ParameterPath, value: Value) {
    let segments = path.segments().collect::<Vec<_>>();
    let (last, parents) = segments
        .split_last()
        .expect("validated parameter paths are nonempty");
    let mut object = root;
    for segment in parents {
        object = object
            .entry((*segment).to_owned())
            .or_insert_with(|| Value::Object(Map::new()))
            .as_object_mut()
            .expect("validated parameter leaves have no structural conflicts");
    }
    object.insert((*last).to_owned(), value);
}

pub(super) fn lookup_path<'a>(document: &'a Value, path: &ParameterPath) -> Option<&'a Value> {
    let mut current = document;
    for segment in path.segments() {
        current = current.as_object()?.get(segment)?;
    }
    Some(current)
}

pub(super) fn paths_conflict(left: &ParameterPath, right: &ParameterPath) -> bool {
    left == right || left.is_ancestor_of(right) || right.is_ancestor_of(left)
}
