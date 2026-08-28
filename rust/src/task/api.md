# Task API

The `task` subsystem owns one uniform unit of scientific workload. A task is
either a registered stateful Rust model combined with one config-owned
parameter combination,
or a resolved external executable. Python declarations are resolved into the
same executable boundary. Users never construct Rust `Task` values; they
implement and register models or declare programs/Python in
`wf_configs/study.json`.

Task owns model-contract enforcement and observation boundaries. It does not
own manifest parsing, model-key matching, phase membership, identities, labels,
paths, scheduling, durable format, messages, UI, or lifecycle policy.

Stateful execution is fixed:

1. Config retains one complete resolved JSON value selected from
   `wf_configs/parameters.json[model-key]`;
2. Study decodes one `M::Constants` preflight instance, resolves the model
   task's explicit `state` key, calls `M::observation_plan(&constants)`, binds
   the result to that named schema, and drops the instance;
3. during active execution, Task independently decodes an equivalent owned
   `M::Constants` from the same immutable resolved JSON value;
4. `M::initialize(constants, schema)` creates the canonical model from that
   runtime instance and the same task-bound schema;
5. Task verifies stable state ownership/schema and target iteration;
6. Runtime observes the initial state automatically;
7. Task calls `step` until `is_complete` or cooperative cancellation;
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

“Directly own” means the implementation contains an ordinary `SystemState`
field and returns `&self.state`; the registration attribute does not create a
hidden state, proxy, wrapper, or accessor. Model code uses `payload[_mut]` for
one field and `borrow_payloads[_mut]::<(T1, T2, ...)>(names)` for typed coupled
access. Those calls are normal generic Rust methods and require no invocation
of a field-access macro.

Associated type:

- `Constants: DeserializeOwned + 'static` is one complete typed value supplied
  only by Config. It need not be `Send` or `Sync`: each owned decode is created
  and consumed on the thread performing preflight or execution and is never
  transferred or shared. It should normally use
  `#[serde(deny_unknown_fields)]` so misspelled scientific settings fail during
  Study preflight. Custom deserialization must be deterministic and
  side-effect-free because preflight and execution decode equivalent instances
  independently from the same retained JSON value.

Methods:

- `observation_plan(&Constants) -> TaskResult<ObservationPlan>` declares
  scientifically meaningful observation streams. Its default is
  `ObservationPlan::all_fields()`. Study invokes it once during effect-free
  preflight, binds it to the model task's selected named state schema, and
  retains that exact result for Runtime. It must not perform external side
  effects or retain the preflight constants borrow.
- `initialize(Constants, &SystemStateSchema) -> TaskResult<Self>` consumes a
  fresh owned decode equivalent to the preflight constants and borrows the
  shared validated schema. It must return a fully initialized model directly
  owning a state created from that schema.
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
`wf_configs/study.json` and the matching top-level section of
`wf_configs/parameters.json`.
It is independent of the task's required `state` key, so models may share a
schema and model/state names need not match.
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
executable or nested Python environment in `wf_configs/study.json`; Config resolves it,
Study places it in the same phase graph as model tasks, and Runtime invokes it
through task's private execution-host port.
Its arguments are passed directly without a shell. The program receives the
captured central configuration and completed dependency facts through files
and environment variables documented by `runtime::api`; task itself performs
no filesystem or process operation.

### Crate-visible peer API

Task's closed Study/Runtime boundary is explicitly exported through
`task::advanced`:

- `ModelCatalog` and `ModelCatalogError` provide deterministic linked-model
  discovery and validation to Study.
- `Task` is the clone-cheap compiled workload. Study constructs it from fully
  resolved Config facts and Runtime sees it only through Study's narrower view.
- `ModelTaskProvenance` borrows the semantic model name, selected state key,
  parameter ordinal/source, and resolved constants. It exposes no descriptor
  enum or Config document representation.
- `TaskDefinition` is the type-erased execution command and
  `TaskExecutionHost` is its model/program lifecycle port. Their methods carry
  only facts needed at execution boundaries.
- `ProgramTaskInvocation` is the borrowed semantic launch view passed through
  that host port. Runtime receives executable, arguments, kind, and optional
  Python provenance without importing Config's `ResolvedProgramTask`.
- program summary accessors expose resolved path, semantic program kind, and
  optional Python script without returning a `ResolvedProgramTask` to Runtime.

`ModelRegistration` remains publicly nameable only for procedural-macro
expansion. Every other item above is `pub(crate)` and is a subsystem replacement
contract, not an application extension point.

## Example

This example shows the complete ordinary model surface. After it, the user
writes JSON and calls `run`; no Task value is constructed. The state's time
stores current progress. The separate `target_iteration` field stores the
configured stopping condition and is not another mutable iteration counter.

```rust,no_run
use serde::Deserialize;
use scientific_workflow::prelude::basic::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Constants { initial: u64, steps: u64 }

struct Population {
    state: SystemState,
    target_iteration: u64,
}

#[scientific_workflow::model("population")]
impl ScientificModel for Population {
    type Constants = Constants;

    fn initialize(constants: Constants, schema: &SystemStateSchema) -> TaskResult<Self> {
        let mut state = schema.create_empty_state(StateTime::from_iteration(0));
        state.initialize_payload("population", constants.initial)?;
        state.initialize_payload("cumulative_births", 0_u64)?;
        Ok(Self {
            state,
            target_iteration: constants.steps,
        })
    }

    fn state(&self) -> &SystemState { &self.state }
    fn is_complete(&self) -> bool {
        self.state.time().iteration() >= self.target_iteration
    }
    fn target_iteration(&self) -> Option<u64> { Some(self.target_iteration) }

    fn step(&mut self) -> TaskResult {
        let (population, cumulative_births) = self
            .state
            .borrow_payloads_mut::<(u64, u64)>(
                ("population", "cumulative_births"),
            )?;
        *population += 1;
        *cumulative_births += 1;
        self.state.advance_time(None)?;
        Ok(())
    }
}
```

With `wf_configs/study.json` referring to model `population` and explicitly selecting its
named state schema, `main` is:

```rust,no_run
fn main() -> Result<(), scientific_workflow::WorkflowError> {
    scientific_workflow::run(std::path::Path::new("."))
}
```

## Not API

`StatefulDefinition`, registration lookup, function pointers, model-contract
errors, state-address tracking, target-progress validation,
`ProgramDefinition`, executable dispatch, descriptor storage, and concrete
execution loops are private. `ModelCatalog`, `Task`, `ModelTaskProvenance`,
`ProgramTaskInvocation`, `TaskDefinition`, and `TaskExecutionHost` are closed
crate-visible peer API; their representation is not. Hidden
crate `__private` re-exports exist solely for macro expansion and are not a
supported API. `ModelRegistration::new` is publicly nameable only because a
procedural macro expanded in a downstream crate must call it; it is not a
manual registration or catalog-construction seam.

Replacement task implementations must preserve model-key parameter selection,
config-owned decode, direct
state ownership checks, explicit named-schema selection, deterministic
observation-plan binding, observation
ordering, generic model/program dispatch, and runtime-owned
cancellation/lifecycle. Programs and Python scripts must remain declarative
tasks rather than requiring a Rust wrapper or public registration API.
