//! Task execution, hosting, and persistence adaptation.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Map, Value};

use crate::config::ConfigSnapshot;
use crate::persistence::{
    MemberRecordingProvenance, PersistencePlan, PersistenceSession, ProgramLaunch,
    ProgramPersistenceSession,
};
use crate::state::SystemState;
use crate::study::StudyTask;
use crate::task::{
    InitializationContext, MemberInitialization, ProgramTaskInvocation, SEED_DERIVATION_ALGORITHM,
    TaskExecutionHost, TaskKind, TaskResult, derive_program_seed,
};

use super::super::error::RuntimeError;
use super::super::presentation::{RuntimePresentation, TaskPresentation};
use super::super::resources::TaskResourceLease;
use super::super::summary::{MemberRunSummary, TaskRunKind, TaskRunSummary};

pub(super) struct TaskRuntime {
    pub(super) persistence_plan: PersistencePlan,
    pub(super) config_snapshot: ConfigSnapshot,
    pub(super) project_root: PathBuf,
    pub(super) replicate_directory: PathBuf,
    pub(super) dependencies_json: Box<[u8]>,
    pub(super) processed_directory: Option<PathBuf>,
    pub(super) configuration: usize,
    pub(super) replicate: u64,
    pub(super) master_seed: Option<u64>,
    pub(super) threads: usize,
    pub(super) resources: TaskResourceLease,
    pub(super) presentation: RuntimePresentation,
}

pub(super) fn run_task(
    task: StudyTask,
    runtime: TaskRuntime,
    cancellation: Arc<AtomicBool>,
    output_directory: PathBuf,
) -> Result<TaskRunSummary, RuntimeError> {
    let processed_directory = runtime.processed_directory.clone();
    let mut resources = runtime.resources;
    if task.kind() == TaskKind::ExecutionUnit {
        resources
            .activate_compute(runtime.replicate, task.output_ordinal())
            .map_err(|source| RuntimeError::Task {
                task: task.identity().to_owned(),
                source: Box::new(source),
            })?;
    }
    let program_seed = task.program_seed_purpose().map(|purpose| {
        let master_seed = runtime
            .master_seed
            .expect("Config rejects a program seed request without a master seed");
        let seed = derive_program_seed(
            master_seed,
            runtime.replicate,
            task.identity(),
            task.kind_name(),
            purpose,
        );
        ProgramSeed::new(
            seed,
            serde_json::json!({
                "algorithm": SEED_DERIVATION_ALGORITHM,
                "master_seed": master_seed,
                "requests": [{
                    "scope": "task",
                    "purpose": purpose,
                    "seed": seed
                }]
            }),
        )
    });
    let initialization_context = task.execution_unit().map(|execution_unit_key| {
        let dependencies = serde_json::from_slice(&runtime.dependencies_json)
            .expect("Runtime's dependency snapshot is valid JSON");
        InitializationContext::with_dependencies(
            runtime.master_seed,
            runtime.replicate,
            task.identity(),
            execution_unit_key,
            dependencies,
        )
    });
    let provenance = task.execution_unit_provenance().map(|provenance| {
        let mut parameters = runtime.config_snapshot.parameters().clone();
        parameters
            .as_object_mut()
            .expect("parameters.json root is an object")
            .insert(
                provenance.execution_unit().to_owned(),
                provenance.constants().clone(),
            );
        MemberRecordingProvenance::new(
            task.identity(),
            provenance.execution_unit(),
            provenance.state(),
            provenance.parameter_ordinal(),
            provenance.parameter_source(),
            provenance.constants().clone(),
            runtime.threads,
        )
        .with_parameters(parameters)
    });
    let environment = RuntimeTaskEnvironment::new(
        runtime.config_snapshot,
        runtime.project_root,
        runtime.replicate_directory,
        runtime.dependencies_json,
        runtime.processed_directory,
    );
    let mut host = RuntimeTaskHost::new(
        runtime.persistence_plan,
        cancellation,
        output_directory,
        RuntimeTaskLaunch::new(
            provenance,
            initialization_context,
            program_seed,
            runtime.threads,
            runtime
                .presentation
                .task(runtime.replicate, task.identity()),
            environment,
            resources,
        ),
    );
    match catch_unwind(AssertUnwindSafe(|| task.definition().execute(&mut host))) {
        Ok(Ok(())) => {}
        Ok(Err(source)) => {
            host.fail(&source.to_string());
            return Err(RuntimeError::Task {
                task: task.identity().to_owned(),
                source,
            });
        }
        Err(payload) => {
            let reason = panic_reason(payload.as_ref());
            host.fail(&format!("task panicked: {reason}"));
            return Err(RuntimeError::TaskPanicked {
                task: task.identity().to_owned(),
            });
        }
    }
    host.flush_progress();
    if host.cancellation_requested() {
        host.fail("runtime cancellation requested");
        return Err(RuntimeError::TaskCancelled {
            task: task.identity().to_owned(),
        });
    }
    let kind = match task.kind() {
        TaskKind::ExecutionUnit => TaskRunKind::ExecutionUnit {
            execution_unit: task
                .execution_unit()
                .expect("execution-unit task retains its registration key")
                .into(),
            members: host.member_summaries(),
        },
        TaskKind::Program if task.is_npy() => TaskRunKind::Npy {
            launcher: task
                .program_path()
                .expect("NPY task retains its resolved Python launcher")
                .to_path_buf(),
            processed_directory: processed_directory
                .expect("an NPY task retains its standard processed directory"),
        },
        TaskKind::Program => TaskRunKind::Program {
            executable: task
                .program_path()
                .expect("program task retains its resolved invocation")
                .to_path_buf(),
            python_script: task.python_script().map(Path::to_path_buf),
        },
    };
    Ok(TaskRunSummary {
        identity: task.identity().into(),
        kind,
        output_directory: host.output_directory().to_path_buf(),
        configuration: runtime.configuration,
    })
}

