# Config API

The `config` subsystem is the sole compiler for a Workflow project's
declarative files. It receives one typed project-root path, reads every
declaration centrally, performs strict JSON parsing and deterministic task
input expansion, and publishes one immutable `ProjectSpecification` for
runtime. Its canonical scopes are `scientific_workflow::config::basic` and
`scientific_workflow::config::advanced`.

The terminology is deliberately precise:

- `study.json` is the **study manifest**, containing Workflow-owned
  orchestration declarations;
- `config/state.json` is the **state schema document**;
- JSON files referenced below `config/inputs/` are **task input documents**;
- one concrete expansion result is a **resolved task input**;
- decoding one resolved task input for `ScientificModel::Constants` produces
  one set of **model constants**; and
- all model-constant combinations produce separate runtime task instances.

These files are not interchangeable “configs.” Config understands the study
manifest grammar and selection markers, but it assigns no meaning to
application fields inside task input documents. The task's declared Rust type
owns that meaning.

The conventional layout is:

```text
<project-root>/
├── study.json
└── config/
    ├── state.json
    └── inputs/
        ├── run.json
        └── analysis.json
```

Config is the only Workflow subsystem that opens or parses these declarative
files. State receives the already parsed state-schema value for semantic field
validation. Task receives `ResolvedTaskInput` and obtains constants only
through its config-owned typed decoding operation. Runtime, record, and UI do
not independently reread or parse project declarations.

## Basic API

### `config::basic`

The basic scope is intentionally empty. Configuration's ordinary user
interface is the project file grammar, not a Rust loader or query API.

Application code writes the study manifest, state schema, and task input
documents, then supplies only `project_root: &Path` to the future
`runtime::basic` entry point. It does not construct a config object, load JSON,
request individual keys, iterate combinations, resolve paths, or attach
provenance.

An empty scope is still an intentional published tier. It lets
`config::advanced` remain the strict supported superset without inventing a
user-facing type solely for symmetry. `prelude::basic` consequently gains no
config-owned names.

## Advanced API

`config::advanced` re-exports the empty Basic API and publishes the complete
read-only integration boundary used by runtime, tests, and replacement
integrations. No exported type has a public constructor other than
`ProjectSpecification::load`.

### `config::advanced::ProjectSpecification`

`ProjectSpecification` is a cheap cloneable handle to one complete validated
project declaration. It is immutable, `Send`, and `Sync`; cloning shares source
documents, parsed state-schema data, phase specifications, and resolved task
inputs rather than rereading files or repeating expansion.

#### `ProjectSpecification::load(project_root: &Path)`

Canonicalizes the borrowed project root, then performs the complete
failure-atomic compilation:

1. canonicalize the project root and its `config` directory;
2. read and strictly parse `<project-root>/study.json`;
3. validate the Workflow-owned manifest grammar and phase dependency graph;
4. read and strictly parse `<project-root>/config/state.json` without yet
   interpreting state fields;
5. resolve every task `input` reference beneath `config/inputs`;
6. canonicalize each input path and reject absolute, traversal, non-JSON, and
   symlink escapes;
7. read and strictly parse every unique task input document once;
8. expand `$sweep` and `$cases` in deterministic declaration order;
9. create one `ResolvedTaskInput` per concrete combination; and
10. preserve every unique source document in first-use order.

Loading performs filesystem reads, canonicalization, parsing, validation,
allocation, and expansion synchronously. It creates no output, starts no
thread, executes no task, loads no payload, constructs no model or writer, and
publishes nothing on failure. A missing root, directory, schema, or input is a
`ConfigError::Read` retaining the attempted typed path.

#### Inspection methods

- `project_root()` returns the canonical absolute root retained by the
  specification.
- `config_root()` returns the canonical `<project-root>/config` directory.
- `manifest()` borrows the validated `StudyManifest`.
- `state_schema()` borrows the centrally parsed `StateSchemaDocument`.
- `phases()` returns `PhaseSpecification` values in study-manifest declaration
  order.
- `documents()` returns every unique `ProjectDocument` in first-use order:
  study manifest, state schema, then task inputs as referenced by phases and
  tasks.

