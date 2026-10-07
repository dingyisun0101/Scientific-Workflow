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

    for marker in ["linspace", "logspace"] {
        if object.contains_key(marker) {
            if object.len() != 1 {
                return Err(ConfigError::invalid(
                    path,
                    pointer,
                    "a spacing marker must contain no sibling fields",
                ));
            }
            return expand_spacing(path, pointer, marker, &object[marker]);
        }
    }

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

/// Numeric axes use the same deterministic product ordering as explicit sweeps.
fn expand_spacing(
    path: &Path,
    pointer: &str,
    marker: &str,
    value: &Value,
) -> Result<Vec<Value>, ConfigError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Spacing {
        start: f64,
        stop: f64,
        num: usize,
        #[serde(default = "include_endpoint")]
        endpoint: bool,
        #[serde(default)]
        base: Option<f64>,
    }
    fn include_endpoint() -> bool {
        true
    }
    let spec: Spacing = serde_json::from_value(value.clone())
        .map_err(|error| ConfigError::invalid(path, pointer, error.to_string()))?;
    let base = spec.base.unwrap_or(10.0);
    if !spec.start.is_finite()
        || !spec.stop.is_finite()
        || spec.num == 0
        || (marker == "linspace" && spec.base.is_some())
        || (marker == "logspace" && (!base.is_finite() || base <= 0.0))
    {
        return Err(ConfigError::invalid(
            path,
            pointer,
            "spacing requires finite bounds, positive num, and a finite positive base only for logspace",
        ));
    }
    let mut values = Vec::new();
    values
        .try_reserve(spec.num)
        .map_err(|_| ConfigError::ExpansionOverflow {
            path: path.to_path_buf(),
        })?;
    let denominator = if spec.endpoint {
        spec.num.saturating_sub(1).max(1)
    } else {
        spec.num
    } as f64;
    for index in 0..spec.num {
        let fraction = index as f64 / denominator;
        let exponent = if index == 0 {
            spec.start
        } else if spec.endpoint && index + 1 == spec.num {
            spec.stop
        } else {
            spec.start * (1.0 - fraction) + spec.stop * fraction
        };
        let number = if marker == "logspace" {
            base.powf(exponent)
        } else {
            exponent
        };
        let number = serde_json::Number::from_f64(number).ok_or_else(|| {
            ConfigError::invalid(path, pointer, "spacing produced a nonfinite value")
        })?;
        values.push(Value::Number(number));
    }
    Ok(values)
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
                if key.starts_with('$') || matches!(key.as_str(), "linspace" | "logspace") {
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

fn flattened_paths(object: &Map<String, Value>) -> Vec<Vec<String>> {
    fn visit(prefix: &mut Vec<String>, value: &Value, paths: &mut Vec<Vec<String>>) {
        if let Value::Object(object) = value
            && !object.is_empty()
        {
            for (key, value) in object {
                prefix.push(key.clone());
                visit(prefix, value, paths);
                prefix.pop();
            }
        } else {
            paths.push(prefix.clone());
        }
    }

    let mut paths = Vec::new();
    for (key, value) in object {
        visit(&mut vec![key.clone()], value, &mut paths);
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

#[cfg(test)]
mod spacing_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn numeric_axes_join_existing_cartesian_sweeps() {
        let values = expand(
            Path::new("parameters.json"),
            &json!({
                "mu": {"linspace": {"start": 0, "stop": 1, "num": 3}},
                "noise": {"logspace": {"start": -2, "stop": 0, "num": 3}},
                "literal": [1, 2]
            }),
        )
        .unwrap();
        assert_eq!(values.len(), 9);
        assert_eq!(
            values[0],
            json!({"mu": 0.0, "noise": 0.01, "literal": [1, 2]})
        );
        assert_eq!(
            values[4],
            json!({"mu": 0.5, "noise": 0.1, "literal": [1, 2]})
        );
        assert_eq!(
            values[8],
            json!({"mu": 1.0, "noise": 1.0, "literal": [1, 2]})
        );
    }

    #[test]
    fn spacing_handles_excluded_endpoints_descending_and_singleton_axes() {
        let path = Path::new("parameters.json");
        assert_eq!(
            expand(
                path,
                &json!({"linspace": {"start": 1, "stop": 0, "num": 4, "endpoint": false}})
            )
            .unwrap(),
            json!([1.0, 0.75, 0.5, 0.25]).as_array().unwrap().clone()
        );
        assert_eq!(
            expand(
                path,
                &json!({"logspace": {"start": 3, "stop": 0, "num": 4, "base": 2}})
            )
            .unwrap(),
            json!([8.0, 4.0, 2.0, 1.0]).as_array().unwrap().clone()
        );
        assert_eq!(
            expand(
                path,
                &json!({"linspace": {"start": 2, "stop": 9, "num": 1}})
            )
            .unwrap(),
            vec![json!(2.0)]
        );
        // Avoid overflow in the interpolation of opposite extreme bounds.
        let values = expand(
            path,
            &json!({"linspace": {"start": -1e308, "stop": 1e308, "num": 3}}),
        )
        .unwrap();
        assert_eq!(values[1], json!(0.0));
    }

    #[test]
    fn invalid_spacing_and_spacing_hidden_in_cases_fail_before_expansion() {
        for value in [
            json!({"linspace": {"start": 0, "stop": 1, "num": 0}}),
            json!({"linspace": {"start": 0, "stop": 1, "num": 2.5}}),
            json!({"linspace": {"start": 0, "stop": 1, "num": 2, "base": 10}}),
            json!({"logspace": {"start": 0, "stop": 1, "num": 2, "base": 0}}),
            json!({"logspace": {"start": 0, "stop": 400, "num": 2}}),
            json!({"linspace": {"start": 0, "stop": 1, "num": 2, "typo": true}}),
            json!({"linspace": {"start": 0, "stop": 1, "num": 2}, "fixed": 2}),
            json!({"$cases": [{"mu": {"linspace": {"start": 0, "stop": 1, "num": 2}}}]}),
        ] {
            assert!(
                expand(Path::new("parameters.json"), &value).is_err(),
                "{value}"
            );
        }
    }
}
