//! Deterministic, read-only study plan export.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use super::{Phase, StudyError, TaskMode};

const STUDY_PLAN_FORMAT: &str = "scientific-workflow.study-plan.v1";

/// Complete serializable phase/task graph registered with one study.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StudyPlan {
    format: &'static str,
    phases: Vec<StudyPlanPhase>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct StudyPlanPhase {
    id: u64,
    label: String,
    registration_order: usize,
    dependencies: Vec<u64>,
    max_active_tasks: usize,
    prepared_task_queue_capacity: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    delay_per_task_ns: Option<u128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    task_timeout_ns: Option<u128>,
    #[serde(skip_serializing_if = "Option::is_none")]
    deadline_after_ns: Option<u128>,
    failure_policy: &'static str,
    requires_confirmation: bool,
    tasks: Vec<StudyPlanTask>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct StudyPlanTask {
    id: String,
    category: String,
    label: String,
    registration_order: usize,
    mode: &'static str,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    delay_rank: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    release_offset_ns: Option<u128>,
    metadata: Value,
}

impl StudyPlan {
    pub(crate) fn from_phases(phases: &[Phase]) -> Self {
        Self {
            format: STUDY_PLAN_FORMAT,
            phases: phases
                .iter()
                .enumerate()
                .map(|(registration_order, phase)| {
                    let mut executable_rank = 0_usize;
                    let tasks = phase
                        .tasks()
                        .iter()
                        .enumerate()
                        .map(|(task_order, task)| {
                            let delay_rank = (!task.is_completed()).then(|| {
                                let rank = executable_rank;
                                executable_rank += 1;
                                rank
                            });
                            let release_offset_ns = phase.delay_per_task().and_then(|delay| {
                                delay_rank.map(|rank| delay.as_nanos().saturating_mul(rank as u128))
                            });
                            StudyPlanTask {
                                id: task.id().to_string(),
                                category: task.category_name().to_owned(),
                                label: task.label().to_owned(),
                                registration_order: task_order,
                                mode: match task.mode() {
                                    TaskMode::Progress => "progress",
                                    TaskMode::OneShot => "one-shot",
                                },
                                status: if task.is_completed() {
                                    "completed"
                                } else {
                                    "pending"
                                },
                                delay_rank,
                                release_offset_ns,
                                metadata: Value::Object(
                                    task.metadata_iter()
                                        .map(|(key, value)| (key.to_owned(), value.clone()))
                                        .collect(),
                                ),
                            }
                        })
                        .collect();
                    StudyPlanPhase {
                        id: phase.id().get(),
                        label: phase.label().to_owned(),
                        registration_order,
                        dependencies: phase
                            .dependencies()
                            .iter()
                            .map(|dependency| dependency.get())
                            .collect(),
                        max_active_tasks: phase.max_active_tasks(),
                        prepared_task_queue_capacity: phase.prepared_task_queue_capacity(),
                        delay_per_task_ns: phase.delay_per_task().map(|value| value.as_nanos()),
                        task_timeout_ns: phase.task_timeout().map(|value| value.as_nanos()),
                        deadline_after_ns: phase.deadline_after().map(|value| value.as_nanos()),
                        failure_policy: phase.failure_policy().as_str(),
                        requires_confirmation: phase.requires_confirmation(),
                        tasks,
                    }
                })
                .collect(),
        }
    }

    /// Serializes the plan as deterministic pretty JSON with a final newline.
    pub fn to_pretty_json(&self) -> Result<Vec<u8>, StudyError> {
        let mut bytes = serde_json::to_vec_pretty(self)
            .map_err(|source| StudyError::SerializeStudyPlan { source })?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    /// Writes the plan without overwriting different existing content.
    pub fn write_json(&self, path: impl AsRef<Path>) -> Result<(), StudyError> {
        let path = path.as_ref();
        let bytes = self.to_pretty_json()?;
        match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(mut file) => file
                .write_all(&bytes)
                .and_then(|()| file.sync_all())
                .map_err(|source| StudyError::WriteStudyPlan {
                    path: path.to_path_buf(),
                    source,
                }),
            Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
                let existing = fs::read(path).map_err(|source| StudyError::WriteStudyPlan {
                    path: path.to_path_buf(),
                    source,
                })?;
                if existing == bytes {
                    Ok(())
                } else {
                    Err(StudyError::StudyPlanConflict {
                        path: path.to_path_buf(),
                    })
                }
            }
            Err(source) => Err(StudyError::WriteStudyPlan {
                path: path.to_path_buf(),
                source,
            }),
        }
    }
}
