# Scientific Workflow

Scientific Workflow is a Rust library for reproducible, inspectable scientific
programs. It provides typed scientific state, application-defined observation,
configuration-driven task inputs, deterministic task behavior, bounded durable
recording, replicate isolation, artifact verification, and RNG provenance.
Equations, numerical methods, stopping rules, and domain validation remain in
the application that understands them.

> **Release status:** this crate is test software. Public API behavior may
> change before 1.0. Treat each version update as a coordinated migration.

## Installation

Scientific Workflow 0.10 requires Rust 1.97 or newer:

```toml
[dependencies]
scientific-workflow = "0.10.0"
```

The crate has no Cargo features. Executable applications should commit their
`Cargo.lock` so deployments resolve the same compatible dependency versions.

## First-time mental model

An application author supplies only the scientific information Workflow cannot
infer:

1. the scientific state and its JSON schema;
2. a writer describing meaningful observations;
3. typed task behavior; and
4. a study manifest plus task input documents.

The target project layout is:

```text
<project-root>/
├── study.json
└── config/
    ├── state.json
    └── inputs/
        ├── run.json
        └── analysis.json
```

The vocabulary is intentionally precise:

- `study.json` is the **study manifest**;
- `config/state.json` is the **state schema document**;
- files referenced below `config/inputs/` are **task input documents**;
- one concrete expansion is a **resolved task input**; and
- its task-declared Rust value is one set of **model constants**.

There is one project-declaration subsystem: `config`. The former
`configuration` module, its manual `combinations()` API, JSON Pointer decoding,
and named-path table have been removed.

## Target workflow

```text
project_root: &Path
        │
        ▼
config::advanced::ProjectSpecification
        ├── exact source documents
        ├── effective replicate and phase policy
        ├── centrally parsed state schema
        └── resolved task inputs
                    │
                    ▼
              typed constants
                    │
                    ▼
Task ──► ScientificModel ──► SystemState ──► Writer
                    │
                    ▼
             runtime + record
```

Config reads and parses project declarations but creates no output and executes
no work. Task owns scientific execution but not identity, paths, scheduling,
progress administration, messages, or lifecycle. The future runtime is the
composition root that will infer those mechanics and persist their effective
values as provenance.

`state`, `writer`, `task`, and `config` already implement their target
boundaries. The current `study`, `execution`, and `storage` modules remain
transitional until their behavior moves into `runtime`, `record`, and `ui`.

## Public API tiers

Every target first-level subsystem exposes two inline scopes:

- `module::basic` is the minimal ordinary user API.
- `module::advanced` is a strict superset for advanced users and peer
  subsystems.

`prelude::basic` centrally aggregates ordinary APIs. `prelude::advanced`
includes all basic names plus supported integration contracts. Direct narrow
imports such as `state::basic::*` and `config::advanced::*` remain supported.
The prelude owns no behavior.

### `state`

`state::basic` provides `StateTime`, `SystemStateSchema`, `SystemState`, and
`StateSeries` with typed heterogeneous payload access. Filesystem loaders take
borrowed `&Path`; state values retain immutable schema allocation identity.

`state::advanced` adds schema inspection and maintenance contracts used by
writer, config composition, and recording backends. Config passes the already
parsed state-schema value through `StateSchemaAccess`, avoiding a second file
read or JSON parser.

```rust,no_run
use std::path::Path;
use scientific_workflow::state::basic::*;

# fn main() -> Result<(), StateError> {
let schema = SystemStateSchema::load_json_template(Path::new("config/state.json"))?;
let mut state = schema.create_empty_state(StateTime::from_iteration(0));
state.initialize_payload("population", vec![10_u64, 20, 30])?;
state.payload_mut::<Vec<u64>>("population")?.push(40);
state.advance_time(None)?;
# Ok(())
# }
```

See `src/state/api.md` for the exhaustive contract.

### `writer`

`writer::basic` provides `Writer`, `Stream`, and `WriterError`.
`Writer::all_fields()` infers one `state` stream containing every schema field
at every iteration. Applications introduce named streams, selected fields, or
larger iteration cadences only when they carry scientific meaning.

