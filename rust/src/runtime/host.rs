//! Runtime adapter from task observation boundaries to automatic persistence.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use serde_json::{Map, Value};
use thiserror::Error;

use crate::config::advanced::{Config, ResolvedProgramTask};
use crate::observation::advanced::BoundObservationPlan;
use crate::persistence::advanced::{
    PersistencePlan, PersistenceSession, ProgramLaunch, ProgramPersistenceSession,
};
use crate::state::advanced::SystemState;
use crate::task::advanced::{TaskExecutionHost, TaskResult};
use crate::ui::advanced::TaskUi;

pub(crate) struct RuntimeTaskHost {
    persistence_plan: PersistencePlan,
    cancellation: Arc<AtomicBool>,
    output_directory: PathBuf,
    metadata: Map<String, Value>,
    persistence: Option<PersistenceSession>,
    final_iteration: Option<u64>,
    task_ui: TaskUi,
    environment: RuntimeTaskEnvironment,
}

pub(crate) struct RuntimeTaskEnvironment {
    config: Config,
    project_root: PathBuf,
    replicate_directory: PathBuf,
    dependencies_json: Box<[u8]>,
}

impl RuntimeTaskEnvironment {
    pub(crate) fn new(
        config: Config,
        project_root: PathBuf,
        replicate_directory: PathBuf,
        dependencies_json: Box<[u8]>,
    ) -> Self {
        Self {
            config,
            project_root,
            replicate_directory,
            dependencies_json,
        }
    }
}

impl RuntimeTaskHost {
    pub(crate) fn new(
        persistence_plan: PersistencePlan,
        cancellation: Arc<AtomicBool>,
        output_directory: PathBuf,
        metadata: Map<String, Value>,
        task_ui: TaskUi,
        environment: RuntimeTaskEnvironment,
    ) -> Self {
        Self {
            persistence_plan,
            cancellation,
            output_directory,
            metadata,
            persistence: None,
            final_iteration: None,
            task_ui,
            environment,
        }
    }

    pub(crate) fn output_directory(&self) -> &Path {
        &self.output_directory
    }

    pub(crate) fn final_iteration(&self) -> Option<u64> {
        self.final_iteration
    }

    pub(crate) fn cancellation_requested(&self) -> bool {
        self.cancellation.load(Ordering::Acquire)
    }

    pub(crate) fn fail(&mut self, reason: &str) {
        if let Some(mut persistence) = self.persistence.take() {
            persistence.fail(reason);
        }
    }
}

impl TaskExecutionHost for RuntimeTaskHost {
    fn cancellation_requested(&self) -> bool {
        self.cancellation_requested()
    }

    fn execute_program(&mut self, program: &ResolvedProgramTask) -> TaskResult {
        let mut persistence = ProgramPersistenceSession::start(
            self.output_directory.clone(),
            self.environment.config.snapshot_json(),
            &self.environment.dependencies_json,
            ProgramLaunch {
                executable: program.program(),
                args: program.args(),
                kind: program.kind_name(),
                python_script: program.python_script(),
                python_environment_manager: program.python_environment_manager(),
            },
        )?;
        let execution_root = self
            .environment
            .replicate_directory
            .parent()
            .unwrap_or(&self.environment.replicate_directory);
        let mut command = Command::new(program.program());
        command
            .args(program.args())
            .current_dir(persistence.artifacts_directory())
            .env("WORKFLOW_CONFIG_PATH", persistence.config_path())
            .env(
                "WORKFLOW_DEPENDENCIES_PATH",
                persistence.dependencies_path(),
            )
            .env("WORKFLOW_PROJECT_ROOT", &self.environment.project_root)
            .env("WORKFLOW_EXECUTION_ROOT", execution_root)
            .env(
                "WORKFLOW_REPLICATE_ROOT",
                &self.environment.replicate_directory,
            )
            .env("WORKFLOW_TASK_OUTPUT", persistence.artifacts_directory())
            .stdin(Stdio::null())
            .stdout(Stdio::from(persistence.take_stdout()))
            .stderr(Stdio::from(persistence.take_stderr()));

        let mut child = command.spawn().map_err(|source| {
            persistence.fail(None, &source.to_string());
            Box::new(ProgramExecutionError::Start {
                program: program.program().to_path_buf(),
                source,
            }) as Box<dyn std::error::Error + Send + Sync>
        })?;

        loop {
            if self.cancellation_requested() {
                let _ = child.kill();
                let _ = child.wait();
                persistence.fail(None, "runtime cancellation requested");
                return Ok(());
            }
            match child.try_wait() {
                Ok(Some(status)) if status.success() => {
                    persistence.complete(status.code())?;
                    return Ok(());
                }
                Ok(Some(status)) => {
                    let reason = format!("program exited with status {status}");
                    persistence.fail(status.code(), &reason);
                    return Err(ProgramExecutionError::Exit {
                        program: program.program().to_path_buf(),
                        status: status.to_string(),
                    }
                    .into());
                }
                Ok(None) => thread::sleep(Duration::from_millis(5)),
                Err(source) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    persistence.fail(None, &source.to_string());
                    return Err(ProgramExecutionError::Wait {
                        program: program.program().to_path_buf(),
                        source,
                    }
                    .into());
                }
            }
        }
    }

    fn begin_model(
        &mut self,
        plan: BoundObservationPlan,
        state: &SystemState,
        target_iteration: Option<u64>,
    ) -> TaskResult {
        let persistence = PersistenceSession::start(
            self.output_directory.clone(),
            plan,
            self.persistence_plan,
            self.metadata.clone(),
            state,
        )?;
        self.final_iteration = Some(state.time().iteration());
        self.persistence = Some(persistence);
        self.publish_progress(state, target_iteration);
        Ok(())
    }

    fn observe_model_step(
        &mut self,
        state: &SystemState,
        target_iteration: Option<u64>,
    ) -> TaskResult {
        self.persistence
            .as_mut()
            .expect("begin_model precedes step observation")
            .observe(state)?;
        self.final_iteration = Some(state.time().iteration());
        self.publish_progress(state, target_iteration);
        Ok(())
    }

    fn observe_model_final(
        &mut self,
        state: &SystemState,
        target_iteration: Option<u64>,
    ) -> TaskResult {
        self.persistence
            .as_mut()
            .expect("begin_model precedes final observation")
            .complete(state)?;
        self.persistence = None;
        self.final_iteration = Some(state.time().iteration());
        self.publish_progress(state, target_iteration);
        Ok(())
    }
}

#[derive(Debug, Error)]
enum ProgramExecutionError {
    #[error("failed to start program `{program}`")]
    Start {
        program: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("program `{program}` failed while waiting for completion")]
    Wait {
        program: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("program `{program}` exited unsuccessfully: {status}")]
    Exit { program: PathBuf, status: String },
}

impl RuntimeTaskHost {
    fn publish_progress(&self, state: &SystemState, target_iteration: Option<u64>) {
        self.task_ui
            .progress(state.time().iteration(), target_iteration);
    }
}
