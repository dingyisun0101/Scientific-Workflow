//! Private automatic model-recording and program-workspace sessions.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::observation::advanced::BoundObservationPlan;
use crate::state::advanced::SystemState;

use super::local::{PersistenceError, StateStreamStorage, SystemStateWriter};
use super::plan::PersistencePlan;

pub(crate) struct PersistenceSession {
    writer: Option<SystemStateWriter>,
}

/// Owned semantic provenance supplied when one model recording begins.
#[derive(Clone)]
pub(crate) struct ModelRecordingProvenance {
    task_identity: Box<str>,
    model: Box<str>,
    state: Box<str>,
    parameter_ordinal: u64,
    parameter_source: PathBuf,
    model_constants: Value,
    member_index: Option<usize>,
    member_identity: Option<Box<str>>,
}

impl ModelRecordingProvenance {
    pub(crate) fn new(
        task_identity: &str,
        model: &str,
        state: &str,
        parameter_ordinal: u64,
        parameter_source: &Path,
        model_constants: Value,
    ) -> Self {
        Self {
            task_identity: task_identity.into(),
            model: model.into(),
            state: state.into(),
            parameter_ordinal,
            parameter_source: parameter_source.to_path_buf(),
            model_constants,
            member_index: None,
            member_identity: None,
        }
    }

    pub(crate) fn with_member(mut self, index: usize, identity: &str) -> Self {
        self.member_index = Some(index);
        self.member_identity = Some(identity.into());
        self
    }

    fn into_metadata(self, persistence_plan: PersistencePlan) -> Map<String, Value> {
        let persistence = Value::Object(Map::from_iter([
            ("backend".to_owned(), "local".into()),
            (
                "chunk_target_bytes".to_owned(),
                persistence_plan.chunk_target().get().into(),
            ),
            (
                "queue_capacity_bytes".to_owned(),
                persistence_plan.queue_capacity().get().into(),
            ),
        ]));
        let workflow = Value::Object(Map::from_iter([
            (
                "task_identity".to_owned(),
                Value::String(self.task_identity.into()),
            ),
            ("kind".to_owned(), "model".into()),
            ("model".to_owned(), Value::String(self.model.into())),
            ("state".to_owned(), Value::String(self.state.into())),
            (
                "member_index".to_owned(),
                self.member_index.map_or(Value::Null, Value::from),
            ),
            (
                "member_identity".to_owned(),
                self.member_identity
                    .map_or(Value::Null, |identity| Value::String(identity.into())),
            ),
            (
                "parameter_ordinal".to_owned(),
                self.parameter_ordinal.into(),
            ),
            (
                "parameter_source".to_owned(),
                self.parameter_source
                    .to_str()
                    .expect("Config preflight requires UTF-8 parameter paths")
                    .into(),
            ),
            ("persistence".to_owned(), persistence),
        ]));
        Map::from_iter([
            ("model_constants".to_owned(), self.model_constants),
            ("workflow".to_owned(), workflow),
        ])
    }
}

/// Durable workspace prepared for one external-program or Python task.
pub(crate) struct ProgramPersistenceSession {
    directory: PathBuf,
    artifacts: PathBuf,
    config_path: PathBuf,
    dependencies_path: PathBuf,
    stdout: Option<File>,
    stderr: Option<File>,
    program: PathBuf,
    args: Box<[String]>,
    program_kind: Box<str>,
    python_script: Option<PathBuf>,
    python_environment_manager: Option<Box<str>>,
}

/// Borrowed resolved launcher provenance used to construct a program workspace.
pub(crate) struct ProgramLaunch<'a> {
    pub(crate) executable: &'a Path,
    pub(crate) args: &'a [OsString],
    pub(crate) kind: &'a str,
    pub(crate) python_script: Option<&'a Path>,
    pub(crate) python_environment_manager: Option<&'a str>,
}

impl ProgramPersistenceSession {
    pub(crate) fn start(
        directory: PathBuf,
        config_json: &[u8],
        dependencies_json: &[u8],
        launch: ProgramLaunch<'_>,
    ) -> Result<Self, PersistenceError> {
        create_directory(&directory)?;
        let artifacts = directory.join("artifacts");
        create_directory(&artifacts)?;
        let config_path = directory.join("workflow-config.json");
        let dependencies_path = directory.join("workflow-dependencies.json");
        write_new(
            &config_path,
            config_json,
            "write program configuration snapshot",
        )?;
        write_new(
            &dependencies_path,
            dependencies_json,
            "write program dependency snapshot",
        )?;
        let stdout_path = directory.join("stdout.log");
        let stderr_path = directory.join("stderr.log");
        let stdout = create_file(&stdout_path, "create program standard-output log")?;
        let stderr = create_file(&stderr_path, "create program standard-error log")?;

        sync_directory(&directory, "synchronize prepared program workspace entries")?;

        let mut session = Self {
            directory,
            artifacts,
            config_path,
            dependencies_path,
            stdout: Some(stdout),
            stderr: Some(stderr),
            program: launch.executable.to_path_buf(),
            args: launch
                .args
                .iter()
                .map(|argument| {
                    argument
                        .to_str()
                        .expect("Config preflight requires UTF-8 program arguments")
                        .to_owned()
                })
                .collect(),
            program_kind: launch.kind.into(),
            python_script: launch.python_script.map(Path::to_path_buf),
            python_environment_manager: launch.python_environment_manager.map(Into::into),
        };
        session.write_status("running", None, None)?;
        Ok(session)
    }

