# Scientific Workflow target architecture

## Status and purpose

This document describes the target architecture for the current Scientific
Workflow refactor. `state`, `writer`, `task`, and `config` now implement their
target boundaries. Current orchestration, legacy configuration, and storage
modules coexist with the remaining target modules until their behavior has
been migrated and verified.

The architecture has one overriding usability goal:

> An application author defines scientific state and how it is written,
> defines task behavior, and writes a study manifest, state schema, and task
> input documents.
> Workflow infers and owns the remaining orchestration.

In particular, application code should not construct replicate executors,
execution scopes, phases, schedulers, recording directories, task identities,
provenance documents, progress trackers, or storage lifecycles. Those are
Workflow implementation details derived from task declarations and validated
JSON.

This design also distinguishes two kinds of API:

- **Definition API:** the small surface an application must use to describe
  state, writing, and task behavior.
- **Inspection API:** the narrow read-only surface needed to consume a
  completed recording. Using it is optional and it does not add steps to task
  definition.

Each subsystem publishes only its documented basic and advanced boundaries.
Everything below those boundaries remains private even when it has its own
source file.

## First-time user mental model

A Workflow application has four authored inputs:

1. A scientific state definition.
2. A writer definition describing which scientific observations are retained.
3. One or more task definitions containing the actual scientific or analysis
   work.
4. A study manifest, state schema, and task input documents beneath the project
   root, containing execution policy and application-owned inputs.

The executable calls the single Workflow entry point. Workflow then:

1. accepts one typed project-root path;
2. strictly validates all JSON before creating output;
3. matches manifest task declarations to compiled task definitions;
4. expands task-input combinations deterministically;
5. derives replicate, phase, task, and recording identities;
6. dispatches isolated replicates when required;
7. schedules the selected tasks;
8. creates and manages writers and recording sessions;
9. infers progress from observed state where possible;
10. records source documents, resolved task inputs, paths, seeds, artifacts,
    timing, and lifecycle
    provenance automatically; and
11. finalizes or marks output failed according to the task result.

The application owns scientific meaning. Workflow owns execution mechanics.

## Tiered public surface

Every first-level subsystem is a self-contained replacement boundary. Each one
publishes two explicit API tiers:

- `module::basic` is the ordinary user-facing API. It contains the smallest
  surface needed by application authors and recording consumers.
- `module::advanced` is the complete supported subsystem boundary for advanced
  users **and** other Workflow subsystems. It re-exports that module's basic
  API, then adds stable integration traits, read-only models, adapters, and
  detailed errors needed to compose or replace the subsystem.

`advanced` does not mean “all internals.” Concrete workers, mutable lifecycle
state, unchecked constructors, filesystem implementation types, and format
machinery remain private. Because external users may import the advanced tier,
every symbol in it is documented and treated as a deliberate compatibility
contract.

The crate aggregates the module tiers centrally:

- `prelude::basic` re-exports every first-level module's `basic` API; and
- `prelude::advanced` aggregates every first-level module's `advanced` scope;
  because each module advanced scope includes its basic scope, the central
  advanced prelude is a strict superset of `prelude::basic`.

Direct imports such as `state::basic::SystemState` and
`record::advanced::RecordingDescriptor` remain valid and are preferred when a
consumer wants a narrow dependency. The preludes are conveniences, not
alternative implementations.

The ordinary definition surface still centers on concepts equivalent to
`SystemState`, `Writer`, `Task`, `ScientificModel`, `run`, and `WorkflowError`. A
high-level completed-recording handle belongs in the basic record API. More
specialized integration seams belong in advanced APIs, not in an undifferentiated
crate-root export list.

### Subsystem encapsulation rules

Self-containment is enforced structurally:

1. `src/<module>.rs` declares the subsystem and defines the `basic` and
   `advanced` export scopes inline; it does not become a third re-export
   surface. The architecture uses the modern Rust module layout and contains no
   `mod.rs` files.
2. The inline `basic` scope re-exports only ordinary user-facing symbols owned
   by that subsystem.
3. The inline `advanced` scope begins by re-exporting `basic`, then adds only
   supported integration APIs.
4. A peer subsystem imports another subsystem through
   `crate::<module>::advanced`; it never imports that subsystem's private
   implementation files.
5. Implementation modules are private to their owner. `pub(crate)` is used
   only to assemble the owner's tier exports, not as an informal cross-module
   API.
6. Cross-module behavior is expressed through advanced-tier ports and adapters
   so an implementation can be replaced without moving scientific policy.
7. Dependency cycles are forbidden. If two subsystems appear to require each
   other's implementation, the owning module exposes a narrow advanced trait
   and the other module implements it.
8. Each subsystem owns its validation and detailed error vocabulary. The root
   error composes subsystem errors at the complete-workflow boundary.

An idiomatic subsystem root follows this shape:

```rust
mod definition;
mod error;
mod implementation;

pub mod basic {
    pub use super::definition::PublicDefinition;
    pub use super::error::PublicError;
}

pub mod advanced {
    pub use super::basic::*;
    pub use super::definition::IntegrationDescriptor;
    pub use super::implementation::ReplacementPort;
}
```

The source types are `pub` only so the inline scopes can re-export them; their
owning implementation modules remain private, so the supported canonical paths
are exclusively `module::basic::*` and `module::advanced::*`.

The central prelude follows the same pattern without owning source types:

```rust
pub mod basic {
    pub use crate::state::basic::*;
    pub use crate::task::basic::*;
    // ...the remaining first-level basic scopes.
}

pub mod advanced {
    pub use crate::state::advanced::*;
    pub use crate::task::advanced::*;
    // ...the remaining first-level advanced scopes. Each already includes basic.
}
```

The intended supported dependency direction is:

```text
writer  ──► state
task    ──► state + writer + config
record  ──► state + writer
runtime ──► config + task + state + writer + record + ui

prelude aggregates published tiers but owns no behavior
```

`runtime` is the composition root and may depend on every subsystem's advanced
boundary. `config` remains independent of compiled task implementations: the
runtime performs registry-to-manifest cross-validation. `task` imports only
config's advanced `ResolvedTaskInput`, making config the sole typed constants
supplier without making config depend on compiled task behavior. `record` implements
writer/task ports instead of causing writer or task to depend on record
internals. `ui` owns the event and snapshot contracts populated by runtime, so
the renderer does not import scheduler internals.

