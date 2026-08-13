use scientific_workflow::prelude::basics::*;

use crate::AppResult;

pub(crate) const POINT_FIELD: &str = "point";

pub(crate) const RADIUS_FIELD: &str = "radius";

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
        let radius = initial_point[0].hypot(initial_point[1]);
        let initial_time = SimulationTime::from_iteration_and_physical_time(0, 0.0)
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
        {
            // A tuple borrow gives simultaneous mutable access to two
            // distinct slots while preserving SystemState's aliasing rules.
            let (point, radius) = self
                .state
                .borrow_payloads_mut::<(Vec<f64>, f64)>((POINT_FIELD, RADIUS_FIELD))?;

            // Both derivatives must use the same pre-step coordinates.
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
            .advance_simulation_time(Some(self.physical_time_increment_per_step))?;
        Ok(())
    }
}
