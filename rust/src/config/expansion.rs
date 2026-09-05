//! Deterministic expansion of execution-unit parameter selections.

use std::path::Path;

use serde_json::{Map, Value};

use super::document::child_pointer;
use super::error::ConfigError;

pub(crate) fn expand(path: &Path, value: &Value) -> Result<Vec<Value>, ConfigError> {
    expand_at(path, "/", value, false)
}

fn expand_at(
    path: &Path,
    pointer: &str,
    value: &Value,
    in_sweep_choice: bool,
) -> Result<Vec<Value>, ConfigError> {
    let Value::Object(object) = value else {
        // Preserve literal arrays while rejecting hidden markers inside a sweep choice.
        if in_sweep_choice {
            reject_reserved_markers(path, pointer, value)?;
        }
        return Ok(vec![value.clone()]);
    };

    if let Some(choices) = object.get("$sweep") {
        if object.len() != 1 {
            return Err(ConfigError::invalid(
                path,
                pointer,
                "a `$sweep` marker must contain no sibling fields",
            ));
        }
        let Value::Array(choices) = choices else {
            return Err(ConfigError::invalid(
                path,
                pointer,
                "`$sweep` must be an array",
            ));
        };
        if choices.is_empty() {
            return Err(ConfigError::invalid(
                path,
                pointer,
                "`$sweep` must contain at least one choice",
            ));
        }
        let mut expanded = Vec::new();
        for (index, choice) in choices.iter().enumerate() {
            let choice_pointer = format!("{}/$sweep/{index}", normalized_pointer(pointer));
            let nested = expand_at(path, &choice_pointer, choice, true)?;
            expanded.len().checked_add(nested.len()).ok_or_else(|| {
                ConfigError::ExpansionOverflow {
                    path: path.to_path_buf(),
                }
            })?;
            expanded
                .try_reserve(nested.len())
                .map_err(|_| ConfigError::ExpansionOverflow {
                    path: path.to_path_buf(),
                })?;
            expanded.extend(nested);
        }
        return Ok(expanded);
    }

    if let Some(cases) = object.get("$cases") {
        return expand_cases(path, pointer, object, cases);
    }

    let mut combinations = vec![Map::new()];
    for (key, value) in object {
        if key.starts_with('$') {
            return Err(ConfigError::invalid(
                path,
                child_pointer(pointer, key),
                format!("unknown reserved parameter marker `{key}`"),
            ));
        }
        let expanded = expand_at(path, &child_pointer(pointer, key), value, in_sweep_choice)?;
        combinations = product_insert(path, combinations, key, &expanded)?;
    }
    Ok(combinations.into_iter().map(Value::Object).collect())
}

fn expand_cases(
    path: &Path,
    pointer: &str,
    object: &Map<String, Value>,
    cases: &Value,
) -> Result<Vec<Value>, ConfigError> {
    let Value::Array(cases) = cases else {
        return Err(ConfigError::invalid(
            path,
            child_pointer(pointer, "$cases"),
            "`$cases` must be an array",
        ));
    };
    if cases.is_empty() {
        return Err(ConfigError::invalid(
            path,
            child_pointer(pointer, "$cases"),
            "`$cases` must contain at least one case",
        ));
    }

    let mut fixed = Map::new();
    for (key, value) in object {
        if key == "$cases" {
            continue;
        }
        if key.starts_with('$') {
            return Err(ConfigError::invalid(
                path,
                child_pointer(pointer, key),
                format!("unknown reserved parameter marker `{key}`"),
            ));
        }
        reject_reserved_markers(path, &child_pointer(pointer, key), value)?;
        fixed.insert(key.clone(), value.clone());
    }

    let mut expected_paths = None;
    let mut expanded = Vec::with_capacity(cases.len());
    for (index, case) in cases.iter().enumerate() {
        let Value::Object(case) = case else {
            return Err(ConfigError::invalid(
                path,
                format!("{}/$cases/{index}", normalized_pointer(pointer)),
                "each case must be an object",
            ));
        };
        if case.is_empty() {
            return Err(ConfigError::invalid(
                path,
                format!("{}/$cases/{index}", normalized_pointer(pointer)),
                "each case must contain at least one field",
            ));
        }
        reject_reserved_markers(path, pointer, &Value::Object(case.clone()))?;
        let paths = flattened_paths(case);
        if let Some(expected) = &expected_paths {
            if expected != &paths {
                return Err(ConfigError::invalid(
                    path,
                    child_pointer(pointer, "$cases"),
                    "all cases must contain the same flattened field set",
                ));
            }
        } else {
            expected_paths = Some(paths);
        }

        let mut merged = fixed.clone();
        merge_objects(path, pointer, &mut merged, case)?;
        expanded.push(Value::Object(merged));
    }
    Ok(expanded)
}