fn panic_reason(payload: &(dyn std::any::Any + Send)) -> String {
    const MAX_CHARS: usize = 1_024;

    let message = payload
        .downcast_ref::<&str>()
        .map(|message| (*message).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-string panic payload".to_owned());
    let mut characters = message.chars();
    let bounded = characters.by_ref().take(MAX_CHARS).collect::<String>();
    if characters.next().is_some() {
        format!("{bounded}…")
    } else {
        bounded
    }
}

pub(crate) struct RuntimeTaskHost {
    persistence_plan: PersistencePlan,
    cancellation: Arc<AtomicBool>,
    output_directory: PathBuf,
    provenance: Option<MemberRecordingProvenance>,
    initialization_context: Option<InitializationContext>,
    program_seed: Option<ProgramSeed>,
    threads: usize,
    persistence: Vec<Option<PersistenceSession>>,
    member_iterations: Vec<u64>,
    member_targets: Vec<Option<u64>>,
    member_identities: Vec<Option<Box<str>>>,
    member_directories: Vec<Option<PathBuf>>,
    task_presentation: TaskPresentation,
    environment: RuntimeTaskEnvironment,
    resources: TaskResourceLease,
}

pub(crate) struct ProgramSeed {
    seed: u64,
    metadata: Value,
}

pub(crate) struct RuntimeTaskLaunch {
    provenance: Option<MemberRecordingProvenance>,
    initialization_context: Option<InitializationContext>,
    program_seed: Option<ProgramSeed>,
    threads: usize,
    task_presentation: TaskPresentation,
    environment: RuntimeTaskEnvironment,
    resources: TaskResourceLease,
}

impl RuntimeTaskLaunch {
    pub(crate) fn new(
        provenance: Option<MemberRecordingProvenance>,
        initialization_context: Option<InitializationContext>,
        program_seed: Option<ProgramSeed>,
        threads: usize,
        task_presentation: TaskPresentation,
        environment: RuntimeTaskEnvironment,
        resources: TaskResourceLease,
    ) -> Self {
        Self {
            provenance,
            initialization_context,
            program_seed,
            threads,
            task_presentation,
            environment,
            resources,
        }
    }
}

impl ProgramSeed {
    pub(crate) fn new(seed: u64, metadata: Value) -> Self {
        Self { seed, metadata }
    }
}

pub(crate) struct RuntimeTaskEnvironment {
    config_snapshot: ConfigSnapshot,
    project_root: PathBuf,
    replicate_directory: PathBuf,
    dependencies_json: Box<[u8]>,
    processed_directory: Option<PathBuf>,
}

impl RuntimeTaskEnvironment {
    pub(crate) fn new(
        config_snapshot: ConfigSnapshot,
        project_root: PathBuf,
        replicate_directory: PathBuf,
        dependencies_json: Box<[u8]>,
        processed_directory: Option<PathBuf>,
    ) -> Self {
        Self {
            config_snapshot,
            project_root,
            replicate_directory,
            dependencies_json,
            processed_directory,
        }
    }
}

impl RuntimeTaskHost {
    pub(crate) fn new(
        persistence_plan: PersistencePlan,
        cancellation: Arc<AtomicBool>,
        output_directory: PathBuf,
        launch: RuntimeTaskLaunch,
    ) -> Self {
        Self {
            persistence_plan,
            cancellation,
            output_directory,
            provenance: launch.provenance,
            initialization_context: launch.initialization_context,
            program_seed: launch.program_seed,
            threads: launch.threads,
            persistence: Vec::new(),
            member_iterations: Vec::new(),
            member_targets: Vec::new(),
            member_identities: Vec::new(),
            member_directories: Vec::new(),
            task_presentation: launch.task_presentation,
            environment: launch.environment,
            resources: launch.resources,
        }
    }

    pub(crate) fn output_directory(&self) -> &Path {
        &self.output_directory
    }

    pub(crate) fn member_summaries(&self) -> Box<[MemberRunSummary]> {
        self.member_identities
            .iter()
            .zip(&self.member_directories)
            .zip(&self.member_iterations)
            .map(
                |((identity, directory), final_iteration)| MemberRunSummary {
                    identity: identity
                        .clone()
                        .expect("every completed member retains its identity"),
                    final_iteration: *final_iteration,
                    output_directory: directory
                        .clone()
                        .expect("every completed member retains its recording directory"),
                },
            )
            .collect()
    }

    pub(crate) fn cancellation_requested(&self) -> bool {
        self.cancellation.load(Ordering::Acquire) || self.task_presentation.control().cancelled()
    }

    pub(crate) fn flush_progress(&self) {
        self.task_presentation.flush();
    }

    pub(crate) fn fail(&mut self, reason: &str) {
        self.flush_progress();
        let compute = self.resources.compute_provenance();
        for persistence in &mut self.persistence {
            if let Some(mut persistence) = persistence.take() {
                persistence.fail(reason, compute.clone());
            }
        }
    }
}

impl TaskExecutionHost for RuntimeTaskHost {
    fn compute(&self, operation: &mut (dyn FnMut() -> TaskResult + Send)) -> TaskResult {
        self.resources.run(operation)
    }

    fn checkpoint(&self) {
        self.task_presentation
            .control()
            .checkpoint(&self.cancellation);
    }

    fn cancellation_requested(&self) -> bool {
        self.cancellation_requested()
    }

    fn initialization_context(&self) -> Option<&InitializationContext> {
        self.initialization_context.as_ref()
    }

    fn execute_program(&mut self, program: ProgramTaskInvocation<'_>) -> TaskResult {
        let persistence = ProgramPersistenceSession::start(
            self.output_directory.clone(),
            self.environment.config_snapshot.bytes(),
            &self.environment.dependencies_json,
            ProgramLaunch {
                executable: program.executable(),
                args: program.args(),
                kind: program.kind(),
                python_script: program.python_script(),
                python_environment_manager: program.python_environment_manager(),
                seed_derivation: self.program_seed.as_ref().map(|request| &request.metadata),
                threads: self.threads,
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
            .env("WORKFLOW_THREADS", self.threads.to_string())
            .env("RAYON_NUM_THREADS", self.threads.to_string())
            .env_remove("WORKFLOW_TASK_SEED")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(processed_directory) = &self.environment.processed_directory {
            command.env("WORKFLOW_NPY_OUTPUT", processed_directory);
        } else {
            command.env_remove("WORKFLOW_NPY_OUTPUT");
        }
        if let Some(seed) = &self.program_seed {
            command.env("WORKFLOW_TASK_SEED", seed.seed.to_string());
        }

        super::super::program::execute(
            command,
            persistence,
            &self.cancellation,
            &self.task_presentation,
            self.environment.processed_directory.is_some(),
        )
    }

    fn begin_member(&mut self, member: MemberInitialization<'_>) -> TaskResult {
        let MemberInitialization {
            index,
            member_count,
            identity,
            seed_derivation,
            plan,
            state,
            target_iteration,
        } = member;
        let provenance = self
            .provenance
            .as_ref()
            .expect("an execution-unit task retains recording provenance")
            .clone()
            .with_member(index, identity)
            .with_seed_derivation(seed_derivation)
            .with_compute(self.resources.compute_provenance());
        if self.persistence.is_empty() {
            self.persistence.resize_with(member_count, || None);
            self.member_iterations.resize(member_count, 0);
            self.member_targets.resize(member_count, None);
            self.member_identities.resize(member_count, None);
            self.member_directories.resize(member_count, None);
        }
        let directory = if member_count == 1 {
            self.output_directory.clone()
        } else {
            self.output_directory
                .join("members")
                .join(format!("member-{index:06}"))
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
        self.persistence[index] = Some(persistence);
        self.publish_progress();
        Ok(())
    }

    fn observe_member_step(
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
            .expect("begin_member precedes step observation")
            .observe(state)?;
        self.member_iterations[index] = state.time().iteration();
        self.member_targets[index] = target_iteration;
        self.publish_progress();
        Ok(())
    }

    fn observe_member_final(
        &mut self,
        index: usize,
        state: &SystemState,
        target_iteration: Option<u64>,
        completion_reason: Option<Map<String, Value>>,
    ) -> TaskResult {
        if self.cancellation_requested() {
            return Ok(());
        }
        self.persistence[index]
            .as_mut()
            .expect("begin_member precedes final observation")
            .complete(
                state,
                completion_reason,
                self.resources.compute_provenance(),
            )?;
        self.persistence[index] = None;
        self.member_iterations[index] = state.time().iteration();
        self.member_targets[index] = target_iteration;
        self.publish_progress();
        Ok(())
    }
}

impl RuntimeTaskHost {
    fn publish_progress(&self) {
        let (iteration, target) = ensemble_progress(&self.member_iterations, &self.member_targets);
        self.task_presentation.progress(iteration, target);
    }
}

fn ensemble_progress(iterations: &[u64], targets: &[Option<u64>]) -> (u64, Option<u64>) {
    let iteration = iterations.iter().copied().max().unwrap_or(0);
    let target = targets
        .iter()
        .copied()
        .collect::<Option<Vec<_>>>()
        .and_then(|targets| targets.into_iter().max());
    (iteration, target)
}

#[cfg(test)]
mod progress_tests {
    use super::ensemble_progress;

    #[test]
    fn ensemble_clock_is_not_multiplied_by_member_count() {
        assert_eq!(
            ensemble_progress(&[100; 12], &[Some(36_000); 12]),
            (100, Some(36_000))
        );
    }

    #[test]
    fn early_finished_members_do_not_hold_back_the_clock() {
        assert_eq!(
            ensemble_progress(&[10, 100], &[Some(10), Some(36_000)]),
            (100, Some(36_000))
        );
    }

    #[test]
    fn unknown_targets_remain_unknown() {
        assert_eq!(
            ensemble_progress(&[10, 100], &[Some(36_000), None]),
            (100, None)
        );
    }
}
