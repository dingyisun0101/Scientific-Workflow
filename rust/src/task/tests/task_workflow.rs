//! Private catalog and execution-contract coverage.

use std::cell::Cell;
use std::ffi::OsString;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use serde::{Deserialize, Deserializer};

use crate::config::advanced::{ResolvedModelParameters, ResolvedProgramTask};
use crate::observation::advanced::{BoundObservationPlan, ObservationPlan};
use crate::state::advanced::{StateTime, SystemState, SystemStateSchema, schema_from_json_value};

use super::catalog::{ModelCatalog, ModelCatalogError, ModelRegistration};
use super::execution::{
    ProgramDefinition, ProgramTaskInvocation, StatefulDefinition, TaskDefinition, TaskExecutionHost,
};
use super::model::ScientificModel;
use super::result::TaskResult;

#[derive(Debug, Eq, PartialEq)]
enum Event {
    Begin(u64, Option<u64>),
    Step(u64, Option<u64>),
    Final(u64, Option<u64>),
}

#[derive(Default)]
struct RecordingHost {
    cancelled: Cell<bool>,
    cancel_after_step: bool,
    events: Vec<Event>,
}

#[derive(Default)]
struct ProgramInvocationHost {
    executed: bool,
}

impl TaskExecutionHost for ProgramInvocationHost {
    fn cancellation_requested(&self) -> bool {
        false
    }

    fn execute_program(&mut self, program: ProgramTaskInvocation<'_>) -> TaskResult {
        assert_eq!(program.executable(), Path::new("/resolved/python"));
        assert_eq!(
            program.args(),
            [OsString::from("script.py"), OsString::from("--plot")]
        );
        assert_eq!(program.kind(), "python");
        assert_eq!(
            program.python_script(),
            Some(Path::new("/project/script.py"))
        );
        assert_eq!(program.python_environment_manager(), Some("system"));
        self.executed = true;
        Ok(())
    }

    fn begin_model(
        &mut self,
        _plan: BoundObservationPlan,
        _state: &SystemState,
        _target_iteration: Option<u64>,
    ) -> TaskResult {
        panic!("program contract tests do not initialize models")
    }

    fn observe_model_step(
        &mut self,
        _state: &SystemState,
        _target_iteration: Option<u64>,
    ) -> TaskResult {
        panic!("program contract tests do not observe models")
    }

    fn observe_model_final(
        &mut self,
        _state: &SystemState,
        _target_iteration: Option<u64>,
    ) -> TaskResult {
        panic!("program contract tests do not observe models")
    }
}

impl TaskExecutionHost for RecordingHost {
    fn cancellation_requested(&self) -> bool {
        self.cancelled.get()
    }

    fn execute_program(&mut self, _program: ProgramTaskInvocation<'_>) -> TaskResult {
        panic!("model contract tests do not execute programs")
    }

    fn begin_model(
        &mut self,
        _plan: BoundObservationPlan,
        state: &SystemState,
        target_iteration: Option<u64>,
    ) -> TaskResult {
        self.events
            .push(Event::Begin(state.time().iteration(), target_iteration));
        Ok(())
    }

    fn observe_model_step(
        &mut self,
        state: &SystemState,
        target_iteration: Option<u64>,
    ) -> TaskResult {
        self.events
            .push(Event::Step(state.time().iteration(), target_iteration));
        if self.cancel_after_step {
            self.cancelled.set(true);
        }
        Ok(())
    }

    fn observe_model_final(
        &mut self,
        state: &SystemState,
        target_iteration: Option<u64>,
    ) -> TaskResult {
        self.events
            .push(Event::Final(state.time().iteration(), target_iteration));
        Ok(())
    }
}

fn schema(source: &str) -> SystemStateSchema {
    schema_from_json_value(
        Path::new(source),
        &serde_json::json!({"fields":[{"name":"value"}]}),
    )
    .unwrap()
}

fn definition<M>(value: serde_json::Value) -> StatefulDefinition<M>
where
    M: ScientificModel,
{
    let schema = schema("wf_configs/states/value.json");
    let plan = BoundObservationPlan::bind(ObservationPlan::all_fields(), &schema).unwrap();
    let parameters = ResolvedModelParameters::new(
        "test".into(),
        PathBuf::from("wf_configs/parameters.json"),
        0,
        value,
        None,
    );
    StatefulDefinition::new(parameters, schema, plan)
}

struct LocalConstants {
    steps: u64,
    _not_send_or_sync: PhantomData<Rc<()>>,
}

impl<'de> Deserialize<'de> for LocalConstants {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            steps: u64,
        }

        let wire = Wire::deserialize(deserializer)?;
        Ok(Self {
            steps: wire.steps,
            _not_send_or_sync: PhantomData,
        })
    }
}

