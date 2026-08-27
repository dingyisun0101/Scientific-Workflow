use std::time::Duration;

use scientific_workflow::prelude::basic::*;
use serde::Deserialize;

const POINT_FIELD: &str = "point";

const RADIUS_FIELD: &str = "radius";

/// Scientific owner for one generated parameter-sweep task.
///
/// The evolving fields live in Workflow's dynamic `SystemState`; the model
/// stores only the coefficients needed to advance them. This deliberately
/// avoids defining an application-specific state mirror.
pub(crate) struct HopfModel {
    state: SystemState,
    mu: f64,
    omega: f64,
    physical_time_increment_per_step: f64,
    artificial_step_delay: Duration,
    step_count: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AttractorConstants {
    initial_point: [f64; 2],
    physical_time_increment_per_step: f64,
    artificial_step_delay_ms: u64,
    step_count: u64,
    trajectory_sampling_interval: u64,
    radius_sampling_interval: u64,
    checkpoint_sampling_interval: u64,
    mu: f64,
    angular_frequency: f64,
}

// This attribute only links the ordinary Rust implementation to the stable
// `attractor` manifest key. HopfModel itself owns and exposes all model state.
#[scientific_workflow::model("attractor")]
impl ScientificModel for HopfModel {
    type Constants = AttractorConstants;

    fn observation_plan(constants: &Self::Constants) -> TaskResult<ObservationPlan> {
        Ok(ObservationPlan::streams([
            ObservationStream::fields("trajectory", [POINT_FIELD])?
                .every_iterations(constants.trajectory_sampling_interval)?,
            ObservationStream::fields("radius", [RADIUS_FIELD])?
                .every_iterations(constants.radius_sampling_interval)?,
            ObservationStream::fields("checkpoint", [POINT_FIELD, RADIUS_FIELD])?
                .every_iterations(constants.checkpoint_sampling_interval)?,
        ])?
        .with_iteration_unit("iteration")?
        .with_physical_time_unit("dimensionless_model_time")?)
    }

    fn initialize(constants: Self::Constants, schema: &SystemStateSchema) -> TaskResult<Self> {
        // Derived values are inserted alongside primary values so every stream
        // can select fields by schema name without knowing this Rust type.
        let radius = constants.initial_point[0].hypot(constants.initial_point[1]);
        let initial_time = StateTime::from_iteration_and_physical_time(0, 0.0)
            .expect("zero is a finite physical-time coordinate");
        let mut state = schema.create_empty_state(initial_time);

        state.insert_payload(POINT_FIELD, constants.initial_point.to_vec())?;
        state.insert_payload(RADIUS_FIELD, radius)?;
        Ok(Self {
            state,
            mu: constants.mu,
            omega: constants.angular_frequency,
            physical_time_increment_per_step: constants.physical_time_increment_per_step,
            artificial_step_delay: Duration::from_millis(constants.artificial_step_delay_ms),
            step_count: constants.step_count,
        })
    }

    fn state(&self) -> &SystemState {
        &self.state
    }

    fn is_complete(&self) -> bool {
        self.state.time().iteration() == self.step_count
    }

    fn target_iteration(&self) -> Option<u64> {
        Some(self.step_count)
    }

    fn step(&mut self) -> TaskResult {
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

        // This project-only pacing delay keeps progress visible in the
        // dashboard. It is not scientific time and Workflow does not infer it.
        std::thread::sleep(self.artificial_step_delay);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn configured_artificial_delay_paces_each_successful_step() {
        let schema_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("config/state.json");
        let schema = SystemStateSchema::load_json_template(&schema_path).unwrap();
        let delay = Duration::from_millis(15);
        let constants = AttractorConstants {
            initial_point: [0.25, 0.0],
            physical_time_increment_per_step: 0.01,
            artificial_step_delay_ms: u64::try_from(delay.as_millis()).unwrap(),
            step_count: 1,
            trajectory_sampling_interval: 1,
            radius_sampling_interval: 1,
            checkpoint_sampling_interval: 1,
            mu: 0.25,
            angular_frequency: 1.0,
        };
        let mut model = HopfModel::initialize(constants, &schema).unwrap();

        let started = Instant::now();
        model.step().unwrap();

        assert!(started.elapsed() >= delay);
        assert!(model.is_complete());
    }
}
