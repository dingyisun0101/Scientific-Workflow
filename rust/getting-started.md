# Getting started with Scientific Workflow

This guide introduces the small set of ideas needed to run a first Workflow
project. It assumes basic familiarity with Rust structs and functions, but it
does not assume familiarity with Serde, traits, Workflow's architecture, or its
API reference.

## The mental model

Workflow turns project configuration into scheduled work and recorded results:

```text
Study
`-- Phase
    `-- Task
        `-- ExecutionUnit
            `-- Member -> SystemState -> recording
```

The words have precise meanings:

| Term | Meaning |
| --- | --- |
| **Study** | The complete experiment described by the project configuration: its phases, tasks, dependencies, parameters, and execution settings. Ordinary applications edit `wf_configs/study.json` and call `run`; they do not construct Rust's `Study` type. |
| **Phase** | A named stage of a study, such as `simulate` or `plot`. A phase contains tasks and may depend on earlier phases. |
| **Task** | One schedulable piece of work inside a phase. A task runs either a registered Rust execution unit, an executable program, or a Python script. A parameter sweep can expand one task declaration into several concrete tasks. |
| **Execution unit** | A Rust scientific model adapted to Workflow's lifecycle. Workflow initializes it, repeatedly asks it to step, and observes the members it exposes. |
| **Member** | One independently tracked state and result inside an execution unit. Each member has its own identity, `SystemState`, completion status, progress target, recording, and final result. Most models expose one member; an ensemble can expose several. |
| **State** | The current typed scientific values and time owned by a member. A JSON state schema names the fields, while the execution unit supplies their Rust payload values. |

For a single model, the common case is one task, one execution unit, and one
member. The separate names matter when a task runs an ensemble: the task still
has one coordinated execution-unit lifecycle, but each member is recorded and
completed independently.

## Why Serde and `Deserialize` appear

[Serde](https://serde.rs/) is Rust's standard framework for converting data
between Rust values and formats such as JSON. The two directions are:

- **serialization**: a Rust value becomes JSON or another stored format; and
- **deserialization**: JSON or another input format becomes a Rust value.

There is no separate "deserde" operation. In an ordinary Workflow execution
unit, the part application code needs is deserialization: Workflow converts one
expanded object from `wf_configs/parameters.json` into the unit's Rust
`Constants` type.

```text
parameters.json                         Rust value

{                                       Constants {
  "initial_population": 10,      ->      initial_population: 10,
  "steps": 100                          steps: 100,
}                                       }
```

The derive attribute generates that conversion implementation:

```rust,ignore
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Constants {
    initial_population: u64,
    steps: u64,
}
```

`deny_unknown_fields` is strongly recommended. It turns a misspelled, obsolete,
or unused JSON property into a preflight error instead of silently ignoring it.
Serde is therefore a direct application dependency even though Workflow is the
code that initiates the conversion.

## What a Rust trait is

A Rust **trait** is a behavioral contract, similar to an interface in other
languages. It names associated types and methods that another type promises to
provide. Workflow defines the `ExecutionUnit` trait; an application implements
that trait for its scientific model so Workflow knows how to initialize,
inspect, and advance it.

```rust,ignore
impl ExecutionUnit for PopulationUnit {
    type Constants = Constants;

    // initialize, member_count, member, and step fulfill the contract.
}
```

Two nearby pieces have different jobs:

- `impl ExecutionUnit for PopulationUnit` defines the behavior Workflow can
  call; and
- `#[scientific_workflow::execution_unit("population")]` registers that
  implementation under the stable `population` key used by JSON.

The matching path is:

```text
#[execution_unit("population")]
          |
          +-- study.json task: {"execution_unit":"population", ...}
          `-- parameters.json["population"] -> Constants through Serde
```

## A minimal runnable project

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
scientific-workflow = "0.13.1"
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
  "workflow_schema": 1,
  "threads": 1,
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

## Where to go next

- Follow the [complete Rust guide](README.md) for parameter sweeps,
  external programs, Python tasks, seeds, observation plans, and all public APIs.
- Study the [two-dimensional attractor example](../examples/attractor_2d/README.md)
  for a complete multi-phase project with six swept Rust tasks, Workflow's
  reserved `$npy` conversion phase, and a Python plot phase that reads the
  processed arrays.
- Read the [architecture guide](../docs/architecture.md) when you need subsystem
  ownership, dependency direction, or implementation and replacement details.