### Required `api.md` contract

Every first-level subsystem contains an `api.md`. It is the human-readable,
exhaustive contract for that replacement boundary and must contain separate
`## Basic API` and `## Advanced API` sections. The prelude contains its own
`api.md` documenting aggregation and name-collision policy.

Every module document also contains an `## Example` section that demonstrates
the ordinary supported workflow from loading or acquisition through useful
work. Advanced integration examples may follow within that section.

For every exported symbol, `api.md` records:

- its canonical module path and kind;
- why it exists and which concern owns it;
- who is expected to use it;
- how it is constructed or obtained;
- all public methods or variants and their behavior;
- ownership, borrowing, thread-safety, and lifetime expectations;
- validation rules and invariants;
- effects, persistence, blocking, and cancellation behavior;
- errors and failure atomicity;
- interactions with other module APIs;
- a minimal example and, for advanced APIs, an integration example; and
- compatibility or replacement constraints.

Each document also contains a “Not API” section naming important private
mechanisms that consumers must not depend on. Rustdoc remains required, but it
does not replace `api.md`: rustdoc explains symbols individually, while
`api.md` explains the complete module boundary and how its APIs compose.

An API change is incomplete until the owning `api.md`, the inline tier exports
in `src/<module>.rs`, and its boundary tests change together. Public symbols
absent from `api.md`, or symbols documented in the wrong tier, are
architectural test failures.

### Documentation synchronization rule

Every source-code change includes a documentation-impact review and updates
all associated documentation in the same change. At minimum, touching a
subsystem requires inspecting its `api.md`. Changes to an exported symbol,
invariant, error, effect, integration contract, or example update the relevant
Basic API or Advanced API section.

Changes to file layout, ownership, tier placement, cross-module dependencies,
inference, lifecycle, persistence, or architectural invariants also update this
`docs/architecture.md`. Affected READMEs, design guides, migration notes,
examples,
and test documentation move with the code that makes them stale.

A code task is not complete while its documentation describes old behavior or
while new behavior lacks documentation. When review finds that no text needs
to change, the handoff explicitly records that conclusion and its reason;
documentation is never changed merely to manufacture diff noise.

### Crate-wide path rule

Filesystem paths are typed at every Rust API boundary. Borrowed path
parameters use `&Path`; APIs that take or retain ownership use `PathBuf`.
Filesystem paths are never accepted as raw `&str` or `String`, and owned paths
decoded from JSON use `PathBuf`. This makes path semantics visible in type
signatures and prevents ordinary text from acquiring accidental path meaning.

The implemented task boundary and the remaining runtime composition preserve
these rules:

- each resolved task input is decoded once by config into the task's declared
  input type;
- task identity is derived, never manually supplied in Rust;
- a stateful task binds its state and writer definitions without rebuilding
  storage infrastructure;
- a one-shot task can run without inventing an empty state or writer;
- task invokes the application writer factory once, while runtime owns writer
  session creation, completion, and failure cleanup;
- progress and routine messages are inferred from state observations and
  runtime lifecycle whenever possible; and
- task/model code receives no scheduler, renderer, filesystem-layout, progress
  counter, message channel, or storage-administration context.

## Target source tree

```text
src/
├── lib.rs
├── error.rs
├── prelude.rs
├── prelude/
│   └── api.md
├── state.rs
├── state/
│   ├── api.md
│   ├── field.rs
│   ├── schema.rs
│   ├── time.rs
│   ├── value.rs
│   ├── state.rs
│   ├── series.rs
│   └── error.rs
├── writer.rs
├── writer/
│   ├── api.md
│   ├── definition.rs
│   ├── stream.rs
│   ├── sampling.rs
│   ├── observation.rs
│   ├── encoding.rs
│   ├── session.rs
│   └── error.rs
├── task.rs
├── task/
│   ├── api.md
│   ├── definition.rs
│   ├── model.rs
│   ├── execution.rs
│   └── result.rs
├── config.rs
├── config/
│   ├── api.md
│   ├── document.rs
│   ├── manifest.rs
│   ├── expansion.rs
│   ├── input.rs
│   ├── specification.rs
│   └── error.rs
├── runtime.rs
├── runtime/
│   ├── api.md
│   ├── bootstrap.rs
│   ├── replicate.rs
│   ├── plan.rs
│   ├── phase.rs
│   ├── scheduler.rs
│   ├── executor.rs
│   ├── scope.rs
│   ├── cancellation.rs
│   ├── completion.rs
│   ├── clock.rs
│   └── error.rs
├── record.rs
├── record/
│   ├── api.md
│   ├── layout.rs
│   ├── session.rs
│   ├── metadata.rs
│   ├── provenance.rs
│   ├── queue.rs
│   ├── chunk.rs
│   ├── format.rs
│   ├── format/
│   │   ├── metadata.rs
│   │   ├── state.rs
│   │   └── checksum.rs
│   ├── checkpoint.rs
│   ├── recovery.rs
│   ├── artifact.rs
│   ├── rng.rs
│   ├── reader.rs
│   └── error.rs
├── ui.rs
└── ui/
    ├── api.md
    ├── event.rs
    ├── snapshot.rs
    ├── renderer.rs
    ├── terminal.rs
    ├── plain.rs
    ├── command.rs
    └── error.rs
```

## Root files

### `src/lib.rs`

`lib.rs` declares the first-level subsystems and the central `prelude`. It does
not flatten every symbol into the crate root. Each subsystem owns its exports
through `basic` and `advanced`; `lib.rs` makes those modules reachable and lets
the prelude aggregate them.

It also contains the crate-level explanation of the user flow and the
ownership rules. It must not become a second implementation layer: startup is
delegated to `runtime::bootstrap`, and concrete behavior stays in the module
that owns it.

Adding a direct crate-root export requires demonstrating that the item cannot
live coherently in an owning module tier. Ordinarily, applications import
`prelude::basic::*`, advanced consumers import `prelude::advanced::*`, and
narrow integrations import one module tier directly.

### `src/error.rs`

