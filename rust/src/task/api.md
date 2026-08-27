# Task API

The `task` subsystem turns heterogeneous compiled application behavior into
one uniform definition that Workflow's runtime can inspect and execute. It is
the boundary that binds resolved typed constants, an application-owned
scientific model, and an application-defined writer. Its canonical public
scopes are `scientific_workflow::task::basic` and
`scientific_workflow::task::advanced`; the central preludes re-export the same
symbols without wrapping them.

Task does **not** own task names, display labels, phase membership, scheduling,
replicate identity, paths, recording layout, durable lifecycle, provenance,
message formatting, or terminal rendering. Those facts are inferred from
validated JSON and the runtime scope. A task definition consequently has no
public ID setter, path setter, progress counter, reporting callback, recording
session, or completion method.

Stateful task execution has one fixed sequence:

1. ask the config-owned `ResolvedTaskInput` to supply the model's declared
   `Constants` type;
2. let the writer factory borrow those constants and produce a `Writer`;
3. obtain the runtime-loaded `config/state.json` schema from the execution
   host;
4. validate the writer against that schema before initializing the model;
5. consume the constants in `ScientificModel::initialize`;
6. validate the model's canonical state owner, schema allocation, and optional
   target iteration;
7. hand the initial state and writer to the host for automatic observation;
8. call `step` until `is_complete` or cooperative cancellation;
9. after each successful step, validate the model contract and ask the host to
   observe the new state and publish its automatic progress snapshot; and
10. ask the host to finalize the completed state boundary exactly once.

The application supplies scientific meaning. The task implementation supplies
deterministic mechanics and rejects observable contract violations.

## Basic API

### `task::basic::Task`

`Task` is an opaque, reusable, type-erased compiled definition. It is
`Clone + Send + Sync + 'static`: cloning increments an internal shared-owner
count and does not clone closures, constants, models, states, or writers.
`Debug` reveals only its read-only descriptor and never formats captured
closure data.

The type exposes two constructors and no public execution method. Ordinary
application code builds tasks; only a runtime importing `task::advanced`
executes them.

#### `Task::stateful::<M, W>(writer)`

Creates a stateful definition for `M: ScientificModel`.

`writer` has the signature
`Fn(&M::Constants) -> TaskResult<Writer> + Send + Sync + 'static`. It borrows
the one config-supplied constants value so observation policy can depend on scientific
input without decoding a second copy or asking the user to pass the
same information twice. The returned `Writer` is owned. The borrow cannot
escape the call, while values intentionally cloned into the closure result
follow their normal Rust ownership rules.

At execution time, constants decoding and writer creation occur before model
initialization. The writer is schema-bound and validated before `M::initialize`
is called, so an unknown writer field cannot start scientific work. A writer
factory or schema-binding failure emits no initial observation. Model
initialization then consumes the constants; no second model copy is retained.

The runtime automatically handles initial, per-step, and final writer
boundaries. Application code neither invokes a writer nor returns observations
from `step`. The task itself performs no filesystem I/O, persistence, thread
creation, or blocking beyond time spent in the application writer factory and
model methods. The runtime host may perform bounded blocking or persistence at
an observation boundary; that is part of the host's advanced contract.

#### `Task::one_shot::<C, F>(run)`

Creates work that needs neither scientific state nor a writer. `C` must be
`DeserializeOwned + Send + Sync + 'static`; `run` has the signature
`Fn(C) -> TaskResult + Send + Sync + 'static`.

The config-owned resolved task input supplies one `C` using its Serde contract.
Unknown fields are accepted or rejected according to `C`; an application that requires
a closed object grammar should use `#[serde(deny_unknown_fields)]`. The owned
value is passed to the callback exactly once after an initial cancellation
check. A one-shot task never requests a state schema and never calls any model
observation method on the host.

`one_shot` is for scientifically meaningful work with no evolving recorded
state, such as generating a derived summary or validating an external result.
It is not a way to bypass runtime identity, lifecycle, error handling, or
provenance.

### `task::basic::ScientificModel`

`ScientificModel` is the application contract for one stateful scientific
workload. Implementors are `Send + Sized + 'static`; a model may own
non-`Sync` numerical structures because one executor mutates it at a time.

#### Direct ownership requirement

The canonical user model **must directly own the `SystemState` returned by
`state()`**. The normal shape is a struct field such as
`state: SystemState`. The state may not be borrowed from a temporary, hidden
behind a replaceable external owner, or exchanged for another `SystemState`
during execution.

Rust can require the return type and lifetime of `state()`, but a trait cannot
prove how an implementor stores a field. Workflow therefore enforces every
observable part of this semantic rule:

