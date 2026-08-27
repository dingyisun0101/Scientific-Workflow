//! Private event-reduced dashboard state.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::time::Instant;

use super::event::UiEvent;

const MESSAGE_HISTORY: usize = 100;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
    Skipped,
}

impl TaskStatus {
    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Skipped => "skipped",
        }
    }
}

#[derive(Clone)]
pub(super) struct TaskSnapshot {
    pub(super) replicate: u64,
    pub(super) phase: String,
    pub(super) identity: String,
    pub(super) label: String,
    pub(super) kind: String,
    pub(super) subject: String,
    pub(super) status: TaskStatus,
    pub(super) iteration: u64,
    pub(super) target: Option<u64>,
    pub(super) started: Option<Instant>,
    pub(super) finished: Option<Instant>,
    pub(super) detail: String,
}

#[derive(Clone)]
pub(super) struct DashboardSnapshot {
    pub(super) heading: String,
    pub(super) output: Option<PathBuf>,
    pub(super) replicate_count: u64,
    pub(super) tasks: Vec<TaskSnapshot>,
    pub(super) messages: Vec<String>,
    pub(super) exit_requested: bool,
    pub(super) started: Instant,
}

pub(super) struct DashboardState {
    heading: String,
    output: Option<PathBuf>,
    replicate_count: u64,
    tasks: BTreeMap<(u64, Box<str>), TaskSnapshot>,
    task_order: Vec<(u64, Box<str>)>,
    messages: VecDeque<String>,
    exit_requested: bool,
    started: Instant,
}

impl DashboardState {
    pub(super) fn new() -> Self {
        Self {
            heading: "Scientific Workflow".to_owned(),
            output: None,
            replicate_count: 0,
            tasks: BTreeMap::new(),
            task_order: Vec::new(),
            messages: VecDeque::with_capacity(MESSAGE_HISTORY),
            exit_requested: false,
            started: Instant::now(),
        }
    }

