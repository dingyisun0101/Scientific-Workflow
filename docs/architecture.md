# Workflow architecture

This document explains the complete Rust architecture from the perspective of
someone opening the repository for the first time. It describes the current
implementation, its supported API tiers, and the direction of the remaining
record/UI migration.

## User workflow

An ordinary project has only two authored surfaces: Rust scientific models and
JSON declarations.

```text
<project-root>/
├── src/
│   ├── main.rs                 calls scientific_workflow::run(&Path)
│   └── <models>.rs             registered ScientificModel implementations
├── study.json                  phase/model organization and runtime policy
└── config/
    ├── state.json              canonical state schema
    └── inputs/
        └── <model>.json        constants, sweeps, or correlated cases
```

A model is connected to the manifest by one stable declaration:

```rust
#[scientific_workflow::model("population")]
impl ScientificModel for PopulationModel {
    // constants, initialization, state, completion, and stepping
}
```

The executable contains no registry list or orchestration builders:

```rust
fn main() -> Result<(), scientific_workflow::WorkflowError> {
    scientific_workflow::run(std::path::Path::new("."))
}
```

The internal flow is:

```text
project root
    │
    ▼
config: strict parse, path validation, defaults, input expansion
    │ ProjectSpecification + ResolvedTaskInput values
    ▼
study: state semantics + model discovery + binding + complete preflight
    │ immutable Study containing phases and internal tasks
    ▼
runtime: output scopes + replicates + scheduling + cancellation + invocation
    │ automatic observation boundaries
    ▼
writer → storage (future record subsystem) and future UI events
```

The decisive ownership distinction is:

```text
Study   = ultimate coordinator of declared intent
Runtime = ultimate coordinator of active execution
```

Config does not discover Rust types. Study does not parse JSON or create
output. Runtime does not reinterpret declarations or ask application code to
construct tasks.

## First-level subsystem map

The target first-level subsystems are:

- `state`: canonical scientific state, schema, coordinates, and in-memory
  series;
- `writer`: application-owned observation meaning and schema-bound encoding;
- `task`: registered scientific behavior behind an internal uniform execution
  contract;
- `config`: sole project-document parser and typed constants supplier;
- `study`: effect-free composition, model binding, identity inference, phase
  organization, and preflight;
- `runtime`: active execution, scheduling, cancellation, and output-scope
  inference;
- `record`: durable observations, provenance, recovery, artifacts, RNG records,
  and verified reads; and
- `ui`: bounded runtime events, snapshots, rendering, and commands.

`record` and `ui` are still migration targets. Their existing mechanics remain
in `storage`, `artifact`, `rng_record`, and parts of the removed legacy study
implementation. New user-facing design must not add APIs to those transitional
surfaces.

`prelude` aggregates APIs but owns no behavior. `error` composes subsystem
errors at the one-call workflow boundary.

## Public API tiers

Every target subsystem root defines two inline scope-management modules:

- `module::basic`: ordinary application-facing declarations;
- `module::advanced`: a strict superset used by advanced consumers and peer
  subsystems.

There are no `mod.rs`, `basic.rs`, or `advanced.rs` files. Implementation files
remain private, and peer modules import only another owner's `advanced` tier.
The central `prelude::basic` and `prelude::advanced` scopes re-export these
module-owned symbols; the prelude never wraps or reimplements them.

Current ordinary API intent is deliberately small:

```text
prelude::basic
├── model attribute
├── ScientificModel + TaskResult
├── SystemState declarations and operations
├── Writer + Stream declarations
├── run(&Path)
└── WorkflowError
```

`config::basic` and `study::basic` are empty because their ordinary interfaces
are JSON documents. `runtime::basic` exports only `run`.

## Dependency direction

```text
writer  ──► state
task    ──► config + state + writer
study   ──► config + state + task
runtime ──► study + task + state + writer + transitional storage/execution

prelude aggregates tiers
error   composes StudyError + RuntimeError
```

Config deliberately does not depend on task. It treats the manifest model key
as opaque text. Study owns the cross-domain match. This prevents the parser
from becoming a model registry and permits config to be replaced independently.

