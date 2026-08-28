# Task API

Task owns Workflow's uniform scientific execution boundary. A configured
scientific task selects one registered `ExecutionUnit`, one resolved constants
value, and one named state schema. The unit may be a standalone model or an
ensemble; Runtime and Study never branch on that distinction.

```text
Task -> ExecutionUnit -> ModelView[0..N] -> one SystemState per model
```

An execution unit owns one lifecycle and one coordinated `step`. Its models
retain independent identities, completion predicates, targets, observations,
recordings, and final results. Every model in one unit uses the task's selected
schema, but each owns a distinct state instance.

## Basic API

### `task::basic::ExecutionUnit`

Canonical path: `scientific_workflow::task::basic::ExecutionUnit`. This public
`Send + Sized + 'static` trait is implemented by application scientific
workloads. A normal model implements it with `model_count() == 1`; an ensemble
implements it directly and keeps its shared inputs, batching, synchronization,
and internal parallelism private.

Associated type:

- `Constants: DeserializeOwned + 'static` is the complete Config-supplied value.
  Constants need not be `Send` or `Sync`: Study and Runtime independently
  deserialize equivalent owned values on their current threads. Deserialization
  must therefore be deterministic and side-effect-free. Using
  `#[derive(Deserialize)]` and `#[serde(deny_unknown_fields)]` is recommended.

Methods:

- `observation_plan(constants: &Self::Constants) -> TaskResult<ObservationPlan>`
  is the optional, effect-free preflight hook. The default records every schema
  field each iteration. Study binds the returned plan to the task's named schema
  once; the common bound plan is applied independently to every model recording.
- `initialize(constants: Self::Constants, schema: &SystemStateSchema)
  -> TaskResult<Self>` consumes Runtime's fresh constants decode and borrows the
  exact schema allocation retained by Study. It must return a fully initialized,
  positive-cardinality unit. It may allocate memory and initialize domain
  resources, but it receives no paths, writers, UI, or scheduling handles.
- `model_count(&self) -> usize` returns the stable positive member count.
  The count cannot change after initialization.
- `model(&self, index: usize) -> Option<ModelView<'_>>` returns a side-effect-free
  borrowed view for each `index < model_count()` and `None` for all other
  indices. Index order, identity, state address, and schema allocation must stay
  stable throughout execution.
- `step(&mut self) -> TaskResult` performs one complete coordinated transition.
  At least one incomplete model must strictly advance its state iteration on
  success. Other incomplete models may wait, supporting synchronized ensembles
  whose members begin at different iterations. A completed model cannot advance
  or become incomplete. An error publishes no successful-step observation.

Workflow checks cancellation between calls to `step`; implementations should
return in bounded time if responsive cancellation matters. Internal worker
parallelism must join before `step` returns so every exposed state is coherent.

### `task::basic::ModelView<'a>`

Canonical path: `scientific_workflow::task::basic::ModelView`. This public,
copyable borrowed descriptor exposes one model owned by an `ExecutionUnit`.
It owns nothing and cannot outlive the unit borrow.

- `ModelView::new(identity, state, complete, target_iteration)` accepts
  `identity: &'a str`, `state: &'a SystemState`, `complete: bool`, and
  `target_iteration: Option<u64>`. Identity must be nonempty, have no surrounding
  whitespace, and be unique in the unit. The target cannot precede the state's
  current iteration.
- `identity(self) -> &'a str` returns the stable member identity used in
  provenance and `ModelRunSummary`.
- `state(self) -> &'a SystemState` returns the model's canonical state borrow.
- `is_complete(self) -> bool` returns whether no further transition is required.
  Completion is monotonic during one execution.
- `target_iteration(self) -> Option<u64>` returns optional progress intent. Once
  present it cannot decrease or disappear.

The implementing unit must directly or transitively own every exposed state.
Moving a state after initial inspection, replacing its schema, returning a
temporary, duplicating identities, or reordering members is a contract error.

### `task::basic::TaskResult<T = ()>`

Canonical path: `scientific_workflow::task::basic::TaskResult`. It aliases
`Result<T, Box<dyn Error + Send + Sync + 'static>>` and is the error boundary for
preflight, initialization, and scientific steps. Errors cross worker threads;
constants and the execution unit itself do not need to be `Sync`.

### Registration attributes