- the address of the returned `SystemState` must remain stable;
- it must share the exact immutable schema allocation supplied to
  `initialize`;
- that schema may never change;
- every successful step must strictly increase its iteration; and
- optional target-iteration claims must remain valid and monotonic.

This requirement creates one unambiguous authority for the scientific state.
Task, runtime, writer, and display only borrow it at controlled boundaries.
An `Arc` counter, progress report, duplicate state DTO, or model-specific
message object is neither required nor supported.

#### `type Constants`

The complete owned model constants supplied from one resolved task input. It must
implement `DeserializeOwned + Send + Sync + 'static`. It should contain only
scientific constants needed by the model or its writer; runtime identities,
paths, labels, phase policy, and recording administration remain outside it.

Serde owns the representation contract. Use `#[serde(deny_unknown_fields)]`
when accepting an unknown key would hide an input mistake. Semantic
validation that depends on scientific meaning belongs in `initialize` and may
return a contextual application error.

#### `ScientificModel::initialize(constants, schema)`

Consumes the decoded constants and borrows the runtime-loaded
`SystemStateSchema`. It returns a fully initialized model or `TaskResult<Self>`.
The model must create its directly owned state from the exact supplied schema,
populate every payload required by the selected writer and its first
scientific step, establish its initial `StateTime`, and finish all scientific
validation before returning success.

The schema borrow lasts only for the call. `SystemState` retains a cheap shared
schema handle when created through `schema.create_empty_state`, so the model
does not need to clone or store a separate schema. Returning an independently
loaded but structurally identical schema is invalid because field-order and
payload contracts must have one allocation authority.

Initialization may allocate, perform application work, and fail through
`TaskResult`. It should avoid publishing external effects that cannot be
rolled back: task validation has already completed, but no initial observation
or runtime lifecycle success boundary exists until this call returns and the
model contract is checked.

#### `ScientificModel::state(&self)`

Borrows the directly owned canonical `SystemState`. It must be side-effect
free, return promptly, and return the same owner and schema allocation for the
entire execution. The borrow never crosses a later mutable `step` call. The
runtime and writer use it synchronously for validation, observation, encoding,
and bounded snapshot extraction.

#### `ScientificModel::is_complete(&self)`

Reports whether another scientific transition is needed. It must be
side-effect free and return promptly. Workflow evaluates it after the initial
observation and after each observed successful step. Once it returns `true`,
Workflow does not call `step` again and requests the final host boundary.

An initially complete model is valid: it receives an initial boundary followed
by a final boundary without a step. The host or writer session is responsible
for reusing/deduplicating the equal state bytes while still finalizing the
lifecycle exactly once.

#### `ScientificModel::step(&mut self)`

Performs exactly one scientifically observable transition. A successful call
must strictly increase `state().time().iteration()`, keep the same state owner
and schema, and leave the canonical state ready for immediate observation. It
returns `TaskResult<()>`; additional scientific values stay in `SystemState`
instead of being duplicated in a step-report type.

On error, Workflow emits neither a successful-step writer observation nor a
successful-step UI snapshot. Application code should therefore return an error
only when the transition cannot be claimed as successful. Because arbitrary
model mutations cannot be rolled back generically, the model owns scientific
failure atomicity; the runtime owns failure lifecycle finalization.

One successful call may advance the iteration by more than one when that is the
model's scientific meaning. It may not leave the iteration equal or decrease
it. Optional physical time remains governed by `StateTime` and may advance
according to the model.

#### `ScientificModel::target_iteration(&self)`

Returns an optional expected final iteration for automatically inferred
progress. The default implementation returns `None`; models with convergence
or data-dependent stopping need not manufacture a target.

When `Some(target)` is returned, `target` must be at least the current state
iteration. A model may change from `None` to `Some` after learning a target.
Once present, the target may stay equal or increase, but it may never decrease
or disappear during that execution. The method must be side-effect free and
return promptly.

This is the only model progress hint. Current iteration always comes from
`SystemState`; elapsed operational time comes from runtime; task label and
lifecycle come from `study.json` and runtime; configured scientific display
fields come from the state itself. The user never updates a separate counter.

### `task::basic::TaskResult<T = ()>`

`TaskResult` is the alias
`Result<T, Box<dyn std::error::Error + Send + Sync + 'static>>`. Its default
success type is `()`.

The boxed error permits application-specific error enums without forcing them
into a central task error vocabulary. `Send + Sync + 'static` lets the runtime
move and retain failures safely across worker and lifecycle boundaries.
Ordinary errors implementing those bounds convert through `?`; their source
chains and display context are preserved. The task layer does not stringify,
classify, retry, or persist an error itself.

