//! Study-level phase headings and line-oriented lifecycle records.
//!
//! # Boundary
//!
//! This module owns only rendering text and input handoff for one study execution.
//! It does not choose scheduling policy, drive workloads, persist data, or track
//! application metadata.

use std::io::{self, BufRead, Write};

use super::phase::{Phase, PhaseId};

const WITHIN_PHASE_WARNING: &str = "Workflow will invoke the phase normally; validation, reuse, cleanup, and continuation within this phase are application-owned";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DisplayMode {
    Auto,
    Terminal,
    Plain,
    Hidden,
}

pub(crate) fn phase_heading(phase: &Phase, position: usize, total: usize) -> String {
    let dependencies = if phase.dependencies().is_empty() {
        "none".to_owned()
    } else {
        phase
            .dependencies()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",")
    };
    let timing = phase_timing(phase, TimingOutputStyle::Heading);
    format!(
        "Phase {position} of {total} — [{}] {} · tasks={} · active≤{} · queue={}{timing} · failure={} · confirm={} · dependencies={dependencies}",
        phase.id(),
        phase.label(),
        phase.tasks().len(),
        phase.max_active_tasks(),
        phase.prepared_task_queue_capacity(),
        phase.failure_policy().as_str(),
        if phase.requires_confirmation() {
            "yes"
        } else {
            "no"
        },
    )
}

pub(crate) fn phase_start(output: DisplayMode, phase: &Phase, position: usize, total: usize) {
    if output == DisplayMode::Plain {
        let timing = phase_timing(phase, TimingOutputStyle::Plain);
        eprintln!(
            "[phase-start] position={position}/{total} phase={} label={} tasks={} max_active_tasks={} prepared_task_queue_capacity={}{timing} failure_policy={} require_confirm={}",
            phase.id(),
            phase.label(),
            phase.tasks().len(),
            phase.max_active_tasks(),
            phase.prepared_task_queue_capacity(),
            phase.failure_policy().as_str(),
            phase.requires_confirmation(),
        );
    }
}

pub(crate) fn completion_examination_disabled(output: DisplayMode) {
    if output != DisplayMode::Hidden {
        eprintln!(
            "[completion] mode=disabled action=execute-selected warning={WITHIN_PHASE_WARNING:?}"
        );
    }
}

pub(crate) fn phase_incomplete(output: DisplayMode, phase: PhaseId, label: &str, detail: &str) {
    if output != DisplayMode::Hidden {
        eprintln!("{}", phase_incomplete_message(phase, label, detail));
    }
}

fn phase_incomplete_message(phase: PhaseId, label: &str, detail: &str) -> String {
    format!(
        "[phase-completion] phase={phase} label={label} status=incomplete action=execute detail={detail:?} warning={WITHIN_PHASE_WARNING:?}"
    )
}

pub(crate) fn phase_reused(output: DisplayMode, phase: PhaseId, label: &str) {
    if output != DisplayMode::Hidden {
        eprintln!("[phase-completion] phase={phase} label={label} status=complete action=reuse");
    }
}

enum TimingOutputStyle {
    Heading,
    Plain,
}

fn phase_timing(phase: &Phase, style: TimingOutputStyle) -> String {
    let delay = phase.delay_per_task();
    let task_timeout = phase.task_timeout();
    let deadline = phase.deadline_after();
    if delay.is_none() && task_timeout.is_none() && deadline.is_none() {
        String::new()
    } else {
        match style {
            TimingOutputStyle::Heading => {
                format!(
                    " · delay={:?} · task-timeout={:?} · deadline={:?}",
                    delay, task_timeout, deadline,
                )
            }
            TimingOutputStyle::Plain => format!(
                " delay_per_task={:?} task_timeout={:?} deadline_after={:?}",
                delay, task_timeout, deadline,
            ),
        }
    }
}

pub(crate) fn confirm_transition(current: PhaseId, next: &Phase) -> io::Result<bool> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let stderr = io::stderr();
    let mut output = stderr.lock();
    let mut answer = String::new();

    loop {
        write!(
            output,
            "[phase-confirm] phase={current} next_phase={} label={} — type yes to continue: ",
            next.id(),
            next.label(),
        )?;
        output.flush()?;
        answer.clear();
        if input.read_line(&mut answer)? == 0 {
            writeln!(output)?;
            return Ok(false);
        }
        if answer.trim().eq_ignore_ascii_case("yes") {
            return Ok(true);
        }
        writeln!(output, "[phase-confirm] confirmation not accepted")?;
    }
}

pub(crate) fn phase_complete(output: DisplayMode, phase: PhaseId, label: &str, success: bool) {
    if output == DisplayMode::Plain {
        eprintln!(
            "[phase-complete] phase={} label={} status={}",
            phase,
            label,
            if success { "completed" } else { "failed" },
        );
    }
}

pub(crate) fn study_complete(
    output: DisplayMode,
    phases: usize,
    reused_phases: usize,
    tasks: usize,
    success: bool,
) {
    if output != DisplayMode::Hidden {
        eprintln!(
            "[study] status={} phases={} reused={} tasks={}",
            if success { "completed" } else { "failed" },
            phases,
            reused_phases,
            tasks,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incomplete_phase_warning_states_the_application_ownership_boundary() {
        let message =
            phase_incomplete_message(PhaseId::new(20), "model dynamics", "checkpoint available");

        assert!(message.contains("status=incomplete action=execute"));
        assert!(message.contains("checkpoint available"));
        assert!(message.contains("continuation within this phase are application-owned"));
    }
}
