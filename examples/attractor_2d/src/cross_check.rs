use crate::{
    hopf_model::{POINT_FIELD, RADIUS_FIELD},
    task_execution::TaskExecutionSummary,
    AppResult,
};
use scientific_workflow::prelude::basics::SystemState;

const CROSS_CHECK_TOLERANCE: f64 = 1e-12;

/// One-file standalone reference implementation used as the example's
/// obligatory numerical reference check.
pub(crate) fn assert_matches_reference(
    live_state: &SystemState,
    initial_point: [f64; 2],
    mu: f64,
    angular_frequency: f64,
    physical_time_increment_per_step: f64,
    step_count: u64,
) -> AppResult<()> {
    let expected_state = reference_state_at_step_count(
        initial_point,
        mu,
        angular_frequency,
        physical_time_increment_per_step,
        step_count,
    );

    let live_point = live_state.payload::<Vec<f64>>(POINT_FIELD)?;
    if live_point.len() != expected_state.2.len() {
        return Err(format!(
            "cross-check point-shape mismatch: expected {} values, got {}",
            expected_state.2.len(),
            live_point.len()
        )
        .into());
    }

    let live_time = live_state.simulation_time();
    if live_time.iteration() != expected_state.0 {
        return Err(
            format!(
                "cross-check iteration mismatch: expected {}, got {}",
                expected_state.0,
                live_time.iteration()
            )
            .into(),
        );
    }

    let live_physical_time = live_time
        .physical_time()
        .ok_or_else(|| "cross-check requires physical_time on simulation state")?;

    if (live_physical_time - expected_state.1).abs() > CROSS_CHECK_TOLERANCE {
        return Err(
            format!(
                "cross-check physical-time mismatch: expected {}, got {}",
                expected_state.1,
                live_physical_time
            )
            .into(),
        );
    }

    if (live_point[0] - expected_state.2[0]).abs() > CROSS_CHECK_TOLERANCE
        || (live_point[1] - expected_state.2[1]).abs() > CROSS_CHECK_TOLERANCE
    {
        return Err(format!(
            "cross-check point mismatch: expected {:?}, got {:?}",
            expected_state.2, live_point
        )
        .into());
    }

    let live_radius = live_state.payload::<f64>(RADIUS_FIELD)?;
    if (live_radius - expected_state.3).abs() > CROSS_CHECK_TOLERANCE {
        return Err(
            format!(
                "cross-check radius mismatch: expected {}, got {}",
                expected_state.3,
                live_radius
            )
            .into(),
        );
    }

    Ok(())
}

/// Prints the deterministic final example report for completed tasks.
pub(crate) fn print_example_report(task_summaries: &[TaskExecutionSummary]) {
    let mut summaries = task_summaries.iter().collect::<Vec<_>>();
    summaries.sort_by_key(|summary| summary.task_ordinal);

    for summary in &summaries {
        println!(
            "task {} validation result: passed (recording {})",
            summary.task_ordinal,
            summary.recording_directory.display()
        );
    }

    for summary in &summaries {
        println!("task {} cross-check result: passed", summary.task_ordinal);
    }
}

fn reference_state_at_step_count(
    initial_point: [f64; 2],
    mu: f64,
    angular_frequency: f64,
    physical_time_increment_per_step: f64,
    step_count: u64,
) -> (u64, f64, [f64; 2], f64) {
    let mut point = initial_point;
    let mut physical_time = 0.0;

    for _ in 0..step_count {
        let [x, y] = point;
        let radius_squared = x * x + y * y;
        let dx = mu * x - angular_frequency * y - radius_squared * x;
        let dy = angular_frequency * x + mu * y - radius_squared * y;
        point = [x + physical_time_increment_per_step * dx, y + physical_time_increment_per_step * dy];
        physical_time += physical_time_increment_per_step;
    }

    let radius = point[0].hypot(point[1]);
    (step_count, physical_time, point, radius)
}
