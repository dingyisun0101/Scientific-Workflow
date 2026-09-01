//! Strict Python task declarations and environment-manager resolution.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use super::error::ConfigError;
use super::program::{ResolvedProgramTask, resolve_executable};
use super::store::ensure_utf8;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PythonTaskDeclaration {
    script: PathBuf,
    environment: PythonEnvironment,
    #[serde(default)]
    args: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "manager", rename_all = "snake_case", deny_unknown_fields)]
enum PythonEnvironment {
    System {
        #[serde(default)]
        executable: Option<PathBuf>,
    },
    Venv {
        path: PathBuf,
    },
    Mamba {
        name: String,
        #[serde(default)]
        executable: Option<PathBuf>,
    },
    Conda {
        name: String,
        #[serde(default)]
        executable: Option<PathBuf>,
    },
    Uv {
        project: PathBuf,
        #[serde(default)]
        executable: Option<PathBuf>,
    },
    Poetry {
        project: PathBuf,
        #[serde(default)]
        executable: Option<PathBuf>,
    },
}

pub(crate) fn resolve(
    project_root: &Path,
    declaration: PythonTaskDeclaration,
    seed_purpose: Option<Box<str>>,
    timeout: Option<Duration>,
    threads: usize,
) -> Result<ResolvedProgramTask, ConfigError> {
    let script = resolve_script(project_root, &declaration.script)?;
    let (program, manager, mut args) = match declaration.environment {
        PythonEnvironment::System { executable } => (
            resolve_manager(project_root, executable.as_deref(), "python3")?,
            "system",
            Vec::<OsString>::new(),
        ),
        PythonEnvironment::Venv { path } => {
            let environment = resolve_directory(project_root, &path, "virtual environment")?;
            #[cfg(unix)]
            let interpreter = environment.join("bin/python");
            #[cfg(not(unix))]
            let interpreter = environment.join("Scripts/python.exe");
            (
                resolve_executable(project_root, &interpreter)?,
                "venv",
                Vec::<OsString>::new(),
            )
        }
        PythonEnvironment::Mamba { name, executable } => {
            validate_name(&declaration.script, &name, "mamba environment")?;
            (
                resolve_manager(project_root, executable.as_deref(), "mamba")?,
                "mamba",
                vec!["run".into(), "-n".into(), name.into(), "python".into()],
            )
        }
        PythonEnvironment::Conda { name, executable } => {
            validate_name(&declaration.script, &name, "conda environment")?;
            (
                resolve_manager(project_root, executable.as_deref(), "conda")?,
                "conda",
                vec!["run".into(), "-n".into(), name.into(), "python".into()],
            )
        }
        PythonEnvironment::Uv {
            project,
            executable,
        } => {
            let project = resolve_directory(project_root, &project, "uv project")?;
            (
                resolve_manager(project_root, executable.as_deref(), "uv")?,
                "uv",
                vec![
                    "run".into(),
                    "--project".into(),
                    project.into_os_string(),
                    "python".into(),
                ],
            )
        }
        PythonEnvironment::Poetry {
            project,
            executable,
        } => {
            let project = resolve_directory(project_root, &project, "Poetry project")?;
            (
                resolve_manager(project_root, executable.as_deref(), "poetry")?,
                "poetry",
                vec![
                    "--directory".into(),
                    project.into_os_string(),
                    "run".into(),
                    "python".into(),
                ],
            )
        }
    };
    args.push(script.as_os_str().to_owned());
    args.extend(declaration.args.into_iter().map(OsString::from));
    Ok(ResolvedProgramTask::for_python(
        program,
        args.into_boxed_slice(),
        seed_purpose,
        timeout,
        script,
        manager.into(),
        threads,
    ))
}

fn resolve_manager(
    project_root: &Path,
    authored: Option<&Path>,
    default: &str,
) -> Result<PathBuf, ConfigError> {
    resolve_executable(project_root, authored.unwrap_or_else(|| Path::new(default)))
}

fn resolve_script(project_root: &Path, authored: &Path) -> Result<PathBuf, ConfigError> {
    if authored.extension().and_then(std::ffi::OsStr::to_str) != Some("py") {
        return Err(ConfigError::InvalidProgram {
            path: authored.to_path_buf(),
            reason: "Python task script must use the `.py` extension".to_owned(),
        });
    }
    resolve_file(project_root, authored, "Python task script")
}

fn resolve_directory(
    project_root: &Path,
    authored: &Path,
    kind: &str,
) -> Result<PathBuf, ConfigError> {
    let resolved = resolve_path(project_root, authored, kind)?;
    if !resolved.is_dir() {
        return Err(ConfigError::InvalidProgram {
            path: authored.to_path_buf(),
            reason: format!("{kind} is not a directory"),
        });
    }
    Ok(resolved)
}

fn resolve_file(project_root: &Path, authored: &Path, kind: &str) -> Result<PathBuf, ConfigError> {
    let resolved = resolve_path(project_root, authored, kind)?;
    if !resolved.is_file() {
        return Err(ConfigError::InvalidProgram {
            path: authored.to_path_buf(),
            reason: format!("{kind} is not a regular file"),
        });
    }
    Ok(resolved)
}

fn resolve_path(project_root: &Path, authored: &Path, kind: &str) -> Result<PathBuf, ConfigError> {
    if authored.as_os_str().is_empty()
        || authored
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err(ConfigError::InvalidProgram {
            path: authored.to_path_buf(),
            reason: format!("{kind} path must be absolute or project-relative without traversal"),
        });
    }
    let candidate = if authored.is_absolute() {
        authored.to_path_buf()
    } else {
        project_root.join(authored)
    };
    let resolved =
        std::fs::canonicalize(&candidate).map_err(|source| ConfigError::InvalidProgram {
            path: authored.to_path_buf(),
            reason: format!("could not resolve {kind}: {source}"),
        })?;
    ensure_utf8(&resolved, "resolved Python task resource")?;
    Ok(resolved)
}

fn validate_name(path: &Path, name: &str, kind: &str) -> Result<(), ConfigError> {
    if name.is_empty() || name.trim() != name || name.starts_with('-') {
        return Err(ConfigError::InvalidProgram {
            path: path.to_path_buf(),
            reason: format!(
                "{kind} name must be nonempty, contain no surrounding whitespace, and not begin with `-`"
            ),
        });
    }
    Ok(())
}
