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
use crate::hopf_model::HopfModel;

/// Immutable model, cadence, and storage settings for one swept task.
#[derive(Debug)]
pub(crate) struct TaskSettings {
    pub(crate) task_index: u64,
    pub(crate) total_steps: u64,
    pub(crate) trajectory_every: NonZeroU64,
    pub(crate) radius_every: NonZeroU64,
    pub(crate) checkpoint_every: NonZeroU64,
    pub(crate) maximum_chunk_bytes: NonZeroU64,
    pub(crate) writer_queue_bytes: NonZeroU64,
}

/// Inputs shared by the top-level application orchestrator.
pub(crate) struct ProjectSetup {
    pub(crate) project: ProjectConfig,
    pub(crate) schema: SystemStateSchema,
    pub(crate) recording_root: PathBuf,
}

/// Loads the project, state schema, and named recording path.
pub(crate) fn load_project(project_root: &Path) -> AppResult<ProjectSetup> {
    let project = ProjectConfig::load(project_root)?;
    let schema =
        SystemStateSchema::load_json_template(project.paths().resolve_path("state_template")?)?;
    let recording_root = project.paths().resolve_path("recording_root")?;

    Ok(ProjectSetup {
        project,
        schema,
        recording_root,
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

/// Creates one model and its execution settings from a resolved task dictionary.
pub(crate) fn prepare_task(
    schema: &SystemStateSchema,
    parameters: &TaskParameters,
) -> AppResult<(HopfModel, TaskSettings)> {
    let settings = TaskSettings {
        task_index: parameters.task_index(),
        total_steps: parameters.decode_value("total_steps")?,
        trajectory_every: parameters.decode_value("trajectory_sample_every_steps")?,
        radius_every: parameters.decode_value("radius_sample_every_steps")?,
        checkpoint_every: parameters.decode_value("checkpoint_every_steps")?,
        maximum_chunk_bytes: parameters.decode_value("maximum_chunk_bytes")?,
        writer_queue_bytes: parameters.decode_value("writer_queue_bytes")?,
    };
    let model = HopfModel::new(
        schema,
        parameters.decode_value("initial_point")?,
        parameters.decode_value("mu")?,
        parameters.decode_value("angular_frequency")?,
        parameters.decode_value("physical_time_step")?,
    )?;
    Ok((model, settings))
}