    pub(super) fn apply(&mut self, event: &UiEvent<'_>) {
        match event {
            UiEvent::TaskPlanned {
                replicate,
                phase,
                identity,
                label,
                kind,
                subject,
            } => {
                let key = (*replicate, Box::<str>::from(*identity));
                if !self.tasks.contains_key(&key) {
                    self.task_order.push(key.clone());
                }
                self.tasks.insert(
                    key,
                    TaskSnapshot {
                        replicate: *replicate,
                        phase: (*phase).to_owned(),
                        identity: (*identity).to_owned(),
                        label: (*label).to_owned(),
                        kind: (*kind).to_owned(),
                        subject: (*subject).to_owned(),
                        status: TaskStatus::Pending,
                        iteration: 0,
                        target: None,
                        started: None,
                        finished: None,
                        detail: String::new(),
                    },
                );
                return;
            }
            UiEvent::ExecutionStarted {
                output_directory,
                replicate_count,
                ..
            } => {
                self.output = Some((*output_directory).to_path_buf());
                self.replicate_count = *replicate_count;
                self.heading = "Scientific Workflow".to_owned();
            }
            UiEvent::PhaseStarted {
                replicate, name, ..
            } => self.heading = format!("Replicate {replicate} · Phase {name}"),
            UiEvent::TaskStarted {
                replicate,
                identity,
                ..
            } => {
                if let Some(task) = self.task_mut(*replicate, identity) {
                    task.status = TaskStatus::Running;
                    task.started = Some(Instant::now());
                    task.detail.clear();
                }
            }
            UiEvent::TaskProgress {
                replicate,
                identity,
                iteration,
                target_iteration,
            } => {
                if let Some(task) = self.task_mut(*replicate, identity) {
                    task.iteration = *iteration;
                    task.target = *target_iteration;
                }
                return;
            }
            UiEvent::TaskCompleted {
                replicate,
                identity,
                final_iteration,
                ..
            } => {
                if let Some(task) = self.task_mut(*replicate, identity) {
                    task.status = TaskStatus::Completed;
                    task.finished = Some(Instant::now());
                    if let Some(iteration) = final_iteration {
                        task.iteration = *iteration;
                    }
                }
            }
            UiEvent::TaskFailed {
                replicate,
                identity,
                reason,
            } => {
                let exit_requested = self.exit_requested;
                if let Some(task) = self.task_mut(*replicate, identity) {
                    task.status = if exit_requested {
                        TaskStatus::Cancelled
                    } else {
                        TaskStatus::Failed
                    };
                    task.finished = Some(Instant::now());
                    task.detail = (*reason).to_owned();
                }
            }
            UiEvent::TaskCancelled {
                replicate,
                identity,
            } => {
                if let Some(task) = self.task_mut(*replicate, identity) {
                    task.status = TaskStatus::Cancelled;
                    task.finished = Some(Instant::now());
                    task.detail = "cancelled by user".to_owned();
                }
            }
            UiEvent::ExecutionCancelled => {
                self.exit_requested = true;
                for task in self.tasks.values_mut() {
                    match task.status {
                        TaskStatus::Pending => task.status = TaskStatus::Skipped,
                        TaskStatus::Running => task.status = TaskStatus::Cancelled,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        if let Some(message) = event_message(event) {
            self.push_message(message);
        }
    }

    pub(super) fn request_exit(&mut self) {
        if !self.exit_requested {
            self.exit_requested = true;
            self.push_message("workflow: exit requested; waiting for active tasks".to_owned());
        }
    }

    pub(super) fn push_message(&mut self, message: String) {
        if self.messages.len() == MESSAGE_HISTORY {
            self.messages.pop_front();
        }
        self.messages.push_back(message);
    }

    pub(super) fn snapshot(&self) -> DashboardSnapshot {
        DashboardSnapshot {
            heading: self.heading.clone(),
            output: self.output.clone(),
            replicate_count: self.replicate_count,
            tasks: self
                .task_order
                .iter()
                .filter_map(|key| self.tasks.get(key).cloned())
                .collect(),
            messages: self.messages.iter().cloned().collect(),
            exit_requested: self.exit_requested,
            started: self.started,
        }
    }

    fn task_mut(&mut self, replicate: u64, identity: &str) -> Option<&mut TaskSnapshot> {
        self.tasks.get_mut(&(replicate, identity.into()))
    }
}

pub(super) fn event_message(event: &UiEvent<'_>) -> Option<String> {
    match event {
        UiEvent::TaskPlanned { .. } | UiEvent::TaskProgress { .. } => None,
        UiEvent::ExecutionStarted {
            output_directory,
            replicate_count,
            task_count_per_replicate,
        } => Some(format!(
            "workflow: started {replicate_count} replicate(s), {task_count_per_replicate} task(s) each → {}",
            output_directory.display()
        )),
        UiEvent::ExecutionCompleted { output_directory } => Some(format!(
            "workflow: completed → {}",
            output_directory.display()
        )),
        UiEvent::ExecutionFailed { reason } => Some(format!("workflow: failed: {reason}")),
        UiEvent::ExecutionCancelled => Some("workflow: cancelled".to_owned()),
        UiEvent::ReplicateStarted { index } => Some(format!("replicate {index}: started")),
        UiEvent::ReplicateCompleted { index } => Some(format!("replicate {index}: completed")),
        UiEvent::ReplicateFailed { index, reason } => {
            Some(format!("replicate {index}: failed: {reason}"))
        }
        UiEvent::ReplicateCancelled { index } => Some(format!("replicate {index}: cancelled")),
        UiEvent::PhaseStarted {
            replicate,
            name,
            task_count,
        } => Some(format!(
            "replicate {replicate}: phase {name} started ({task_count} task(s))"
        )),
        UiEvent::PhaseCompleted { replicate, name } => {
            Some(format!("replicate {replicate}: phase {name} completed"))
        }
        UiEvent::PhaseFailed {
            replicate,
            name,
            reason,
        } => Some(format!(
            "replicate {replicate}: phase {name} failed: {reason}"
        )),
        UiEvent::PhaseCancelled { replicate, name } => {
            Some(format!("replicate {replicate}: phase {name} cancelled"))
        }
        UiEvent::TaskStarted {
            replicate,
            phase,
            identity,
            label,
            kind,
            subject,
        } => Some(format!(
            "replicate {replicate}: {identity} started {label} ({kind} {subject}, phase {phase})"
        )),
        UiEvent::TaskCompleted {
            replicate,
            identity,
            final_iteration,
            output_directory,
        } => Some(match final_iteration {
            Some(iteration) => format!(
                "replicate {replicate}: {identity} completed at iteration {iteration} → {}",
                output_directory.display()
            ),
            None => format!(
                "replicate {replicate}: {identity} completed → {}",
                output_directory.display()
            ),
        }),
        UiEvent::TaskFailed {
            replicate,
            identity,
            reason,
        } => Some(format!(
            "replicate {replicate}: {identity} failed: {reason}"
        )),
        UiEvent::TaskCancelled {
            replicate,
            identity,
        } => Some(format!("replicate {replicate}: {identity} cancelled")),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn runtime_events_drive_rows_and_bounded_dashboard_messages() {
        let mut state = DashboardState::new();
        state.apply(&UiEvent::TaskPlanned {
            replicate: 0,
            phase: "simulate",
            identity: "simulate/000000/model-000000",
            label: "model #0",
            kind: "model",
            subject: "model",
        });
        state.apply(&UiEvent::ExecutionStarted {
            output_directory: Path::new("output/execution-0"),
            replicate_count: 1,
            task_count_per_replicate: 1,
        });
        state.apply(&UiEvent::TaskStarted {
            replicate: 0,
            phase: "simulate",
            identity: "simulate/000000/model-000000",
            label: "model #0",
            kind: "model",
            subject: "model",
        });
        state.apply(&UiEvent::TaskProgress {
            replicate: 0,
            identity: "simulate/000000/model-000000",
            iteration: 25,
            target_iteration: Some(100),
        });

        let snapshot = state.snapshot();
        assert_eq!(snapshot.tasks.len(), 1);
        assert_eq!(snapshot.tasks[0].status, TaskStatus::Running);
        assert_eq!(snapshot.tasks[0].iteration, 25);
        assert_eq!(snapshot.tasks[0].target, Some(100));
        assert!(
            snapshot
                .messages
                .iter()
                .any(|line| line.contains("started"))
        );

        for index in 0..120 {
            state.push_message(format!("message {index}"));
        }
        let snapshot = state.snapshot();
        assert_eq!(snapshot.messages.len(), MESSAGE_HISTORY);
        assert_eq!(snapshot.messages.first().unwrap(), "message 20");
    }

    #[test]
    fn task_rows_retain_planned_order_instead_of_sorting_identities() {
        let mut state = DashboardState::new();
        for identity in ["z-first", "a-second"] {
            state.apply(&UiEvent::TaskPlanned {
                replicate: 0,
                phase: "phase",
                identity,
                label: identity,
                kind: "program",
                subject: identity,
            });
        }

        let snapshot = state.snapshot();
        assert_eq!(snapshot.tasks[0].identity, "z-first");
        assert_eq!(snapshot.tasks[1].identity, "a-second");
    }

    #[test]
    fn exit_marks_unstarted_and_active_work_without_fabricating_failure() {
        let mut state = DashboardState::new();
        for identity in ["active", "pending"] {
            state.apply(&UiEvent::TaskPlanned {
                replicate: 0,
                phase: "simulate",
                identity,
                label: identity,
                kind: "model",
                subject: "model",
            });
        }
        state.apply(&UiEvent::TaskStarted {
            replicate: 0,
            phase: "simulate",
            identity: "active",
            label: "active",
            kind: "model",
            subject: "model",
        });
        state.request_exit();
        state.apply(&UiEvent::TaskCancelled {
            replicate: 0,
            identity: "active",
        });
        state.apply(&UiEvent::ExecutionCancelled);

        let snapshot = state.snapshot();
        assert_eq!(snapshot.tasks[0].status, TaskStatus::Cancelled);
        assert_eq!(snapshot.tasks[1].status, TaskStatus::Skipped);
        assert!(snapshot.exit_requested);
        assert!(
            snapshot
                .messages
                .iter()
                .any(|line| line.contains("exit requested"))
        );
    }
}
