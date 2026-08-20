//! Parsing of arbitrarily nested Cartesian axis trees and correlated cases.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::super::error::ConfigurationError;
use super::super::parameter_path::ParameterPath;
use super::super::parameter_tree::{
    ParameterLeaf, flatten_candidate, flatten_root, paths_conflict,
};
use super::super::source::{StrictValue, invalid, require_object, validate_name};
use super::plan::{SweepDimension, SweepPlan};

pub(crate) fn parse_sweep(
    path: &Path,
    document: StrictValue,
) -> Result<SweepPlan, ConfigurationError> {
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

fn parse_cartesian(path: &Path, axes: StrictValue) -> Result<SweepPlan, ConfigurationError> {
    let StrictValue::Object(entries) = axes else {
        return invalid(path, "cartesian sweep field `axes` must be an object");
    };
    let mut dimensions = parse_nested_axes(path, entries)?;
    validate_dimension_paths(path, &dimensions)?;

    let mut task_count = 1_u64;
    for dimension in dimensions.iter_mut().rev() {
        dimension.stride = task_count;
        let length = u64::try_from(dimension.candidates.len()).map_err(|_| {
            ConfigurationError::TaskCountOverflow {
                axis: dimension.path.identifier().to_owned(),
            }
        })?;
        task_count = task_count.checked_mul(length).ok_or_else(|| {
            ConfigurationError::TaskCountOverflow {
                axis: dimension.path.identifier().to_owned(),
            }
        })?;
    }
    let selectable_paths = dimensions
        .iter()
        .map(|dimension| dimension.path.clone())
        .collect();
    Ok(SweepPlan::cartesian(
        dimensions,
        task_count,
        selectable_paths,
    ))
}

fn parse_nested_axes(
    path: &Path,
    entries: Vec<(String, StrictValue)>,
) -> Result<Vec<SweepDimension>, ConfigurationError> {
    let mut dimensions = Vec::new();
    for (key, value) in entries {
        validate_name(path, &key, "sweep path segment")?;
        parse_nested_axis_at(
            path,
            ParameterPath::root(key.into_boxed_str()),
            value,
            &mut dimensions,
        )?;
    }
    Ok(dimensions)
}

fn parse_nested_axis_at(
    path: &Path,
    axis_path: ParameterPath,
    value: StrictValue,
    dimensions: &mut Vec<SweepDimension>,
) -> Result<(), ConfigurationError> {
    let StrictValue::Object(mut entries) = value else {
        return invalid(
            path,
            format!("nested sweep path `{axis_path}` must contain an object"),
        );
    };
    if entries.iter().any(|(key, _)| key == "values") {
        let values = take_required(path, &mut entries, "values")?;
        reject_remaining(path, &entries, &format!("nested sweep axis `{axis_path}`"))?;
        dimensions.push(parse_dimension_values(path, axis_path, values)?);
        return Ok(());
    }
    if entries.is_empty() {
        return invalid(path, format!("nested sweep path `{axis_path}` is empty"));
    }
    for (key, value) in entries {
        validate_name(path, &key, "sweep path segment")?;
        parse_nested_axis_at(
            path,
            axis_path.appended(key.into_boxed_str()),
            value,
            dimensions,
        )?;
    }
    Ok(())
}

fn parse_dimension_values(
    path: &Path,
    axis_path: ParameterPath,
    values: StrictValue,
) -> Result<SweepDimension, ConfigurationError> {
    let StrictValue::Array(values) = values else {
        return invalid(
            path,
            format!("cartesian axis `{axis_path}` field `values` must be an array"),
        );
    };
    if values.is_empty() {
        return invalid(
            path,
            format!("cartesian axis `{axis_path}` has no candidates"),
        );
    }
    let mut candidates = values
        .into_iter()
        .map(|candidate| flatten_candidate(&axis_path, candidate))
        .collect::<Vec<_>>();
    normalize_candidate_paths(path, &axis_path, &mut candidates)?;
    Ok(SweepDimension {
        candidates,
        path: axis_path,
        stride: 0,
    })
}

fn normalize_candidate_paths(
    path: &Path,
    axis_path: &ParameterPath,
    candidates: &mut [Vec<ParameterLeaf>],
) -> Result<(), ConfigurationError> {
    let order = candidates[0]
        .iter()
        .map(|leaf| leaf.path.clone())
        .collect::<Vec<_>>();
    let expected = order.iter().collect::<HashSet<_>>();
    for candidate in candidates.iter_mut().skip(1) {
        if candidate
            .iter()
            .map(|leaf| &leaf.path)
            .collect::<HashSet<_>>()
            != expected
        {
            return invalid(
                path,
                format!(
                    "cartesian axis `{axis_path}` candidates do not contain the same flattened key set"
                ),
            );
        }
        let mut by_path = candidate
            .drain(..)
            .map(|leaf| (leaf.path.clone(), leaf))
            .collect::<HashMap<_, _>>();
        *candidate = order
            .iter()
            .map(|parameter_path| {
                by_path
                    .remove(parameter_path)
                    .expect("validated candidate contains every ordered path")
            })
            .collect();
    }
    Ok(())
}

fn validate_dimension_paths(
    path: &Path,
    dimensions: &[SweepDimension],
) -> Result<(), ConfigurationError> {
    for (index, dimension) in dimensions.iter().enumerate() {
        for previous in &dimensions[..index] {
            if paths_conflict(&dimension.path, &previous.path)
                || candidate_paths(&dimension.candidates).any(|candidate| {
                    candidate_paths(&previous.candidates)
                        .any(|other| paths_conflict(candidate, other))
                })
            {
                return invalid(
                    path,
                    format!(
                        "sweep axes `{}` and `{}` produce overlapping parameter paths",
                        previous.path, dimension.path
                    ),
                );
            }
        }
    }
    Ok(())
}

fn candidate_paths(candidates: &[Vec<ParameterLeaf>]) -> impl Iterator<Item = &ParameterPath> {
    candidates
        .iter()
        .flat_map(|candidate| candidate.iter().map(|leaf| &leaf.path))
}

fn parse_cases(path: &Path, cases: StrictValue) -> Result<SweepPlan, ConfigurationError> {
    let StrictValue::Array(documents) = cases else {
        return invalid(path, "explicit sweep field `cases` must be an array");
    };
    if documents.is_empty() {
        return invalid(path, "explicit sweep must contain at least one case");
    }
    let mut parsed = Vec::with_capacity(documents.len());
    for (position, document) in documents.into_iter().enumerate() {
        let entries = require_object(
            path,
            document,
            format!("explicit case at position {position} must be an object"),
        )?;
        if entries.is_empty() {
            return invalid(
                path,
                "explicit sweep cases must contain at least one parameter",
            );
        }
        for (name, _) in &entries {
            validate_name(path, name, "explicit-case parameter")?;
        }
        parsed.push(flatten_root(entries));
    }
    let order = parsed[0]
        .iter()
        .map(|leaf| leaf.path.clone())
        .collect::<Vec<_>>();
    let expected = order.iter().collect::<HashSet<_>>();
    for case in parsed.iter_mut().skip(1) {
        if case.iter().map(|leaf| &leaf.path).collect::<HashSet<_>>() != expected {
            return invalid(
                path,
                "explicit cases do not contain the same flattened key set as case 0",
            );
        }
        let mut by_path = case
            .drain(..)
            .map(|leaf| (leaf.path.clone(), leaf))
            .collect::<HashMap<_, _>>();
        *case = order
            .iter()
            .map(|parameter_path| {
                by_path
                    .remove(parameter_path)
                    .expect("validated case contains every ordered path")
            })
            .collect();
    }
    let selectable_paths = parsed[0].iter().map(|leaf| leaf.path.clone()).collect();
    Ok(SweepPlan::cases(parsed, selectable_paths))
}

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