## Advanced API

`task::advanced` is a strict superset of the Basic API. It adds only the stable
runtime boundary. Application model modules normally should not import it.

### `task::advanced::TaskKind`

A `Copy + Eq` enum describing execution shape:

- `TaskKind::Stateful` means typed constants initialize a `ScientificModel`, a
  state schema is required, and automatic writer/model observations occur.
- `TaskKind::OneShot` means typed constants feed one callback and neither a
  state schema nor writer is required.

The enum selects runtime capabilities; it is not lifecycle state, phase
membership, a display category, or stable identity.

### `task::advanced::TaskDescriptor`

Immutable `Clone + Eq` planning metadata retained by every `Task`. Consumers
borrow it through `TaskDefinition::descriptor`; it has no public constructor or
mutation API.

- `kind()` returns the `TaskKind`.
- `constants_type_name()` returns Rust's diagnostic type name for the decoded
  constants. Compiler spelling is not a stable external identifier and must
  not be used for task matching, paths, caches, or durable provenance keys.
- `requires_state_schema()` is exactly `kind() == TaskKind::Stateful` and lets
  a runtime avoid interpreting `config/state.json` for one-shot work.

Descriptor inspection performs no allocation, I/O, blocking, persistence, or
cancellation check.

### `task::advanced::TaskDefinition`

The object-safe `Send + Sync` execution contract implemented by `Task`.
Application code normally uses the two safe `Task` constructors instead of
implementing this trait.

- `descriptor()` borrows immutable metadata for planning and validation.
- `execute(input, host)` borrows one config-owned `ResolvedTaskInput`, obtains
  the definition's complete typed constants through `input.decode()`, and runs through a mutable
  `TaskExecutionHost`. The input is not modified. Stateful execution follows
  the fixed sequence at the top of this document; one-shot execution performs
  only cancellation, decoding, and callback invocation.

`execute` is synchronous. It creates no thread. Calls into the model, writer
factory, and host may consume arbitrary application/runtime time. Host methods
may apply bounded backpressure. The runtime decides which worker invokes it.

A task checks cancellation before decoding and before every new model step. If
cancellation is observed, `execute` returns `Ok(())` without a final model
boundary. The runtime must inspect its own cancellation state and record a
cancelled lifecycle rather than interpreting that return as completion.
Cancellation cannot safely interrupt a currently executing Rust callback or
model step; it is cooperative between boundaries.

Config-owned typed decode, writer construction/binding, initialization, model
contract, or host errors are returned through `TaskResult`. Before the initial host call,
failure produces no task observation. Afterward, the host/runtime owns failure
cleanup and durable lifecycle disposition. Private contract error variants are
intentionally not matchable API; callers retain and display their error chain.

### `task::advanced::TaskExecutionHost`

The replaceable runtime port used during `TaskDefinition::execute`. A host is
borrowed mutably for one synchronous execution and need not be `Send` or
`Sync`; its owning runtime chooses the worker and synchronization strategy.

#### `state_schema(&self)`

Returns the schema previously loaded from the project's
`config/state.json`, or
a `TaskResult` error if the runtime cannot supply it. Stateful execution calls
this once and cheaply clones its schema handle before any mutable host callback
so the borrow does not escape. One-shot execution never calls it.

The host must return one canonical allocation for the execution. Paths remain
inside config/runtime; this port accepts no raw or typed path because the task
does not choose where the state schema document lives.

#### `cancellation_requested(&self)`

Reads cooperative cancellation state. It should be nonblocking and
side-effect free. Task calls it before any decoding/application work and
between successful model steps. The host owns the cancellation token and the
eventual cancelled lifecycle status.

#### `begin_model(writer, state, target_iteration)`

Consumes the already schema-validated `Writer`, borrows the fully initialized
canonical state, and receives the validated optional target. A normal host
creates its private writer/record session, observes the initial state, starts
automatic lifecycle/progress reporting, and publishes an initialized snapshot.

This is the ownership-transfer boundary for `Writer`; task retains only the
model. If the method fails, no step is called. The host owns cleanup for any
resources it partially created and must preserve the underlying error chain.

#### `observe_model_step(state, target_iteration)`

Borrows the validated canonical state immediately after one successful step.
The host applies writer cadence, bounded persistence/backpressure, and an
automatic progress/message snapshot. It must complete all synchronous uses of
the borrow before returning. Failure stops execution before another step.

#### `observe_model_final(state, target_iteration)`

