# Task API

The `task` subsystem owns the irreducible bridge between typed application
science and uniform Workflow execution. Users define registered
`ScientificModel` implementations. Study combines a registration with one
config-owned resolved input to create an internal task; users never construct
tasks, pass constants, or coordinate recording themselves.

Task owns model-contract enforcement and observation boundaries. It does not
own manifest parsing, model-key matching, phase membership, identities, labels,
paths, scheduling, durable format, messages, UI, or lifecycle policy.

Stateful execution is fixed:

1. config-owned `ResolvedTaskInput` decodes one complete `M::Constants`;
2. `M::writer(&constants)` returns the deterministic writer definition;
3. task binds that writer to runtime's prevalidated schema;
4. `M::initialize(constants, schema)` creates the canonical model;
5. task verifies stable state ownership/schema and target iteration;
6. runtime observes the initial state automatically;
7. task calls `step` until `is_complete` or cooperative cancellation;
8. each successful step must strictly advance iteration and is observed;
9. the final state is observed/completed exactly once.

## Basic API

### `task::basic::ScientificModel`

`ScientificModel` is a `Send + Sized + 'static` trait implemented by each
canonical application model. The model **must directly own** the `SystemState`
returned by `state()` for its entire execution. Rust cannot express field-level
ownership in a trait, so task enforces observable consequences: stable state
address, exact schema allocation, strictly advancing successful steps, and
valid target progression.

Associated type:

- `Constants: DeserializeOwned + Send + Sync + 'static` is one complete typed
  value supplied only by config. It should normally use
  `#[serde(deny_unknown_fields)]` so misspelled scientific settings fail during
  Study preflight.

Methods:

- `writer(&Constants) -> TaskResult<Writer>` declares scientifically meaningful
  observation streams. Its default is `Writer::all_fields()`. Study invokes it
  during effect-free preflight and task invokes it again at execution startup;
  therefore it must be deterministic, pure with respect to external state, and
  must not retain the constants borrow.
- `initialize(Constants, &SystemStateSchema) -> TaskResult<Self>` consumes the
  exact typed constants and borrows the shared validated schema. It must return
  a fully initialized model directly owning a state created from that schema.
  It runs only during active execution, so model setup errors may occur after
  output scope creation but before the model's recording opens.
- `state(&self) -> &SystemState` is side-effect-free and must always return the
  same owner/schema allocation. Returning a temporary, indirection that can be
  swapped, or another schema violates the contract.
- `is_complete(&self) -> bool` is side-effect-free. Once true, Workflow performs
  final observation without another step.
- `step(&mut self) -> TaskResult` performs exactly one observable scientific
  transition. On success it must strictly advance state iteration. On error,
  task emits no successful-step observation and runtime marks any opened
  recording failed.
- `target_iteration(&self) -> Option<u64>` defaults to unknown. A present target
  must not precede current iteration; once present, it cannot decrease or
  disappear. Runtime uses it for inferred progress, never as the completion
  authority.

Models receive no path, task identity, phase, replicate, recording session,
generic context, progress counter, shared atomic, message callback, or
cancellation token. Cancellation is checked between steps by the adapter.

### `task::basic::TaskResult<T = ()>`

Alias for:

```rust,ignore
Result<T, Box<dyn std::error::Error + Send + Sync + 'static>>
```

It lets application-specific error enums preserve owned source chains across
runtime worker boundaries. Any compatible error can be converted with `?`.
TaskResult carries no status/progress/message fields.

### `scientific_workflow::model` attribute

The model attribute is re-exported at crate root and through `prelude::basic`:

```rust,ignore
#[scientific_workflow::model("population")]
impl ScientificModel for PopulationModel { ... }
```

The nonempty, whitespace-exact string is the stable semantic key used by
`study.json`. It is deliberately not inferred from `type_name`, module paths,
or source order because those are refactor-unstable. The macro preserves the
impl and submits an immutable `ModelRegistration`. Duplicate/invalid keys fail
during Study preflight before output. Linked registration order is never used;
the catalog sorts keys.

The attribute performs no runtime work by itself and creates no mutable global
registry. A model must be linked into the final executable for its registration
to be discoverable.

## Advanced API

Advanced re-exports every Basic symbol and adds the supported Study/runtime
integration boundary.

### `task::advanced::ModelRegistration`

A `Copy` immutable association containing a stable `&'static str` key plus
model-specific task-construction and pure-preflight function pointers.