`writer::advanced` provides schema-bound descriptors, checked borrowed
observations, owned encoded handoffs, and the replaceable `ObservationSink`
port. Writer owns observation semantics, not output paths, recording lifecycle,
chunk framing, or scheduler state.

See `src/writer/api.md` for the exhaustive contract.

### `task`

`task::basic` provides `Task`, `ScientificModel`, and `TaskResult`. A canonical
model directly owns its `SystemState`. Config supplies one complete typed
`Constants` value; the writer factory borrows the same value before model
initialization.

```rust,no_run
use scientific_workflow::prelude::basic::*;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Constants {
    initial: u64,
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
        state.initialize_payload("population", constants.initial)?;
        Ok(Self { state, remaining: constants.steps })
    }

    fn state(&self) -> &SystemState { &self.state }
    fn is_complete(&self) -> bool { self.remaining == 0 }

    fn step(&mut self) -> TaskResult {
        *self.state.payload_mut::<u64>("population")? += 1;
        self.state.advance_time(None)?;
        self.remaining -= 1;
        Ok(())
    }
}

let task = Task::stateful::<Model, _>(|_| Ok(Writer::all_fields()));
# let _ = task;
```

`task::advanced` provides read-only descriptors and the execution host port.
Runtime handles initial, per-step, and final observations automatically. Model
code receives no generic context, path, task identity, progress counter,
message callback, recording session, or completion control.

See `src/task/api.md` for the exhaustive contract.

### `config`

`config::basic` is intentionally empty: the ordinary user interface is the
file grammar. `config::advanced::ProjectSpecification::load(&Path)` is the sole
project loader. It strictly parses every declaration, rejects duplicate JSON
keys, validates paths and phase dependencies, preserves exact source bytes, and
expands task inputs deterministically.

Exact `{"$sweep":[...]}` objects introduce independent Cartesian selections.
`$cases` introduces correlated alternatives. Ordinary arrays are always
literal. Runtime creates one task invocation per resulting
`ResolvedTaskInput`; users never iterate combinations themselves.

See `src/config/api.md` for the complete manifest and input grammar, every
advanced symbol, defaults, errors, effects, and examples.

## Transitional modules

The following APIs still compile existing orchestration and recording work but
are not the target application surface:

- `study` owns manual phases, scheduling, display, cancellation, and durable
  study records. It no longer parses or expands project declarations.
- `execution` owns current replicate subprocess dispatch and filesystem scopes.
  `ReplicateExecutor` consumes config's validated `ReplicatePolicy` and an
  owned output `PathBuf`. `ReplicateContext::seed_deriver()` returns `None`
  when the manifest declares no base seed.
- `storage` owns the current durable writer and verified reader. It accepts
  caller metadata generically and contains no configuration-specific adapter.
- `artifact` and `rng_record` retain immutable artifact verification and
  reproducibility provenance until they move beneath `record`.

These modules will be replaced or absorbed during the runtime and record
passes. New scientific behavior should use the target state/writer/task/config
boundaries rather than introducing another transitional convenience API.

## Attractor example

`examples/attractor_2d` loads one `ProjectSpecification`, uses its resolved task
inputs and typed `AttractorConstants`, obtains the centrally parsed state
schema, and passes the validated replicate policy to the current execution
adapter. It temporarily maps resolved inputs into legacy study phases until the
runtime pass provides the single run entry point.

```bash
mamba run -n DSES cargo run --manifest-path examples/attractor_2d/Cargo.toml
```

The final phase invokes Python and Matplotlib through the maintained `DSES`
Mamba environment.

## Validation

From the repository root:

```bash
cargo fmt --manifest-path rust/Cargo.toml -- --check
cargo clippy --manifest-path rust/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path rust/Cargo.toml --all-targets
RUSTDOCFLAGS="-D warnings" cargo doc --manifest-path rust/Cargo.toml --no-deps
```

See `../docs/tests.md` for the responsibility-oriented test map and
`../docs/architecture.md` for the complete target file tree, ownership rules,
and migration direction.
