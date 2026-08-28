//! Runtime adapter from task observation boundaries to automatic persistence.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use thiserror::Error;

use crate::config::advanced::ConfigSnapshot;
use crate::persistence::advanced::{
    ModelRecordingProvenance, PersistencePlan, PersistenceSession, ProgramLaunch,
    ProgramPersistenceSession,
};
use crate::state::advanced::SystemState;
use crate::task::advanced::{
    InitializationContext, ModelInitialization, ProgramTaskInvocation, TaskExecutionHost,
    TaskResult,
};
use crate::ui::advanced::TaskUi;

use super::summary::ModelRunSummary;

pub(crate) struct RuntimeTaskHost {
    persistence_plan: PersistencePlan,
    cancellation: Arc<AtomicBool>,
    output_directory: PathBuf,
    provenance: Option<ModelRecordingProvenance>,
    initialization_context: Option<InitializationContext>,
    persistence: Vec<Option<PersistenceSession>>,
    member_iterations: Vec<u64>,
    member_targets: Vec<Option<u64>>,
    member_identities: Vec<Option<Box<str>>>,
    member_directories: Vec<Option<PathBuf>>,
    final_iteration: Option<u64>,
    task_ui: TaskUi,
    environment: RuntimeTaskEnvironment,
}

pub(crate) struct RuntimeTaskEnvironment {
    config_snapshot: ConfigSnapshot,
    project_root: PathBuf,
    replicate_directory: PathBuf,
    dependencies_json: Box<[u8]>,
}

impl RuntimeTaskEnvironment {
    pub(crate) fn new(
        config_snapshot: ConfigSnapshot,
        project_root: PathBuf,
        replicate_directory: PathBuf,
        dependencies_json: Box<[u8]>,
    ) -> Self {
        Self {
            config_snapshot,
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
        provenance: Option<ModelRecordingProvenance>,
        initialization_context: Option<InitializationContext>,
        task_ui: TaskUi,
        environment: RuntimeTaskEnvironment,
    ) -> Self {
        Self {
            persistence_plan,
            cancellation,
            output_directory,
            provenance,
            initialization_context,
            persistence: Vec::new(),
            member_iterations: Vec::new(),
            member_targets: Vec::new(),
            member_identities: Vec::new(),
            member_directories: Vec::new(),
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

    pub(crate) fn model_summaries(&self) -> Box<[ModelRunSummary]> {
        self.member_identities
            .iter()
            .zip(&self.member_directories)
            .zip(&self.member_iterations)
            .map(|((identity, directory), final_iteration)| ModelRunSummary {
                identity: identity
                    .clone()
                    .expect("every completed model retains its identity"),
                final_iteration: *final_iteration,
                output_directory: directory
                    .clone()
                    .expect("every completed model retains its recording directory"),
            })
            .collect()
    }

    pub(crate) fn cancellation_requested(&self) -> bool {
        self.cancellation.load(Ordering::Acquire)
    }

    pub(crate) fn fail(&mut self, reason: &str) {
        for persistence in &mut self.persistence {
            if let Some(mut persistence) = persistence.take() {
                persistence.fail(reason);
            }
        }
    }
}

impl TaskExecutionHost for RuntimeTaskHost {
    fn cancellation_requested(&self) -> bool {
        self.cancellation_requested()
    }

    fn initialization_context(&self) -> Option<&InitializationContext> {
        self.initialization_context.as_ref()
    }

    fn execute_program(&mut self, program: ProgramTaskInvocation<'_>) -> TaskResult {
        let mut persistence = ProgramPersistenceSession::start(
            self.output_directory.clone(),
            self.environment.config_snapshot.bytes(),
            &self.environment.dependencies_json,
            ProgramLaunch {
                executable: program.executable(),
                args: program.args(),
                kind: program.kind(),
                python_script: program.python_script(),
                python_environment_manager: program.python_environment_manager(),
            },
        )?;
        let execution_root = self
            .environment
            .replicate_directory
            .parent()
            .unwrap_or(&self.environment.replicate_directory);
        let mut command = Command::new(program.executable());
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
                program: program.executable().to_path_buf(),
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
                        program: program.executable().to_path_buf(),
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
                        program: program.executable().to_path_buf(),
                        source,
                    }
                    .into());
                }
            }
        }
    }

    fn begin_model(&mut self, model: ModelInitialization<'_>) -> TaskResult {
        let ModelInitialization {
            index,
            model_count,
            identity,
            seed_derivation,
            plan,
            state,
            target_iteration,
        } = model;
        let provenance = self
            .provenance
            .as_ref()
            .expect("a model task retains recording provenance")
            .clone()
            .with_member(index, identity)
            .with_seed_derivation(seed_derivation);
        if self.persistence.is_empty() {
            self.persistence.resize_with(model_count, || None);
            self.member_iterations.resize(model_count, 0);
            self.member_targets.resize(model_count, None);
            self.member_identities.resize(model_count, None);
            self.member_directories.resize(model_count, None);
        }
        let directory = if model_count == 1 {
            self.output_directory.clone()
        } else {
            self.output_directory
                .join("models")
                .join(format!("model-{index:06}"))
        };
        let persistence = PersistenceSession::start(
            directory.clone(),
            plan,
            self.persistence_plan,
            provenance,
            state,
        )?;
        self.member_iterations[index] = state.time().iteration();
        self.member_targets[index] = target_iteration;
        self.member_identities[index] = Some(identity.into());
        self.member_directories[index] = Some(directory);
        self.final_iteration = self.member_iterations.iter().copied().max();
        self.persistence[index] = Some(persistence);
        self.publish_progress();
        Ok(())
    }

    fn observe_model_step(
        &mut self,
        index: usize,
        state: &SystemState,
        target_iteration: Option<u64>,
    ) -> TaskResult {
        if self.cancellation_requested() {
            return Ok(());
        }
        self.persistence[index]
            .as_mut()
            .expect("begin_model precedes step observation")
            .observe(state)?;
        self.member_iterations[index] = state.time().iteration();
        self.member_targets[index] = target_iteration;
        self.final_iteration = self.member_iterations.iter().copied().max();
        self.publish_progress();
        Ok(())
    }

    fn observe_model_final(
        &mut self,
        index: usize,
        state: &SystemState,
        target_iteration: Option<u64>,
    ) -> TaskResult {
        if self.cancellation_requested() {
            return Ok(());
        }
        self.persistence[index]
            .as_mut()
            .expect("begin_model precedes final observation")
            .complete(state)?;
        self.persistence[index] = None;
        self.member_iterations[index] = state.time().iteration();
        self.member_targets[index] = target_iteration;
        self.final_iteration = self.member_iterations.iter().copied().max();
        self.publish_progress();
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
    fn publish_progress(&self) {
        let iteration = self
            .member_iterations
            .iter()
            .copied()
            .fold(0_u64, u64::saturating_add);
        let target = self
            .member_targets
            .iter()
            .copied()
            .collect::<Option<Vec<_>>>()
            .map(|targets| targets.into_iter().fold(0_u64, u64::saturating_add));
        self.task_ui.progress(iteration, target);
    }
}
