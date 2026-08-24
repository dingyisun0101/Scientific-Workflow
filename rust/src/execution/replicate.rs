//! Process isolation and output scopes for complete study replicates.

use std::env;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::thread;
use std::time::Duration;

use crate::configuration::{ReplicateExecutionMode, ReplicateFailurePolicy, ReplicateSettings};
use crate::rng_record::ReplicateSeedDeriver;

use super::{ExecutionScope, ExecutionScopeError};

const REPLICATE_INDEX_ENVIRONMENT_VARIABLE: &str = "SCIENTIFIC_WORKFLOW_REPLICATE_INDEX";
const PARALLEL_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Controller that dispatches the current executable once per declared replicate.
///
/// The initial process becomes the controller. Each child receives one reserved
/// environment variable and re-enters the same API as a replicate worker. The
/// caller runs scientific work only when [`Self::dispatch_current_executable`]
/// returns `Some`.
#[derive(Clone, Debug)]
pub struct ReplicateExecutor {
    settings: ReplicateSettings,
    output_root: PathBuf,
}

impl ReplicateExecutor {
    /// Creates a dispatcher for validated settings and an application-resolved output root.
    pub fn new(settings: ReplicateSettings, output_root: impl Into<PathBuf>) -> Self {
        Self {
            settings,
            output_root: output_root.into(),
        }
    }

    /// Dispatches the current executable or enters one dispatched replicate.
    ///
    /// The controller returns `Ok(None)` after every required child process has
    /// completed successfully. A child returns `Ok(Some(context))` immediately;
    /// the application must perform exactly one study run using that context.
    /// Original command-line arguments and inherited stdio are preserved.
    pub fn dispatch_current_executable(
        &self,
    ) -> Result<Option<ReplicateContext>, ReplicateExecutionError> {
        if let Some(raw_index) = env::var_os(REPLICATE_INDEX_ENVIRONMENT_VARIABLE) {
            return self.enter_worker(raw_index).map(Some);
        }

        let executable = env::current_exe().map_err(ReplicateExecutionError::CurrentExecutable)?;
        let arguments = env::args_os().skip(1).collect::<Vec<_>>();
        self.prepare_output_scopes()?;
        match self.settings.execution() {
            ReplicateExecutionMode::Sequential => {
                self.run_sequential(&executable, &arguments)?;
            }
            ReplicateExecutionMode::Parallel => {
                self.run_parallel(&executable, &arguments)?;
            }
        }
        Ok(None)
    }

    fn enter_worker(
        &self,
        raw_index: OsString,
    ) -> Result<ReplicateContext, ReplicateExecutionError> {
        let display = raw_index.to_string_lossy().into_owned();
        let index =
            display
                .parse::<u64>()
                .map_err(|_| ReplicateExecutionError::InvalidWorkerIndex {
                    variable: REPLICATE_INDEX_ENVIRONMENT_VARIABLE,
                    value: display,
                })?;
        if index >= self.settings.replicates() {
            return Err(ReplicateExecutionError::WorkerIndexOutOfRange {
                index,
                replicates: self.settings.replicates(),
            });
        }
        let directory = self.output_root.join(replicate_directory_name(index));
        let execution_scope = ExecutionScope::open_existing(directory)
            .map_err(|source| ReplicateExecutionError::PrepareOutput { index, source })?;
        Ok(ReplicateContext {
            index,
            count: self.settings.replicates(),
            execution_scope,
            seed_deriver: ReplicateSeedDeriver::new(self.settings.seed(), index),
        })
    }

    fn run_sequential(
        &self,
        executable: &Path,
        arguments: &[OsString],
    ) -> Result<(), ReplicateExecutionError> {
        let mut failures = Vec::new();
        for index in 0..self.settings.replicates() {
            let status = self
                .child_command(executable, arguments, index)
                .status()
                .map_err(|source| ReplicateExecutionError::RunProcess { index, source })?;
            if !status.success() {
                failures.push(index);
                if self.settings.failure_policy() == ReplicateFailurePolicy::FailFast {
                    break;
                }
            }
        }
        finish_batch(failures)
    }

