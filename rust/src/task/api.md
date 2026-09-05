# Task API

This guide documents the `scientific-workflow` 0.13.5 subsystem contract.

Task owns Workflow's uniform scientific execution boundary. A configured
scientific task selects one registered `ExecutionUnit`, one resolved constants
value, and one resolved state schema. The schema is either explicitly named by
the project or supplied by the unit's standard upstream provider. The unit may be a standalone member or an
ensemble; Runtime and Study never branch on that distinction.

```text
Task -> ExecutionUnit -> MemberView[0..N] -> one SystemState per member
```

An execution unit owns one lifecycle and one coordinated `step`. Its members
retain independent identities, completion predicates, targets, observations,
recordings, and final results. Every member in one unit uses the task's selected
schema, but each owns a distinct state instance.

## Basic API

### `ExecutionUnit`

Canonical path: `scientific_workflow::ExecutionUnit`. This public
`Send + Sized + 'static` trait is implemented by application scientific
workloads. A single-member execution unit implements it with
`member_count() == 1`; a member is the stateful result exposed by that unit, not
the trait implementor itself. An ensemble also implements `ExecutionUnit`
directly, reports several members, and keeps its shared inputs, batching,
synchronization, and internal parallelism private.

Associated type:

- `Constants: DeserializeOwned + 'static` is the complete Config-supplied value.
  Constants need not be `Send` or `Sync`: Study and Runtime independently
  deserialize equivalent owned values on their current threads. Deserialization
  must therefore be deterministic and side-effect-free. Using
  `#[derive(Deserialize)]` and `#[serde(deny_unknown_fields)]` is recommended.

Methods:

- `standard_state_schema() -> Option<StateSchemaProvider>` optionally declares
  the linked upstream schema used when an execution-unit task omits `state`.
  The default is `None`. A receiver normally returns a descriptor exported by
  the upstream crate that owns the JSON; it must not read a project file here.
  An explicit project `state` always takes precedence, so this hook is a
  default rather than an override.
- `preflight(constants: &Self::Constants, schema: &SystemStateSchema)
  -> UnitResult<ObservationPlan>` is the optional, effect-free preflight hook.
  The unit owns its domain validation and Study trusts a successful result. The
  default records every schema field each iteration. Study binds the returned
  plan to the task's named schema once; the common bound plan is applied
  independently to every member recording.
- `initialize(constants: Self::Constants, schema: &SystemStateSchema,
  context: &InitializationContext) -> UnitResult<Self>` consumes Runtime's
  fresh constants decode, borrows the exact schema allocation retained by
  Study, and receives immutable initialization facts. It must return a fully
  initialized, positive-cardinality unit. It may allocate memory and initialize
  domain resources, but it receives no paths, writers, UI, or scheduling
  handles. Deterministic units simply ignore `context`.
- `member_count(&self) -> usize` returns the stable positive member count.
  The count cannot change after initialization.
- `member(&self, index: usize) -> Option<MemberView<'_>>` returns a side-effect-free
  borrowed view for each `index < member_count()` and `None` for all other
  indices. Index order, identity, state address, and schema allocation must stay
  stable throughout execution.
- `step(&mut self) -> UnitResult` performs one complete coordinated transition.
  At least one incomplete member must strictly advance its state iteration on
  success. Other incomplete members may wait, supporting synchronized ensembles
  whose members begin at different iterations. A completed member cannot advance
  or become incomplete. An error publishes no successful-step observation.

Workflow checks cancellation between calls to `step`; implementations should
return in bounded time if responsive cancellation matters. Internal worker
parallelism must join before `step` returns so every exposed state is coherent.

### `InitializationContext`

Canonical path: `scientific_workflow::InitializationContext`.
Workflow creates one context per execution-unit initialization; application
code borrows it and cannot construct or customize it. It is `Send + Sync` but
not `Clone`; initialization may share the borrow with scoped worker threads,
and the unit cannot retain it after `initialize` returns.

- `has_master_seed(&self) -> bool` reports whether the optional top-level
  `study.json.seed` exists. Merely inspecting this value records nothing.
- `dependencies(&self) -> &task::dependencies::Dependencies` borrows completed summaries from
  the task's declared dependency phases. When global parameters sweep, Workflow
  includes only upstream task copies belonging to the same resolved global
  configuration.
- `shared_seed(&self, purpose: &str) -> Result<u64, SeedError>` derives a seed
  for coordinated behavior shared by every member in the unit.
- `member_seed(&self, member_identity: &str, purpose: &str) -> Result<u64,
  SeedError>` derives a seed scoped to one eventual `MemberView` identity.

Purposes and member identities must be stable, nonempty, and have no surrounding
whitespace. Derivation includes the master seed, replicate ordinal, task
identity, registered execution-unit key, scope, optional member identity, and
purpose. It uses a versioned SHA-256 derivation and has no mutable counter, so
request order, thread scheduling, and unrelated new requests cannot perturb an
existing seed. Text domains are UTF-8 and unsigned-64-length-prefixed, numeric
domains use little-endian `u64`, and the first eight digest bytes become a
little-endian `u64`. Repeated identical requests return the same value.

