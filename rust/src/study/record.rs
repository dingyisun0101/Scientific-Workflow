//! Always-on, compact study lifecycle recording.

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use serde::Serialize;
use serde_json::Value;

use super::error::StudyError;
use super::phase::{Phase, PhaseId, TaskKey, TaskMode};
use super::renderer::{ProgressSummary, TaskExecutionSnapshot};
use crate::clock::{duration_nanoseconds, utc_now_rfc3339};

const STUDY_RECORD_FORMAT: &str = "scientific-workflow.study-record.v2";

/// Durable lifecycle summary for one study execution.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StudyRecord {
    format: &'static str,
    status: &'static str,
    started_at_utc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    ended_at_utc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_ns: Option<u64>,
    phase_count: usize,
    task_count: usize,
    phases: Vec<PhaseRecord>,
}

/// Lifecycle facts for one selected phase.
///
/// `disposition` distinguishes normal task execution from application-verified
/// whole-phase reuse. Reused task statuses inherit that phase verdict and do
/// not imply task-level examination by Workflow.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PhaseRecord {
    id: u64,
    label: String,
    status: &'static str,
    disposition: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    started_at_utc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ended_at_utc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_ns: Option<u64>,
    progress: TaskCounts,
    tasks: Vec<TaskRecord>,
}

/// Lifecycle facts for one selected task.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TaskRecord {
    id: String,
    category: String,
    label: String,
    mode: &'static str,
    status: &'static str,
    metadata: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    started_at_utc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ended_at_utc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_ns: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    start_offset_ns: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    final_iteration: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_iteration: Option<u64>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
struct TaskCounts {
    total: u64,
    completed: u64,
    failed: u64,
    cancelled: u64,
    skipped: u64,
}

pub(crate) struct StudyRecorder {
    inner: Arc<Mutex<RecorderState>>,
}

struct RecorderState {
    path: PathBuf,
    record: StudyRecord,
    study_started: Instant,
    phase_positions: HashMap<PhaseId, usize>,
    task_positions: HashMap<TaskKey, (usize, usize)>,
    phase_started: HashMap<PhaseId, Instant>,
    finished: bool,
}

pub(crate) struct TaskTimer {
    recorder: Arc<Mutex<RecorderState>>,
    key: TaskKey,
    started: Instant,
}

impl StudyRecorder {
    pub(crate) fn start(path: PathBuf, phases: &[&Phase]) -> Result<Self, StudyError> {
        let started_at_utc = timestamp("start study execution record")?;
        let study_started = Instant::now();
        let mut phase_positions = HashMap::with_capacity(phases.len());
        let mut task_positions = HashMap::new();
        let mut phase_records = Vec::with_capacity(phases.len());
        let mut task_count = 0_usize;
        for (phase_position, phase) in phases.iter().enumerate() {
            phase_positions.insert(phase.id(), phase_position);
            let mut tasks = Vec::with_capacity(phase.tasks().len());
            for (task_position, task) in phase.tasks().iter().enumerate() {
                task_positions.insert(task.key().clone(), (phase_position, task_position));
                tasks.push(TaskRecord {
                    id: task.id().to_string(),
                    category: task.category_name().to_owned(),
                    label: task.label().to_owned(),
                    mode: match task.mode() {
                        TaskMode::Progress => "progress",
                        TaskMode::OneShot => "one-shot",
                    },
                    status: "pending",
                    metadata: Value::Object(
                        task.metadata_iter()
                            .map(|(key, value)| (key.to_owned(), value.clone()))
                            .collect(),
                    ),
                    started_at_utc: None,
                    ended_at_utc: None,
                    duration_ns: None,
                    start_offset_ns: None,
                    final_iteration: None,
                    target_iteration: None,
                });
            }
            task_count += tasks.len();
            phase_records.push(PhaseRecord {
                id: phase.id().get(),
                label: phase.label().to_owned(),
                status: "pending",
                disposition: "pending",
                started_at_utc: None,
                ended_at_utc: None,
                duration_ns: None,
                progress: TaskCounts::default(),
                tasks,
            });
        }
        let recorder = Self {
            inner: Arc::new(Mutex::new(RecorderState {
                path,
                record: StudyRecord {
                    format: STUDY_RECORD_FORMAT,
                    status: "running",
                    started_at_utc,
                    ended_at_utc: None,
                    duration_ns: None,
                    phase_count: phases.len(),
                    task_count,
                    phases: phase_records,
                },
                study_started,
                phase_positions,
                task_positions,
                phase_started: HashMap::with_capacity(phases.len()),
                finished: false,
            })),
        };
        recorder.persist()?;
        Ok(recorder)
    }

    pub(crate) fn phase_started(&self, id: PhaseId) -> Result<(), StudyError> {
        let timestamp = timestamp("record phase start")?;
        let mut state = lock(&self.inner);
        let position = state.phase_positions[&id];
        state.record.phases[position].status = "running";
        state.record.phases[position].disposition = "executed";
        state.record.phases[position].started_at_utc = Some(timestamp);
        state.phase_started.insert(id, Instant::now());
        persist_state(&state)
    }

