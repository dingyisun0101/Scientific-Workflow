//! Minimal application task definitions and the replaceable runtime port.

use std::cell::Cell;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use scientific_workflow::prelude::advanced::*;
use serde::Deserialize;

static NEXT_PROJECT: AtomicUsize = AtomicUsize::new(0);

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/state.json")
}

fn schema() -> SystemStateSchema {
    SystemStateSchema::load_json_template(&fixture_path()).unwrap()
}

fn resolved_input(value: serde_json::Value) -> ResolvedTaskInput {
    let sequence = NEXT_PROJECT.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "scientific-workflow-task-input-{}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(root.join("config/inputs")).unwrap();
    fs::copy(fixture_path(), root.join("config/state.json")).unwrap();
    fs::write(
        root.join("study.json"),
        serde_json::to_vec(&serde_json::json!({
            "phases": {
                "test": {
                    "tasks": [{"definition": "test", "input": "inputs/test.json"}]
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();
    fs::write(
        root.join("config/inputs/test.json"),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    let specification = ProjectSpecification::load(&root).unwrap();
    let input = specification.phases()[0].tasks()[0].clone();
    fs::remove_dir_all(root).unwrap();
    input
}

#[derive(Clone, Copy, Debug, Deserialize)]
struct CountdownConstants {
    initial: u64,
    steps: u64,
}

struct CountdownModel {
    state: SystemState,
    remaining: u64,
    target: u64,
}

impl ScientificModel for CountdownModel {
    type Constants = CountdownConstants;

    fn initialize(constants: Self::Constants, schema: &SystemStateSchema) -> TaskResult<Self> {
        let mut state = schema.create_empty_state(StateTime::from_iteration(0));
        state.initialize_payload("population", vec![constants.initial])?;
        state.initialize_payload("space", vec![0_u64])?;
        state.initialize_payload("activity", true)?;
        Ok(Self {
            state,
            remaining: constants.steps,
            target: constants.steps,
        })
    }

    fn state(&self) -> &SystemState {
        &self.state
    }

    fn is_complete(&self) -> bool {
        self.remaining == 0
    }

    fn step(&mut self) -> TaskResult {
        self.state.payload_mut::<Vec<u64>>("population")?[0] += 1;
        self.state.advance_time(None)?;
        self.remaining -= 1;
        Ok(())
    }

    fn target_iteration(&self) -> Option<u64> {
        Some(self.target)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EventKind {
    Begin,
    Step,
    Final,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Event {
    kind: EventKind,
    iteration: u64,
    target: Option<u64>,
}

struct MemoryHost {
    schema: SystemStateSchema,
    schema_requests: Cell<usize>,
    events: Vec<Event>,
    cancel_before_start: bool,
    cancel_after_steps: Option<usize>,
}

impl MemoryHost {
    fn new(schema: SystemStateSchema) -> Self {
        Self {
            schema,
            schema_requests: Cell::new(0),
            events: Vec::new(),
            cancel_before_start: false,
            cancel_after_steps: None,
        }
    }

    fn record(&mut self, kind: EventKind, state: &SystemState, target: Option<u64>) -> TaskResult {
        self.events.push(Event {
            kind,
            iteration: state.time().iteration(),
            target,
        });
        Ok(())
    }
}

impl TaskExecutionHost for MemoryHost {
    fn state_schema(&self) -> TaskResult<&SystemStateSchema> {
        self.schema_requests.set(self.schema_requests.get() + 1);
        Ok(&self.schema)
    }

    fn cancellation_requested(&self) -> bool {
        self.cancel_before_start
            || self.cancel_after_steps.is_some_and(|limit| {
                self.events
                    .iter()
                    .filter(|event| event.kind == EventKind::Step)
                    .count()
                    >= limit
            })
    }

    fn begin_model(
        &mut self,
        _writer: Writer,
        state: &SystemState,
        target_iteration: Option<u64>,
    ) -> TaskResult {
        self.record(EventKind::Begin, state, target_iteration)
    }

    fn observe_model_step(
        &mut self,
        state: &SystemState,
        target_iteration: Option<u64>,
    ) -> TaskResult {
        self.record(EventKind::Step, state, target_iteration)
    }

    fn observe_model_final(
        &mut self,
        state: &SystemState,
        target_iteration: Option<u64>,
    ) -> TaskResult {
        self.record(EventKind::Final, state, target_iteration)
    }
}

#[test]
fn stateful_task_decodes_constants_and_automates_every_observation_boundary() {
    let task = Task::stateful::<CountdownModel, _>(|constants| {
        assert_eq!(constants.initial, 10);
        Writer::fields(["population"]).map_err(Into::into)
    });
    let mut host = MemoryHost::new(schema());

    task.execute(
        &resolved_input(serde_json::json!({"initial": 10, "steps": 2})),
        &mut host,
    )
    .unwrap();

    assert_eq!(task.descriptor().kind(), TaskKind::Stateful);
    assert!(task.descriptor().requires_state_schema());
    assert!(
        task.descriptor()
            .constants_type_name()
            .ends_with("CountdownConstants")
    );
    assert_eq!(host.schema_requests.get(), 1);
    assert_eq!(
        host.events,
        [
            Event {
                kind: EventKind::Begin,
                iteration: 0,
                target: Some(2),
            },
            Event {
                kind: EventKind::Step,
                iteration: 1,
                target: Some(2),
            },
            Event {
                kind: EventKind::Step,
                iteration: 2,
                target: Some(2),
            },
            Event {
                kind: EventKind::Final,
                iteration: 2,
                target: Some(2),
            },
        ]
    );
}

#[test]
fn one_shot_decodes_typed_input_without_requesting_state_or_writer() {
    #[derive(Deserialize)]
    struct Input {
        value: usize,
    }

    static OBSERVED: AtomicUsize = AtomicUsize::new(0);
    let task = Task::one_shot::<Input, _>(|input| {
        OBSERVED.store(input.value, Ordering::SeqCst);
        Ok(())
    });
    let mut host = MemoryHost::new(schema());
    task.execute(&resolved_input(serde_json::json!({"value": 17})), &mut host)
        .unwrap();

    assert_eq!(OBSERVED.load(Ordering::SeqCst), 17);
    assert_eq!(task.descriptor().kind(), TaskKind::OneShot);
    assert!(!task.descriptor().requires_state_schema());
    assert_eq!(host.schema_requests.get(), 0);
    assert!(host.events.is_empty());
}

#[test]
fn decoding_and_writer_validation_fail_before_model_initialization() {
    static INITIALIZATIONS: AtomicUsize = AtomicUsize::new(0);

    struct CountedModel(CountdownModel);

    impl ScientificModel for CountedModel {
        type Constants = CountdownConstants;

        fn initialize(constants: Self::Constants, schema: &SystemStateSchema) -> TaskResult<Self> {
            INITIALIZATIONS.fetch_add(1, Ordering::SeqCst);
            Ok(Self(CountdownModel::initialize(constants, schema)?))
        }

        fn state(&self) -> &SystemState {
            self.0.state()
        }

        fn is_complete(&self) -> bool {
            self.0.is_complete()
        }

        fn step(&mut self) -> TaskResult {
            self.0.step()
        }
    }

    INITIALIZATIONS.store(0, Ordering::SeqCst);
    let invalid_writer =
        Task::stateful::<CountedModel, _>(|_| Writer::fields(["not_declared"]).map_err(Into::into));
    let mut host = MemoryHost::new(schema());
    assert!(
        invalid_writer
            .execute(
                &resolved_input(serde_json::json!({"initial": 1, "steps": 1})),
                &mut host,
            )
            .is_err()
    );
    assert_eq!(INITIALIZATIONS.load(Ordering::SeqCst), 0);
    assert!(host.events.is_empty());

    let valid_writer = Task::stateful::<CountedModel, _>(|_| Ok(Writer::all_fields()));
    assert!(
        valid_writer
            .execute(
                &resolved_input(serde_json::json!({"initial": "wrong"})),
                &mut host,
            )
            .is_err()
    );
    assert_eq!(INITIALIZATIONS.load(Ordering::SeqCst), 0);
}

#[test]
fn cancellation_is_checked_before_start_and_between_successful_steps() {
    let task = Task::stateful::<CountdownModel, _>(|_| Ok(Writer::all_fields()));
    let mut before = MemoryHost::new(schema());
    before.cancel_before_start = true;
    task.execute(
        &resolved_input(serde_json::json!({"initial": 0, "steps": 3})),
        &mut before,
    )
    .unwrap();
    assert!(before.events.is_empty());

    let mut between = MemoryHost::new(schema());
    between.cancel_after_steps = Some(1);
    task.execute(
        &resolved_input(serde_json::json!({"initial": 0, "steps": 3})),
        &mut between,
    )
    .unwrap();
    assert_eq!(
        between
            .events
            .iter()
            .map(|event| event.kind)
            .collect::<Vec<_>>(),
        [EventKind::Begin, EventKind::Step]
    );
}

struct WrongSchemaModel {
    state: SystemState,
}

impl ScientificModel for WrongSchemaModel {
    type Constants = ();

    fn initialize(_: (), _: &SystemStateSchema) -> TaskResult<Self> {
        let other = schema();
        Ok(Self {
            state: other.create_empty_state(StateTime::from_iteration(0)),
        })
    }

    fn state(&self) -> &SystemState {
        &self.state
    }

    fn is_complete(&self) -> bool {
        true
    }

    fn step(&mut self) -> TaskResult {
        Ok(())
    }
}

struct SwappingModel {
    first: SystemState,
    second: SystemState,
    swapped: bool,
}

impl ScientificModel for SwappingModel {
    type Constants = ();

    fn initialize(_: (), schema: &SystemStateSchema) -> TaskResult<Self> {
        Ok(Self {
            first: schema.create_empty_state(StateTime::from_iteration(0)),
            second: schema.create_empty_state(StateTime::from_iteration(1)),
            swapped: false,
        })
    }

    fn state(&self) -> &SystemState {
        if self.swapped {
            &self.second
        } else {
            &self.first
        }
    }

    fn is_complete(&self) -> bool {
        self.swapped
    }

    fn step(&mut self) -> TaskResult {
        self.swapped = true;
        Ok(())
    }
}

struct StalledModel {
    state: SystemState,
    complete: bool,
}

impl ScientificModel for StalledModel {
    type Constants = ();

    fn initialize(_: (), schema: &SystemStateSchema) -> TaskResult<Self> {
        Ok(Self {
            state: schema.create_empty_state(StateTime::from_iteration(0)),
            complete: false,
        })
    }

    fn state(&self) -> &SystemState {
        &self.state
    }

    fn is_complete(&self) -> bool {
        self.complete
    }

    fn step(&mut self) -> TaskResult {
        self.complete = true;
        Ok(())
    }
}

#[test]
fn runtime_rejects_observable_scientific_model_contract_violations() {
    for task in [
        Task::stateful::<WrongSchemaModel, _>(|_| Ok(Writer::all_fields())),
        Task::stateful::<SwappingModel, _>(|_| Ok(Writer::all_fields())),
        Task::stateful::<StalledModel, _>(|_| Ok(Writer::all_fields())),
    ] {
        let mut host = MemoryHost::new(schema());
        assert!(
            task.execute(&resolved_input(serde_json::json!(null)), &mut host)
                .is_err()
        );
    }
}

struct InvalidTargetModel {
    state: SystemState,
    complete: bool,
}

impl ScientificModel for InvalidTargetModel {
    type Constants = ();

    fn initialize(_: (), schema: &SystemStateSchema) -> TaskResult<Self> {
        Ok(Self {
            state: schema.create_empty_state(StateTime::from_iteration(5)),
            complete: true,
        })
    }

    fn state(&self) -> &SystemState {
        &self.state
    }

    fn is_complete(&self) -> bool {
        self.complete
    }

    fn step(&mut self) -> TaskResult {
        Ok(())
    }

    fn target_iteration(&self) -> Option<u64> {
        Some(4)
    }
}

#[test]
fn target_iterations_cannot_precede_current_state() {
    let task = Task::stateful::<InvalidTargetModel, _>(|_| Ok(Writer::all_fields()));
    let mut host = MemoryHost::new(schema());
    let error = task
        .execute(&resolved_input(serde_json::json!(null)), &mut host)
        .unwrap_err();
    assert!(error.to_string().contains("before current iteration"));
    assert!(host.events.is_empty());
}

#[derive(Deserialize)]
struct ChangingTargetConstants {
    remove: bool,
}

struct ChangingTargetModel {
    state: SystemState,
    remove: bool,
    stepped: bool,
}

impl ScientificModel for ChangingTargetModel {
    type Constants = ChangingTargetConstants;

    fn initialize(constants: Self::Constants, schema: &SystemStateSchema) -> TaskResult<Self> {
        Ok(Self {
            state: schema.create_empty_state(StateTime::from_iteration(0)),
            remove: constants.remove,
            stepped: false,
        })
    }

    fn state(&self) -> &SystemState {
        &self.state
    }

    fn is_complete(&self) -> bool {
        self.stepped
    }

    fn step(&mut self) -> TaskResult {
        self.state.advance_time(None)?;
        self.stepped = true;
        Ok(())
    }

    fn target_iteration(&self) -> Option<u64> {
        if !self.stepped {
            Some(2)
        } else if self.remove {
            None
        } else {
            Some(1)
        }
    }
}

#[test]
fn reported_targets_cannot_decrease_or_disappear() {
    let task = Task::stateful::<ChangingTargetModel, _>(|_| Ok(Writer::all_fields()));

    for remove in [false, true] {
        let mut host = MemoryHost::new(schema());
        let error = task
            .execute(
                &resolved_input(serde_json::json!({"remove": remove})),
                &mut host,
            )
            .unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("decreased") || message.contains("disappeared"),
            "unexpected contract error: {message}"
        );
        assert_eq!(
            host.events
                .iter()
                .map(|event| event.kind)
                .collect::<Vec<_>>(),
            [EventKind::Begin]
        );
    }
}

#[test]
fn task_module_and_central_advanced_scopes_are_strict_basic_supersets() {
    fn accepts_task(_: scientific_workflow::task::advanced::Task) {}
    fn accepts_result(_: scientific_workflow::task::advanced::TaskResult) {}
    fn accepts_descriptor(_: &dyn scientific_workflow::task::advanced::TaskDefinition) {}

    let task = scientific_workflow::task::basic::Task::one_shot::<(), _>(|_| Ok(()));
    accepts_task(task.clone());
    accepts_result(Ok(()));
    accepts_descriptor(&task);

    let central = scientific_workflow::prelude::basic::Task::one_shot::<(), _>(|_| Ok(()));
    let _: scientific_workflow::prelude::advanced::Task = central;
}
