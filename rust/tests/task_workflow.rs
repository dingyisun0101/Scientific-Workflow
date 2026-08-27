use scientific_workflow::prelude::basic::*;
use scientific_workflow::task::advanced::{ModelCatalog, ModelRegistration, Task};
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
fn advanced_task_definition_is_derived_from_the_model() {
    let task = Task::for_model::<Model>();
    assert!(format!("{task:?}").starts_with("Task"));
}

#[test]
fn model_catalog_sorts_keys_and_rejects_invalid_declarations() {
    let catalog = ModelCatalog::from_registrations([
        ModelRegistration::new::<Model>("zeta"),
        ModelRegistration::new::<Model>("alpha"),
    ])
    .unwrap();
    assert_eq!(catalog.keys().collect::<Vec<_>>(), ["alpha", "zeta"]);
    assert!(ModelCatalog::from_registrations([ModelRegistration::new::<Model>(" ")]).is_err());
}

#[test]
fn basic_prelude_contains_the_model_contract() {
    fn accepts_model<M: ScientificModel>() {}
    accepts_model::<Model>();
}
