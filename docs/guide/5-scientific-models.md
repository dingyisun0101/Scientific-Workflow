# 5. Scientific models

**Outcome:** adapt your own scientific model to the first study's execution-unit
contract. Use chapter 3's complete implementation as the working starting point.

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
    const THREAD_COUNT_INVARIANT: bool = true;

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

## Implement the lifecycle

| Method | What your model supplies | Practical check |
| --- | --- | --- |
| `preflight` | Optional validation and observation plan; defaults record all fields. | Reject invalid scientific constants before work starts. |
| `initialize` | Owned member state initialized from constants and the supplied schema. | Initialize each required payload with its intended Rust type. |
| `member_count` | Number of exposed members. | Keep member indexing and identities consistent. |
| `member` | Borrowed state, completion status, identity, and optional target. | Return `None` for invalid indices; do not clone state for display. |
| `step` | One scientific advance, including updating state time. | Return regularly so cooperative pause/cancellation can take effect. |

The chapter 3 model owns one `SystemState`. An ensemble owns several member
states and returns a separate `MemberView` for each. Workflow records and tracks
members independently while calling the unit's shared `step` method.

`THREAD_COUNT_INVARIANT = true` is a scientific promise that automatic changes
to a unit's compute allocation do not change its result. Declare it only if that
is true. Otherwise use isolated mode and fixed task resources. Workflow's private
Rayon pool supplies execution context; a sequential model remains sequential
unless its implementation performs parallel scientific work.

## Extend the population model

To add a growth parameter, change its Rust `Constants`, the matching
`parameters.json` section, and `step` together. Preserve `deny_unknown_fields`
so obsolete settings fail clearly. Do not add scheduling loops, output filenames,
or JSON parsing inside the model: those remain Workflow responsibilities.

To expose sampling cadence through JSON, add a cadence field to `Constants`
and use it in `preflight` to construct an observation plan. Observation policy
is scientific model behavior; it is not an arbitrary `study.json` property.

## Complete implementation and advanced APIs

### Define an execution unit

The execution unit directly owns its canonical `SystemState`. Its associated constants
type is the complete typed form of one expanded section selected from
`wf_configs/parameters.json` by the execution unit's registration key.
In the fixed-duration example below, `StateTime::iteration()` is the current
completed iteration, while `target_iteration` retains the configured stopping
target; it is not a second progress counter.

`ExecutionUnit` is a Rust trait: a behavioral contract that lists what Workflow
must be able to ask a scientific model to do. Writing
`impl ExecutionUnit for PopulationUnit` promises that `PopulationUnit` supplies
that lifecycle. The separate `#[execution_unit("population")]` attribute only
registers the implementation under the JSON key `population`; it does not
generate the model, state fields, or method bodies.

```rust,no_run
use serde::Deserialize;
use scientific_workflow::prelude::*;

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
        constants: Constants,
        schema: &SystemStateSchema,
        _context: &InitializationContext,
    ) -> UnitResult<Self> {
        let mut state = schema.create_empty_state(StateTime::from_iteration(0));
        state.initialize_payload("population", constants.initial_population)?;
        state.initialize_payload("cumulative_births", 0_u64)?;
        Ok(Self {
            state,
            target_iteration: constants.steps,
        })
    }

    fn member_count(&self) -> usize { 1 }

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
```

`ExecutionUnit::preflight` defaults to
`ObservationPlan::all_fields()`. Override it only
when domain validation or selected fields, named streams, cadence, or units
carry scientific meaning. The preflight function may inspect constants and
the selected schema but must be deterministic and side-effect-free.

The execution unit is an ordinary Rust owner: its single `MemberView` returns a direct
borrow of its `SystemState`, and coupled field access uses a typed tuple expansion. The
attribute does not define the execution unit or generate field access. Its stable key is
only the automatic bridge from `wf_configs/study.json` to compiled Rust behavior, so there
is no separate registry list in `main`.

For an ensemble, the registered implementor owns a stable collection of members:

```text
Task -> ExecutionUnit (ensemble; one coordinated lifecycle)
          +-- MemberView[0] -> SystemState A -> recording A
          `-- MemberView[1] -> SystemState B -> recording B
```

Its `member_count()` returns the collection length, `member(index)` reports each
member's independent completion and target, and `step()` owns all shared inputs,
synchronization, and internal parallelism. Workflow does not inspect or control
that parallelism. Member count/order/identity and state addresses remain stable;
completed members stay in the collection and are skipped internally rather than
removed or reordered.

Standalone executable and Python tasks require no Rust trait or wrapper.
Declare them in `wf_configs/study.json`; they receive the same captured central project
configuration and dependency results at runtime. A Python task declares its
environment locally inside its nested `python` object—there is no global
environment registry. Either external-task form may declare one optional
`seed: {"purpose":"..."}` request. Workflow derives a task-scoped value from
the study master seed, passes only that value as `WORKFLOW_TASK_SEED`, and
records the request in `program.json`.

### Standard state providers

Use a provider when one upstream crate—not each application project—owns the
canonical state layout. The upstream embeds its JSON and exports a typed
descriptor; it does not know the receiver, dispatcher, or project root:

```rust,ignore
use scientific_workflow::state::StateSchemaProvider;

pub const fn ecological_state_schema() -> StateSchemaProvider {
    StateSchemaProvider::new(
        "ecological-state-toolkit.ecological-state.v1",
        include_bytes!("../schemas/ecological_state.json"),
    )
}
```

The downstream execution unit is the receiver and delegates its optional trait
hook to that upstream API:

```rust,ignore
impl ExecutionUnit for GlvUnit {
    const THREAD_COUNT_INVARIANT: bool = true;

    type Constants = GlvConstants;

    fn standard_state_schema() -> Option<StateSchemaProvider> {
        Some(ecological_state_toolkit::state_schema::ecological_state_schema())
    }

    // preflight, initialize, member_count, member, and step follow.
}
```

Its project task can then be `{"execution_unit":"glv"}` with no `state` and no
`paths.states`. Study validates and caches the embedded document, passes the
resulting `SystemStateSchema` to `preflight` and `initialize`, and records
`ecological-state-toolkit.ecological-state.v1` as state provenance. If a
project does declare `{"execution_unit":"glv","state":"experiment"}`, that
explicit schema wins. Omission without either an explicit selection or a
provider fails before output.

For contracts covering lifetimes, failure behavior, and every supported method,
see [Task API](../../rust/src/task/api.md) and [Rust API overview](../reference/rust-api.md).

---

**Previous:** [4. Project configuration](4-project-configuration.md) · **Guide index:** [Documentation](../README.md)

**Next:** [6. State and recording](6-state-and-recording.md)
