//! Reconstructs completed streams and summarizes the recorded dynamics.

use scientific_workflow::prelude::*;

use crate::AppResult;
use crate::hopf_model::{HopfModel, POINT_FIELD, RADIUS_FIELD};
use crate::state_recording::{CHECKPOINT_STREAM, RADIUS_STREAM, TRAJECTORY_STREAM};

const PLOT_WIDTH: usize = 41;
const PLOT_HEIGHT: usize = 17;

/// Numbers of reconstructed records in the three logical streams.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct SampleCounts {
    pub(crate) trajectory: u64,
    pub(crate) radius: u64,
    pub(crate) checkpoint: u64,
}

/// User-facing metrics calculated from the reconstructed series.
pub(crate) struct AnalysisSummary {
    pub(crate) samples: SampleCounts,
    pub(crate) x_bounds: [f64; 2],
    pub(crate) y_bounds: [f64; 2],
    pub(crate) radius_bounds: [f64; 2],
    pub(crate) final_radius: f64,
    pub(crate) expected_attractor_radius: f64,
    pub(crate) phase_portrait: String,
}

/// Reads all streams, verifies their final state, and computes plot metrics.
pub(crate) fn analyze_recording(
    model: &HopfModel,
    recording: &CompletedRecording,
) -> AppResult<AnalysisSummary> {
    let decoders = JsonPayloadDecoderRegistry::new()
        .with_json_field::<Vec<f64>>(POINT_FIELD)?
        .with_json_field::<f64>(RADIUS_FIELD)?;

    let reader =
        StoredStateSeriesReader::open_completed_recording(recording.directory(), decoders)?;
    let trajectory = reader.read_stream_as_state_series(TRAJECTORY_STREAM)?;
    let radius = reader.read_stream_as_state_series(RADIUS_STREAM)?;
    let checkpoint = reader.read_latest_state_from_stream(CHECKPOINT_STREAM)?;

    let samples = SampleCounts {
        trajectory: trajectory.len() as u64,
        radius: radius.len() as u64,
        checkpoint: reader.stream_record_count(CHECKPOINT_STREAM)?,
    };
    let points = trajectory
        .iter()
        .map(|state| {
            let point = state.payload::<Vec<f64>>(POINT_FIELD)?;
            Ok([point[0], point[1]])
        })
        .collect::<Result<Vec<_>, StateError>>()?;
    let radii = radius
        .iter()
        .map(|state| state.payload::<f64>(RADIUS_FIELD).copied())
        .collect::<Result<Vec<_>, StateError>>()?;

    verify_final_state(model.state(), &trajectory, &radius, &checkpoint)?;
    let (x_bounds, y_bounds) = point_bounds(&points);
    let radius_bounds = scalar_bounds(&radii);

    Ok(AnalysisSummary {
        samples,
        x_bounds,
        y_bounds,
        radius_bounds,
        final_radius: *radii.last().expect("the radius stream contains samples"),
        expected_attractor_radius: model.mu().max(0.0).sqrt(),
        phase_portrait: render_phase_portrait(&points, x_bounds, y_bounds),
    })
}

/// Confirms that every applicable final record equals the model's actual state.
fn verify_final_state(
    live: &SystemState,
    trajectory: &StateSeries,
    radius: &StateSeries,
    checkpoint_final: &SystemState,
) -> Result<(), StateError> {
    let trajectory_final = trajectory
        .last_state()
        .expect("the trajectory stream contains samples");
    let radius_final = radius
        .last_state()
        .expect("the radius stream contains samples");
    assert_eq!(trajectory_final.simulation_time(), live.simulation_time());
    assert_eq!(radius_final.simulation_time(), live.simulation_time());
    assert_eq!(checkpoint_final.simulation_time(), live.simulation_time());
    assert_eq!(
        trajectory_final.payload::<Vec<f64>>(POINT_FIELD)?,
        live.payload::<Vec<f64>>(POINT_FIELD)?
    );
    assert_eq!(
        checkpoint_final.payload::<Vec<f64>>(POINT_FIELD)?,
        live.payload::<Vec<f64>>(POINT_FIELD)?
    );
    assert_eq!(
        radius_final.payload::<f64>(RADIUS_FIELD)?,
        live.payload::<f64>(RADIUS_FIELD)?
    );
    assert_eq!(
        checkpoint_final.payload::<f64>(RADIUS_FIELD)?,
        live.payload::<f64>(RADIUS_FIELD)?
    );
    Ok(())
}

/// Computes coordinate bounds for the nonempty trajectory.
fn point_bounds(points: &[[f64; 2]]) -> ([f64; 2], [f64; 2]) {
    let first = points[0];
    points[1..].iter().fold(
        ([first[0], first[0]], [first[1], first[1]]),
        |(mut x, mut y), point| {
            x = [x[0].min(point[0]), x[1].max(point[0])];
            y = [y[0].min(point[1]), y[1].max(point[1])];
            (x, y)
        },
    )
}

/// Computes the minimum and maximum radius.
fn scalar_bounds(values: &[f64]) -> [f64; 2] {
    let first = values[0];
    values[1..].iter().fold([first, first], |bounds, value| {
        [bounds[0].min(*value), bounds[1].max(*value)]
    })
}

/// Renders the sampled phase trajectory into a fixed ASCII grid.
fn render_phase_portrait(points: &[[f64; 2]], x: [f64; 2], y: [f64; 2]) -> String {
    let mut cells = vec![vec![' '; PLOT_WIDTH]; PLOT_HEIGHT];
    draw_axes(&mut cells, x, y);
    for point in points {
        let (row, column) = plot_position(*point, x, y);
        cells[row][column] = '*';
    }
    let (start_row, start_column) = plot_position(points[0], x, y);
    cells[start_row][start_column] = 'S';
    let (end_row, end_column) = plot_position(points[points.len() - 1], x, y);
    cells[end_row][end_column] = 'E';

    cells
        .into_iter()
        .map(|row| row.into_iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Draws zero axes when the observed bounds include zero.
fn draw_axes(cells: &mut [Vec<char>], x: [f64; 2], y: [f64; 2]) {
    let zero_column = (x[0] <= 0.0 && x[1] >= 0.0).then(|| scale(0.0, x, PLOT_WIDTH));
    let zero_row =
        (y[0] <= 0.0 && y[1] >= 0.0).then(|| PLOT_HEIGHT - 1 - scale(0.0, y, PLOT_HEIGHT));
    if let Some(column) = zero_column {
        for row in cells.iter_mut() {
            row[column] = '|';
        }
    }
    if let Some(row) = zero_row {
        cells[row].fill('-');
    }
    if let (Some(column), Some(row)) = (zero_column, zero_row) {
        cells[row][column] = '+';
    }
}

/// Maps a point onto the terminal grid.
fn plot_position(point: [f64; 2], x: [f64; 2], y: [f64; 2]) -> (usize, usize) {
    (
        PLOT_HEIGHT - 1 - scale(point[1], y, PLOT_HEIGHT),
        scale(point[0], x, PLOT_WIDTH),
    )
}

/// Scales one value into an integer cell coordinate.
fn scale(value: f64, bounds: [f64; 2], cells: usize) -> usize {
    let span = bounds[1] - bounds[0];
    if span == 0.0 {
        return cells / 2;
    }
    (((value - bounds[0]) / span).clamp(0.0, 1.0) * (cells - 1) as f64).round() as usize
}
