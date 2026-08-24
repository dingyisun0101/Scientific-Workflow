//! Parsing of one inline parameter scope.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use super::super::error::ConfigurationError;
use super::super::parameter_path::ParameterPath;
use super::super::parameter_tree::{
    ParameterLeaf, flatten_candidate, flatten_root, paths_conflict,
};
use super::super::source::{StrictValue, invalid, require_object, validate_name};
use super::plan::SweepDimension;

/// Ordinary leaves and independent selection dimensions declared by one scope.
#[derive(Clone)]
pub(crate) struct ParsedScope {
    pub(crate) fixed: Vec<ParameterLeaf>,
    pub(crate) dimensions: Vec<SweepDimension>,
}

pub(crate) fn parse_scope(
    source_path: &Path,
    entries: Vec<(String, StrictValue)>,
    scope: &str,
) -> Result<ParsedScope, ConfigurationError> {
    let mut fixed = Vec::new();
    let mut dimensions = Vec::new();
    let mut cases = None;
    for (name, value) in entries {
        validate_name(source_path, &name, "parameter")?;
        if name == "$cases" {
            if cases.replace(value).is_some() {
                return invalid(source_path, format!("{scope} repeats `$cases`"));
            }
            continue;
        }
        if name.starts_with('$') {
            return invalid(
                source_path,
                format!("{scope} contains unknown reserved field `{name}`"),
            );
        }
        parse_at(
            source_path,
            ParameterPath::root(name.into_boxed_str()),
            value,
            &mut fixed,
            &mut dimensions,
        )?;
    }
    if let Some(cases) = cases {
        if !dimensions.is_empty() {
            return invalid(
                source_path,
                format!("{scope} cannot combine `$cases` with `$sweep`"),
            );
        }
        dimensions.push(parse_cases(source_path, cases, scope)?);
    }
    validate_disjoint(source_path, &fixed, &dimensions)?;
    Ok(ParsedScope { fixed, dimensions })
}

fn parse_at(
    source_path: &Path,
    path: ParameterPath,
    value: StrictValue,
    fixed: &mut Vec<ParameterLeaf>,
    dimensions: &mut Vec<SweepDimension>,
) -> Result<(), ConfigurationError> {
    match value {
        StrictValue::Object(mut entries) => {
            if let Some(position) = entries.iter().position(|(name, _)| name == "$sweep") {
                if entries.len() != 1 {
                    return invalid(
                        source_path,
                        format!("sweep marker `{path}` must contain only `$sweep`"),
                    );
                }
                let (_, choices) = entries.remove(position);
                dimensions.push(parse_choices(source_path, &path, choices)?);
                return Ok(());
            }
            if let Some((name, _)) = entries.iter().find(|(name, _)| name.starts_with('$')) {
                return invalid(
                    source_path,
                    format!("parameter object `{path}` contains unknown reserved field `{name}`"),
                );
            }
            if entries.is_empty() {
                fixed.push(ParameterLeaf::new(
                    path,
                    serde_json::Value::Object(Default::default()),
                ));
                return Ok(());
            }
            for (name, value) in entries {
                validate_name(source_path, &name, "parameter path segment")?;
                parse_at(
                    source_path,
                    path.appended(name.into_boxed_str()),
                    value,
                    fixed,
                    dimensions,
                )?;
            }
            Ok(())
        }
        value => {
            fixed.push(ParameterLeaf::new(path, value.into_json()));
            Ok(())
        }
    }
}

fn parse_choices(
    source_path: &Path,
    path: &ParameterPath,
    choices: StrictValue,
) -> Result<SweepDimension, ConfigurationError> {
    let StrictValue::Array(choices) = choices else {
        return invalid(
            source_path,
            format!("`$sweep` at `{path}` must be an array"),
        );
    };
    if choices.is_empty() {
        return invalid(source_path, format!("`$sweep` at `{path}` has no choices"));
    }
    let mut candidates = choices
        .into_iter()
        .map(|choice| flatten_candidate(path, choice))
        .collect::<Vec<_>>();
    normalize_candidates(source_path, &path.to_string(), &mut candidates)?;
    Ok(SweepDimension {
        label: path.to_string(),
        candidates,
        stride: 0,
    })
}

fn parse_cases(
    source_path: &Path,
    cases: StrictValue,
    scope: &str,
) -> Result<SweepDimension, ConfigurationError> {
    let StrictValue::Array(cases) = cases else {
        return invalid(source_path, format!("`$cases` in {scope} must be an array"));
    };
    if cases.is_empty() {
        return invalid(source_path, format!("`$cases` in {scope} has no cases"));
    }
    let mut candidates = Vec::with_capacity(cases.len());
    for (position, case) in cases.into_iter().enumerate() {
        let entries = require_object(
            source_path,
            case,
            format!("case {position} in {scope} must be an object"),
        )?;
        if entries.is_empty() {
            return invalid(source_path, format!("case {position} in {scope} is empty"));
        }
        for (name, _) in &entries {
            validate_name(source_path, name, "case parameter")?;
            if name.starts_with('$') {
                return invalid(
                    source_path,
                    format!("case {position} contains reserved field `{name}`"),
                );
            }
        }
        candidates.push(flatten_root(entries));
    }
    normalize_candidates(source_path, &format!("{scope}/$cases"), &mut candidates)?;
    Ok(SweepDimension {
        label: format!("{scope}/$cases"),
        candidates,
        stride: 0,
    })
}

fn normalize_candidates(
    source_path: &Path,
    label: &str,
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
                source_path,
                format!("choices for `{label}` do not contain the same flattened key set"),
            );
        }
        let mut by_path = candidate
            .drain(..)
            .map(|leaf| (leaf.path.clone(), leaf))
            .collect::<HashMap<_, _>>();
        *candidate = order
            .iter()
            .map(|path| by_path.remove(path).expect("validated candidate key"))
            .collect();
    }
    Ok(())
}

fn validate_disjoint(
    source_path: &Path,
    fixed: &[ParameterLeaf],
    dimensions: &[SweepDimension],
) -> Result<(), ConfigurationError> {
    let mut paths: Vec<&ParameterPath> = Vec::with_capacity(fixed.len());
    for leaf in fixed {
        if let Some(previous) = paths
            .iter()
            .find(|previous| paths_conflict(previous, &leaf.path))
        {
            return invalid(
                source_path,
                format!("parameter paths `{previous}` and `{}` overlap", leaf.path),
            );
        }
        paths.push(&leaf.path);
    }
    for dimension in dimensions {
        for candidate in &dimension.candidates {
            for leaf in candidate {
                if let Some(previous) = paths
                    .iter()
                    .find(|previous| paths_conflict(previous, &leaf.path))
                {
                    return invalid(
                        source_path,
                        format!("parameter paths `{previous}` and `{}` overlap", leaf.path),
                    );
                }
            }
        }
        paths.extend(dimension.candidates[0].iter().map(|leaf| &leaf.path));
    }
    Ok(())
}