These methods perform no I/O, parsing, allocation, blocking, persistence, or
cancellation work. Returned borrows remain valid for the specification borrow.

### `config::advanced::StudyManifest`

`StudyManifest` is the validated Workflow-owned portion of `study.json`. It is
`Copy`, immutable, and obtained only through `ProjectSpecification::manifest`.
Its sole method, `replicate_policy()`, returns the complete effective
`ReplicatePolicy`.

Phase and task traversal is deliberately exposed only from
`ProjectSpecification`, where every task input has already been resolved. No
second raw-manifest traversal model can disagree with the effective launch
description.

### `config::advanced::ReplicatePolicy`

An immutable `Copy` effective policy:

- `count()` returns the positive number of isolated replicate executions;
- `scheduling()` returns `ReplicateScheduling`;
- `failure_policy()` returns `FailurePolicy`; and
- `base_seed()` returns `Option<u64>`. `None` means deterministic work need not
  invent RNG material or provenance.

When the entire `replicates` object is absent, config infers one sequential,
fail-fast replicate with no seed. When the object exists, any omitted field
uses that same deterministic default. The effective values—not merely authored
ones—are retained for planning and provenance.

### `config::advanced::ReplicateScheduling`

A `Copy` enum with two variants:

- `Sequential`: complete one replicate before launching the next;
- `Parallel`: admit eligible replicates concurrently.

`as_str()` returns the exact manifest spellings `"sequential"` and
`"parallel"`. This policy expresses deliberate operational intent; config
does not launch processes itself.

### `config::advanced::FailurePolicy`

A shared `Copy` policy used for replicate and sibling-task failure propagation:

- `FailFast`: prevent further admission after the first failure;
- `FinishAll`: allow already declared sibling work to finish.

`as_str()` returns `"fail_fast"` or `"finish_all"`. Config only validates and
retains the choice. Runtime implements its lifecycle effects.

### `config::advanced::PhaseSpecification`

One immutable validated phase obtained from `ProjectSpecification::phases`.
It owns no scheduler or mutable lifecycle state.

- `name()` returns the exact manifest object key. Phase names must be nonempty
  and contain no surrounding whitespace.
- `dependencies()` iterates validated dependency names in declaration order.
  Self-dependencies, duplicates, missing names, and dependency cycles prevent
  the entire specification from being published.
- `tasks()` returns fully expanded `ResolvedTaskInput` values in deterministic
  order: task declaration order first, then combination order within each
  declaration.
- `max_concurrency()` returns the positive effective active-task bound. The
  default is one.
- `start_interval()` returns the effective delay between admissions as
  `Duration`; the default is zero.
- `timeout()` returns the optional effective whole-phase timeout.
- `failure_policy()` returns the effective sibling-task policy; the default is
  `FailFast`.

Phase inspection performs no effects. Runtime may derive identities, labels,
output scopes, and scheduler structures from these declarations but cannot
mutate them.

### `config::advanced::ResolvedTaskInput`

One immutable, cheap-cloneable concrete task input. It is the only bridge from
an application-authored task input document to typed model constants or a
typed one-shot input.

- `definition()` returns the nonempty manifest string selecting compiled task
  behavior. Config treats it as opaque; runtime cross-validates it against
  compiled `Task` definitions.
- `source_path()` returns the canonical task input document path.
- `ordinal()` returns the zero-based combination ordinal within that one task
  declaration's input space.
- `display_fields()` iterates optional additional scientific state field names
  in authored order. Config checks only nonblank uniqueness; runtime validates
  them against the state schema before execution.
- `timeout()` returns the optional task-specific effective timeout.
- `resolved_json()` returns deterministic compact JSON bytes for provenance.
  It does not expose a mutable or general-purpose JSON dictionary.
- `decode<T: DeserializeOwned>()` decodes the complete resolved input as one
  owned typed value. It never rereads or reparses the source file. A mismatch
  returns `ConfigError::DecodeTaskInput` with definition, source path,
  combination ordinal, and the underlying Serde error.