    fn run_parallel(
        &self,
        executable: &Path,
        arguments: &[OsString],
    ) -> Result<(), ReplicateExecutionError> {
        let mut active = Vec::new();
        for index in 0..self.settings.replicates() {
            let child = match self.child_command(executable, arguments, index).spawn() {
                Ok(child) => child,
                Err(source) => {
                    terminate_children(&mut active);
                    return Err(ReplicateExecutionError::RunProcess { index, source });
                }
            };
            active.push(ActiveReplicate { index, child });
        }

        let mut failures = Vec::new();
        while !active.is_empty() {
            let mut position = 0;
            let mut completed_any = false;
            while position < active.len() {
                let status = match active[position].child.try_wait() {
                    Ok(status) => status,
                    Err(source) => {
                        let index = active[position].index;
                        terminate_children(&mut active);
                        return Err(ReplicateExecutionError::RunProcess { index, source });
                    }
                };
                let Some(status) = status else {
                    position += 1;
                    continue;
                };
                completed_any = true;
                let completed = active.swap_remove(position);
                if !status.success() {
                    failures.push(completed.index);
                    if self.settings.failure_policy() == ReplicateFailurePolicy::FailFast {
                        terminate_children(&mut active);
                        failures.sort_unstable();
                        return finish_batch(failures);
                    }
                }
            }
            if !completed_any && !active.is_empty() {
                thread::sleep(PARALLEL_POLL_INTERVAL);
            }
        }
        failures.sort_unstable();
        finish_batch(failures)
    }

    fn prepare_output_scopes(&self) -> Result<(), ReplicateExecutionError> {
        let mut created = Vec::new();
        for index in 0..self.settings.replicates() {
            match ExecutionScope::create_named(&self.output_root, replicate_directory_name(index)) {
                Ok(scope) => created.push(scope),
                Err(source) => {
                    for scope in created.into_iter().rev() {
                        let _ = std::fs::remove_dir(scope.directory());
                    }
                    return Err(ReplicateExecutionError::PrepareOutput { index, source });
                }
            }
        }
        Ok(())
    }

    fn child_command(&self, executable: &Path, arguments: &[OsString], index: u64) -> Command {
        let mut command = Command::new(executable);
        command
            .args(arguments)
            .env(REPLICATE_INDEX_ENVIRONMENT_VARIABLE, index.to_string());
        command
    }
}

/// Immutable identity and output scope for one replicate worker process.
#[derive(Clone, Debug)]
pub struct ReplicateContext {
    index: u64,
    count: u64,
    execution_scope: ExecutionScope,
    seed_deriver: ReplicateSeedDeriver,
}

impl ReplicateContext {
    /// Returns the zero-based replicate index.
    pub const fn index(&self) -> u64 {
        self.index
    }

    /// Returns the total declared replicate count.
    pub const fn count(&self) -> u64 {
        self.count
    }

    /// Returns the existing `replicate_<index>` output scope.
    pub const fn execution_scope(&self) -> &ExecutionScope {
        &self.execution_scope
    }

    /// Returns the replicate output directory.
    pub fn output_directory(&self) -> &Path {
        self.execution_scope.directory()
    }

    /// Returns the lazy, namespace-separated seed deriver for this replicate.
    pub const fn seed_deriver(&self) -> ReplicateSeedDeriver {
        self.seed_deriver
    }
}

/// Failure while dispatching or entering a replicate subprocess.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReplicateExecutionError {
    /// The operating system could not identify the running executable.
    #[error("failed to resolve the current executable for replicate dispatch")]
    CurrentExecutable(#[source] io::Error),

    /// The reserved worker environment variable was not a valid index.
    #[error("environment variable `{variable}` contains invalid replicate index `{value}`")]
    InvalidWorkerIndex {
        /// Reserved environment-variable name.
        variable: &'static str,
        /// Rejected environment value.
        value: String,
    },

    /// A worker index does not belong to the currently loaded settings.
    #[error("replicate worker index {index} is outside declared count {replicates}")]
    WorkerIndexOutOfRange {
        /// Rejected zero-based index.
        index: u64,
        /// Positive count loaded from `study.json`.
        replicates: u64,
    },

    /// A replicate output scope could not be created or reopened.
    #[error("failed to prepare output for replicate {index}")]
    PrepareOutput {
        /// Affected zero-based replicate index.
        index: u64,
        /// Filesystem scope failure.
        #[source]
        source: ExecutionScopeError,
    },

    /// A replicate process could not be started or observed.
    #[error("failed to run subprocess for replicate {index}")]
    RunProcess {
        /// Affected zero-based replicate index.
        index: u64,
        /// Operating-system process failure.
        #[source]
        source: io::Error,
    },

    /// One or more replicate subprocesses returned unsuccessful statuses.
    #[error("replicate subprocesses failed at indices {indices:?}")]
    ReplicatesFailed {
        /// Failed indices in ascending order.
        indices: Vec<u64>,
    },
}

struct ActiveReplicate {
    index: u64,
    child: Child,
}

fn replicate_directory_name(index: u64) -> String {
    format!("replicate_{index}")
}

fn finish_batch(failures: Vec<u64>) -> Result<(), ReplicateExecutionError> {
    if failures.is_empty() {
        Ok(())
    } else {
        Err(ReplicateExecutionError::ReplicatesFailed { indices: failures })
    }
}

fn terminate_children(children: &mut Vec<ActiveReplicate>) {
    for active in children.iter_mut() {
        let _ = active.child.kill();
    }
    for mut active in children.drain(..) {
        let _ = active.child.wait();
    }
}