`error.rs` defines `WorkflowError`, the composed error returned by complete
workflow operations. It provides contextual top-level variants such as invalid
study input, task failure, recording failure, and runtime failure.

Subsystems own precise errors in their own `error.rs` files. A simple module
error may appear in basic when ordinary users must handle it; detailed
diagnostic or adapter errors may appear in advanced. Private implementation
errors still remain private. All supported subsystem errors preserve source
chains and convert into `WorkflowError` at the composition boundary.

## `prelude`: central API aggregation

The `prelude` module owns no behavior. It provides the two central import
surfaces requested by the architecture while preserving direct, self-contained
module APIs.

### `src/prelude.rs`

Defines `prelude::basic` and `prelude::advanced` as inline export scopes. The
basic scope re-exports the contents of every first-level `module::basic`. The
advanced scope aggregates every first-level `module::advanced`, each of which
already includes its corresponding basic scope.

The file contains no independent aliases or implementation logic, preventing a
third accidental API tier. Name collisions must be resolved in the owning
modules rather than hidden with prelude-only aliases. The central prelude is an
index, never the original owner of an API.

### `prelude::basic` export scope

The recommended import for ordinary application authors. It is the exact union
of the first-level basic scopes and contains no prelude-only API.

### `prelude::advanced` export scope

The supported superset for advanced users and subsystem integrations. A
consumer importing it receives the basic API plus every published advanced
integration contract.

### `prelude/api.md`

Lists every aggregated module tier, documents collision and glob-import
policy, and provides side-by-side examples of basic, advanced, and narrow
direct imports. Its Basic API section documents `prelude::basic`; its Advanced
API section documents `prelude::advanced` and explicitly identifies the
additional integration responsibilities accepted by advanced consumers.

## `state`: scientific state ownership

The `state` module owns scientific state structure, values, scientific time,
and ordered in-memory series. Its standalone schema loader reads one JSON
template through a typed `&Path`; it knows nothing about task scheduling, the
study manifest, recording layout, or terminal display.

State remains application-owned. Workflow may validate, borrow, encode, and
reconstruct it, but does not attach model semantics to fields.

### `src/state.rs`

Declares `state::basic`, `state::advanced`, and the private implementation
modules. It contains no third set of re-exports. Internal schema, erased-value,
and validation machinery reaches consumers only when deliberately published
through one of the two inline tier scopes.

### `state::basic` export scope

Exports `StateTime`, `SystemStateSchema`, `SystemState`, `StateSeries`, and their
ownership-preserving error types. This is the ordinary loading, initialization,
typed payload access, time advancement, and analysis contract. It excludes
field metadata, structural maintenance, erased values, decoder hooks, and
reconstruction adapters.

### `state::advanced` export scope

Re-exports `state::basic`, then adds `StateFieldSchema`, `StateSchemaAccess`,
`StateMaintenance`, `StateSchemaSource`, and the documentation-hidden sealed
`PayloadTuple` bound. Writer, storage during migration, and the future record
module depend on this boundary rather than on `field.rs`, `schema.rs`,
`value.rs`, or `series.rs` directly.

Advanced state APIs must preserve type safety: they may expose checked schema
and traversal contracts, but never raw erased payload mutation or unchecked
field positions.

### `state/api.md`

Documents every state symbol in separate Basic API and Advanced API sections.
For fields and payload access it specifies initialization-versus-replacement,
type-mismatch behavior, borrowing rules, clone behavior, schema identity,
iteration ordering, and serialization expectations. Its Example section runs
from JSON loading through typed work and a state series. Its advanced section
explains how a replacement writer or record backend consumes schemas and
reconstructs states without depending on the current container implementation.

### `state/field.rs`

Defines the immutable description of one state field: its stable normalized
name, template position, and optional scientific description. Rust payload
types are intentionally bound by state initialization rather than duplicated
in JSON field metadata. Schema loading validates field names and prevents
duplicate declarations.

Field position is an internal encoding optimization. Writers refer to fields
through checked definitions rather than asking users to coordinate numeric
indices.

### `state/schema.rs`

Loads and validates the complete schema from a strict JSON template. It
establishes stable field order, publishes allocation-identity inspection
through its advanced tier, and produces the schema description persisted with recordings. The public
loader accepts `&Path`, while internal reconstruction parses embedded bytes
without publishing a second construction API.

Direct loading remains supported for standalone use and tests. The future
runtime should derive and load the configured schema once rather than asking
task code to pass schema handles or paths repeatedly.

### `state/time.rs`

Defines `StateTime`, the generic scientific coordinate consisting of a
mandatory iteration and optional finite physical time. It validates safe
one-step iteration advancement and finite physical arithmetic. Strict ordering
between snapshots is enforced by `StateSeries`, not by the coordinate value.

Axis display names and units come from the state or writer definition, while
operational UTC time belongs to recording metadata. Tasks do not manually copy
iteration values into a progress bar; writer observations make current
simulation time visible to the runtime.

### `state/value.rs`

Contains private type-erasure machinery needed to store heterogeneous
scientific payloads while returning their concrete Rust types through checked
accessors. It owns downcasting and serializable borrowing.

No erased-value type crosses the public boundary.

### `state/state.rs`

Implements `SystemState`. It owns one-time payload initialization, deliberate
same-type replacement, checked borrowing, mutation, extraction, structural
cloning through the advanced maintenance trait, and transactional time
advancement.

The file preserves clone-free access for large payloads. It does not serialize
recording envelopes or decide when a state should be sampled.

### `state/series.rs`

Implements `StateSeries`, an ordered in-memory collection of complete states
for analysis. It enforces shared schema-allocation identity and strictly
increasing iteration. Borrowing `&StateSeries` is the lightweight view; no
separate public view wrapper exists.

This is the in-memory scientific view used after decoding. It performs no
filesystem I/O and knows nothing about chunks or recording completion.

### `state/error.rs`

Defines state-owned failures such as unknown fields, type mismatches, duplicate
schema entries, incomplete states, and invalid time progression. The basic and
advanced scopes export the documented portions needed by their consumers;
implementation-only representation failures remain private. Complete workflow
calls convert supported state failures into `WorkflowError` without losing
their source information.

## `writer`: application-defined observation

