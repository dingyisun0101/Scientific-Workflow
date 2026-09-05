//! Accessors for Workflow's required project layout and resolved program snapshots.
use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// A missing standard path, environment variable, or invalid resolved snapshot.
#[derive(Debug, Error)]
#[error("{message}; preserve Workflow's required study layout")]
pub struct ProjectLayoutError {
    message: String,
    #[source]
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
}
fn failure(message: String) -> ProjectLayoutError {
    ProjectLayoutError {
        message,
        source: None,
    }
}
fn environment_path(variable: &str, directory: bool) -> Result<PathBuf, ProjectLayoutError> {
    let value = std::env::var_os(variable).ok_or_else(|| failure(format!("missing {variable}")))?;
    let path = PathBuf::from(value);
    if !path.is_absolute()
        || !(if directory {
            path.is_dir()
        } else {
            path.is_file()
        })
    {
        return Err(failure(format!(
            "expected {} at {} from {variable}",
            if directory { "directory" } else { "file" },
            path.display()
        )));
    }
    Ok(path)
}
/// Returns `WORKFLOW_PROJECT_ROOT` for a launched program; does not change cwd.
pub fn project_root() -> Result<PathBuf, ProjectLayoutError> {
    environment_path("WORKFLOW_PROJECT_ROOT", true)
}
/// Returns `WORKFLOW_TASK_OUTPUT`, the existing writable artifacts directory.
pub fn output_directory() -> Result<PathBuf, ProjectLayoutError> {
    environment_path("WORKFLOW_TASK_OUTPUT", true)
}
/// Requires `<root>/wf_configs/study.json` and returns its path without parsing it.
pub fn study_path(root: &Path) -> Result<PathBuf, ProjectLayoutError> {
    let path = root.join("wf_configs/study.json");
    if !path.is_file() {
        return Err(failure(format!("expected {}", path.display())));
    }
    Ok(path)
}
/// Loads resolved parameters from `WORKFLOW_CONFIG_PATH` and deserializes `T`.
///
/// `section` selects one exact top-level key; `None` selects all parameters.
/// This does not read unresolved project declarations or repeat Config resolution.
pub fn parameters<T: DeserializeOwned>(section: Option<&str>) -> Result<T, ProjectLayoutError> {
    parameters_from_snapshot(&environment_path("WORKFLOW_CONFIG_PATH", false)?, section)
}
/// Loads resolved parameters from an explicit standard `workflow-config.json` file.
///
/// The file must contain `config["parameters.json"]`. Filesystem/JSON/typed
/// deserialization errors retain their cause. A failed read returns no value.
pub fn parameters_from_snapshot<T: DeserializeOwned>(
    path: &Path,
    section: Option<&str>,
) -> Result<T, ProjectLayoutError> {
    let read = || -> Result<T, Box<dyn std::error::Error + Send + Sync>> {
        let bytes = std::fs::read(path)?;
        let snapshot: serde_json::Value = serde_json::from_slice(&bytes)?;
        let parameters = snapshot
            .get("config")
            .and_then(|v| v.get("parameters.json"))
            .filter(|v| v.is_object())
            .ok_or("expected config[parameters.json] object")?;
        let selected = match section {
            Some(key) => parameters
                .get(key)
                .ok_or_else(|| format!("missing parameter section {key:?}"))?,
            None => parameters,
        };
        Ok(serde_json::from_value(selected.clone())?)
    };
    read().map_err(|source| ProjectLayoutError {
        message: format!(
            "cannot read resolved parameters at {}: {source}",
            path.display()
        ),
        source: Some(source),
    })
}
