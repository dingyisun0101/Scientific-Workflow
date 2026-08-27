use scientific_workflow::prelude::basic::*;
use serde::Deserialize;

#[derive(Deserialize)]
struct Constants;

struct Model {
    state: SystemState,
}

impl ScientificModel for Model {
    type Constants = Constants;

    fn initialize(_: Constants, schema: &SystemStateSchema) -> TaskResult<Self> {
        Ok(Self {
            state: schema.create_empty_state(StateTime::from_iteration(0)),
        })
    }

    fn state(&self) -> &SystemState {
        &self.state
    }

    fn is_complete(&self) -> bool {
        true
    }

    fn step(&mut self) -> TaskResult {
        unreachable!("the model starts complete")
    }
}

#[test]
fn basic_prelude_contains_the_model_contract() {
    fn accepts_model<M: ScientificModel>() {}
    accepts_model::<Model>();
}