The `writer` module is the boundary between scientific state and durable
recording. The application defines **what** should be observed. Workflow owns
**where**, **when the lifecycle starts and ends**, and **how bytes are made
durable**.

A writer definition may declare scientifically meaningful streams, selected
fields, positive iteration cadences, and optional axis units. Axis names are
inferred from `StateTime`. It must not ask the application
for output directories, task ordinals, metadata duplication, queue plumbing,
chunk filenames, or explicit completion calls.

### `src/writer.rs`

Declares the two writer tiers and private implementation modules. The private
`WriterSession` is visible only to the transitional storage adapter; no session
or concrete encoder is part of either supported tier.

### `writer::basic` export scope

Exports exactly `Writer`, `Stream`, and `WriterError`. `Writer::all_fields`
infers one `state` stream containing every schema field at every iteration;
applications introduce named streams, selected fields, or a larger iteration
cadence only when those choices carry scientific meaning. It offers no public
field-selection, sampling, time-axis, directory, chunk, queue, provenance, or
lifecycle-administration type.

### `writer::advanced` export scope

Re-exports `writer::basic`, then adds the stable ports used to plug a writer
into another recording backend: `WriterDescriptor`, `StreamDescriptor`,
borrowed `Observation` views, owned `EncodedObservation` handoffs,
`ObservationSink`, and `SessionOutcome`.

The advanced boundary separates definition from persistence. Record implements
the sink/session ports; writer never imports record implementation files.
Unchecked encoding state and concrete queue handles remain private.

### `writer/api.md`

Documents every writer definition and integration symbol under separate Basic
API and Advanced API sections. It explains inferred defaults, stream
uniqueness, field validation, iteration cadence, schema identity, borrowed
encoding, backpressure, final-state handling, and runtime-owned lifecycle. The
advanced section includes the complete port a replacement record backend must
implement, including failure atomicity, terminal outcomes, and ownership of
buffers.

### `writer/definition.rs`

Defines the application-facing `Writer` contract. A definition binds to one
immutable state schema and declares its observations. `WriterDescriptor::bind`
validates it once before work begins and canonicalizes every field selection
into schema order.

The definition contains no recording directory and no replicate or task ID.
Those are injected internally after task-input expansion has produced a stable
runtime identity.

### `writer/stream.rs`

Defines one scientifically named `Stream` and its selected state fields, plus
the schema-bound `StreamDescriptor`. Construction validates normalized names,
duplicate selections, and positive cadence; binding validates field existence
and infers whether the stream covers the complete schema.

Stream names and field selection survive because they carry scientific
meaning. Relative directories and metadata filenames do not; recording layout
derives them.

### `writer/sampling.rs`

Implements the private positive iteration cadence and its efficient due/not-due
decision. The representation is not public: `Stream::every_iterations(u64)`
accepts the irreducible value and `StreamDescriptor::every_iterations()`
reports it.

If a writer has only one natural observation cadence, the default is inferred.
Applications specify a sampling rule only when multiple scientifically valid
choices exist.

### `writer/observation.rs`

Represents a checked borrowed observation of a state at one simulation time.
It verifies exact schema-allocation identity and is the synchronous handoff to
canonical encoding. It does not allocate copies of large payloads. The future
runtime may derive progress from observations around this boundary without
adding progress mutation to the observation object itself.

### `writer/encoding.rs`

Converts selected, borrowed state fields into an owned `EncodedObservation`.
It enforces schema order and rejects missing or non-serializable payloads
before a record reaches the durable queue. It also defines the replaceable
`ObservationSink` port and runtime-owned `SessionOutcome`.

It owns scientific payload encoding, not chunk framing, checksums, or
filesystem format installation; those belong to `record`.

### `writer/session.rs`

Implements the private runtime handle behind a writer definition. It applies
cadence, prevents decreasing observations, deduplicates equal and terminal
iterations, and produces owned observations before advancing any accepted
iteration marker.

The runtime creates and finalizes this session. A task cannot publish a
completed recording while it is still running or forget to mark a failed
recording after returning an error.

### `writer/error.rs`

Defines the non-exhaustive `WriterError` for declaration, field selection,
schema binding, observation ordering, state access, and encoding. Every
failure preserves stream, field, and iteration context where applicable and
chains state or Serde sources without exposing record internals.

## `task`: application-owned work

The `task` module owns one irreducible boundary: it turns heterogeneous typed
application behavior into a uniform definition that runtime can invoke. A
stateful task binds resolved typed constants, an application-owned
`ScientificModel`, and a `Writer`; a one-shot task binds typed constants and
one callback. Workflow derives identity, display label, resolved-input
combination, phase, output scope, lifecycle, progress representation,
messaging, and provenance outside this module.

Task wraps the complete scientific workload from calculation through the
automatic observation boundary. It does not perform durable recording itself.
The runtime manages tasks as members of phases, while task remains independent
of phase and scheduling types.

### `src/task.rs`

Declares the task tiers and private task implementation modules. It creates no
task registry or global state merely by being imported.

### `task::basic` export scope

Exports exactly `Task`, `ScientificModel`, and `TaskResult`. `Task::stateful`
binds a model type to a constants-borrowing writer factory;
`Task::one_shot` binds a typed callback without inventing an empty state or
writer. `ScientificModel` requires typed constants, initialization, canonical
state access, a completion predicate, and one-step scientific evolution. It
offers only an optional target-iteration hint beyond those requirements.

The canonical application model must directly own the `SystemState` returned
by `state()`. Rust cannot structurally prove field ownership through a trait,
so the requirement is prominently documented and runtime enforces the
observable contract: stable state address, exact schema allocation, strictly
increasing iteration after every successful step, and valid monotonic optional
targets.

There is no `TaskRun` or task/model context. Model state is the authority for
scientific output; it does not duplicate information into progress objects,
step reports, shared counters, or messages.

### `task::advanced` export scope

Re-exports `task::basic`, then adds `TaskKind`, `TaskDescriptor`,
`TaskDefinition`, and `TaskExecutionHost`. The descriptor exposes only
stateful-versus-one-shot shape, a diagnostic constants type name, and whether
a schema is required. `TaskDefinition` lets runtime inspect and synchronously
execute an opaque task against one config-owned `ResolvedTaskInput`. `TaskExecutionHost`
is the runtime port for the canonical schema, cancellation checks, initial
writer handoff, successful-step observations, and the final model boundary.

