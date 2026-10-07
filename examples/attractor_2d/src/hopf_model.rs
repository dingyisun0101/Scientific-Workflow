use std::time::Duration;

use scientific_workflow::prelude::*;
use serde::Deserialize;

const POINT_FIELD: &str = "point";

const RADIUS_FIELD: &str = "radius";

/// Domain model for one generated parameter-sweep task.
///
/// `HopfModel` is application terminology: implementing `ExecutionUnit` below
/// makes this model a unit that Workflow can validate, initialize, and run.
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
// `ExecutionUnit::Constants`. `deny_unknown_fields` also makes stale or
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
    /// scientific time or persisted constants.
    const DEMONSTRATION_STEP_DELAY: Duration = Duration::from_millis(1);
}

// This attribute only links the ordinary Rust implementation to the stable
// `attractor` manifest key. HopfModel itself owns and exposes its member state.
#[scientific_workflow::execution_unit("attractor")]
impl ExecutionUnit for HopfModel {
    type Constants = AttractorConstants;

    /// Validates this model's task and declares what Workflow must record.
    ///
    /// `Study::load` calls `preflight` with decoded constants and the selected
    /// state schema before Runtime creates output or starts the model. This
    /// model checks finite coefficients and a positive finite timestep; the
    /// observation builders validate stream names, field selections, and
    /// sampling intervals while constructing the complete recording plan.
    /// Workflow then binds that plan to the schema and rejects missing fields.
    /// `Ok(plan)` accepts the model-owned portion of validation; any `Err`
    /// rejects the study. Keep this hook free of side effects because it may
    /// run without the task ever being executed.
    fn preflight(
        constants: &Self::Constants,
        _schema: &SystemStateSchema,
    ) -> UnitResult<ObservationPlan> {
        if constants
            .initial_point
            .iter()
            .any(|value| !value.is_finite())
            || !constants.initial_point[0]
                .hypot(constants.initial_point[1])
                .is_finite()
            || !constants.mu.is_finite()
            || !constants.angular_frequency.is_finite()
            || !constants.physical_time_increment_per_step.is_finite()
            || constants.physical_time_increment_per_step <= 0.0
        {
            return Err("Hopf coefficients and initial point must be finite; timestep must be positive and finite".into());
        }
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

    /// Constructs one runnable model from owned constants and its state schema.
    ///
    /// Runtime calls this once when the task starts. The constants have already
    /// passed `preflight`; the schema is the state selected by the task, and the
    /// context is available for recorded seed requests when a model is
    /// stochastic. This deterministic model ignores the context, creates its
    /// only `SystemState`, initializes every required payload exactly once, and
    /// returns the fully owned model that subsequent lifecycle calls mutate.
    fn initialize(
        constants: Self::Constants,
        schema: &SystemStateSchema,
        _context: &InitializationContext,
    ) -> UnitResult<Self> {
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

    /// Reports how many independently observable members the model exposes.
    ///
    /// Workflow uses this value to enumerate member views and metadata. A
    /// standalone Hopf model always has exactly one member; an ensemble could
    /// expose several while implementing the same `ExecutionUnit` contract.
    fn member_count(&self) -> usize {
        1
    }

    /// Borrows the requested member's current state and progress information.
    ///
    /// Index zero returns a `MemberView` over this model's sole state, stable
    /// identity, optional completion, and target iteration. Any other index
    /// returns `None`, matching the bound declared by `member_count`. The view
    /// borrows `self`, so callers cannot retain it while mutating the model.
    fn member(&self, index: usize) -> Option<MemberView<'_>> {
        (index == 0).then(|| {
            MemberView::new(
                "hopf-attractor",
                &self.state,
                (self.state.time().iteration() == self.constants.step_count)
                    .then_some(MemberCompletion::without_reason()),
                Some(self.constants.step_count),
            )
        })
    }

    /// Advances the scientific model by one iteration.
    ///
    /// Runtime repeatedly calls `step` until `member(0)` reports completion.
    /// The method updates the point and its derived radius atomically from the
    /// same pre-step coordinates, advances the canonical state time only after
    /// payload updates succeed, then applies the example-only dashboard delay.
    /// An error stops the task and is recorded by Workflow.
    fn step(&mut self) -> UnitResult {
        let next_time = self
            .state
            .time()
            .checked_advance(Some(self.constants.physical_time_increment_per_step))?;
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
            let next_radius = next_x.hypot(next_y);

            if !next_x.is_finite() || !next_y.is_finite() || !next_radius.is_finite() {
                return Err("Hopf Euler step produced a nonfinite point or radius".into());
            }

            point[0] = next_x;
            point[1] = next_y;
            *radius = next_radius;
        }

        // The next time and payloads were validated before any assignment.
        // Committing time cannot fail after the scientific payloads change.
        self.state.replace_time(next_time);

        // Keep this presentation delay: without it, the bundled calculation
        // finishes too quickly for developers to inspect the live dashboard.
        std::thread::sleep(Self::DEMONSTRATION_STEP_DELAY);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(point: [f64; 2], timestep: f64, time: StateTime) -> HopfModel {
        let schema = SystemStateSchema::load_json_template(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("wf_configs/states/attractor.json"),
        )
        .unwrap();
        let mut state = schema.create_empty_state(time);
        state
            .initialize_payload(POINT_FIELD, point.to_vec())
            .unwrap();
        state
            .initialize_payload(RADIUS_FIELD, point[0].hypot(point[1]))
            .unwrap();
        HopfModel {
            state,
            constants: AttractorConstants {
                initial_point: point,
                physical_time_increment_per_step: timestep,
                step_count: 2,
                trajectory_sampling_interval: 1,
                radius_sampling_interval: 1,
                checkpoint_sampling_interval: 1,
                mu: 1.0,
                angular_frequency: 1.0,
            },
        }
    }

    fn time(physical: f64) -> StateTime {
        StateTime::from_iteration_and_physical_time(0, physical).unwrap()
    }

    #[test]
    fn euler_step_uses_the_same_pre_step_point_and_commits_matching_time() {
        let mut model = model([0.25, 0.0], 0.01, time(0.0));
        model.step().unwrap();
        let point = model.state.payload::<Vec<f64>>(POINT_FIELD).unwrap();
        assert_eq!(point, &[0.25234375, 0.0025]);
        assert_eq!(
            *model.state.payload::<f64>(RADIUS_FIELD).unwrap(),
            point[0].hypot(point[1])
        );
        assert_eq!(
            model.state.time(),
            StateTime::from_iteration_and_physical_time(1, 0.01).unwrap()
        );
    }

    #[test]
    fn rejected_euler_or_time_advance_preserves_the_complete_state() {
        for mut model in [
            model([1e200, 1.0], 0.01, time(0.0)),
            model([0.25, 0.0], f64::MAX, time(f64::MAX)),
        ] {
            let before_point = model
                .state
                .payload::<Vec<f64>>(POINT_FIELD)
                .unwrap()
                .clone();
            let before_radius = *model.state.payload::<f64>(RADIUS_FIELD).unwrap();
            let before_time = model.state.time();
            assert!(model.step().is_err());
            assert_eq!(
                model.state.payload::<Vec<f64>>(POINT_FIELD).unwrap(),
                &before_point
            );
            assert_eq!(
                *model.state.payload::<f64>(RADIUS_FIELD).unwrap(),
                before_radius
            );
            assert_eq!(model.state.time(), before_time);
        }
    }

    #[test]
    fn preflight_rejects_invalid_timestep_and_initial_radius() {
        for model in [
            model([0.25, 0.0], 0.0, time(0.0)),
            model([0.25, 0.0], -0.01, time(0.0)),
            model([f64::MAX, f64::MAX], 0.01, time(0.0)),
        ] {
            assert!(HopfModel::preflight(&model.constants, model.state.schema()).is_err());
        }
    }
}
