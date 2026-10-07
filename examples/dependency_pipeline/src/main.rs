//! Runnable initialization → simulation → NPY → analysis example.
use scientific_workflow::persistence::{JsonPayloadDecoderRegistry, StoredStateSeriesReader};
use scientific_workflow::prelude::*;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Initial {
    value: u64,
}
struct Initialize {
    state: SystemState,
}
#[scientific_workflow::execution_unit("initialize")]
impl ExecutionUnit for Initialize {
    type Constants = Initial;
    fn preflight(_: &Initial, _: &SystemStateSchema) -> UnitResult<ObservationPlan> {
        Ok(ObservationPlan::streams([ObservationStream::all_fields(
            "checkpoint",
        )?
        .initial_and_final()])?)
    }
    fn initialize(
        constants: Initial,
        schema: &SystemStateSchema,
        _: &InitializationContext,
    ) -> UnitResult<Self> {
        let mut state = schema.create_empty_state(StateTime::from_iteration(0));
        state.initialize_payload("value", constants.value)?;
        Ok(Self { state })
    }
    fn member_count(&self) -> usize {
        1
    }
    fn member(&self, index: usize) -> Option<MemberView<'_>> {
        (index == 0).then(|| {
            MemberView::new(
                "initialization",
                &self.state,
                Some(MemberCompletion::without_reason()),
                Some(0),
            )
        })
    }
    fn step(&mut self) -> UnitResult {
        unreachable!("initial state is already complete")
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Simulation {
    steps: u64,
}
struct Evolve {
    state: SystemState,
    target: u64,
}
#[scientific_workflow::execution_unit("simulation")]
impl ExecutionUnit for Evolve {
    type Constants = Simulation;
    fn initialize(
        constants: Simulation,
        schema: &SystemStateSchema,
        context: &InitializationContext,
    ) -> UnitResult<Self> {
        let recording = context
            .dependencies()
            .recordings()
            .execution_unit("initialize")
            .member("initialization")
            .one()?;
        let decoders = JsonPayloadDecoderRegistry::new().with_json_field::<u64>("value")?;
        let mut checkpoint =
            StoredStateSeriesReader::open_completed_recording(recording.directory(), decoders)?
                .read_latest_state_from_stream("checkpoint")?;
        let value = checkpoint.take_payload::<u64>("value")?;
        value
            .checked_add(constants.steps)
            .ok_or("initial value plus requested steps exceeds u64")?;
        let mut state = schema.create_empty_state(StateTime::from_iteration(0));
        state.initialize_payload("value", value)?;
        Ok(Self {
            state,
            target: constants.steps,
        })
    }
    fn member_count(&self) -> usize {
        1
    }
    fn member(&self, index: usize) -> Option<MemberView<'_>> {
        (index == 0).then(|| {
            MemberView::new(
                "simulation",
                &self.state,
                (self.state.time().iteration() >= self.target)
                    .then_some(MemberCompletion::without_reason()),
                Some(self.target),
            )
        })
    }
    fn step(&mut self) -> UnitResult {
        let next_time = self.state.time().checked_advance(None)?;
        let next_value = self
            .state
            .payload::<u64>("value")?
            .checked_add(1)
            .ok_or("simulation value exceeds u64")?;
        *self.state.payload_mut::<u64>("value")? = next_value;
        self.state.replace_time(next_time);
        Ok(())
    }
}
fn main() -> Result<(), scientific_workflow::WorkflowError> {
    let root = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    scientific_workflow::run(&root)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(value: u64, iteration: u64) -> Evolve {
        let schema = SystemStateSchema::load_json_template(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("wf_configs/states/value.json"),
        )
        .unwrap();
        let mut state = schema.create_empty_state(StateTime::from_iteration(iteration));
        state.initialize_payload("value", value).unwrap();
        Evolve {
            state,
            target: iteration.saturating_add(1),
        }
    }

    #[test]
    fn overflow_rejects_the_step_without_changing_value_or_time() {
        for mut model in [model(u64::MAX, 0), model(7, u64::MAX)] {
            let value = *model.state.payload::<u64>("value").unwrap();
            let time = model.state.time();
            assert!(model.step().is_err());
            assert_eq!(*model.state.payload::<u64>("value").unwrap(), value);
            assert_eq!(model.state.time(), time);
        }
    }
}