Identity, registry mutation, prepared models, worker handles, lifecycle
outcomes, display models, and concrete cancellation/storage mechanisms remain
private to config/runtime/record/UI. Advanced task access cannot redirect
output, construct identity, or independently mark a task complete.

### `task/api.md`

Documents all task-facing and integration APIs in separate Basic API and
Advanced API sections. It exhaustively describes typed constants ownership,
direct canonical state ownership, stateful and one-shot construction,
initialization and step contracts, schema/state/iteration/target enforcement,
cooperative cancellation, automatic observation and messaging boundaries,
task result conversion, descriptor stability, and host lifetimes. Its example
runs from a typed model definition to the minimal replacement-runtime call.

### `task/definition.rs`

Defines opaque reusable `Task` ownership and the two constructors. Each task
stores a shared type-erased definition so cloning does not duplicate closures,
models, states, constants, or writers. It supplies no public execution,
identity, label, path, metadata, progress, recording, or lifecycle methods.

### `task/model.rs`

Defines `ScientificModel`. Initialization consumes the one decoded constants
value and borrows the runtime-loaded schema. The returned model is fully usable
and directly owns its canonical state. `state`, `is_complete`, and
`target_iteration` are side-effect-free inspection; `step` performs exactly
one observable transition and returns no duplicate report value.

### `task/execution.rs`

Defines the supported advanced descriptor, type-erased definition, and host
port, plus private stateful/one-shot adapters and model-contract validation.
Stateful execution validates its writer before model initialization, emits an
initial host boundary, checks cancellation between steps, validates the same
state owner/schema and advancing iteration after each success, and emits step
and final host boundaries. One-shot execution skips schema and writer access.

Runtime and UI use these observation boundaries to automate messages. The
standard snapshot includes the label from `study.json`, lifecycle, current
`StateTime` iteration, optional target, and elapsed time. Users may select
additional scientific state fields in `study.json`; runtime validates the
selection against the schema and extracts bounded values at the same boundary.
Formatting, truncation, throttling, channels, and shared counters are internal.

### `task/result.rs`

Defines `TaskResult<T = ()>` as a thread-safe owned boxed-error result. This
lets application error enums retain their source chains without publishing a
large task-owned union. Contract-check error representations remain private;
runtime treats returned failure, its own cancellation flag, and successful
completion as distinct lifecycle inputs.

## `config`: strict JSON and inference

The `config` module is the sole compiler for a project's declarative files. It
turns one typed project-root path into an immutable `ProjectSpecification`.
Ordinary users write files and never invoke config from Rust; runtime consumes
its advanced boundary.

Vocabulary separates Workflow declarations from application input:

- `study.json` is the **study manifest**;
- `config/state.json` is the **state schema document**;
- referenced files below `config/inputs/` are **task input documents**;
- one concrete selection is a **resolved task input**; and
- its typed Rust value is one set of **model constants** or one-shot input.

Config is the only subsystem that opens or parses these project files. State
receives an already parsed schema document for field-semantic validation. Task
receives `ResolvedTaskInput` and obtains typed constants only through the
config-owned `decode` operation. Runtime, record, and UI do not reread sources
or inspect raw task-input JSON.

```text
<project-root>/
├── study.json
└── config/
    ├── state.json
    └── inputs/
        └── <application-selected>.json
```

The future runtime takes only `project_root: &Path`. Config derives the study
manifest, `config` directory, and state schema path. Each manifest task
declares an opaque compiled `definition` and an `input` path below
`config/inputs`. Absolute paths, parent traversal, non-normal components,
non-JSON inputs, and canonical symlink escapes are rejected.

### `src/config.rs`

Declares an intentionally empty basic tier and the supported advanced project
compiler boundary. It coordinates private document parsing, manifest
validation, selection expansion, typed-input storage, and specification
assembly. Importing it performs no loading.

### `config::basic` export scope

Intentionally exports no symbols. Configuration's user API is its documented
file grammar. Application code declares `ScientificModel::Constants` or a
one-shot input type and receives one owned config-supplied value through task
execution. It does not load, query, expand, or resolve project files.

### `config::advanced` export scope

Re-exports the empty basic tier, then publishes `ProjectSpecification`,
`StudyManifest`, `ReplicatePolicy`, `ReplicateScheduling`, `FailurePolicy`,
`PhaseSpecification`, `ResolvedTaskInput`, `ProjectDocument`,
`StateSchemaDocument`, and `ConfigError`.

`ProjectSpecification::load(&Path)` is the sole entry. It canonicalizes the
root, parses each unique source once, resolves deterministic defaults,
validates the dependency graph and path containment, expands task input
selections, and preserves exact source bytes. The result is immutable,
clone-cheap, `Send`, and `Sync`.

Config remains independent of compiled task implementations. Runtime matches
each opaque `definition` to `task::advanced::TaskDefinition`. Task depends only
on config's advanced `ResolvedTaskInput` contract so typed constants cannot be
supplied by raw JSON or another parser.

### `config/api.md`

Documents the intentionally empty Basic API and every advanced symbol. It
defines the study-manifest and task-input grammars, central parser authority,
terminology, typed constants supply, duplicate-key behavior, source
preservation, deterministic expansion, defaults, containment, dependency
validation, errors, examples, and replacement constraints.

### `config/document.rs`

Owns strict duplicate-preserving JSON parsing and immutable project source
documents. `ProjectDocument` retains canonical paths and exact bytes for
provenance. `StateSchemaDocument` also retains the parsed value passed to the
state advanced semantic-validation seam. Raw syntax trees and visitors remain
private.

### `config/manifest.rs`

Defines the private strict manifest deserialization grammar and public
read-only effective policy models. It validates names, replicate counts, task
declarations, display-field uniqueness, dependency existence, duplicates,
self-dependencies, and cycles. Object order remains deterministic phase order.

### `config/expansion.rs`

Recursively expands literal values, independent `$sweep` markers, and
correlated `$cases` into deterministic complete inputs. Ordinary arrays remain
literal. It validates markers, choices, case field sets, disjoint paths, and
safe Cartesian allocation.