Task depends on config only through `ResolvedTaskInput::decode<T>()`, preserving
config as the sole supplier of typed constants. Runtime invokes only tasks that
Study already bound and validated.

## Source tree and file responsibilities

```text
workflow/
├── AGENTS.md                         local/private repository rules
├── docs/
│   ├── architecture.md               this ownership and layout guide
│   └── tests.md                      responsibility-oriented validation map
├── macros/
│   ├── Cargo.toml                    proc-macro support crate
│   └── src/lib.rs                    #[model] registration expansion
├── rust/
│   ├── Cargo.toml                    primary library package
│   ├── README.md                     first-time user guide
│   ├── src/
│   │   ├── lib.rs                    crate modules, macro/run/error exports
│   │   ├── error.rs                  WorkflowError composition
│   │   ├── prelude.rs                central basic/advanced aggregation
│   │   ├── prelude/api.md            exhaustive prelude contract
│   │   ├── state.rs                  state tiers
│   │   ├── state/api.md              exhaustive state API
│   │   ├── state/error.rs            state and series errors
│   │   ├── state/field.rs            immutable field declarations
│   │   ├── state/schema.rs           schema loading/semantic validation
│   │   ├── state/series.rs           ordered in-memory state collection
│   │   ├── state/state.rs            typed heterogeneous payload owner
│   │   ├── state/time.rs             StateTime and checked advancement
│   │   ├── state/value.rs            erased payload implementation
│   │   ├── writer.rs                 writer tiers
│   │   ├── writer/api.md             exhaustive writer API
│   │   ├── writer/definition.rs      Writer and bound WriterDescriptor
│   │   ├── writer/stream.rs          Stream and StreamDescriptor
│   │   ├── writer/sampling.rs        private cadence mechanics
│   │   ├── writer/observation.rs     checked borrowed state observation
│   │   ├── writer/encoding.rs        owned canonical encoded handoff
│   │   ├── writer/session.rs         private cadence/dedup session
│   │   ├── writer/error.rs           writer validation/encoding errors
│   │   ├── task.rs                   task tiers
│   │   ├── task/api.md               exhaustive task/model API
│   │   ├── task/model.rs             ScientificModel contract
│   │   ├── task/catalog.rs           registration and deterministic catalog
│   │   ├── task/definition.rs        opaque advanced Task definition
│   │   ├── task/execution.rs         host port and model contract checks
│   │   ├── task/result.rs            application error boundary
│   │   ├── config.rs                 config tiers
│   │   ├── config/api.md             grammar and exhaustive config API
│   │   ├── config/document.rs        strict JSON and exact source bytes
│   │   ├── config/manifest.rs        study-manifest grammar/defaults
│   │   ├── config/expansion.rs       $sweep/$cases expansion
│   │   ├── config/input.rs           resolved input and typed decode
│   │   ├── config/specification.rs   one-root loading transaction
│   │   ├── config/error.rs           contextual config failures
│   │   ├── study.rs                  study tiers
│   │   ├── study/api.md              exhaustive study API
│   │   ├── study/compilation.rs      private binding/preflight compiler
│   │   ├── study/plan.rs             immutable Study/Phase/Task views
│   │   ├── study/error.rs            composition/preflight failures
│   │   ├── runtime.rs                runtime tiers
│   │   ├── runtime/api.md            exhaustive runtime API
│   │   ├── runtime/execution.rs      run entry and active schedulers
│   │   ├── runtime/host.rs           task-to-storage execution adapter
│   │   ├── runtime/summary.rs        successful read-only run summaries
│   │   ├── runtime/error.rs          active execution failures
│   │   ├── storage.rs                transitional durable record owner
│   │   ├── storage/error.rs          storage failures
│   │   ├── storage/json_payload_decoder.rs
│   │   ├── storage/json_payload_decoder/string.rs
│   │   ├── storage/json_payload_decoder/vec_f64.rs
│   │   ├── storage/jsonl_format.rs   durable metadata/chunk wire format
│   │   ├── storage/queued_state_writer.rs
│   │   ├── storage/resume.rs         checkpoint reconstruction/rewind
│   │   ├── storage/stored_state_series_reader.rs
│   │   ├── execution.rs              transitional output/process scopes
│   │   ├── execution/error.rs        scope errors
│   │   ├── execution/replicate.rs    legacy subprocess replicate adapter
│   │   ├── execution/scope.rs        safe execution directory creation
│   │   ├── artifact.rs               transitional immutable artifacts
│   │   ├── rng_record.rs             transitional RNG provenance
│   │   └── clock.rs                  private UTC/duration utilities
│   └── tests/*.rs                    subsystem and boundary tests
└── examples/attractor_2d/
    ├── src/main.rs                   one run(&Path) call
    ├── src/hopf_model.rs             registered model and custom writer
    ├── study.json                    phase/model declaration
    └── config/                       state schema and model constants
```

