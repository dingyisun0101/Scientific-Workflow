use scientific_workflow::prelude::basic::*;
use serde::Deserialize;

#[derive(Deserialize)]
struct Constants {
    population: u64,
    active: bool,
}

struct Model {
    state: SystemState,
}

#[scientific_workflow::model("downstream-contract-test")]
impl ExecutionUnit for Model {
    type Constants = Constants;

    fn initialize(
        constants: Constants,
        schema: &SystemStateSchema,
        _context: &InitializationContext,
    ) -> TaskResult<Self> {
        let mut state = schema.create_empty_state(StateTime::from_iteration(0));
        state.initialize_payload("population", constants.population)?;
        state.initialize_payload("activity", constants.active)?;
        Ok(Self { state })
    }

    fn model_count(&self) -> usize {
        1
    }

    fn model(&self, index: usize) -> Option<ModelView<'_>> {
        (index == 0).then(|| {
            ModelView::new(
                "model",
                &self.state,
                self.state.time().iteration() == 1,
                Some(1),
            )
        })
    }

    fn step(&mut self) -> TaskResult {
        let (population, active) = self
            .state
            .borrow_payloads_mut::<(u64, bool)>(("population", "activity"))?;
        *population += 1;
        *active = !*active;
        self.state.advance_time(None)?;
        Ok(())
    }
}

#[test]
fn basic_prelude_contains_the_execution_unit_contract() {
    fn accepts_unit<M: ExecutionUnit>() {}
    accepts_unit::<Model>();
}