`decode` is the sole supported constants-supply operation. Stateful task
execution requests `M::Constants`; one-shot execution requests its declared
input type. Users do not call it in normal application code, and no module
decodes individual JSON Pointers. Types that require a closed object grammar
should use `#[serde(deny_unknown_fields)]`.

The resolved value is owned behind a shared immutable handle. Cloning an input
does not duplicate its JSON tree or provenance bytes. Decoding allocates only
as required by `T` and application deserializers. It performs no filesystem
I/O, persistence, background work, or cancellation check.

### `config::advanced::ProjectDocument`

An immutable cheap-cloneable pair of canonical source path and exact validated
source bytes:

- `path()` returns `&Path`;
- `bytes()` returns the exact bytes originally read.

Documents have no public constructor. Exact bytes allow record to persist
provenance without rereading a file that may have changed. A document is not a
parsed-value query interface and does not expose mutable storage.

### `config::advanced::StateSchemaDocument`

The centrally parsed state schema awaiting state-owned semantic validation:

- `path()` returns the canonical `config/state.json` path;
- `bytes()` returns exact source bytes; and
- `json_value()` borrows the strictly parsed JSON value for the state module's
  `StateSchemaAccess::from_json_template_value` integration.

`json_value()` is a narrow subsystem seam, not a general task-input escape
hatch. Config owns JSON syntax, duplicate-key rejection, source reading, and
source preservation. State owns field declarations, normalized names,
descriptions, ordering, and schema allocation. Runtime composes the two once.

### `config::advanced::ConfigError`

`ConfigError` is a non-exhaustive error enum. Consumers should match variants
of interest and preserve a fallback arm.

- `Read { path, source }`: canonicalization or required filesystem access
  failed.
- `Parse { path, source }`: a source was not syntactically valid JSON.
- `DuplicateKey { path, key }`: an object repeated a key that ordinary JSON
  maps would silently overwrite.
- `InvalidDocument { path, pointer, reason }`: valid JSON violated the study,
  task-input selection, or naming grammar. `pointer` is the nearest meaningful
  JSON Pointer and `/` means the document root.
- `PathOutsideConfig { path, config_root }`: an authored or canonical input
  path was absolute, traversed, used a disallowed component, was not a JSON
  file beneath `inputs`, or escaped through a symlink.
- `UnknownDependency { phase, dependency }`: a phase references no declared
  phase.
- `ExpansionOverflow { path }`: a selection product or allocation cannot be
  represented safely.
- `DecodeTaskInput { definition, path, ordinal, source }`: the complete
  resolved input does not deserialize as the task-requested type.

Loading is failure-atomic: no `ProjectSpecification` escapes an error. Typed
decode does not mutate the resolved input and leaves it reusable after error.
All underlying I/O and Serde sources remain in the standard error chain.

## Study manifest grammar

The manifest root accepts exactly `replicates` and `phases`:

```json
{
  "replicates": {
    "count": 4,
    "scheduling": "parallel",
    "failure_policy": "finish_all",
    "base_seed": 1101
  },
  "phases": {
    "simulate": {
      "tasks": [
        {
          "definition": "model",
          "input": "inputs/run.json",
          "display": {
            "include": ["population"]
          },
          "timeout_ms": 30000
        }
      ],
      "max_concurrency": 4,
      "start_interval_ms": 10,
      "timeout_ms": 3600000,
      "failure_policy": "fail_fast"
    },
    "analyze": {
      "after": ["simulate"],
      "tasks": [
        {
          "definition": "analysis",
          "input": "inputs/analysis.json"
        }
      ]
    }
  }
}
```

Workflow-owned objects reject unknown fields. `phases` is a nonempty ordered
object. Each phase contains a nonempty `tasks` array plus optional `after`,
`max_concurrency`, `start_interval_ms`, `timeout_ms`, and `failure_policy`.
Each task contains exactly `definition`, `input`, optional `display`, and
optional `timeout_ms`. `display` accepts only `include`.

All millisecond values are `u64` and convert to `Duration`. Config does not
interpret timeouts or delays as scientific time.

## Task input selection grammar

Ordinary values, including arrays, are literal. An object containing exactly
`$sweep` introduces one independent Cartesian selection:

