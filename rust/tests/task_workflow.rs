use std::path::Path;

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

    fn initialize(constants: Constants, schema: &SystemStateSchema) -> TaskResult<Self> {
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

#[test]
fn model_directly_exposes_and_mutates_its_typed_tuple_state() {
    let schema = SystemStateSchema::load_json_template(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/state.json"),
    )
    .unwrap();
    let mut model = Model::initialize(
        Constants {
            population: 41,
            active: false,
        },
        &schema,
    )
    .unwrap();

    assert!(std::ptr::eq(model.model(0).unwrap().state(), &model.state));
    assert_eq!(
        model
            .model(0)
            .unwrap()
            .state()
            .borrow_payloads::<(u64, bool)>(("population", "activity"))
            .map(|(population, active)| (*population, *active))
            .unwrap(),
        (41, false)
    );

    model.step().unwrap();

    assert!(std::ptr::eq(model.model(0).unwrap().state(), &model.state));
    assert_eq!(
        model
            .model(0)
            .unwrap()
            .state()
            .borrow_payloads::<(u64, bool)>(("population", "activity"))
            .map(|(population, active)| (*population, *active))
            .unwrap(),
        (42, true)
    );
    assert!(model.model(0).unwrap().is_complete());
}
