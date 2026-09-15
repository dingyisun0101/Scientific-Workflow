# 3. Your first study

**Outcome:** run a model with one member and inspect a completed recording.
Complete [installation](2-installation.md) first. This example needs only Rust
to run; its optional readback command uses the Python core reader.

## Create the application

```sh
cargo new population-workflow
cd population-workflow
mkdir -p wf_configs/states
```

Replace the generated manifest and source with the files below. All paths in
this chapter are relative to `population-workflow/`.

## Complete project files

This project has one phase, one task, one execution unit, and one member. It has
no sweep, seed, Python task, custom observation plan, or advanced state provider.
Workflow's default observation plan records both state fields after the initial
state and every successful step.

```text
population-workflow/
+-- Cargo.toml
+-- src/
|   `-- main.rs
`-- wf_configs/
    +-- study.json
    +-- parameters.json
    `-- states/
        `-- population.json
```

`Cargo.toml`:

```toml
[package]
name = "population-workflow"
version = "0.1.0"
edition = "2024"
rust-version = "1.97"

[dependencies]
scientific-workflow = "0.15.4"
serde = { version = "1", features = ["derive"] }
```

`src/main.rs`:

```rust,no_run
use scientific_workflow::prelude::*;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Constants {
    initial_population: u64,
    steps: u64,
}

struct PopulationUnit {
    state: SystemState,
    target_iteration: u64,
}

#[scientific_workflow::execution_unit("population")]
impl ExecutionUnit for PopulationUnit {
    const THREAD_COUNT_INVARIANT: bool = true;

    type Constants = Constants;

    fn initialize(
        constants: Self::Constants,
        schema: &SystemStateSchema,
        _context: &InitializationContext,
    ) -> UnitResult<Self> {
        let mut state = schema.create_empty_state(StateTime::from_iteration(0));
        state.initialize_payload("population", constants.initial_population)?;
        state.initialize_payload("cumulative_births", 0_u64)?;
        Ok(Self { state, target_iteration: constants.steps })
    }

    fn member_count(&self) -> usize {
        1
    }

    fn member(&self, index: usize) -> Option<MemberView<'_>> {
        (index == 0).then(|| MemberView::new(
            "population",
            &self.state,
            (self.state.time().iteration() >= self.target_iteration)
                .then_some(MemberCompletion::without_reason()),
            Some(self.target_iteration),
        ))
    }

    fn step(&mut self) -> UnitResult {
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

fn main() -> Result<(), WorkflowError> {
    run(std::path::Path::new(env!("CARGO_MANIFEST_DIR")))
}
```

`wf_configs/states/population.json` names the state fields:

```json
{
  "fields": [
    {"name": "population"},
    {"name": "cumulative_births"}
  ]
}
```

`wf_configs/parameters.json` supplies the `Constants` fields:

```json
{
  "population": {
    "initial_population": 10,
    "steps": 5
  }
}
```

`wf_configs/study.json` selects the registered unit and state schema:

```json
{
  "active_phases": [0],
  "workflow_schema": 1,
  "threads": 1,
  "compute": {"mode": "auto"},
  "paths": {
    "states": {
      "population": "wf_configs/states/population.json"
    }
  },
  "phases": {
    "simulate": {
      "tasks": [
        {"execution_unit": "population", "state": "population"}
      ]
    }
  }
}
```

Run it from the project directory:

```bash
cargo run
```

Workflow validates all configuration before creating output, initializes the
unit, calls `step` until its member reports completion, and writes the completed
recording beneath `output/` automatically.

## Inspect the outcome

Expect one task to complete at iteration 5. The final population is 15 and
`cumulative_births` is 5. The default `state` stream contains iterations 0–5,
including the initial state. Type `exit` and Enter after completion to close
the dashboard; successful work does not immediately dismiss it.

From the application root, with the Python core reader installed:

```sh
python3 - <<'PYTHON'
from pathlib import Path
from scientific_workflow import open_completed_recording

execution = max(Path("output").glob("execution-*"), key=lambda p: p.stat().st_mtime_ns)
recording = execution / "replicate-000000" / "task-000000"
reader = open_completed_recording(recording)
rows = list(reader.read_stream("state"))
assert [row.iteration for row in rows] == list(range(6))
assert rows[-1].values["population"] == 15
assert rows[-1].values["cumulative_births"] == 5
print("Verified six recorded states; final population:", rows[-1].values["population"])
PYTHON
```

This path is specific to this one-task, one-member example. General pipelines
should discover recordings through the dependency API rather than guess task
ordinals. Each invocation creates a new execution directory.

## If it fails

- An unknown execution-unit key means the registration and JSON do not agree.
- A missing field usually means the schema, constants, or payload initialization
  does not match the files above.
- A terminal error means the run needs interactive stdin and stderr.
- If the dashboard completed but the command remains open, type `exit`.

Keep this project: later chapters extend its parameters and analysis pipeline.

---

**Previous:** [2. Installation](2-installation.md) · **Guide index:** [Documentation](../README.md)

**Next:** [4. Project configuration](4-project-configuration.md)