struct CountingModel {
    state: SystemState,
    steps: u64,
}

impl ScientificModel for CountingModel {
    type Constants = LocalConstants;

    fn initialize(constants: Self::Constants, schema: &SystemStateSchema) -> TaskResult<Self> {
        let mut state = schema.create_empty_state(StateTime::from_iteration(0));
        state.initialize_payload("value", 0_u64)?;
        Ok(Self {
            state,
            steps: constants.steps,
        })
    }

    fn state(&self) -> &SystemState {
        &self.state
    }

    fn is_complete(&self) -> bool {
        self.state.time().iteration() >= self.steps
    }

    fn step(&mut self) -> TaskResult {
        *self.state.payload_mut::<u64>("value")? += 1;
        self.state.advance_time(None)?;
        Ok(())
    }

    fn target_iteration(&self) -> Option<u64> {
        Some(self.steps)
    }
}

struct PanicOnInitialize;

impl ScientificModel for PanicOnInitialize {
    type Constants = ();

    fn initialize(_constants: Self::Constants, _schema: &SystemStateSchema) -> TaskResult<Self> {
        panic!("initialization must not run after pre-execution cancellation")
    }

    fn state(&self) -> &SystemState {
        unreachable!()
    }

    fn is_complete(&self) -> bool {
        unreachable!()
    }

    fn step(&mut self) -> TaskResult {
        unreachable!()
    }
}

struct NonAdvancingModel {
    state: SystemState,
}

impl ScientificModel for NonAdvancingModel {
    type Constants = ();

    fn initialize(_constants: Self::Constants, schema: &SystemStateSchema) -> TaskResult<Self> {
        let mut state = schema.create_empty_state(StateTime::from_iteration(0));
        state.initialize_payload("value", 0_u64)?;
        Ok(Self { state })
    }

    fn state(&self) -> &SystemState {
        &self.state
    }

    fn is_complete(&self) -> bool {
        false
    }

    fn step(&mut self) -> TaskResult {
        Ok(())
    }
}

struct OwnerSwitchModel {
    first: SystemState,
    second: SystemState,
    use_second: bool,
}

impl ScientificModel for OwnerSwitchModel {
    type Constants = ();

    fn initialize(_constants: Self::Constants, schema: &SystemStateSchema) -> TaskResult<Self> {
        let mut first = schema.create_empty_state(StateTime::from_iteration(0));
        first.initialize_payload("value", 0_u64)?;
        let mut second = schema.create_empty_state(StateTime::from_iteration(0));
        second.initialize_payload("value", 0_u64)?;
        Ok(Self {
            first,
            second,
            use_second: false,
        })
    }

    fn state(&self) -> &SystemState {
        if self.use_second {
            &self.second
        } else {
            &self.first
        }
    }

    fn is_complete(&self) -> bool {
        false
    }

    fn step(&mut self) -> TaskResult {
        self.use_second = true;
        self.second.advance_time(None)?;
        Ok(())
    }
}

struct SchemaSwitchModel {
    state: SystemState,
}

impl ScientificModel for SchemaSwitchModel {
    type Constants = ();

    fn initialize(_constants: Self::Constants, schema: &SystemStateSchema) -> TaskResult<Self> {
        let mut state = schema.create_empty_state(StateTime::from_iteration(0));
        state.initialize_payload("value", 0_u64)?;
        Ok(Self { state })
    }

    fn state(&self) -> &SystemState {
        &self.state
    }

    fn is_complete(&self) -> bool {
        false
    }

    fn step(&mut self) -> TaskResult {
        let replacement_schema = schema("wf_configs/states/replacement.json");
        let mut replacement = replacement_schema.create_empty_state(StateTime::from_iteration(1));
        replacement.initialize_payload("value", 1_u64)?;
        self.state = replacement;
        Ok(())
    }
}

#[derive(Deserialize)]
struct TargetConstants {
    mode: String,
}

struct InvalidTargetModel {
    state: SystemState,
    target: Option<u64>,
    mode: String,
}

impl ScientificModel for InvalidTargetModel {
    type Constants = TargetConstants;

    fn initialize(constants: Self::Constants, schema: &SystemStateSchema) -> TaskResult<Self> {
        let iteration = u64::from(constants.mode == "before");
        let mut state = schema.create_empty_state(StateTime::from_iteration(iteration));
        state.initialize_payload("value", 0_u64)?;
        let target = match constants.mode.as_str() {
            "before" => Some(0),
            "decrease" => Some(3),
            "disappear" => Some(2),
            _ => unreachable!(),
        };
        Ok(Self {
            state,
            target,
            mode: constants.mode,
        })
    }

    fn state(&self) -> &SystemState {
        &self.state
    }

    fn is_complete(&self) -> bool {
        false
    }