Only successful requests are recorded. Every member recording receives all
shared requests plus only the requests for its own identity under
`user_metadata.workflow.seed_derivation`; each entry includes the actual `u64`
seed. A member-scoped request whose identity is not exposed after initialization
fails the execution-unit contract instead of disappearing from provenance.

### `SeedError`

Canonical path: `scientific_workflow::SeedError`. This
non-exhaustive error reports a request made without `study.json.seed`, an
invalid purpose/member identity, or a member identity not exposed by the
initialized unit. It implements `Error + Send + Sync`, so `?` converts it into
`UnitResult`.

- `MissingMasterSeed` means a seed was requested from a deterministic context
  whose study omitted the top-level seed.
- `InvalidName { field, value }` preserves whether the rejected name was the
  purpose or member identity and the invalid owned value.
- `UnknownMemberIdentity { identity }` is raised after initialization when a
  member-scoped request cannot be associated with any exposed `MemberView`.

### `MemberView<'a>`

Canonical path: `scientific_workflow::MemberView`. This public,
copyable borrowed descriptor exposes one member owned by an `ExecutionUnit`.
It owns nothing and cannot outlive the unit borrow.

- `MemberView::new(identity, state, completion, target_iteration)` accepts
  `identity: &'a str`, `state: &'a SystemState`,
  `completion: Option<MemberCompletion<'a>>`, and
  `target_iteration: Option<u64>`. Identity must be nonempty, have no surrounding
  whitespace, and be unique in the unit. The target cannot precede the state's
  current iteration.
- `identity(self) -> &'a str` returns the stable member identity used in
  provenance and `MemberRunSummary`.
- `state(self) -> &'a SystemState` returns the member's canonical state borrow.
- `completion(self) -> Option<MemberCompletion<'a>>` returns `None` while the
  member is incomplete and `Some` when no further transition is required.
  Completion is monotonic during one execution.
- `target_iteration(self) -> Option<u64>` returns optional progress intent. Once
  present it cannot decrease or disappear.

The implementing unit must directly or transitively own every exposed state.
Moving a state after initial inspection, replacing its schema, returning a
temporary, duplicating identities, or reordering members is a contract error.

### `MemberCompletion<'a>`

Canonical path: `scientific_workflow::MemberCompletion`. This
copyable borrowed descriptor represents the two completed states without
requiring a second execution-unit callback.

- `MemberCompletion::without_reason()` declares completion without metadata.
- `MemberCompletion::with_reason(reason)` accepts a borrowed
  `&'a serde_json::Map<String, Value>` retained by the execution unit.
- `reason(self)` returns that optional structured object.

Workflow clones a supplied object exactly once at the first completion
boundary and stores it under `terminal_metadata.completion_reason`. A
reasonless completion leaves terminal metadata empty.

### `UnitResult<T = ()>`

Canonical path: `scientific_workflow::UnitResult`. It aliases
`Result<T, Box<dyn Error + Send + Sync + 'static>>` and is the error boundary for
preflight, initialization, and scientific steps. Errors cross worker threads;
constants and the execution unit itself do not need to be `Sync`.

### Registration attributes

`#[scientific_workflow::execution_unit("key")]` registers the attributed
`ExecutionUnit` implementation under one nonempty, whitespace-exact manifest
key. The attribute creates inventory metadata only: it does not
wrap the implementation, create state, generate accessors, or change stepping.

The `execution_unit` field in `wf_configs/study.json` selects this stable registration
key and the same top-level key in `wf_configs/parameters.json`. The field name is
retained as the scientific workload key even when the implementation is an
ensemble; upstream compilation and Runtime remain cardinality-agnostic.

The documentation-hidden `ExecutionUnitRegistration` is public solely through
the macro-support namespace because procedural-macro expansion occurs in
downstream crates. Constructing it directly is unsupported.

Resolved executable and Python tasks share Task's private erased execution
port but expose no user-implemented Rust trait. They are declared in
`wf_configs/study.json` and receive Runtime's standardized program environment.
An optional task-level `seed: {"purpose":"..."}` declaration is carried
through this private invocation path; it does not add a public Rust program
trait or expose the study master seed to the child. The private descriptor also
retains the Config-validated external thread request so Study can inspect it and
Runtime can enforce the global compute budget; execution units retain no
per-task resource setting.

## Advanced API

### Errors and failure atomicity

User errors returned during unit-owned preflight prevent Study creation
and therefore create no output. Initialization and step errors occur during
Runtime execution. Runtime marks every still-open member recording failed and
reports the task error. A member recording already finalized before a later
ensemble member fails remains a truthful completed recording; the enclosing
task and phase still fail.

Private contract validation rejects:

- empty units, missing indices, indices outside the declared count, or changing
  member count;
