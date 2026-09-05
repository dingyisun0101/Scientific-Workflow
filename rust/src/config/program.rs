//! Centrally resolved external-program task declarations and launchers.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use super::error::ConfigError;
use super::store::{canonicalize, ensure_utf8};

/// One validated external program invocation, including a lowered Python task.
#[derive(Clone)]
pub(crate) struct ResolvedProgramTask {
    inner: Arc<ResolvedProgramTaskInner>,
}

struct ResolvedProgramTaskInner {
    program: PathBuf,
    args: Box<[OsString]>,
    timeout: Option<Duration>,
    seed_purpose: Option<Box<str>>,
    threads: usize,
    subject: Box<str>,
    source: ProgramSource,
}

#[derive(Clone, Debug)]
enum ProgramSource {
    Executable,
    Npy,
    Python {
        script: PathBuf,
        environment_manager: Box<str>,
    },
}

impl ResolvedProgramTask {
    pub(crate) fn new(
        program: PathBuf,
        args: Box<[Box<str>]>,
        seed_purpose: Option<Box<str>>,
        timeout: Option<Duration>,
        threads: usize,
    ) -> Self {
        let subject = file_subject(&program, "program");
        Self {
            inner: Arc::new(ResolvedProgramTaskInner {
                program,
                args: args
                    .into_vec()
                    .into_iter()
                    .map(|argument| OsString::from(String::from(argument)))
                    .collect(),
                timeout,
                seed_purpose,
                threads,
                subject,
                source: ProgramSource::Executable,
            }),
        }
    }

    pub(crate) fn for_python(
        program: PathBuf,
        args: Box<[OsString]>,
        seed_purpose: Option<Box<str>>,
        timeout: Option<Duration>,
        script: PathBuf,
        environment_manager: Box<str>,
        threads: usize,
    ) -> Self {
        let subject = file_subject(&script, "python");
        Self {
            inner: Arc::new(ResolvedProgramTaskInner {
                program,
                args,
                timeout,
                seed_purpose,
                threads,
                subject,
                source: ProgramSource::Python {
                    script,
                    environment_manager,
                },
            }),
        }
    }

    pub(crate) fn for_npy(program: PathBuf) -> Self {
        Self {
            inner: Arc::new(ResolvedProgramTaskInner {
                program,
                args: [
                    OsString::from("-m"),
                    OsString::from("scientific_workflow.npy"),
                    OsString::from("--workflow-dependencies"),
                ]
                .into(),
                timeout: None,
                seed_purpose: None,
                threads: 1,
                subject: "NPY conversion".into(),
                source: ProgramSource::Npy,
            }),
        }
    }

    pub(crate) fn program(&self) -> &Path {
        &self.inner.program
    }

    pub(crate) fn args(&self) -> &[OsString] {
        &self.inner.args
    }

    pub(crate) fn timeout(&self) -> Option<Duration> {
        self.inner.timeout
    }

    pub(crate) fn seed_purpose(&self) -> Option<&str> {
        self.inner.seed_purpose.as_deref()
    }

    pub(crate) fn threads(&self) -> usize {
        self.inner.threads
    }

    pub(crate) fn subject(&self) -> &str {
        &self.inner.subject
    }

    pub(crate) fn kind_name(&self) -> &'static str {
        match self.inner.source {
            ProgramSource::Executable => "program",
            ProgramSource::Npy => "npy",
            ProgramSource::Python { .. } => "python",
        }
    }

    pub(crate) fn python_script(&self) -> Option<&Path> {
        match &self.inner.source {
            ProgramSource::Executable | ProgramSource::Npy => None,
            ProgramSource::Python { script, .. } => Some(script),
        }
    }

    pub(crate) fn python_environment_manager(&self) -> Option<&str> {
        match &self.inner.source {
            ProgramSource::Executable | ProgramSource::Npy => None,
            ProgramSource::Python {
                environment_manager,
                ..
            } => Some(environment_manager),
        }
    }

    pub(crate) fn is_npy(&self) -> bool {
        matches!(self.inner.source, ProgramSource::Npy)
    }
}

impl std::fmt::Debug for ResolvedProgramTask {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResolvedProgramTask")
            .field("program", &self.program())
            .field("args", &self.args())
            .field("timeout", &self.timeout())
            .field("seed_purpose", &self.seed_purpose())
            .field("threads", &self.threads())
            .field("kind", &self.kind_name())
            .field("subject", &self.subject())
            .finish()
    }
}

pub(crate) fn resolve_executable(
    project_root: &Path,
    authored: &Path,
) -> Result<PathBuf, ConfigError> {
    if authored.as_os_str().is_empty()
        || authored
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err(ConfigError::InvalidProgram {
            path: authored.to_path_buf(),
            reason: "program must be an absolute path, a project-relative path, or a command name"
                .to_owned(),
        });
    }

    let candidate = if authored.is_absolute() {
        Some(authored.to_path_buf())
    } else {
        let project_candidate = project_root.join(authored);
        if project_candidate.is_file() {
            Some(project_candidate)
        } else if authored.components().count() == 1 {
            std::env::var_os("PATH").and_then(|path| {
                std::env::split_paths(&path)
                    .map(|directory| directory.join(authored))
                    .find(|candidate| is_executable(candidate))
            })
        } else {
            None
        }
    }
    .ok_or_else(|| ConfigError::InvalidProgram {
        path: authored.to_path_buf(),
        reason: "program was not found in the project or executable search path".to_owned(),
    })?;

    let resolved = canonicalize(&candidate)?;
    ensure_utf8(&resolved, "resolved executable")?;
    if !is_executable(&resolved) {
        return Err(ConfigError::InvalidProgram {
            path: authored.to_path_buf(),
            reason: "resolved program is not an executable regular file".to_owned(),
        });
    }
    Ok(resolved)
}

fn file_subject(path: &Path, fallback: &str) -> Box<str> {
    path.file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or(fallback)
        .into()
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;

    path.metadata()
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}