    fn step(&mut self) -> TaskResult {
        self.state.advance_time(None)?;
        self.target = match self.mode.as_str() {
            "decrease" => Some(2),
            "disappear" => None,
            _ => self.target,
        };
        Ok(())
    }

    fn target_iteration(&self) -> Option<u64> {
        self.target
    }
}

struct FailingStepModel {
    state: SystemState,
}

impl ScientificModel for FailingStepModel {
    type Constants = ();

    fn initialize(_constants: Self::Constants, schema: &SystemStateSchema) -> TaskResult<Self> {
        let mut state = schema.create_empty_state(StateTime::from_iteration(0));
        state.initialize_payload("value", 0_u64)?;
        Ok(Self { state })
    }

    fn state(&self) -> &SystemState {
        &self.state
    }

    fn is_complete(&self) -> bool {
        false
    }

    fn step(&mut self) -> TaskResult {
        Err(std::io::Error::other("step failed").into())
    }
}

#[test]
fn catalog_rejects_invalid_and_duplicate_registration_keys() {
    assert!(matches!(
        ModelCatalog::from_registrations([ModelRegistration::new::<CountingModel>(" ")]),
        Err(ModelCatalogError::InvalidKey { key }) if key == " "
    ));

    let duplicate = ModelRegistration::new::<CountingModel>("counter");
    assert!(matches!(
        ModelCatalog::from_registrations([duplicate, duplicate]),
        Err(ModelCatalogError::DuplicateKey { key }) if key == "counter"
    ));

    let catalog = ModelCatalog::from_registrations([duplicate]).unwrap();
    assert!(catalog.get("counter").is_some());
}

#[test]
fn execution_observes_initial_steps_and_final_state_in_order() {
    let mut host = RecordingHost::default();
    definition::<CountingModel>(serde_json::json!({"steps":2}))
        .execute(&mut host)
        .unwrap();

    assert_eq!(
        host.events,
        [
            Event::Begin(0, Some(2)),
            Event::Step(1, Some(2)),
            Event::Step(2, Some(2)),
            Event::Final(2, Some(2)),
        ]
    );
}

#[test]
fn cancellation_prevents_initialization_and_stops_between_steps() {
    let mut before = RecordingHost::default();
    before.cancelled.set(true);
    definition::<PanicOnInitialize>(serde_json::Value::Null)
        .execute(&mut before)
        .unwrap();
    assert!(before.events.is_empty());

    let mut between = RecordingHost {
        cancel_after_step: true,
        ..RecordingHost::default()
    };
    definition::<CountingModel>(serde_json::json!({"steps":3}))
        .execute(&mut between)
        .unwrap();
    assert_eq!(
        between.events,
        [Event::Begin(0, Some(3)), Event::Step(1, Some(3))]
    );
}

#[test]
fn execution_rejects_changed_state_owner_schema_and_nonadvancing_steps() {
    for (result, expected) in [
        (
            definition::<OwnerSwitchModel>(serde_json::Value::Null)
                .execute(&mut RecordingHost::default()),
            "different SystemState owner",
        ),
        (
            definition::<SchemaSwitchModel>(serde_json::Value::Null)
                .execute(&mut RecordingHost::default()),
            "changed its state schema",
        ),
        (
            definition::<NonAdvancingModel>(serde_json::Value::Null)
                .execute(&mut RecordingHost::default()),
            "did not advance iteration",
        ),
    ] {
        assert!(result.unwrap_err().to_string().contains(expected));
    }
}

#[test]
fn execution_rejects_invalid_target_progression() {
    for (mode, expected) in [
        ("before", "before current iteration"),
        ("decrease", "target iteration decreased"),
        ("disappear", "target iteration disappeared"),
    ] {
        let result = definition::<InvalidTargetModel>(serde_json::json!({"mode":mode}))
            .execute(&mut RecordingHost::default());
        assert!(result.unwrap_err().to_string().contains(expected));
    }
}

#[test]
fn failed_steps_publish_no_successful_step_or_final_observation() {
    let mut host = RecordingHost::default();
    let result = definition::<FailingStepModel>(serde_json::Value::Null).execute(&mut host);

    assert_eq!(result.unwrap_err().to_string(), "step failed");
    assert_eq!(host.events, [Event::Begin(0, None)]);
}

#[test]
fn program_execution_uses_task_owned_semantic_invocation_view() {
    let definition = ProgramDefinition::new(ResolvedProgramTask::for_python(
        PathBuf::from("/resolved/python"),
        [OsString::from("script.py"), OsString::from("--plot")].into(),
        None,
        PathBuf::from("/project/script.py"),
        "system".into(),
    ));
    let mut host = ProgramInvocationHost::default();

    definition.execute(&mut host).unwrap();

    assert!(host.executed);
}