fn product_insert(
    path: &Path,
    current: Vec<Map<String, Value>>,
    key: &str,
    choices: &[Value],
) -> Result<Vec<Map<String, Value>>, ConfigError> {
    current
        .len()
        .checked_mul(choices.len())
        .ok_or_else(|| ConfigError::ExpansionOverflow {
            path: path.to_path_buf(),
        })?;
    let mut next = Vec::new();
    next.try_reserve(current.len().saturating_mul(choices.len()))
        .map_err(|_| ConfigError::ExpansionOverflow {
            path: path.to_path_buf(),
        })?;
    for base in current {
        for choice in choices {
            let mut candidate = base.clone();
            candidate.insert(key.to_owned(), choice.clone());
            next.push(candidate);
        }
    }
    Ok(next)
}

fn merge_objects(
    path: &Path,
    pointer: &str,
    destination: &mut Map<String, Value>,
    source: &Map<String, Value>,
) -> Result<(), ConfigError> {
    for (key, value) in source {
        match (destination.get_mut(key), value) {
            (None, value) => {
                destination.insert(key.clone(), value.clone());
            }
            (Some(Value::Object(destination)), Value::Object(source)) => {
                merge_objects(path, &child_pointer(pointer, key), destination, source)?;
            }
            (Some(_), _) => {
                return Err(ConfigError::invalid(
                    path,
                    child_pointer(pointer, key),
                    "fixed parameters and `$cases` define overlapping fields",
                ));
            }
        }
    }
    Ok(())
}

fn reject_reserved_markers(path: &Path, pointer: &str, value: &Value) -> Result<(), ConfigError> {
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if key.starts_with('$') {
                    return Err(ConfigError::invalid(
                        path,
                        child_pointer(pointer, key),
                        "selection markers are not allowed in literal choice arrays or correlated cases",
                    ));
                }
                reject_reserved_markers(path, &child_pointer(pointer, key), value)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                reject_reserved_markers(path, pointer, value)?;
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
    Ok(())
}

fn flattened_paths(object: &Map<String, Value>) -> Vec<String> {
    fn visit(prefix: &str, value: &Value, paths: &mut Vec<String>) {
        if let Value::Object(object) = value
            && !object.is_empty()
        {
            for (key, value) in object {
                let path = format!("{prefix}/{key}");
                visit(&path, value, paths);
            }
        } else {
            paths.push(prefix.to_owned());
        }
    }

    let mut paths = Vec::new();
    for (key, value) in object {
        visit(&format!("/{key}"), value, &mut paths);
    }
    paths.sort_unstable();
    paths
}

fn normalized_pointer(pointer: &str) -> &str {
    pointer.strip_suffix('/').unwrap_or(pointer)
}

#[cfg(test)]
mod nested_sweep_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn expands_independent_axes_inside_one_alternative_with_one_base() {
        let input = json!({
            "fixed_model": {"steps": 1000},
            "noise": {"$sweep": [null, {
                "type": "checkerboard",
                "cell_size": {"$sweep": [[2,2], [4,4], [8,8], [16,16], [32,32], [64,64]]},
                "minimum": {"$sweep": [0.1, 0.2, 0.4, 0.8]},
                "maximum": 1.0
            }]}
        });
        let results = expand(Path::new("parameters.json"), &input).unwrap();
        assert_eq!(results.len(), 25);
        assert_eq!(results.iter().filter(|v| v["noise"].is_null()).count(), 1);
        for width in [2, 4, 8, 16, 32, 64] {
            for strength in [0.1, 0.2, 0.4, 0.8] {
                assert_eq!(
                    results
                        .iter()
                        .filter(|v| {
                            v["noise"]["cell_size"] == json!([width, width])
                                && v["noise"]["minimum"] == json!(strength)
                        })
                        .count(),
                    1
                );
            }
        }
        assert!(
            results
                .iter()
                .all(|v| v["fixed_model"] == input["fixed_model"])
        );
    }

    #[test]
    fn nested_alternatives_combine_with_siblings_and_keep_arrays_literal() {
        let literal = json!([{"$sweep": [1, 2]}]);
        let input = json!({
            "opaque": literal,
            "choice": {"$sweep": [null, {"width": {"$sweep": [[2, 2], [4, 4]]}}]},
            "replicate": {"$sweep": [10, 20]}
        });
        let results = expand(Path::new("parameters.json"), &input).unwrap();
        let expected = [
            Value::Null,
            json!({"width": [2, 2]}),
            json!({"width": [4, 4]}),
        ]
        .into_iter()
        .flat_map(|choice| {
            let literal = &literal;
            [10, 20].into_iter().map(move |replicate| {
                    json!({"opaque": literal, "choice": choice, "replicate": replicate})
                })
        })
        .collect::<Vec<_>>();
        assert_eq!(results, expected);
    }

    #[test]
    fn rejects_invalid_nested_sweeps_and_markers_hidden_in_literal_arrays() {
        for input in [
            json!({"$sweep": [null, {"size": {"$sweep": []}}]}),
            json!({"$sweep": [null, {"size": {"$sweep": [2], "other": 4}}]}),
            json!({"$sweep": [[{"$sweep": [2,4]}]]}),
            json!({"$sweep": [{"$cases": [{"size": {"$sweep": [2, 4]}}]}]}),
        ] {
            assert!(expand(Path::new("parameters.json"), &input).is_err());
        }
    }
}
