use std::thread;
use std::time::Duration;

use scientific_workflow::prelude::basic::*;

use crate::AppResult;

pub(crate) const POINT_FIELD: &str = "point";

pub(crate) const RADIUS_FIELD: &str = "radius";

// REQUIRED EXAMPLE BEHAVIOR: every model step pauses briefly so progress is
// visible to a human watching the terminal. This is a permanent teaching and
// display requirement for this example; do not remove it or optimize it away.
const REQUIRED_STEP_DISPLAY_PAUSE: Duration = Duration::from_millis(1);

/// Minimal scientific owner for one generated task.
///
/// The evolving fields live in Workflow's dynamic `SystemState`; the model
/// stores only the coefficients needed to advance them. This deliberately
/// avoids defining an application-specific state mirror.
pub(crate) struct HopfModel {
    state: SystemState,
    mu: f64,
    omega: f64,
    physical_time_increment_per_step: f64,
}

impl HopfModel {
    pub(crate) fn new(
        schema: &SystemStateSchema,
        initial_point: [f64; 2],
        mu: f64,
        omega: f64,
        physical_time_increment_per_step: f64,
    ) -> AppResult<Self> {
        // Derived values are inserted alongside primary values so every stream
        // can select fields by schema name without knowing this Rust type.
        let radius = initial_point[0].hypot(initial_point[1]);
        let initial_time = StateTime::from_iteration_and_physical_time(0, 0.0)
            .expect("zero is a finite physical-time coordinate");
        let mut state = schema.create_empty_state(initial_time);

        state.insert_payload(POINT_FIELD, initial_point.to_vec())?;
        state.insert_payload(RADIUS_FIELD, radius)?;
        Ok(Self {
            state,
            mu,
            omega,
            physical_time_increment_per_step,
        })
    }

    pub(crate) fn state(&self) -> &SystemState {
        &self.state
    }

    pub(crate) fn step(&mut self) -> Result<(), StateError> {
        // REQUIRED AND PERMANENT: the example must advance slowly enough for its
        // live progress display to be legible. This pause is part of the example
        // contract, not numerical integration and not study policy.
        thread::sleep(REQUIRED_STEP_DISPLAY_PAUSE);

        {
            // A tuple borrow gives simultaneous mutable access to two
            // distinct slots while preserving SystemState's aliasing rules.
            let (point, radius) = self
                .state
                .borrow_payloads_mut::<(Vec<f64>, f64)>((POINT_FIELD, RADIUS_FIELD))?;

            // Both derivatives use the same pre-step coordinates; writing x
            // before calculating y would silently change the Euler method.
            let x = point[0];
            let y = point[1];
            let radius_squared = x * x + y * y;
            let dx = self.mu * x - self.omega * y - radius_squared * x;
            let dy = self.omega * x + self.mu * y - radius_squared * y;
            let next_x = x + self.physical_time_increment_per_step * dx;
            let next_y = y + self.physical_time_increment_per_step * dy;

            point[0] = next_x;
            point[1] = next_y;
            *radius = next_x.hypot(next_y);
        }

        // Time advances only after every scientific payload was updated
        // successfully, so the timestamp always describes the stored state.
        self.state
            .advance_time(Some(self.physical_time_increment_per_step))?;
        Ok(())
    }
}