### `config/input.rs`

Defines `ResolvedTaskInput`, owning one expanded value, definition key,
canonical source path, combination ordinal, display selection, timeout, and
provenance JSON. Generic `decode<T>` is the sole constants supplier and
contextualizes failures without exposing individual-key or raw-value APIs.

### `config/specification.rs`

Implements the one-root loading transaction, canonical containment, unique
input cache, state-document acquisition, expansion assembly, and immutable
`ProjectSpecification`. It performs no output creation, compiled-task matching,
state semantics, scheduling, or execution.

### `config/error.rs`

Defines non-exhaustive config-owned errors for reads, syntax, duplicate keys,
document grammar, containment, dependency references, expansion, and complete
typed-input decoding. Errors retain typed paths, JSON locations, definition
and ordinal context, and underlying I/O/Serde chains.

The old `configuration` module remains only as a temporary compatibility
surface for unmigrated study/execution/example code. It is not an alias and is
not part of the target config tiers.

## `runtime`: inferred orchestration

The `runtime` module owns startup and execution. It replaces public study,
phase, replicate, and execution-scope builders with one JSON-derived plan.

Applications cannot mutate runtime policy after validation. Scientific tasks
remain ordinary application code invoked behind the private executor boundary.

### `src/runtime.rs`

Declares private runtime implementation modules and defines the inline
`runtime::basic` and `runtime::advanced` export scopes. It is the composition
boundary, not a container for scheduler implementation.

### `runtime::basic` export scope

Exports the single ordinary run entry point and its high-level completed
summary. It accepts task definitions through the task basic contract and
infers launch mechanics from JSON. It does not expose builders for plans,
phases, replicates, scopes, or schedulers.

### `runtime::advanced` export scope

Re-exports `runtime::basic`, then adds supported read-only plan and execution
models plus ports for embedding the runtime, supplying a process launcher,
observing lifecycle events, injecting a clock, or selecting an executor.

Advanced runtime APIs permit replacement at subsystem seams but do not allow
mutation of a validated plan or direct construction of contradictory phase,
task, and recording identities.

### `runtime/api.md`

Documents startup, plan inspection, embedding, execution, cancellation,
completion examination, and summaries in separate Basic API and Advanced API
sections. It specifies the controller/worker contract, process and thread
behavior, blocking calls, failure policy, side effects, path creation point,
event ordering, cancellation guarantees, and adapter protocols. The advanced
section includes a complete example of replacing one runtime adapter while
retaining the validated plan and lifecycle invariants.

### `runtime/bootstrap.rs`

Implements the end-to-end startup transaction. It loads the project
specification, validates task-definition matches, builds the plan, decides controller versus
worker mode, initializes records and UI, executes selected work, and returns
the root result.

This is the sole composition root. No other module independently recreates a
partial startup sequence.

### `runtime/replicate.rs`

Owns controller/worker process isolation, replicate scheduling, failure
policy, deterministic replicate indices, and reserved environment transfer.
Output roots and seeds come from the validated effective plan.

Applications never instantiate a replicate executor or branch manually on an
optional replicate context.

### `runtime/plan.rs`

Builds the immutable effective launch plan from the validated project
specification, resolved task inputs, registered definitions, and inferred defaults.
The plan is deterministic and serializable before execution.

It is the single source for scheduler input, UI declaration, output identities,
and persisted plan provenance.

### `runtime/phase.rs`

Represents a validated internal phase and its tasks, dependencies, barriers,
and effective policies. Phase IDs and labels derive from `study.json`; they are
not Rust builder parameters.

### `runtime/scheduler.rs`

Owns phase ordering, dependency barriers, concurrency, prepared-work bounds,
delayed admission, timeouts, deadlines, and failure propagation. It consumes
the immutable plan and emits lifecycle events.

The scheduler knows nothing about model equations or state field meaning.

### `runtime/executor.rs`

Runs one prepared task by implementing `TaskExecutionHost`. It supplies the
canonical schema and cancellation state, opens the runtime-managed
writer/recording session at the initial model boundary, observes successful
steps and completion, catches outcomes, requests finalization or failure
marking, and reports automatic lifecycle/progress/message events.

This file is the only place where application task execution and recording
lifecycle meet.

### `runtime/scope.rs`

Owns validated filesystem scopes for the study launch, replicate, phase, task,
recording, artifact, and lifecycle files. Every path derives from the plan and
is checked for containment.

No raw scope or path-construction helper is public. A task receives verified
resource handles rather than assembling paths.

### `runtime/cancellation.rs`

Implements study-wide and task-local cooperative cancellation tokens and
propagation. It has no terminal or task-model knowledge.

### `runtime/completion.rs`

Examines reusable whole-phase results once per launch, caches each verdict,
and applies the same verdict to dependency selection, execution, recording,
and display. Task-level continuation remains the responsibility of validated
recording recovery and the task definition.

### `runtime/clock.rs`

Provides monotonic timing and UTC timestamps behind an injectable internal
clock. Tests use deterministic clocks without changing public execution APIs.

### `runtime/error.rs`

Defines runtime-owned failures for bootstrap, replicate processes, plan
creation, scheduling, scopes, cancellation, completion examination, and task
execution. Supported diagnostic forms are exported through the appropriate
tier; scheduler bookkeeping failures remain private.

## `record`: durable output and provenance

The `record` module owns how output becomes durable, inspectable, recoverable,
and reproducible. It is below `writer`: writer definitions select scientific
observations, while `record` owns queues, chunks, checksums, metadata, and
filesystem installation.

Most of this module remains private. Its basic tier publishes a narrow
completed-recording inspection handle; its advanced tier publishes replacement
seams without exposing unchecked format machinery.

### `src/record.rs`

Declares private recording implementation modules, defines inline
`record::basic` and `record::advanced` scopes, and coordinates recording
services used through those boundaries. It is the sole owner of the durable
format.

### `record::basic` export scope

Exports high-level completed-recording inspection, verified state-series and
latest-state readback, immutable artifact handles, and concise recording
summaries. Ordinary users cannot create lifecycle metadata, select chunk
filenames, bypass verification, or drive recovery manually.