- `ModelRegistration::new::<M>(key: &'static str)` constructs a registration
  without initializing `M`, parsing files, or allocating. Ordinary code uses
  the attribute; explicit construction is for catalogs in tests/embedders.
- `key() -> &'static str` returns the authored semantic key.

Private function pointers are intentionally not exposed. Registration does not
own mutable state and is thread-safe.

### `task::advanced::ModelCatalog`

An immutable, cloneable `BTreeMap`-backed catalog.

- `discovered()` collects linked attribute registrations, validates every key,
  rejects duplicates, and sorts by key independent of linker order.
- `from_registrations(iter)` builds the same catalog explicitly. The iterator
  yields `ModelRegistration` by value; the catalog owns its map afterward.
- `keys()` returns an exact-size lexical-order iterator of static keys.

The lookup operation used by Study is crate-private so applications cannot
partially reproduce task binding. Catalog creation has no filesystem,
initialization, or output effects.

### `task::advanced::ModelCatalogError`

Non-exhaustive validation error:

- `InvalidKey { key }`: empty or surrounding-whitespace key;
- `DuplicateKey { key }`: more than one compiled model claims the same key.

Failure publishes no partial catalog.

### `task::advanced::Task`

Opaque `Clone + Send + Sync + 'static` type-erased model definition. Cloning
increments an `Arc`; it does not clone models, states, constants, or writers.
`Debug` is bounded and does not reveal captures.

- `Task::for_model::<M>()` constructs a definition for an already selected
  model type. It performs no decode, initialization, or IO. Ordinary users do
  not call it; registration/Study use it automatically. It exists for
  replacement Study compilers and focused integrations.

Task exposes no public execution method; execution is available only through
the `TaskDefinition` trait.

### `task::advanced::TaskDefinition`

A `Send + Sync` type-erased runtime port:

- `execute(&ResolvedTaskInput, &mut dyn TaskExecutionHost) -> TaskResult`
  obtains typed constants through config and runs the complete model lifecycle.

Replacement runtimes may invoke it. Applications should not implement it.
Returning `Ok` after host cancellation means cooperative cancellation, not
successful completion; runtime owns that distinction.

### `task::advanced::TaskExecutionHost`

Runtime-owned service port with call-scoped borrows:

- `state_schema() -> TaskResult<&SystemStateSchema>` returns the exact shared
  schema;
- `cancellation_requested() -> bool` is checked before initialization and
  between steps;
- `begin_model(Writer, &SystemState, Option<u64>)` accepts the validated writer,
  initial state, and target hint;
- `observe_model_step(&SystemState, Option<u64>)` handles each successful
  transition; and
- `observe_model_final(&SystemState, Option<u64>)` commits the final boundary.

The host may block for storage backpressure. State borrows last only for each
call and may not be retained. The task module never exposes concrete host,
storage, cancellation, or UI implementations.

## Example

This example shows the complete ordinary model surface. After it, the user
writes JSON and calls `run`; no Task value is constructed.

```rust,no_run
use serde::Deserialize;
use scientific_workflow::prelude::basic::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Constants { initial: u64, steps: u64 }

struct Population {
    state: SystemState,
    steps: u64,
}

#[scientific_workflow::model("population")]
impl ScientificModel for Population {
    type Constants = Constants;

    fn initialize(constants: Constants, schema: &SystemStateSchema) -> TaskResult<Self> {
        let mut state = schema.create_empty_state(StateTime::from_iteration(0));
        state.initialize_payload("population", constants.initial)?;
        Ok(Self { state, steps: constants.steps })
    }

    fn state(&self) -> &SystemState { &self.state }
    fn is_complete(&self) -> bool { self.state.time().iteration() == self.steps }
    fn target_iteration(&self) -> Option<u64> { Some(self.steps) }

    fn step(&mut self) -> TaskResult {
        *self.state.payload_mut::<u64>("population")? += 1;
        self.state.advance_time(None)?;
        Ok(())
    }
}
```

With `study.json` referring to model `population`, `main` is:

```rust,no_run
fn main() -> Result<(), scientific_workflow::WorkflowError> {
    scientific_workflow::run(std::path::Path::new("."))
}
```

## Not API

`StatefulDefinition`, registration lookup, function pointers, model contract
error variants, state-address tracking, target-progress validation, and concrete
execution loops are private. The hidden crate `__private::inventory` re-export
exists solely for macro expansion and is not a supported API.

Replacement task implementations must preserve config-owned decode, direct
state ownership checks, deterministic writer binding, observation ordering,
and runtime-owned cancellation/lifecycle.