`#[scientific_workflow::execution_unit("key")]` registers the attributed
`ExecutionUnit` implementation under one nonempty, whitespace-exact manifest
key. `#[scientific_workflow::model("key")]` is a compatibility spelling with
identical expansion. The attribute creates inventory metadata only: it does not
wrap the implementation, create state, generate accessors, or change stepping.

The `model` field in `wf_configs/study.json` selects this stable registration
key and the same top-level key in `wf_configs/parameters.json`. The field name is
retained as the scientific workload key even when the implementation is an
ensemble; upstream compilation and Runtime remain cardinality-agnostic.

## Advanced API

Task adds no supported Advanced-only application symbols. `task::advanced::*`
is a strict superset re-exporting `ExecutionUnit`, `ModelView`, and `TaskResult`.
The documentation-hidden `ModelRegistration` is public solely because
procedural-macro expansion occurs in downstream crates. Constructing it
directly is unsupported.

Resolved executable and Python tasks share Task's private erased execution
port but expose no user-implemented Rust trait. They are declared in
`wf_configs/study.json` and receive Runtime's standardized program environment.

## Errors and failure atomicity

User errors returned during observation-plan preflight prevent Study creation
and therefore create no output. Initialization and step errors occur during
Runtime execution. Runtime marks every still-open model recording failed and
reports the task error. A model recording already finalized before a later
ensemble member fails remains a truthful completed recording; the enclosing
task and phase still fail.

Private contract validation rejects:

- empty units, missing indices, indices outside the declared count, or changing
  member count;
- invalid, duplicate, or changing identities;
- moved state owners or changed schema allocations;
- regressing iterations, successful steps advancing no incomplete model, or
  advancement after completion;
- completion reversal; and
- targets before current state, decreasing targets, or removed targets.

Beginning several recordings is failure-safe: a later begin failure causes
Runtime to fail every recording that was already opened. Persistence owns all
directory creation and durable status transitions.

## Cross-module contract

- Config alone reads and expands constants and named schema documents.
- Study matches registration keys, binds the common observation plan to the
  selected schema, and retains immutable execution intent.
- Task decodes constants, owns unit contract enforcement, and emits per-model
  observation boundaries.
- State owns each model's `SystemState`; Task only borrows it.
- Runtime owns cancellation, aggregate progress, scheduling, and summaries.
- Persistence opens one recording for a standalone model or one recording per
  ensemble member. Multi-model task recordings live beneath
  `models/model-<index>` in stable index order.
- UI receives aggregate unit progress and never inspects the concrete unit.

Replacement implementations may change Task internals if they preserve the
registered generic boundary, preflight/runtime double decode, stable per-model
validation, per-model observation ordering, cooperative cancellation points,
and TaskExecutionHost semantics. They must not expose Persistence or Runtime
handles to application units.

## Example

```rust
use serde::Deserialize;
use scientific_workflow::prelude::basic::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Constants { initial: u64, steps: u64 }

struct Counter {
    state: SystemState,
    target: u64,
}

#[scientific_workflow::execution_unit("counter")]
impl ExecutionUnit for Counter {
    type Constants = Constants;

    fn initialize(constants: Constants, schema: &SystemStateSchema) -> TaskResult<Self> {
        let mut state = schema.create_empty_state(StateTime::from_iteration(0));
        state.initialize_payload("count", constants.initial)?;
        Ok(Self { state, target: constants.steps })
    }

    fn model_count(&self) -> usize { 1 }

    fn model(&self, index: usize) -> Option<ModelView<'_>> {
        (index == 0).then(|| ModelView::new(
            "counter",
            &self.state,
            self.state.time().iteration() >= self.target,
            Some(self.target),
        ))
    }

    fn step(&mut self) -> TaskResult {
        *self.state.payload_mut::<u64>("count")? += 1;
        self.state.advance_time(None)?;
        Ok(())
    }
}
```

An ensemble uses the same trait: it returns `members.len()` from
`model_count`, constructs one `ModelView` for `members[index]`, and performs its
shared or parallel member advancement inside `step`. No caller changes.

## Not API

`ModelCatalog`, `ModelRegistration` construction, `Task`, `TaskKind`,
`StatefulDefinition`, `ProgramDefinition`, `TaskDefinition`,
`TaskExecutionHost`, program invocation views, contract-error types, and all
registration inventory details are private mechanisms. Their names, layouts,
and call ordering are not stable downstream APIs.