### `lib.rs` and `error.rs`

`lib.rs` declares crate modules, re-exports the `model` attribute, the sole
ordinary `run` function, and `WorkflowError`. Its hidden `__private` namespace
exists only so procedural macro expansion can reach `inventory`; applications
must not use it.

`error.rs` owns `WorkflowError`. Study failures occur before output creation.
Runtime failures occur only after a valid immutable Study exists. Preserving
that distinction lets callers decide whether cleanup or retry is meaningful.

### State subsystem

`state.rs` exports ordinary state construction/manipulation through `basic` and
schema inspection/maintenance through `advanced`. `schema.rs` is the semantic
authority for `config/state.json`; config passes its already parsed value via
`StateSchemaAccess`, so Study performs no second read or generic parse.

`state.rs` owns the canonical `SystemState`. A `ScientificModel` must directly
own the exact state returned by `state()` for its entire execution. The task
adapter verifies stable address, schema allocation identity, and strictly
advancing iteration after successful steps.

### Writer subsystem

`Writer` describes scientific observation meaning only: selected fields,
stream names, cadence, and units. `ScientificModel::writer` returns this
definition and defaults to `Writer::all_fields()`. The function may inspect
constants but must be deterministic and effect-free because Study invokes it
during preflight and Task invokes it again at execution startup.

Writer does not own paths, buffering, persistence, task identity, or lifecycle.
The private session applies sampling and final-state deduplication. The runtime
host currently adapts encoded observations to `storage`; this moves behind the
future `record` boundary without changing model code.

### Task subsystem

Task's Basic API contains only `ScientificModel` and `TaskResult`. There is no
ordinary `Task` constructor. A model declares:

- an owned Serde-decodable `Constants` type;
- an optional deterministic writer definition;
- initialization from constants and the shared schema;
- canonical state borrowing;
- a completion predicate;
- one-step evolution; and
- an optional monotonic target iteration.

The `#[model("key")]` attribute submits one immutable `ModelRegistration`.
`ModelCatalog::discovered` collects linked registrations, validates keys,
rejects duplicates, and sorts by key rather than linker order. Advanced
`ModelCatalog::from_registrations` avoids hidden global discovery in tests and
embedders.

`Task` is an advanced opaque type created internally from a model. It erases
the Rust model type without erasing constants validation. `TaskDefinition` is
runtime's invocation port; `TaskExecutionHost` supplies schema, cancellation,
and automatic observation boundaries. There is no task context, progress
counter, message callback, one-shot variant, identity setter, or path setter.

### Config subsystem

Config alone opens and parses:

- `<root>/study.json`;
- `<root>/config/state.json`; and
- manifest-referenced `.json` files below `<root>/config/inputs/`.

One manifest task has `model`, `input`, optional `display`, and optional
`timeout_ms`. The old ambiguous `definition` field is rejected. `$sweep`
creates independent Cartesian choices; `$cases` creates correlated choices;
ordinary arrays remain literal. Config caches each unique input source,
preserves exact bytes, expands deterministically, and constructs
`ResolvedTaskInput` values. Its generic `decode<T>()` is the sole typed
constants-supply operation.

Config validates JSON grammar, duplicate keys, path containment, positive
limits, dependency existence, and dependency acyclicity. It deliberately does
not know whether a compiled model key exists, whether constants match a Rust
type, or whether writer/display fields exist. Those are cross-domain Study
checks.

### Study subsystem

