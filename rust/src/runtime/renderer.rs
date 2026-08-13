//! Runtime-level phase headings and line-oriented lifecycle records.

use std::io::{self, BufRead, Write};

use super::phase::{Phase, PhaseId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RuntimeOutput {
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
    format!(
        "Phase {position} of {total} — [{}] {} · tasks={} · active≤{} · queue={} · failure={} · confirm={} · dependencies={dependencies}",
        phase.id(),
        phase.label(),
        phase.tasks().len(),
        phase.max_concurrent_workloads(),
        phase.queue_capacity(),
        phase.failure_policy().as_str(),
        if phase.requires_confirmation() {
            "yes"
        } else {
            "no"
        },
    )
}

pub(crate) fn phase_start(output: RuntimeOutput, phase: &Phase, position: usize, total: usize) {
    if output == RuntimeOutput::Plain {
        eprintln!(
            "[phase-start] position={position}/{total} phase={} label={} tasks={} active_limit={} queue_capacity={} failure_policy={} require_confirm={}",
            phase.id(),
            phase.label(),
            phase.tasks().len(),
            phase.max_concurrent_workloads(),
            phase.queue_capacity(),
            phase.failure_policy().as_str(),
            phase.requires_confirmation(),
        );
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

pub(crate) fn phase_complete(output: RuntimeOutput, phase: PhaseId, label: &str, success: bool) {
    if output == RuntimeOutput::Plain {
        eprintln!(
            "[phase-complete] phase={} label={} status={}",
            phase,
            label,
            if success { "completed" } else { "failed" },
        );
    }
}

pub(crate) fn runtime_complete(output: RuntimeOutput, phases: usize, tasks: usize, success: bool) {
    if output != RuntimeOutput::Hidden {
        eprintln!(
            "[runtime] status={} phases={} tasks={}",
            if success { "completed" } else { "failed" },
            phases,
            tasks,
        );
    }
}
