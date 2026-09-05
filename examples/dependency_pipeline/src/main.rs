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
        let mut state = schema.create_empty_state(StateTime::from_iteration(0));
        state.initialize_payload("value", checkpoint.take_payload::<u64>("value")?)?;
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
        *self.state.payload_mut::<u64>("value")? += 1;
        self.state.advance_time(None)?;
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
