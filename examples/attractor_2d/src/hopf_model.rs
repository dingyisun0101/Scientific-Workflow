//! Live state ownership and the two-dimensional scientific evolution kernel.
//!
//! [`HopfModel`] directly owns the sole continuously evolving
//! [`SystemState`] for one task. Persistence receives only temporary immutable
//! borrows of that state; analysis reconstructs separate states after the
//! recording is complete. This separation keeps the simulation's ownership
//! unambiguous and avoids cloning scientific payloads at sampling boundaries.

use scientific_workflow::prelude::*;

use crate::AppResult;
use crate::project_setup::TaskSettings;

/// Exact state-template key for the phase-space point.
pub(crate) const POINT_FIELD: &str = "point";

/// Exact state-template key for the scalar radial diagnostic.
pub(crate) const RADIUS_FIELD: &str = "radius";

/// Scientific model and sole owner of its continuously evolving state.
pub(crate) struct HopfModel {
    state: SystemState,
}

impl HopfModel {
    /// Creates the initial complete state and transfers both payload owners.
    pub(crate) fn new(schema: &SystemStateSchema, initial_point: Vec<f64>) -> AppResult<Self> {
        let radius = initial_point[0].hypot(initial_point[1]);
        let initial_time = SimulationTime::from_step_and_physical_time(0, 0.0)
            .expect("zero is a finite physical-time coordinate");
        let mut state = schema.create_empty_state(initial_time);

        state.insert_payload(POINT_FIELD, initial_point)?;
        state.insert_payload(RADIUS_FIELD, radius)?;
        Ok(Self { state })
    }

    /// Borrows the current complete state for zero-copy sample encoding.
    pub(crate) fn state(&self) -> &SystemState {
        &self.state
    }

    /// Advances the Hopf normal form by one explicit-Euler transition.
    ///
    /// Both derivatives use the same old point. The point and derived radius
    /// are then committed within one coordinated mutable borrow, after which
    /// simulation and physical time advance transactionally.
    pub(crate) fn advance(&mut self, settings: &TaskSettings) -> Result<(), StateError> {
        {
            let (point, radius) = self
                .state
                .borrow_payloads_mut::<(Vec<f64>, f64)>((POINT_FIELD, RADIUS_FIELD))?;
            let x = point[0];
            let y = point[1];
            let radius_squared = x * x + y * y;
            let dx = settings.mu * x - settings.omega * y - radius_squared * x;
            let dy = settings.omega * x + settings.mu * y - radius_squared * y;
            let next_x = x + settings.time_step * dx;
            let next_y = y + settings.time_step * dy;

            point[0] = next_x;
            point[1] = next_y;
            *radius = next_x.hypot(next_y);
        }
        self.state
            .advance_simulation_time(Some(settings.time_step))?;
        Ok(())
    }

    /// Borrows the model's current point without copying its vector.
    pub(crate) fn point(&self) -> Result<&[f64], StateError> {
        self.state
            .payload::<Vec<f64>>(POINT_FIELD)
            .map(Vec::as_slice)
    }

    /// Returns the model's current scalar radius.
    pub(crate) fn radius(&self) -> Result<f64, StateError> {
        self.state.payload::<f64>(RADIUS_FIELD).copied()
    }
}