### `record::advanced` export scope

Re-exports `record::basic`, then adds supported backend integration contracts:
record sinks, validated recording descriptors, lifecycle requests, provenance
inputs, checkpoint/recovery policies, artifact stores, RNG provenance ports,
and detailed record errors.

Mutable format documents, queue workers, partial chunks, atomic-installation
guards, and unchecked decoders remain private. Writer and runtime use the
advanced contracts rather than importing these mechanisms.

### `record/api.md`

Documents every inspection and integration API under separate Basic API and
Advanced API sections. It specifies lifecycle states, format/version
verification, schema checks, checksum guarantees, chunk ordering,
backpressure, checkpoint compatibility, recovery authority, artifact
immutability, RNG derivation, provenance completeness, and failure atomicity.
The advanced section gives the full protocol for replacing the durable backend
without weakening writer or runtime guarantees.

### `record/layout.rs`

Derives and validates every filename and relative directory inside a recording.
It prevents path traversal, collisions, and inconsistent layout logic between
writers, readers, continuation, and cleanup.

### `record/session.rs`

Owns the lifecycle of one recording: create, running, continuing, completed,
or failed. It atomically installs metadata transitions and ensures that only a
successfully finalized recording is visible as complete.

The executor, not application task code, drives these transitions.

### `record/metadata.rs`

Defines in-memory recording metadata and validates lifecycle, state schema,
stream descriptions, effective settings, timing, and terminal summaries.

### `record/provenance.rs`

Collects provenance automatically from the validated launch: exact source
documents, resolved task inputs, effective inferred defaults, task and
replicate identity, named resources, software format version, artifacts, RNG
records, and lifecycle timing.

Applications do not attach source documents, resolved task inputs, or project
paths manually.

### `record/queue.rs`

Implements bounded asynchronous persistence and backpressure. Queue capacity is
an effective runtime setting, normally inferred and always recorded.

### `record/chunk.rs`

Seals encoded state records into size-bounded chunks without splitting one
record. It tracks counts, iteration ranges, encoded byte totals, and digests.

### `record/format.rs`

Defines the current durable format version and coordinates its metadata, state
framing, and checksum components. Version dispatch for readers is centralized
here.

### `record/format/metadata.rs`

Serializes and strictly decodes the on-disk metadata documents. It rejects
unknown or internally contradictory durable fields instead of guessing.

### `record/format/state.rs`

Defines state-record framing and decoding. It keeps payload order consistent
with the persisted schema and validates simulation time during readback.

### `record/format/checksum.rs`

Computes and verifies content digests for chunks, artifacts, and other
immutable payloads. Digest comparison occurs before bytes are accepted as
scientific data.

### `record/checkpoint.rs`

Identifies restart-capable streams, validates complete checkpoint states, and
reconstructs the latest compatible checkpoint for continuation.

### `record/recovery.rs`

Validates interrupted recordings, removes or quarantines only provably
incomplete temporary state, and prepares a consistent continuation boundary.
It never silently overwrites a completed recording.

### `record/artifact.rs`

Publishes immutable content-addressed inputs and derived resources, then
verifies them on load. Artifact paths and descriptors integrate automatically
with task provenance.

### `record/rng.rs`

Derives namespace-separated seeds from the study seed, replicate index, and
task identity. It records each requested RNG source lazily without owning the
application's random-number generator implementation.

### `record/reader.rs`

Implements high-level completed-recording inspection. It verifies lifecycle,
format version, schema, framing, byte counts, and digests before reconstructing
state series or the latest state.

The basic record scope exposes a small recording handle rather than the format
types used internally by this file.

### `record/error.rs`

Defines record-owned failures for layout, lifecycle, metadata, queueing,
chunks, checksums, checkpoints, recovery, artifacts, RNG provenance, and
readback. Basic and advanced select documented diagnostic views; raw worker and
temporary-installation failures remain private.

## `ui`: runtime-owned human interaction

The `ui` module owns all human-facing execution display and command input. It
observes immutable snapshots and sends control requests; it has no scientific
or storage authority.

### `src/ui.rs`

Declares private UI implementations and defines inline `ui::basic` and
`ui::advanced` export scopes. Study-manifest policy selects terminal, plain,
or hidden presentation through these contracts.

### `ui::basic` export scope

Exports only ordinary task-facing presentation concepts that cannot be
expressed as plain text, such as message severity when retained. Display mode
and renderer selection normally come from `study.json`, so the basic UI API is
intentionally small and may require no direct application import.

### `ui::advanced` export scope

Re-exports `ui::basic`, then adds stable event, immutable snapshot, renderer,
command-source, and output-sink contracts. Runtime emits UI-owned events and
snapshots through this boundary; UI never imports scheduler implementation.

Terminal handles, mutable render slots, raw channel endpoints, and command
editing state remain private.

### `ui/api.md`

Documents the basic message concepts and all advanced event/rendering seams in
separate sections. It specifies event ordering, bounded-channel behavior,
snapshot consistency, refresh and blocking behavior, exclusive terminal
ownership, plain-output determinism, command authority, cancellation requests,
and renderer shutdown. The advanced section includes an example replacement
renderer that consumes supported snapshots only.

### `ui/event.rs`

Defines bounded lifecycle and message events sent by the runtime and active
tasks. Events carry stable internal identities but never application-owned
state payloads.

### `ui/snapshot.rs`

Builds immutable display snapshots from current phase, task, progress,
message, and cancellation state. Renderers consume snapshots rather than
locking scheduler internals while drawing.

### `ui/renderer.rs`

Owns the renderer loop, event draining, refresh timing, and exclusive output
coordination. It is the only component allowed to write live study display.

### `ui/terminal.rs`

Implements interactive terminal rendering, including phase sections, task
progress, messages, status, and command input placement.

### `ui/plain.rs`

Implements deterministic append-only, uncolored lifecycle output suitable for
logs and non-interactive processes. Hidden mode uses the same lifecycle path
while suppressing output.

### `ui/command.rs`

Parses supported user commands and converts them into runtime control requests.
It cannot mutate scheduler or recording state directly.

### `ui/error.rs`

