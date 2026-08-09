//! Live state ownership and the two-dimensional scientific evolution kernel.
//!
//! [`HopfModel`] directly owns the sole continuously evolving
//! [`SystemState`] for one task. Persistence receives only temporary immutable
//! borrows of that state; analysis reconstructs separate states after the
//! recording is complete. This separation keeps the simulation's ownership
//! unambiguous and avoids cloning scientific payloads at sampling boundaries.

use scientific_workflow::prelude::*;

use crate::AppResult;

/// Exact state-template key for the phase-space point.
pub(crate) const POINT_FIELD: &str = "point";

/// Exact state-template key for the scalar radial diagnostic.
pub(crate) const RADIUS_FIELD: &str = "radius";

/// Scientific model and sole owner of its continuously evolving state.
pub(crate) struct HopfModel {
    state: SystemState,
    mu: f64,
    omega: f64,
    physical_time_increment_per_step: f64,
}

impl HopfModel {
    /// Creates the initial complete state and transfers both payload owners.
    pub(crate) fn new(
        schema: &SystemStateSchema,
        initial_point: Vec<f64>,
        mu: f64,
        omega: f64,
        physical_time_increment_per_step: f64,
    ) -> AppResult<Self> {
        let radius = initial_point[0].hypot(initial_point[1]);
        let initial_time = SimulationTime::from_iteration_and_physical_time(0, 0.0)
            .expect("zero is a finite physical-time coordinate");
        let mut state = schema.create_empty_state(initial_time);

        state.insert_payload(POINT_FIELD, initial_point)?;
        state.insert_payload(RADIUS_FIELD, radius)?;
        Ok(Self {
            state,
            mu,
            omega,
            physical_time_increment_per_step,
        })
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
    pub(crate) fn step(&mut self) -> Result<(), StateError> {
        {
            let (point, radius) = self
                .state
                .borrow_payloads_mut::<(Vec<f64>, f64)>((POINT_FIELD, RADIUS_FIELD))?;
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
        self.state
            .advance_simulation_time(Some(self.physical_time_increment_per_step))?;
        Ok(())
    }

    /// Returns the linear radial-growth coefficient.
    pub(crate) fn mu(&self) -> f64 {
        self.mu
    }

    /// Returns the angular-frequency coefficient.
    pub(crate) fn omega(&self) -> f64 {
        self.omega
    }

    /// Returns the physical-time increment applied by one model step.
    pub(crate) fn physical_time_increment_per_step(&self) -> f64 {
        self.physical_time_increment_per_step
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
