//! Runtime facts accepted by the automatic UI.

use std::path::Path;

/// One borrowed lifecycle or progress fact published synchronously by Runtime.
#[derive(Debug)]
pub(crate) enum UiEvent<'a> {
    ExecutionStarted {
        output_directory: &'a Path,
        replicate_count: u64,
        task_count_per_replicate: usize,
    },
    ExecutionCompleted {
        output_directory: &'a Path,
    },
    ExecutionFailed {
        reason: &'a str,
    },
    ReplicateStarted {
        index: u64,
    },
    ReplicateCompleted {
        index: u64,
    },
    ReplicateFailed {
        index: u64,
        reason: &'a str,
    },
    PhaseStarted {
        replicate: u64,
        name: &'a str,
        task_count: usize,
    },
    PhaseCompleted {
        replicate: u64,
        name: &'a str,
    },
    PhaseFailed {
        replicate: u64,
        name: &'a str,
        reason: &'a str,
    },
    TaskStarted {
        replicate: u64,
        phase: &'a str,
        identity: &'a str,
        label: &'a str,
        kind: &'a str,
        subject: &'a str,
    },
    TaskProgress {
        replicate: u64,
        identity: &'a str,
        iteration: u64,
        target_iteration: Option<u64>,
    },
    TaskCompleted {
        replicate: u64,
        identity: &'a str,
        final_iteration: Option<u64>,
        output_directory: &'a Path,
    },
    TaskFailed {
        replicate: u64,
        identity: &'a str,
        reason: &'a str,
    },
}