Defines UI-owned failures for terminal ownership, rendering, event transport,
and command input. Adapter-relevant failures may be advanced API; terminal
backend implementation details remain private.

## End-to-end ownership flow

```text
application State + Writer + Task definitions
                         │
study manifest + state schema + task input documents
             │           │
             ▼           ▼
       config loading, strict parsing, expansion, and validation
                         │
                         ▼
               immutable effective runtime plan
                         │
             ┌───────────┴───────────┐
             ▼                       ▼
      replicate controller      replicate worker
                                     │
                                     ▼
                           phase scheduler/executor
                                     │
                    model constants ─┤
                                     ▼
                              application Task
                                     │
                              State observations
                                     │
                                     ▼
                          runtime-managed Writer
                                     │
                                     ▼
                    durable Record + inferred progress
                                     │
                       ┌─────────────┴─────────────┐
                       ▼                           ▼
                completed output            UI snapshots
                       │
                       ▼
              verified recording reader
```

The important direction is downward: runtime invokes a task with constants
supplied by config, and task execution automatically exposes checked state observation
boundaries to its runtime host, but application models, state, and writer do
not reach upward into orchestration.

## What is inferred

The refactor should remove the following application-supplied Rust parameters:

- every project path below the one user-supplied project root;
- output root and replicate directory construction;
- replicate executor construction and controller/worker branching;
- phase IDs and builder calls;
- task IDs, labels, categories, and resolved-input ordinals;
- task recording directories and study-record paths;
- source-document and resolved-input provenance attachment;
- explicit progress initialization when state or task input supplies it;
- writer lifecycle calls for create, complete, and fail;
- operational timestamps and timing metadata;
- chunk filenames, stream directories, queue workers, and atomic installation;
- RNG provenance bookkeeping; and
- explicit plan/display assembly.

Inference must be deterministic, validated, and inspectable in the effective
plan or provenance. “Inferred” never means unrecorded guesswork.

## What remains application-defined

The application still supplies information that carries irreducible scientific
meaning:

- state fields, payload types, and scientific descriptions;
- equations, algorithms, stopping rules, and domain validation;
- meaningful observation streams and selected fields;
- scientifically meaningful sampling policy when no unique default exists;
- task input types and task behavior;
- replicate count, seed, study structure, and task dependencies in JSON when
  the study requires them;
- scientific input values, sweeps, and explicit cases in task input documents;
- references to external scientific inputs.

An option does not remain public merely because the current implementation has
a builder method for it. It remains only when Workflow cannot infer one safe,
deterministic behavior and the choice materially changes scientific intent or
resource policy.

## Mapping from the current top-level modules

The refactor consolidates the current boundaries as follows:

| Current concept | Target owner |
|---|---|
| `system_state` | `state` |
| `time_series` | `state::series` |
| public storage writer configuration | `writer` definition |
| storage queues, chunks, continuation, and readers | `record` |
| `configuration` | self-contained `config` tiers with private parser implementation |
| `study`, phases, tasks, plans, and scheduling | `task` definition plus `runtime` tiers with private scheduling implementation |
| `execution` and replicate scopes | `runtime`, with scopes private and replacement ports advanced |
| `artifact` | `record::artifact` |
| `rng_record` | `record::rng` with runtime-owned task integration |
| terminal study display | private `ui` |
| current split preludes | replaced by central `prelude::basic` and `prelude::advanced`, each aggregating module-owned tiers |
| private `clock` | `runtime::clock` |

This is a move in ownership, not permission to duplicate behavior. During
migration, old modules should delegate to the new owner or be removed once all
callers move; two independent implementations must not survive.

## Architectural tests

The architecture is successful when tests can demonstrate all of the
following:

1. A minimal application contains only state, writer, task, and JSON
   definitions plus one run call.
2. No application test imports a phase builder, replicate executor, execution
   scope, configuration iterator, storage builder, task ID, or metadata map.
3. Reordering Rust task registration cannot change persisted task identity.
4. The same inputs produce the same expanded plan and output layout.
5. All input errors discoverable without effects occur before output creation.
6. Every inferred effective setting is visible in the plan or provenance.
7. A successful task automatically finalizes its recording.
8. A failed or cancelled task cannot leave a recording marked complete.
9. Stateful progress advances through writer observation without duplicate
   task-context iteration updates.
10. Completed readback verifies lifecycle, schema, framing, byte counts, and
    digests before returning scientific state.
11. Public API tests fail if concrete internal builders, mutable paths/scopes,
    unchecked format types, or scheduler implementations become reachable
    through either supported tier.
12. Current scientific consumers can migrate without reimplementing Workflow
    behavior downstream.
13. Every first-level subsystem root defines inline `basic` and `advanced`
    scopes, and advanced is a strict superset of basic.
14. `prelude::basic` exactly aggregates module basic scopes, while
    `prelude::advanced` exactly aggregates every supported module export.
15. Every first-level subsystem contains `api.md` with separate Basic API and
    Advanced API sections and an exhaustive canonical-path inventory.
16. Boundary tests compare public exports to `api.md` so undocumented or
    incorrectly tiered APIs fail validation.
17. Peer subsystems import one another only through `module::advanced`; source
    checks reject imports from sibling implementation files.
18. The source tree contains no `mod.rs`, `basic.rs`, or `advanced.rs`; tier
    scopes live in modern-layout root files such as `src/state.rs`.
19. Every filesystem-path API uses `&Path` for borrowing and `PathBuf` for
    ownership; no path parameter is raw string data.

## Decision rule for future API additions

Before adding a public type, function, parameter, or builder method, answer:

1. Does the application own the scientific meaning of this choice?
2. Can the value be derived uniquely from state, writer, task, or validated
   JSON?
3. Can the behavior live behind an existing owning module type or advanced
   replacement port?
4. Would exposing it let applications create contradictory identity,
   lifecycle, path, or provenance state?
5. Is the choice needed by ordinary users, advanced integrators, or a peer
   subsystem through a genuine replacement seam—or only by the current
   implementation and its tests?

If the value can be inferred safely, infer and record it. If it is an
implementation mechanism, keep it private. Basic API is reserved for
irreducible ordinary user intent; advanced API is reserved for deliberate,
documented user and cross-subsystem integration contracts.
