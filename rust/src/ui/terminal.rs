//! Best-effort plain terminal rendering.

use std::fmt::Write as _;
use std::io::{Write as _, stderr};

use super::event::UiEvent;

pub(crate) fn render(event: &UiEvent<'_>) {
    let line = format_event(event);
    let mut terminal = stderr().lock();
    let _ = writeln!(terminal, "{line}");
}

fn format_event(event: &UiEvent<'_>) -> String {
    match event {
        UiEvent::ExecutionStarted {
            output_directory,
            replicate_count,
            task_count_per_replicate,
        } => format!(
            "[workflow] started: {replicate_count} replicate(s), {task_count_per_replicate} task(s) each -> {}",
            output_directory.display()
        ),
        UiEvent::ExecutionCompleted { output_directory } => {
            format!("[workflow] completed -> {}", output_directory.display())
        }
        UiEvent::ExecutionFailed { reason } => format!("[workflow] failed: {reason}"),
        UiEvent::ReplicateStarted { index } => {
            format!("[replicate {index}] started")
        }
        UiEvent::ReplicateCompleted { index } => {
            format!("[replicate {index}] completed")
        }
        UiEvent::ReplicateFailed { index, reason } => {
            format!("[replicate {index}] failed: {reason}")
        }
        UiEvent::PhaseStarted {
            replicate,
            name,
            task_count,
        } => format!("[replicate {replicate}] phase {name}: started ({task_count} task(s))"),
        UiEvent::PhaseCompleted { replicate, name } => {
            format!("[replicate {replicate}] phase {name}: completed")
        }
        UiEvent::PhaseFailed {
            replicate,
            name,
            reason,
        } => format!("[replicate {replicate}] phase {name}: failed: {reason}"),
        UiEvent::TaskStarted {
            replicate,
            phase,
            identity,
            label,
            kind,
            subject,
        } => format!(
            "[replicate {replicate}] task {identity}: started {label} ({kind} {subject}, phase {phase})"
        ),
        UiEvent::TaskProgress {
            replicate,
            identity,
            iteration,
            target_iteration,
        } => {
            let mut line =
                format!("[replicate {replicate}] task {identity}: iteration {iteration}");
            if let Some(target) = target_iteration {
                let percent = if *target == 0 {
                    100.0
                } else {
                    (*iteration as f64 / *target as f64) * 100.0
                };
                let _ = write!(line, "/{target} ({percent:.1}%)");
            }
            line
        }
        UiEvent::TaskCompleted {
            replicate,
            identity,
            final_iteration,
            output_directory,
        } => match final_iteration {
            Some(iteration) => format!(
                "[replicate {replicate}] task {identity}: completed at iteration {iteration} -> {}",
                output_directory.display()
            ),
            None => format!(
                "[replicate {replicate}] task {identity}: completed -> {}",
                output_directory.display()
            ),
        },
        UiEvent::TaskFailed {
            replicate,
            identity,
            reason,
        } => format!("[replicate {replicate}] task {identity}: failed: {reason}"),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{UiEvent, format_event};

    #[test]
    fn terminal_lines_are_derived_only_from_runtime_facts() {
        assert_eq!(
            format_event(&UiEvent::TaskProgress {
                replicate: 2,
                identity: "simulate/000003/model-000000",
                iteration: 25,
                target_iteration: Some(100),
            }),
            "[replicate 2] task simulate/000003/model-000000: iteration 25/100 (25.0%)"
        );
        assert_eq!(
            format_event(&UiEvent::TaskCompleted {
                replicate: 2,
                identity: "simulate/000003/model-000000",
                final_iteration: Some(100),
                output_directory: Path::new("output/task-000003"),
            }),
            "[replicate 2] task simulate/000003/model-000000: completed at iteration 100 -> output/task-000003"
        );
    }
}