- invalid, duplicate, or changing identities;
- moved state owners or changed schema allocations;
- regressing iterations, successful steps advancing no incomplete member, or
  advancement after completion;
- completion reversal; and
- targets before current state, decreasing targets, or removed targets.

Beginning several recordings is failure-safe: a later begin failure causes
Runtime to fail every recording that was already opened. Persistence owns all
directory creation and durable status transitions.

### Cross-module contract

- Config alone reads and expands constants and explicit project schema documents.
- Study matches registration keys, binds the common observation plan to the
  selected explicit schema or the unit's validated standard provider, and
  retains immutable execution intent.
- Task decodes constants, owns unit contract enforcement, and emits per-member
  observation boundaries.
- State owns each member's `SystemState`; Task only borrows it.
- Runtime owns cancellation, aggregate progress, scheduling, and summaries.
- Persistence opens one recording for a standalone member or one recording per
  ensemble member. Multi-member task recordings live beneath
  `members/member-<index>` in stable index order.
- UI receives aggregate unit progress and never inspects the concrete unit.

Replacement implementations may change Task internals if they preserve the
registered generic boundary, preflight/runtime double decode, stable per-member
validation, per-member observation ordering, cooperative cancellation points,
and TaskExecutionHost semantics. They must not expose Persistence or Runtime
handles to application units.

## Example

```rust
use serde::Deserialize;
use scientific_workflow::prelude::*;

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

    fn standard_state_schema() -> Option<scientific_workflow::state::StateSchemaProvider> {
        Some(scientific_workflow::state::StateSchemaProvider::new(
            "example.counter-state.v1",
            br#"{"fields":[{"name":"count"}]}"#,
        ))
    }

    fn initialize(
        constants: Constants,
        schema: &SystemStateSchema,
        _context: &InitializationContext,
    ) -> UnitResult<Self> {
        let mut state = schema.create_empty_state(StateTime::from_iteration(0));
        state.initialize_payload("count", constants.initial)?;
        Ok(Self { state, target: constants.steps })
    }

    fn member_count(&self) -> usize { 1 }

    fn member(&self, index: usize) -> Option<MemberView<'_>> {
        (index == 0).then(|| MemberView::new(
            "counter",
            &self.state,
            (self.state.time().iteration() >= self.target)
                .then_some(MemberCompletion::without_reason()),
            Some(self.target),
        ))
    }

    fn step(&mut self) -> UnitResult {
        *self.state.payload_mut::<u64>("count")? += 1;
        self.state.advance_time(None)?;
        Ok(())
    }
}
```

An ensemble uses the same trait: it returns `members.len()` from
`member_count`, constructs one `MemberView` for `members[index]`, and performs its
shared or parallel member advancement inside `step`. No caller changes.

## Not API

`ExecutionUnitCatalog`, `ExecutionUnitRegistration` construction, `Task`, `TaskKind`,
`ExecutionUnitDefinition`, `ProgramDefinition`, `TaskDefinition`,
`TaskExecutionHost`, program invocation views, contract-error types, and all
registration inventory details are private mechanisms. Their names, layouts,
and call ordering are not stable downstream APIs.

## Dependency and project accessors

See [the exhaustive dependency reference](dependencies/api.md) for every export
under `task::dependencies`.

`task::project` provides focused, standard-layout accessors. **REQUIRED LAYOUT:**
project declarations remain in `<root>/wf_configs/study.json` and
`parameters.json`; programs use Runtime's resolved `workflow-config.json` snapshot.
Missing/moved required paths fail with the expected path; discovery is not supported.

- `project_root() -> Result<PathBuf, ProjectLayoutError>` reads and verifies the
  directory from WORKFLOW_PROJECT_ROOT.
- `output_directory() -> Result<PathBuf, ProjectLayoutError>` reads and verifies
  WORKFLOW_TASK_OUTPUT, the program artifacts directory.
- `study_path(&Path) -> Result<PathBuf, ProjectLayoutError>` verifies the standard
  `<root>/wf_configs/study.json` path; does not parse it.
- `parameters<T: DeserializeOwned>(Option<&str>) -> Result<T, ProjectLayoutError>`
  reads WORKFLOW_CONFIG_PATH and deserializes all resolved parameters or one exact
  top-level key. No scientific validation beyond T's deserializer is performed.
- `parameters_from_snapshot<T>(&Path, Option<&str>)` provides the same operation
  using an explicit runtime snapshot for standalone use/tests.
- `ProjectLayoutError`: Error + Send + Sync with a contextual Display and an
  underlying cause for file/JSON/deserialization failures; fields are private.

Functions do synchronous environment/filesystem reads and return owned results;
errors return no partial result. They create no files, change no working directory,
and perform no environment activation or cancellation. There is no ProgramContext.

Runtime parks execution units at private safe boundaries around initialization
and between complete steps. ExecutionUnit's public contract is unchanged. Task
publishes its dependency/project accessors through the public `task` module;
existing crate-root unit-authoring exports remain supported aliases.
