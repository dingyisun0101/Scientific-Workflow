# Task API

The `task` subsystem owns one uniform unit of scientific workload. A task is
either a registered stateful Rust model combined with one config-owned
parameter combination,
or a resolved external executable. Python declarations are resolved into the
same executable boundary. Users never construct Rust `Task` values; they
implement and register models or declare programs/Python in `study.json`.

Task owns model-contract enforcement and observation boundaries. It does not
own manifest parsing, model-key matching, phase membership, identities, labels,
paths, scheduling, durable format, messages, UI, or lifecycle policy.

Stateful execution is fixed:

1. config-owned `ResolvedModelParameters` decodes one complete `M::Constants`
   from `parameters.json[model-key]`;
2. Study calls `M::observation_plan(&constants)` and stores its schema-bound result;
3. `M::initialize(constants, schema)` creates the canonical model at execution;
4. task verifies stable state ownership/schema and target iteration;
5. runtime observes the initial state automatically;
6. task calls `step` until `is_complete` or cooperative cancellation;
7. each successful step must strictly advance iteration and is observed;
8. the final state is observed/completed exactly once.

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

- `observation_plan(&Constants) -> TaskResult<ObservationPlan>` declares
  scientifically meaningful observation streams. Its default is
  `ObservationPlan::all_fields()`. Study invokes it once during effect-free
  preflight, binds it to the state schema, and retains that exact result for
  runtime. It must not perform external side effects or retain the constants
  borrow.
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
`study.json` and the matching top-level section of `config/parameters.json`.
It is deliberately not inferred from `type_name`, module paths, or source order
because those are refactor-unstable. The macro preserves the impl and submits
only hidden immutable registration metadata. Duplicate or invalid keys fail
during Study preflight before output. Linked registration order is never used;
the internal catalog sorts keys.

The attribute performs no runtime work, scheduling, persistence, progress, or
UI rendering and creates no mutable global registry. A model must be linked
into the final executable for its registration to be discoverable.

## Advanced API

`task::advanced` is the strict public superset of Basic and currently adds no
supported public symbols. Workflow peers use crate-visible catalog,
type-erased task, and execution-host ports through this scope. The procedural
macro reaches registration metadata only through the crate's documentation-hidden
`__private` expansion namespace.

Program and Python tasks add no Rust export to either tier. A user declares an
executable or nested Python environment in `study.json`; Config resolves it,
Study places it in the same phase graph as model tasks, and Runtime invokes it
through task's private execution-host port.
Its arguments are passed directly without a shell. The program receives the
captured central configuration and completed dependency facts through files
and environment variables documented by `runtime::api`; task itself performs
no filesystem or process operation.

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

`ModelRegistration`, `ModelCatalog`, `ModelCatalogError`, `Task`,
`TaskDefinition`, `TaskExecutionHost`, `StatefulDefinition`, registration
lookup, function pointers, model-contract errors, state-address tracking,
target-progress validation, `ProgramDefinition`, executable dispatch, and
concrete execution loops are private. Hidden
crate `__private` re-exports exist solely for macro expansion and are not a
supported API.

Replacement task implementations must preserve model-key parameter selection,
config-owned decode, direct
state ownership checks, deterministic observation-plan binding, observation
ordering, generic model/program dispatch, and runtime-owned
cancellation/lifecycle. Programs and Python scripts must remain declarative
tasks rather than requiring a Rust wrapper or public registration API.