Marks the one final model boundary after `is_complete` returns true. The host
must ensure terminal state inclusion according to writer policy without
duplicating equal iteration bytes already accepted by the preceding step, and
must emit the completion snapshot exactly once. Durable record sealing and
overall task lifecycle finalization remain runtime/record concerns after
`execute` returns successfully.

The host, together with runtime and UI, constructs routine messages
automatically. A standard snapshot includes the task label inferred from
`study.json`, lifecycle, current `StateTime` iteration, optional target, and
elapsed operational time. If `study.json` selects extra scientific display
fields, runtime validates those names against the schema before execution and
extracts bounded scalar-oriented values from `SystemState` at these same
observation boundaries. Formatting, truncation, throttling, channels, terminal
state, and any internal shared counters belong to runtime/UI—not the model and
not this task API.

## Example

The canonical model directly owns its state, consumes typed constants, and
keeps every observable scientific value in that state:

```rust,no_run
use scientific_workflow::task::basic::{ScientificModel, Task, TaskResult};
use scientific_workflow::state::basic::{StateTime, SystemState, SystemStateSchema};
use scientific_workflow::writer::basic::Writer;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Constants {
    initial_population: u64,
    steps: u64,
}

struct PopulationModel {
    // Required canonical shape: the model directly owns this state.
    state: SystemState,
    remaining: u64,
    target: u64,
}

impl ScientificModel for PopulationModel {
    type Constants = Constants;

    fn initialize(constants: Constants, schema: &SystemStateSchema) -> TaskResult<Self> {
        let mut state = schema.create_empty_state(StateTime::from_iteration(0));
        state.initialize_payload("population", constants.initial_population)?;
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
        *self.state.payload_mut::<u64>("population")? += 1;
        self.state.advance_time(None)?;
        self.remaining -= 1;
        Ok(())
    }

    fn target_iteration(&self) -> Option<u64> {
        Some(self.target)
    }
}

fn population_task() -> Task {
    Task::stateful::<PopulationModel, _>(|_constants| {
        // No schema, paths, stream names, cadence, progress, or lifecycle
        // plumbing is required for the inferred all-field writer.
        Ok(Writer::all_fields())
    })
}

#[derive(Deserialize)]
struct SummaryConstants {
    input_count: usize,
}

fn summary_task() -> Task {
    Task::one_shot::<SummaryConstants, _>(|constants| {
        produce_summary(constants.input_count)
    })
}

fn produce_summary(_input_count: usize) -> TaskResult {
    Ok(())
}
```

Application startup registers `population_task()` or `summary_task()` against
the corresponding declaration in `study.json`. The future runtime/config
composition layer supplies the resolved task input and schema automatically;
neither task constructor receives an identity or filesystem path.

A minimal replacement-runtime adapter uses only the advanced boundary:

```rust,no_run
use scientific_workflow::prelude::advanced::*;

fn execute_compiled_task(
    task: &Task,
    resolved: &ResolvedTaskInput,
    host: &mut dyn TaskExecutionHost,
) -> TaskResult {
    task.execute(resolved, host)
}
```

Real hosts must implement the observation, persistence, automatic messaging,
cancellation, and terminal-lifecycle responsibilities described above; the
small function demonstrates that no task implementation internals are needed.

## Not API

The following implementation details are intentionally private and may change
during a task-subsystem replacement:

- the `Arc<dyn TaskDefinition>` stored by `Task`;
- generic `StatefulDefinition` and `OneShotDefinition` adapters;
- phantom type markers and closure storage;
- the concrete order and number of internal validation helper calls;
- state-address comparison mechanics;
- private model-contract error types and variants;
- runtime task registries and manifest-to-definition matching;
- prepared tasks, model owners, worker handles, cancellation tokens, and
  internal shared progress state;
- writer sessions, record sessions, snapshot queues, and final-state
  deduplication machinery; and
- lifecycle events, renderer messages, display extraction, throttling, and
  channel implementations.

There is no public `TaskRun`, task context, model context, user report type,
progress counter, `Arc` counter, message callback, step return payload,
identity constructor, label setter, category, metadata map, schema path,
recording path, manual observation method, manual completion method, scheduler
handle, renderer handle, or registry mutation API in this subsystem.

Replacement implementations must preserve typed single-decode behavior,
direct canonical state ownership semantics, schema-allocation validation,
strictly increasing successful iterations, target monotonicity, cooperative
cancellation boundaries, automatic observation boundaries, and the separation
between application scientific work and runtime-owned mechanics. They need not
preserve the current type-erasure, allocation, or private error representation.