    /// Records whole-phase reuse without pretending that Workflow examined or
    /// scheduled the phase's individual tasks.
    pub(crate) fn phase_reused(&self, id: PhaseId) -> Result<(), StudyError> {
        let ended_at_utc = timestamp("record reused phase")?;
        let mut state = lock(&self.inner);
        let position = state.phase_positions[&id];
        let phase = &mut state.record.phases[position];
        let total = phase.tasks.len() as u64;
        phase.status = "completed";
        phase.disposition = "reused";
        phase.ended_at_utc = Some(ended_at_utc);
        phase.progress = TaskCounts {
            total,
            completed: total,
            ..TaskCounts::default()
        };
        for task in &mut phase.tasks {
            task.status = "reused";
        }
        persist_state(&state)
    }

    pub(crate) fn task_started(&self, key: &TaskKey) -> Result<TaskTimer, StudyError> {
        let timestamp = timestamp("record task start")?;
        let started = Instant::now();
        let mut state = lock(&self.inner);
        let (phase_position, task_position) = state.task_positions[key];
        let phase_id = key.phase_id();
        let offset = state
            .phase_started
            .get(&phase_id)
            .map(|phase_started| nanoseconds(phase_started.elapsed()));
        let task = &mut state.record.phases[phase_position].tasks[task_position];
        task.status = "running";
        task.started_at_utc = Some(timestamp);
        task.start_offset_ns = offset;
        Ok(TaskTimer {
            recorder: Arc::clone(&self.inner),
            key: key.clone(),
            started,
        })
    }

    pub(crate) fn phase_finished(
        &self,
        id: PhaseId,
        success: bool,
        progress: &ProgressSummary,
        tasks: Vec<TaskExecutionSnapshot>,
    ) -> Result<(), StudyError> {
        let ended_at_utc = timestamp("record phase completion")?;
        let mut state = lock(&self.inner);
        for snapshot in tasks {
            let (phase_position, task_position) = state.task_positions[&snapshot.key];
            let task = &mut state.record.phases[phase_position].tasks[task_position];
            task.status = snapshot.status.as_str();
            task.final_iteration = snapshot.current_iteration;
            task.target_iteration = snapshot.target_iteration;
        }
        let position = state.phase_positions[&id];
        let duration_ns = state
            .phase_started
            .remove(&id)
            .map(|start| nanoseconds(start.elapsed()));
        let phase = &mut state.record.phases[position];
        phase.status = if success { "completed" } else { "failed" };
        phase.ended_at_utc = Some(ended_at_utc);
        phase.duration_ns = duration_ns;
        phase.progress = counts(progress);
        persist_state(&state)
    }

    pub(crate) fn finish(&self, success: bool) -> Result<StudyRecord, StudyError> {
        let ended_at_utc = timestamp("finish study execution record")?;
        let mut state = lock(&self.inner);
        state.record.status = if success { "completed" } else { "failed" };
        state.record.ended_at_utc = Some(ended_at_utc);
        state.record.duration_ns = Some(nanoseconds(state.study_started.elapsed()));
        state.finished = true;
        persist_state(&state)?;
        Ok(state.record.clone())
    }

    fn persist(&self) -> Result<(), StudyError> {
        persist_state(&lock(&self.inner))
    }
}

impl Drop for StudyRecorder {
    fn drop(&mut self) {
        let mut state = lock(&self.inner);
        if state.finished {
            return;
        }
        state.record.status = "failed";
        state.record.ended_at_utc = utc_now_rfc3339().ok();
        state.record.duration_ns = Some(nanoseconds(state.study_started.elapsed()));
        let _ = persist_state(&state);
        state.finished = true;
    }
}

impl Drop for TaskTimer {
    fn drop(&mut self) {
        let ended_at_utc = utc_now_rfc3339().ok();
        let mut state = lock(&self.recorder);
        let (phase_position, task_position) = state.task_positions[&self.key];
        let task = &mut state.record.phases[phase_position].tasks[task_position];
        task.ended_at_utc = ended_at_utc;
        task.duration_ns = Some(nanoseconds(self.started.elapsed()));
    }
}

fn counts(summary: &ProgressSummary) -> TaskCounts {
    TaskCounts {
        total: summary.total(),
        completed: summary.completed(),
        failed: summary.failed(),
        cancelled: summary.cancelled(),
        skipped: summary.skipped(),
    }
}

fn timestamp(operation: &'static str) -> Result<String, StudyError> {
    utc_now_rfc3339().map_err(|source| StudyError::StudyRecordTimestamp { operation, source })
}

fn nanoseconds(duration: std::time::Duration) -> u64 {
    duration_nanoseconds(duration).unwrap_or(u64::MAX)
}

fn persist_state(state: &RecorderState) -> Result<(), StudyError> {
    let path = &state.path;
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|source| StudyError::WriteStudyRecord {
            path: path.clone(),
            source,
        })?;
    }
    let mut bytes = serde_json::to_vec_pretty(&state.record)
        .map_err(|source| StudyError::SerializeStudyRecord { source })?;
    bytes.push(b'\n');
    let temporary = temporary_path(path);
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&temporary)
        .map_err(|source| StudyError::WriteStudyRecord {
            path: temporary.clone(),
            source,
        })?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|source| StudyError::WriteStudyRecord {
            path: temporary.clone(),
            source,
        })?;
    fs::rename(&temporary, path).map_err(|source| StudyError::WriteStudyRecord {
        path: path.clone(),
        source,
    })?;
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|source| StudyError::WriteStudyRecord {
                path: parent.to_path_buf(),
                source,
            })?;
    }
    Ok(())
}

fn temporary_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("study-record.json");
    path.with_file_name(format!(".{name}.tmp"))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