    pub(crate) fn artifacts_directory(&self) -> &Path {
        &self.artifacts
    }

    pub(crate) fn config_path(&self) -> &Path {
        &self.config_path
    }

    pub(crate) fn dependencies_path(&self) -> &Path {
        &self.dependencies_path
    }

    pub(crate) fn take_stdout(&mut self) -> File {
        self.stdout
            .take()
            .expect("program stdout is transferred exactly once")
    }

    pub(crate) fn take_stderr(&mut self) -> File {
        self.stderr
            .take()
            .expect("program stderr is transferred exactly once")
    }

    pub(crate) fn complete(&mut self, exit_code: Option<i32>) -> Result<(), PersistenceError> {
        self.write_status("complete", exit_code, None)
    }

    pub(crate) fn fail(&mut self, exit_code: Option<i32>, reason: &str) {
        let _ = self.write_status("failed", exit_code, Some(reason));
    }

    fn write_status(
        &mut self,
        status: &str,
        exit_code: Option<i32>,
        reason: Option<&str>,
    ) -> Result<(), PersistenceError> {
        let metadata_path = self.directory.join("program.json");
        let temporary_path = self.directory.join(".program.json.tmp");
        let value = serde_json::json!({
            "format": "scientific-workflow-program-v1",
            "status": status,
            "kind": self.program_kind,
            "program": self.program,
            "args": self.args,
            "python_script": self.python_script,
            "python_environment_manager": self.python_environment_manager,
            "exit_code": exit_code,
            "reason": reason,
            "config": "workflow-config.json",
            "dependencies": "workflow-dependencies.json",
            "artifacts": "artifacts",
            "stdout": "stdout.log",
            "stderr": "stderr.log"
        });
        let bytes = serde_json::to_vec_pretty(&value).map_err(|source| PersistenceError::Json {
            operation: "serialize program metadata",
            path: metadata_path.clone(),
            source,
        })?;
        let _ = fs::remove_file(&temporary_path);
        write_new(&temporary_path, &bytes, "write temporary program metadata")?;
        fs::rename(&temporary_path, &metadata_path).map_err(|source| PersistenceError::Io {
            operation: "commit program metadata",
            path: metadata_path.clone(),
            source,
        })?;
        sync_directory(&self.directory, "synchronize program metadata transition")
    }
}

fn create_directory(path: &Path) -> Result<(), PersistenceError> {
    fs::create_dir(path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::AlreadyExists {
            PersistenceError::RecordingDirectoryExists {
                path: path.to_path_buf(),
            }
        } else {
            PersistenceError::Io {
                operation: "create program workspace directory",
                path: path.to_path_buf(),
                source,
            }
        }
    })
}

fn create_file(path: &Path, operation: &'static str) -> Result<File, PersistenceError> {
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|source| PersistenceError::Io {
            operation,
            path: path.to_path_buf(),
            source,
        })
}

fn write_new(path: &Path, bytes: &[u8], operation: &'static str) -> Result<(), PersistenceError> {
    let mut file = create_file(path, operation)?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|source| PersistenceError::Io {
            operation,
            path: path.to_path_buf(),
            source,
        })
}

fn sync_directory(path: &Path, operation: &'static str) -> Result<(), PersistenceError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| PersistenceError::Io {
            operation,
            path: path.to_path_buf(),
            source,
        })
}

impl PersistenceSession {
    pub(crate) fn start(
        directory: PathBuf,
        observation_plan: BoundObservationPlan,
        persistence_plan: PersistencePlan,
        provenance: ModelRecordingProvenance,
        initial_state: &SystemState,
    ) -> Result<Self, PersistenceError> {
        if let Some(parent) = directory.parent() {
            fs::create_dir_all(parent).map_err(|source| PersistenceError::Io {
                operation: "create model recording parent directories",
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let storage = StateStreamStorage::chunked(
            persistence_plan.chunk_target(),
            persistence_plan.queue_capacity(),
        );
        let metadata = provenance.into_metadata(persistence_plan);
        let mut writer = SystemStateWriter::create(directory, observation_plan, metadata, storage)?;
        if let Err(error) = writer.observe_state(initial_state) {
            let _ = writer.mark_recording_failed(error.to_string());
            return Err(error);
        }
        Ok(Self {
            writer: Some(writer),
        })
    }

    pub(crate) fn observe(&mut self, state: &SystemState) -> Result<(), PersistenceError> {
        self.writer
            .as_mut()
            .expect("active persistence session owns its backend")
            .observe_state(state)
    }

    pub(crate) fn complete(&mut self, state: &SystemState) -> Result<(), PersistenceError> {
        self.writer
            .take()
            .expect("active persistence session owns its backend")
            .complete_recording_with_final_state(state)?;
        Ok(())
    }

    pub(crate) fn fail(&mut self, reason: &str) {
        if let Some(writer) = self.writer.take() {
            let _ = writer.mark_recording_failed(reason.to_owned());
        }
    }
}
