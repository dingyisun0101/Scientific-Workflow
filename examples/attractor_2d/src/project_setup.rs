//! Loads the example project and prepares its parameter-sweep tasks.
//!
//! The checked-in JSON files are part of this demonstration, so this module
//! shows the normal typed configuration path without reproducing a production
//! application's domain-validation framework.

use std::fs;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

use scientific_workflow::prelude::*;

use crate::AppResult;

/// Immutable model, cadence, and storage settings for one swept task.
#[derive(Debug)]
pub(crate) struct TaskSettings {
    pub(crate) task_index: u64,
    pub(crate) model_name: String,
    pub(crate) mu: f64,
    pub(crate) omega: f64,
    pub(crate) time_step: f64,
    pub(crate) total_steps: u64,
    pub(crate) trajectory_every: NonZeroU64,
    pub(crate) radius_every: NonZeroU64,
    pub(crate) checkpoint_every: NonZeroU64,
    pub(crate) maximum_chunk_bytes: NonZeroU64,
    pub(crate) writer_queue_bytes: NonZeroU64,
}

/// One resolved parameter dictionary and the initial point it will consume.
pub(crate) struct TaskPlan {
    pub(crate) parameters: TaskParameters,
    pub(crate) settings: TaskSettings,
    pub(crate) initial_point: Vec<f64>,
}

/// Inputs shared by the top-level application orchestrator.
pub(crate) struct ProjectPlan {
    pub(crate) schema: SystemStateSchema,
    pub(crate) tasks: Vec<TaskPlan>,
    pub(crate) configuration_directory: PathBuf,
    pub(crate) recording_root: PathBuf,
}

/// Loads the project, state schema, named output path, and resolved tasks.
pub(crate) fn load_project(project_root: &Path) -> AppResult<ProjectPlan> {
    let project = ProjectConfig::load(project_root)?;
    let schema =
        SystemStateSchema::load_json_template(project.paths().resolve_path("state_template")?)?;
    let tasks = project
        .parameters()
        .tasks()
        .map(decode_task_plan)
        .collect::<AppResult<Vec<_>>>()?;

    Ok(ProjectPlan {
        schema,
        tasks,
        configuration_directory: project.configuration_directory().to_path_buf(),
        recording_root: project.paths().resolve_path("recording_root")?,
    })
}

/// Creates a fresh timestamped directory for one application execution.
pub(crate) fn create_execution_directory(recording_root: &Path) -> AppResult<PathBuf> {
    fs::create_dir_all(recording_root)?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the example requires a system clock after the Unix epoch")
        .as_nanos();
    let directory = recording_root.join(format!("run_{timestamp}_{}", process::id()));
    fs::create_dir(&directory)?;
    Ok(directory)
}

/// Decodes one fixed-plus-sweep parameter view into a task plan.
fn decode_task_plan(parameters: TaskParameters) -> AppResult<TaskPlan> {
    let settings = TaskSettings {
        task_index: parameters.task_index(),
        model_name: parameters.decode_value("model_name")?,
        mu: parameters.decode_value("mu")?,
        omega: parameters.decode_value("angular_frequency")?,
        time_step: parameters.decode_value("physical_time_step")?,
        total_steps: parameters.decode_value("total_steps")?,
        trajectory_every: decode_nonzero(&parameters, "trajectory_sample_every_steps")?,
        radius_every: decode_nonzero(&parameters, "radius_sample_every_steps")?,
        checkpoint_every: decode_nonzero(&parameters, "checkpoint_every_steps")?,
        maximum_chunk_bytes: decode_nonzero(&parameters, "maximum_chunk_bytes")?,
        writer_queue_bytes: decode_nonzero(&parameters, "writer_queue_bytes")?,
    };
    let initial_point = parameters.decode_value("initial_point")?;

    Ok(TaskPlan {
        parameters,
        settings,
        initial_point,
    })
}

/// Decodes a checked-in nonzero cadence or byte limit required by the writer.
fn decode_nonzero(parameters: &TaskParameters, key: &str) -> AppResult<NonZeroU64> {
    let value = parameters.decode_value(key)?;
    Ok(NonZeroU64::new(value).expect("checked-in writer settings are nonzero"))
}
