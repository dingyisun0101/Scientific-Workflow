# Prelude API

`prelude` is a central index of module-owned APIs. It owns no implementation,
validation, state, filesystem effect, or compatibility contract beyond which
canonical module exports it aggregates. Direct narrow imports remain fully
supported and are preferred in reusable libraries.

## Basic API

`scientific_workflow::prelude::basic::*` is the ordinary model-author surface.
It re-exports:

- every `state::basic` symbol: `StateError`, `StateSeriesError`,
  `PayloadInsertError`, `StateSeriesPushError`, `StateTime`,
  `SystemStateSchema`, `SystemState`, and `StateSeries`;
- every `observation::basic` symbol: `ObservationPlan`, `ObservationStream`,
  and `ObservationError`;
- every `task::basic` symbol: `ExecutionUnit`, `ModelView`, and `TaskResult`;
- every `error::basic` symbol: only `WorkflowError`;
- the crate-level `run(&Path)` facade;
- the crate-level `execution_unit` attribute and its `model` compatibility spelling; and
- no symbols from the intentionally empty `runtime::basic` scope.

`config::basic`, `study::basic`, `runtime::basic`, `persistence::basic`, and
`ui::basic` are also aggregated but intentionally empty. Their ordinary
behavior is reached through project declarations and the crate facade.
Re-exporting empty tiers keeps the uniform subsystem rule without inventing
construction APIs.

The Basic prelude deliberately does not export `Task`, `Study`, model catalogs,
project parsers, resolved model parameters, output allocators, persistence writers/readers,
schedulers, summaries, or runtime adapters. Those
are not required to complete an ordinary project.

Importing the prelude has no side effects. It does not trigger registration
discovery, read files, or initialize runtime. The `#[execution_unit]` attribute itself
places an immutable registration in the final linked application; discovery
occurs only when Study loads.

## Advanced API

`scientific_workflow::prelude::advanced::*` is a strict superset of Basic. It
re-exports the complete supported advanced tiers from:

- `config::advanced`: only `ConfigError`; the compiled declaration graph is
  crate-private;
- `error::advanced`: the Basic `WorkflowError` and no additional symbols;
- `state::advanced`: all Basic state types plus `StateFieldSchema`,
  `StateSchemaAccess`, and `StateMaintenance`; the hidden
  generated `PayloadTuple` implementation detail is re-exported only for trait
  resolution and must not be named;
- `observation::advanced`: the Basic observation declarations; binding and
  encoding machinery is crate-private;
- `persistence::advanced`: `PersistenceError`, verified reader/timing types,
  and JSON payload decoder contracts;
- `task::advanced`: the Basic model APIs; its documentation-hidden
  `ModelRegistration` is visible only because downstream attribute-macro
  expansion must name registration metadata, while catalogs, tasks, and host
  machinery remain crate-private;
- `study::advanced`: only `Study` and `StudyError`; its phase/task graph is
  crate-private; and
- `runtime::advanced`: `execute`, `RuntimeError`, `RunSummary`,
  `ReplicateRunSummary`, `PhaseRunSummary`, `TaskRunKind`, `ModelRunSummary`, and
  `TaskRunSummary`; and
- `ui::advanced`: no public additions; its plan, events, session, and renderer
  remain crate-private.

Each symbol retains its owning module's semantics and canonical documentation.
The prelude does not resolve name collisions with downstream imports and does
not promise an independently versioned flat API.

Persistence is aggregated through its formal tiers, but its Basic tier is
empty and its Advanced tier exposes no plan, writer, or lifecycle constructor.

## Example

An ordinary project normally needs one glob import in its model file:

```rust,no_run
use serde::Deserialize;
use scientific_workflow::prelude::basic::*;

#[derive(Deserialize)]
struct Constants { initial: u64, steps: u64 }

struct Model { state: SystemState, target_iteration: u64 }

#[scientific_workflow::execution_unit("example")]
impl ExecutionUnit for Model {
    type Constants = Constants;
    fn initialize(constants: Constants, schema: &SystemStateSchema) -> TaskResult<Self> {
        let mut state = schema.create_empty_state(StateTime::from_iteration(0));
        state.initialize_payload("population", constants.initial)?;
        state.initialize_payload("cumulative_births", 0_u64)?;
        Ok(Self {
            state,
            target_iteration: constants.steps,
        })
    }
    fn model_count(&self) -> usize { 1 }
    fn model(&self, index: usize) -> Option<ModelView<'_>> {
        (index == 0).then(|| ModelView::new(
            "example",
            &self.state,
            self.state.time().iteration() >= self.target_iteration,
            Some(self.target_iteration),
        ))
    }
    fn step(&mut self) -> TaskResult {
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

The executable needs only the crate-level entry:

```rust,no_run
fn main() -> Result<(), scientific_workflow::WorkflowError> {
    scientific_workflow::run(std::path::Path::new("."))
}
```

Advanced inspection can use narrow imports to make ownership obvious:

```rust,no_run
use scientific_workflow::runtime::advanced::execute;
use scientific_workflow::study::advanced::Study;

# fn inspect() -> Result<(), Box<dyn std::error::Error>> {
let study = Study::load(std::path::Path::new("."))?;
let summary = execute(study)?;
println!("{}", summary.output_directory().display());
# Ok(())
# }
```

## Not API

Prelude ordering, individual `pub use` statements, hidden `PayloadTuple`,
hidden `ModelRegistration`, and the crate's macro-support `__private` namespace
are not APIs. `PayloadTuple` is reachable solely because it seals the public
tuple-borrow bounds. `ModelRegistration` is pulled through the Task Advanced
glob solely for attribute-macro expansion; applications register execution units with
`#[execution_unit]` (or compatibility `#[model]`) and must not construct it. Consumers must not infer subsystem
ownership from a prelude path; ownership is always the symbol's canonical
`module::basic` or `module::advanced` path.