`Study::load(&Path)` asks config for a `ProjectSpecification`, asks state to
validate the parsed schema, discovers the immutable model catalog, and binds
every resolved input to its selected model. It validates constants decoding,
writer/schema compatibility, display fields, and model-key coverage before
returning.

Study infers immutable task identities and labels. Identity uses manifest phase
key, global plan ordinal, model key, and input expansion ordinal. Output paths
are not embedded in tasks; Study infers only the project output root
`<project-root>/output`. It preserves phase policy, replicate policy, and exact
source documents for runtime and provenance.

Loading is effect-free with respect to output and models: it creates no output,
starts no threads, and does not call `ScientificModel::initialize`. A writer
declaration is evaluated during preflight under its documented pure-function
contract.

### Runtime subsystem

`runtime::basic::run(&Path)` is re-exported at crate root. It loads Study and
executes it, returning `()` on success. `runtime::advanced::execute(Study)`
accepts an already compiled Study and returns a `RunSummary`.

Only runtime creates output. Each invocation creates a unique generated
directory below `<project-root>/output`, then an isolated
`replicate-XXXXXX` directory and deterministic `task-XXXXXX` recording paths.
It executes phases in stable topological order, admits tasks according to phase
concurrency/start intervals, and honors fail-fast versus finish-all admission.
Task and phase timeouts are cooperative: runtime raises cancellation, and the
task adapter checks between scientific steps. A model that blocks inside one
step cannot be forcibly unwound safely.

The runtime host creates the storage writer at `begin_model`, records the
initial state and each successful step, commits the final state exactly once,
and marks an opened recording failed when model execution errors or is
cancelled. Resolved constants and Workflow-owned identity/provenance are
retained under separate recording-metadata namespaces so inferred fields can
never overwrite scientific constants with the same key.

### Transitional record-related modules

`storage` is the currently used durable backend. It owns exclusive recording
directories, bounded asynchronous queues, chunk framing, checksums, atomic
metadata transitions, final-state checkpoints, resume, and verified reading.
It is intentionally absent from `prelude::basic`; ordinary model authors never
construct it.

`execution::ExecutionScope` currently supplies safe unique/named directory
creation. Runtime uses that narrow behavior. The older subprocess
`ReplicateExecutor` remains directly available only for compatibility tests and
will be absorbed or removed when record/runtime process isolation is finalized.

`artifact` and `rng_record` retain existing direct APIs until the `record`
subsystem consolidates durable provenance. They are not part of the ordinary
prelude and must not leak into model or Study contracts.

## Inference and validation invariants

1. Every filesystem API uses `&Path` for a borrowed path and `PathBuf` for an
   owned path.
2. Config is the only project JSON reader/parser and constants supplier.
3. Model registration keys are stable authored semantics, never Rust type names.
4. Registration discovery is order-independent; catalogs are key-sorted and
   duplicate-checked.
5. Every manifest model reference resolves before output creation.
6. All constants decode and writer/display schema checks complete during Study
   preflight.
7. Models are not initialized during preflight.
8. Study is immutable and clone-cheap; runtime cannot mutate declared intent.
9. Task identities and paths are deterministic within an effective plan.
10. Runtime alone creates output and owns active lifecycle.
11. A canonical model directly owns one stable `SystemState` allocation.
12. Each successful `step` advances iteration strictly.
13. Writer selection is deterministic for a constants value.
14. Application code never reports routine progress or manually completes a
    recording.

## Replacement boundaries

A subsystem may be replaced if its advanced contract and invariants remain
intact:

- a config replacement must preserve strict parsing, deterministic expansion,
  source bytes, containment, and typed decode;
- a study replacement must remain effect-free, consume only supported peer
  APIs, perform complete binding preflight, and publish immutable intent;
- a runtime replacement must accept an immutable Study, preserve phase/task
  policy and cancellation semantics, and own all output creation;
- a record replacement must implement observation persistence without moving
  file mechanics into Writer or model code; and
- a UI replacement must consume bounded runtime-owned snapshots rather than
  shared mutable model contexts.

Every source change requires review of the owning `api.md`; layout, ownership,
tiering, dependency, inference, lifecycle, and persistence changes also require
this document to change in the same patch.
