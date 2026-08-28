use scientific_workflow::prelude::basic::*;
use serde::Deserialize;

#[derive(Deserialize)]
struct Constants {
    population: u64,
    active: bool,
}

struct Unit {
    state: SystemState,
}

#[scientific_workflow::execution_unit("downstream-contract-test")]
impl ExecutionUnit for Unit {
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

    fn member_count(&self) -> usize {
        1
    }

    fn member(&self, index: usize) -> Option<MemberView<'_>> {
        (index == 0).then(|| {
            MemberView::new(
                "unit",
                &self.state,
                (self.state.time().iteration() == 1).then_some(MemberCompletion::without_reason()),
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
    accepts_unit::<Unit>();
}
