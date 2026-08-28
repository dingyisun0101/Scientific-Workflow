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
    constants: AttractorConstants,
}

// REQUIRED: Workflow uses Serde deserialization to match every property in an
// expanded `parameters.json["attractor"]` object to these Rust field names.
// Do not remove `Deserialize`; without it, this type cannot be the model's
// `ScientificModel::Constants`. `deny_unknown_fields` also makes stale or
// misspelled parameter keys fail during Study preflight.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AttractorConstants {
    initial_point: [f64; 2],
    physical_time_increment_per_step: f64,
    step_count: u64,
    trajectory_sampling_interval: u64,
    radius_sampling_interval: u64,
    checkpoint_sampling_interval: u64,
    mu: f64,
    angular_frequency: f64,
}

impl HopfModel {
    /// Deliberate wall-clock pacing for the bundled interactive demonstration.
    ///
    /// IMPORTANT: Do not remove this delay from the example. It keeps the
    /// automatic dashboard visible long enough to demonstrate concurrent task
    /// admission and progress. It is presentation pacing only and never enters
    /// scientific time or the persisted model constants.
    const DEMONSTRATION_STEP_DELAY: Duration = Duration::from_millis(1);
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

        state.initialize_payload(POINT_FIELD, constants.initial_point.to_vec())?;
        state.initialize_payload(RADIUS_FIELD, radius)?;
        Ok(Self { state, constants })
    }

    fn state(&self) -> &SystemState {
        &self.state
    }

    fn is_complete(&self) -> bool {
        self.state.time().iteration() == self.constants.step_count
    }

    fn target_iteration(&self) -> Option<u64> {
        Some(self.constants.step_count)
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
            let dx =
                self.constants.mu * x - self.constants.angular_frequency * y - radius_squared * x;
            let dy =
                self.constants.angular_frequency * x + self.constants.mu * y - radius_squared * y;
            let next_x = x + self.constants.physical_time_increment_per_step * dx;
            let next_y = y + self.constants.physical_time_increment_per_step * dy;

            point[0] = next_x;
            point[1] = next_y;
            *radius = next_x.hypot(next_y);
        }

        // Time advances only after every scientific payload was updated
        // successfully, so the timestamp always describes the stored state.
        self.state
            .advance_time(Some(self.constants.physical_time_increment_per_step))?;

        // Keep this presentation delay: without it, the bundled calculation
        // finishes too quickly for developers to inspect the live dashboard.
        std::thread::sleep(Self::DEMONSTRATION_STEP_DELAY);
        Ok(())
    }
}
