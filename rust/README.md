# Scientific Workflow Rust crate

Scientific Workflow is an inference-first library for typed scientific state,
configuration-driven model or program execution, and durable outputs.

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

`ScientificModel::observation_plan` defaults to
`ObservationPlan::all_fields()`. Override it only
when selected fields, named streams, cadence, or units carry scientific
meaning. The observation-plan function may inspect constants but must be
deterministic and side-effect-free.

The stable attribute key is the only bridge from `study.json` to compiled Rust
behavior. There is no separate registry list in `main`.

Standalone executable and Python tasks require no Rust trait or wrapper.
Declare them in `study.json`; they receive the same captured central project
configuration and dependency results at runtime. A Python task declares its
environment locally inside its nested `python` object—there is no global
environment registry.

### 2. Write project JSON

```text
project/
├── study.json
├── scripts/
│   └── plot.py
└── config/
    ├── state.json
    ├── inputs/
    │   └── population.json
    └── plot.json
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
    },
    "plot": {
      "after": ["simulate"],
      "tasks": [{
        "python": {
          "script": "scripts/plot.py",
          "environment": {
            "manager": "mamba",
            "name": "DSES"
          }
        }
      }]
    }
  }
}
```

Persistence is automatic. The omitted root `persistence` object infers the
local backend with 64 MiB chunk and queue settings. Projects that need explicit
operational sizing may add:

```json
"persistence": {
  "chunk_target_bytes": 67108864,
  "queue_capacity_bytes": 67108864
}
```

Users never provide output paths, construct a backend, submit observations, or
finalize a recording.

UI is also automatic. No `ui` object or model display fields are required.
Workflow shows throttled lifecycle and iteration progress only when standard
error is attached to an interactive terminal; redirected/test output remains
silent.

Config alone reads `study.json` and every `.json` file recursively beneath
`config/`, once. Reserved Workflow documents and arbitrary application files
share the same immutable lookup graph. `$sweep` creates independent Cartesian
choices; `$cases` creates correlated alternatives; ordinary arrays remain
literal. Program arguments are optional opaque strings and executables are
started directly without a shell. Python-specific `script`, `environment`, and
`args` stay nested beneath `python`; generic timeout policy remains on the
containing task. Supported managers are `system`, `venv`, `mamba`, `conda`,
`uv`, and `poetry`, with manager-specific paths/names validated during Study
loading.

### 3. Run

```rust,no_run
fn main() -> Result<(), scientific_workflow::WorkflowError> {
    scientific_workflow::run(std::path::Path::new("."))
}
```

That call loads all declarations, discovers models, validates typed constants,
binds observation plans to the state schema, resolves program paths and Python
environments, creates immutable generic tasks and phases, compiles the
effective persistence plan,
infers identities/output paths, schedules work, and persists every task
automatically while publishing inferred terminal progress.

Each program or Python script receives absolute `WORKFLOW_CONFIG_PATH` and
`WORKFLOW_DEPENDENCIES_PATH` snapshot files plus project, execution, replicate,
and task-output paths through `WORKFLOW_*` environment variables. It runs in an
isolated `artifacts/` directory; Workflow captures stdout, stderr, and terminal
status. Editing project JSON after `Study::load` does not alter that execution.

Output is created beneath `<project-root>/output` only after Study preflight
succeeds.

## Architecture at a glance

- `state`: canonical typed scientific state and schema;
- `observation`: observation meaning and borrowed encoding;
- `task`: generic model/program tasks (including Python), `ScientificModel`,
  registration, and uniform invocation;
- `config`: sole all-JSON parser, immutable snapshot, executable/Python
  environment resolver, and typed constants supplier;
- `study`: effect-free binding and immutable declared intent;
- `runtime`: active execution and output creation;
- `persistence`: automatic durable lifecycle and verified reading;
- `ui`: automatic best-effort terminal presentation of Runtime facts;
- `error`: complete-workflow Study/Runtime error composition; and
- `prelude`: central aggregation of module-owned Basic/Advanced tiers.

The crate-level `run(&Path)` facade loads a Study and then invokes Runtime.
Study coordinates declared intent; runtime accepts that completed Study and
coordinates active execution.
Advanced consumers may use `Study::load` and `runtime::advanced::execute`, but
ordinary projects should not.

The repository's [`attractor_2d`](../examples/attractor_2d) project combines
these pieces end to end: six swept Rust model tasks feed one directly declared
Python plotting phase that opens the verified recordings and emits an SVG.

Each target subsystem publishes `module::basic` and `module::advanced`, with
the latter a strict superset. `prelude::basic` contains the small model-author
surface. Persistence writing is automatic and private; only verified reading
appears in the Advanced tier.

See [`src/state/api.md`](src/state/api.md),
[`src/observation/api.md`](src/observation/api.md),
[`src/task/api.md`](src/task/api.md),
[`src/config/api.md`](src/config/api.md),
[`src/study/api.md`](src/study/api.md),
[`src/persistence/api.md`](src/persistence/api.md),
[`src/runtime/api.md`](src/runtime/api.md),
[`src/ui/api.md`](src/ui/api.md),
[`src/error/api.md`](src/error/api.md),
[`src/prelude/api.md`](src/prelude/api.md), and the repository
[`architecture.md`](../docs/architecture.md).

## Validation

From the repository root:

```bash
cargo fmt --manifest-path rust/Cargo.toml -- --check
cargo clippy --manifest-path rust/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path rust/Cargo.toml --all-targets
RUSTDOCFLAGS="-D warnings" cargo doc --manifest-path rust/Cargo.toml --no-deps
```
