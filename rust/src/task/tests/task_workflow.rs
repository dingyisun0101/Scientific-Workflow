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
    ModelInitialization, ProgramDefinition, ProgramTaskInvocation, StatefulDefinition,
    TaskDefinition, TaskExecutionHost,
};
use super::result::TaskResult;
use super::unit::{ExecutionUnit, InitializationContext, ModelView};

fn test_initialization_context() -> &'static InitializationContext {
    static CONTEXT: std::sync::OnceLock<InitializationContext> = std::sync::OnceLock::new();
    CONTEXT.get_or_init(|| InitializationContext::new(Some(7), 1, "test-task", "test-unit"))
}

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
    events: Vec<(usize, Event)>,
    identities: Vec<Box<str>>,
}

#[derive(Default)]
struct ProgramInvocationHost {
    executed: bool,
}

impl TaskExecutionHost for ProgramInvocationHost {
    fn cancellation_requested(&self) -> bool {
        false
    }

    fn initialization_context(&self) -> Option<&InitializationContext> {
        None
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

    fn begin_model(&mut self, _model: ModelInitialization<'_>) -> TaskResult {
        panic!("program contract tests do not initialize models")
    }

    fn observe_model_step(
        &mut self,
        _index: usize,
        _state: &SystemState,
        _target_iteration: Option<u64>,
    ) -> TaskResult {
        panic!("program contract tests do not observe models")
    }

    fn observe_model_final(
        &mut self,
        _index: usize,
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

    fn initialization_context(&self) -> Option<&InitializationContext> {
        Some(test_initialization_context())
    }

    fn execute_program(&mut self, _program: ProgramTaskInvocation<'_>) -> TaskResult {
        panic!("model contract tests do not execute programs")
    }

    fn begin_model(&mut self, model: ModelInitialization<'_>) -> TaskResult {
        self.identities.push(model.identity.into());
        self.events.push((
            model.index,
            Event::Begin(model.state.time().iteration(), model.target_iteration),
        ));
        Ok(())
    }

    fn observe_model_step(
        &mut self,
        _index: usize,
        state: &SystemState,
        target_iteration: Option<u64>,
    ) -> TaskResult {
        self.events.push((
            _index,
            Event::Step(state.time().iteration(), target_iteration),
        ));
        if self.cancel_after_step {
            self.cancelled.set(true);
        }
        Ok(())
    }

    fn observe_model_final(
        &mut self,
        _index: usize,
        state: &SystemState,
        target_iteration: Option<u64>,
    ) -> TaskResult {
        self.events.push((
            _index,
            Event::Final(state.time().iteration(), target_iteration),
        ));
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
    M: ExecutionUnit,
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

impl ExecutionUnit for CountingModel {
    type Constants = LocalConstants;

    fn initialize(
        constants: Self::Constants,
        schema: &SystemStateSchema,
        _context: &InitializationContext,
    ) -> TaskResult<Self> {
        let mut state = schema.create_empty_state(StateTime::from_iteration(0));
        state.initialize_payload("value", 0_u64)?;
        Ok(Self {
            state,
            steps: constants.steps,
        })
    }

    fn model_count(&self) -> usize {
        1
    }

    fn model(&self, index: usize) -> Option<ModelView<'_>> {
        (index == 0).then(|| {
            ModelView::new(
                "counting",
                &self.state,
                self.state.time().iteration() >= self.steps,
                Some(self.steps),
            )
        })
    }

    fn step(&mut self) -> TaskResult {
        *self.state.payload_mut::<u64>("value")? += 1;
        self.state.advance_time(None)?;
        Ok(())
    }
}

struct PanicOnInitialize;

impl ExecutionUnit for PanicOnInitialize {
    type Constants = ();

    fn initialize(
        _constants: Self::Constants,
        _schema: &SystemStateSchema,
        _context: &InitializationContext,
    ) -> TaskResult<Self> {
        panic!("initialization must not run after pre-execution cancellation")
    }

    fn model_count(&self) -> usize {
        unreachable!()
    }

    fn model(&self, _index: usize) -> Option<ModelView<'_>> {
        unreachable!()
    }

    fn step(&mut self) -> TaskResult {
        unreachable!()
    }
}

struct NonAdvancingModel {
    state: SystemState,
}

impl ExecutionUnit for NonAdvancingModel {
    type Constants = ();

    fn initialize(
        _constants: Self::Constants,
        schema: &SystemStateSchema,
        _context: &InitializationContext,
    ) -> TaskResult<Self> {
        let mut state = schema.create_empty_state(StateTime::from_iteration(0));
        state.initialize_payload("value", 0_u64)?;
        Ok(Self { state })
    }

    fn model_count(&self) -> usize {
        1
    }

    fn model(&self, index: usize) -> Option<ModelView<'_>> {
        (index == 0).then(|| ModelView::new("nonadvancing", &self.state, false, None))
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

impl ExecutionUnit for OwnerSwitchModel {
    type Constants = ();

    fn initialize(
        _constants: Self::Constants,
        schema: &SystemStateSchema,
        _context: &InitializationContext,
    ) -> TaskResult<Self> {
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

    fn model_count(&self) -> usize {
        1
    }

    fn model(&self, index: usize) -> Option<ModelView<'_>> {
        (index == 0).then(|| {
            let state = if self.use_second {
                &self.second
            } else {
                &self.first
            };
            ModelView::new("owner-switch", state, false, None)
        })
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

impl ExecutionUnit for SchemaSwitchModel {
    type Constants = ();

    fn initialize(
        _constants: Self::Constants,
        schema: &SystemStateSchema,
        _context: &InitializationContext,
    ) -> TaskResult<Self> {
        let mut state = schema.create_empty_state(StateTime::from_iteration(0));
        state.initialize_payload("value", 0_u64)?;
        Ok(Self { state })
    }

    fn model_count(&self) -> usize {
        1
    }

    fn model(&self, index: usize) -> Option<ModelView<'_>> {
        (index == 0).then(|| ModelView::new("schema-switch", &self.state, false, None))
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

impl ExecutionUnit for InvalidTargetModel {
    type Constants = TargetConstants;

    fn initialize(
        constants: Self::Constants,
        schema: &SystemStateSchema,
        _context: &InitializationContext,
    ) -> TaskResult<Self> {
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

    fn model_count(&self) -> usize {
        1
    }

    fn model(&self, index: usize) -> Option<ModelView<'_>> {
        (index == 0).then(|| ModelView::new("invalid-target", &self.state, false, self.target))
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
}

struct FailingStepModel {
    state: SystemState,
}

struct PairedUnit {
    states: Vec<SystemState>,
    targets: [u64; 2],
}

impl ExecutionUnit for PairedUnit {
    type Constants = ();

    fn initialize(
        _constants: Self::Constants,
        schema: &SystemStateSchema,
        _context: &InitializationContext,
    ) -> TaskResult<Self> {
        let mut states = Vec::with_capacity(2);
        for _ in 0..2 {
            let mut state = schema.create_empty_state(StateTime::from_iteration(0));
            state.initialize_payload("value", 0_u64)?;
            states.push(state);
        }
        Ok(Self {
            states,
            targets: [1, 2],
        })
    }

    fn model_count(&self) -> usize {
        self.states.len()
    }

    fn model(&self, index: usize) -> Option<ModelView<'_>> {
        let state = self.states.get(index)?;
        let target = self.targets[index];
        Some(ModelView::new(
            ["short", "long"][index],
            state,
            state.time().iteration() >= target,
            Some(target),
        ))
    }

    fn step(&mut self) -> TaskResult {
        for (state, target) in self.states.iter_mut().zip(self.targets) {
            if state.time().iteration() < target {
                *state.payload_mut::<u64>("value")? += 1;
                state.advance_time(None)?;
            }
        }
        Ok(())
    }
}

impl ExecutionUnit for FailingStepModel {
    type Constants = ();

    fn initialize(
        _constants: Self::Constants,
        schema: &SystemStateSchema,
        _context: &InitializationContext,
    ) -> TaskResult<Self> {
        let mut state = schema.create_empty_state(StateTime::from_iteration(0));
        state.initialize_payload("value", 0_u64)?;
        Ok(Self { state })
    }

    fn model_count(&self) -> usize {
        1
    }

    fn model(&self, index: usize) -> Option<ModelView<'_>> {
        (index == 0).then(|| ModelView::new("failing", &self.state, false, None))
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
            (0, Event::Begin(0, Some(2))),
            (0, Event::Step(1, Some(2))),
            (0, Event::Step(2, Some(2))),
            (0, Event::Final(2, Some(2))),
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
        [(0, Event::Begin(0, Some(3))), (0, Event::Step(1, Some(3)))]
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
            "did not advance any incomplete model",
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
    assert_eq!(host.events, [(0, Event::Begin(0, None))]);
}

#[test]
fn execution_manages_an_ensemble_as_one_unit_with_independent_model_lifecycles() {
    let mut host = RecordingHost::default();
    definition::<PairedUnit>(serde_json::Value::Null)
        .execute(&mut host)
        .unwrap();

    assert_eq!(
        host.identities.iter().map(Box::as_ref).collect::<Vec<_>>(),
        ["short", "long"]
    );
    assert_eq!(
        host.events,
        [
            (0, Event::Begin(0, Some(1))),
            (1, Event::Begin(0, Some(2))),
            (0, Event::Step(1, Some(1))),
            (0, Event::Final(1, Some(1))),
            (1, Event::Step(1, Some(2))),
            (1, Event::Step(2, Some(2))),
            (1, Event::Final(2, Some(2))),
        ]
    );
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
