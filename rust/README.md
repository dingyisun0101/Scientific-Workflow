# Scientific Workflow Rust crate

Scientific Workflow is an inference-first library for typed scientific state,
configuration-driven model execution, and durable observations.

## Installation

```toml
[dependencies]
scientific-workflow = "0.10.0"
serde = { version = "1", features = ["derive"] }
```

Rust 1.97 or newer is required. Executables should commit `Cargo.lock`.

## Complete user procedure

### 1. Define a model

The model directly owns its canonical `SystemState`. Its associated constants
type is the complete typed form of one expanded input document.

```rust,no_run
use serde::Deserialize;
use scientific_workflow::prelude::basic::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Constants {
    initial_population: u64,
    steps: u64,
}

struct PopulationModel {
    state: SystemState,
    steps: u64,
}

#[scientific_workflow::model("population")]
impl ScientificModel for PopulationModel {
    type Constants = Constants;

    fn initialize(constants: Constants, schema: &SystemStateSchema) -> TaskResult<Self> {
        let mut state = schema.create_empty_state(StateTime::from_iteration(0));
        state.initialize_payload("population", constants.initial_population)?;
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

`ScientificModel::writer` defaults to `Writer::all_fields()`. Override it only
when selected fields, named streams, cadence, or units carry scientific
meaning. The writer function may inspect constants but must be deterministic
and side-effect-free.

The stable attribute key is the only bridge from `study.json` to compiled Rust
behavior. There is no separate registry list in `main`.

### 2. Write project JSON

```text
project/
├── study.json
└── config/
    ├── state.json
    └── inputs/
        └── population.json
```

`config/state.json`:

```json
{"fields":[{"name":"population"}]}
```

`config/inputs/population.json`:

```json
{
  "initial_population": 10,
  "steps": {"$sweep":[100, 200]}
}
```

`study.json`:

```json
{
  "phases": {
    "simulate": {
      "tasks": [
        {"model":"population", "input":"inputs/population.json"}
      ],
      "max_concurrency": 2
    }
  }
}
```

Config alone reads these files. `$sweep` creates independent Cartesian choices;
`$cases` creates correlated alternatives; ordinary arrays remain literal.

### 3. Run

```rust,no_run
fn main() -> Result<(), scientific_workflow::WorkflowError> {
    scientific_workflow::run(std::path::Path::new("."))
}
```

That call loads all declarations, discovers models, validates typed constants,
binds writers and display fields to the state schema, creates immutable tasks
and phases, infers identities/output paths, schedules work, and records every
model automatically.

Output is created beneath `<project-root>/output` only after Study preflight
succeeds.

## Architecture at a glance

- `state`: canonical typed scientific state and schema;
- `writer`: observation meaning and borrowed encoding;
- `task`: `ScientificModel`, registration, and uniform invocation;
- `config`: sole JSON parser and typed constants supplier;
- `study`: effect-free binding and immutable declared intent;
- `runtime`: active execution and output creation.

Study coordinates declared intent; runtime coordinates active execution.
Advanced consumers may use `Study::load` and `runtime::advanced::execute`, but
ordinary projects should not.

Each target subsystem publishes `module::basic` and `module::advanced`, with
the latter a strict superset. `prelude::basic` contains the small model-author
surface. Transitional storage/execution/artifact/RNG APIs remain directly
importable but are intentionally absent from that prelude.

See [`src/task/api.md`](src/task/api.md),
[`src/config/api.md`](src/config/api.md),
[`src/study/api.md`](src/study/api.md),
[`src/runtime/api.md`](src/runtime/api.md), and the repository
[`architecture.md`](../docs/architecture.md).

## Validation

From the repository root:

```bash
cargo fmt --manifest-path rust/Cargo.toml -- --check
cargo clippy --manifest-path rust/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path rust/Cargo.toml --all-targets
RUSTDOCFLAGS="-D warnings" cargo doc --manifest-path rust/Cargo.toml --no-deps
```