```json
{
  "shape": [64, 64],
  "temperature": {"$sweep": [280.0, 300.0]},
  "solver": {
    "method": {"$sweep": ["rk4", "euler"]}
  }
}
```

This yields four resolved inputs in declaration-order mixed-radix order:
temperature first as the outer dimension, solver method second as the inner
dimension. Sweep choices may be arbitrary literal JSON values, but must not
contain nested selection markers. Empty sweeps are invalid.

An object may instead declare correlated alternatives with `$cases`:

```json
{
  "shape": [64],
  "$cases": [
    {"temperature": 280.0, "step": 0.02},
    {"temperature": 300.0, "step": 0.01}
  ]
}
```

Cases must be nonempty objects with identical flattened field sets. They merge
with disjoint fixed siblings. A case cannot overlap a fixed leaf, contain
selection markers, or mix with a sweep inside the same object. Cases at one
object may still participate in Cartesian products introduced by an ancestor
or sibling object.

Keys beginning with `$` are reserved. Currently only exact `$sweep` and
`$cases` positions are accepted. Expansion never treats ordinary array
contents as selection syntax.

## Example

An ordinary application defines its typed model constants and task but calls
no config API:

```rust,no_run
use scientific_workflow::prelude::basic::*;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Constants {
    initial_population: u64,
    steps: u64,
}

struct Model {
    state: SystemState,
    remaining: u64,
}

impl ScientificModel for Model {
    type Constants = Constants;

    fn initialize(constants: Constants, schema: &SystemStateSchema) -> TaskResult<Self> {
        let mut state = schema.create_empty_state(StateTime::from_iteration(0));
        state.initialize_payload("population", constants.initial_population)?;
        Ok(Self {
            state,
            remaining: constants.steps,
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
}

fn model_task() -> Task {
    Task::stateful::<Model, _>(|_| Ok(Writer::all_fields()))
}
```

The user writes `config/inputs/run.json`:

```json
{
  "initial_population": 10,
  "steps": {"$sweep": [100, 200]}
}
```

Config produces two resolved task inputs and supplies two separate `Constants`
values to two executions. The user never calls `combinations()`.

A runtime integration loads the advanced boundary and composes state semantic
validation without another filesystem read:

```rust,no_run
use std::path::Path;

use scientific_workflow::config::advanced::ProjectSpecification;
use scientific_workflow::state::advanced::{StateSchemaAccess, SystemStateSchema};

fn prepare(project_root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let project = ProjectSpecification::load(project_root)?;
    let state_document = project.state_schema();
    let schema = <SystemStateSchema as StateSchemaAccess>::from_json_template_value(
        state_document.path(),
        state_document.json_value(),
    )?;

    for phase in project.phases() {
        for input in phase.tasks() {
            println!(
                "definition={} combination={}",
                input.definition(),
                input.ordinal(),
            );
        }
    }

    let _ = schema;
    Ok(())
}
```

Runtime later matches each opaque `definition` to compiled task behavior,
validates selected display fields against `schema`, and invokes
`TaskDefinition` with the corresponding `ResolvedTaskInput`.

## Not API

The following are intentionally private and replaceable:

- duplicate-preserving JSON syntax-tree types and Serde visitors;
- raw manifest deserialization structs;
- mutable maps used while parsing and validating;
- path canonicalization and containment helper functions;
- input-document caches and first-use bookkeeping;
- Cartesian product allocation and recursive object-merging algorithms;
- flattened case-path comparison;
- JSON Pointer escaping helpers;
- `Arc` layouts and resolved-value storage;
- dependency graph traversal state; and
- concrete defaulting implementation.

There is no public `StudyConfiguration`, `WorkloadConfiguration`,
`ResolvedConfiguration`, `ConfigurationIter`, `ProjectPaths`, JSON Pointer
lookup, individual-value decoder, component/workload hierarchy, configuration
builder, source mutation API, arbitrary document loader, task registry,
scheduler, state-schema interpreter, output path, or execution method in this
subsystem.

The legacy `configuration` module remains temporarily available only for the
unmigrated study/execution/example implementation. It is not part of the target
`config::basic` or `config::advanced` contract and will be removed when runtime
adopts `ProjectSpecification`.
