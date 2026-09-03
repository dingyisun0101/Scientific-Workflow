# Prelude API

This guide documents the `scientific-workflow` 0.13.3 subsystem contract.

`scientific_workflow::prelude::*` is the single ordinary execution-unit
authoring import. It owns no behavior and importing it performs no IO,
registration discovery, or runtime initialization.

## Basic API

It re-exports exactly:

- crate entry and registration: `run`, `execution_unit`, and `WorkflowError`;
- unit contracts: `ExecutionUnit`, `InitializationContext`,
  `MemberCompletion`, `MemberView`, `SeedError`, and `UnitResult`;
- state contracts: `StateError`, `StateSeriesError`, `PayloadInsertError`,
  `StateSeriesPushError`, `StateTime`, `SystemStateSchema`, `SystemState`, and
  `StateSeries`; and
- observation declarations: `ObservationPlan`, `ObservationStream`, and
  `ObservationError`.

## Advanced API

The prelude deliberately excludes project loading, active execution, summaries,
completed-recording readers, decoder registries, configuration errors, and
state metadata inspection. Those less common APIs remain at their owning
module roots:

```rust,no_run
use scientific_workflow::persistence::{
    JsonPayloadDecoderRegistry, StoredStateSeriesReader,
};
use scientific_workflow::runtime::execute;
use scientific_workflow::state::StateFieldSchema;
use scientific_workflow::study::Study;
```

## Example

An ordinary execution unit normally needs only:

```rust,no_run
use serde::Deserialize;
use scientific_workflow::prelude::*;

#[derive(Deserialize)]
struct Constants { initial: u64, steps: u64 }

struct ExampleUnit { state: SystemState, target: u64 }

#[scientific_workflow::execution_unit("example")]
impl ExecutionUnit for ExampleUnit {
    type Constants = Constants;

    fn initialize(
        constants: Constants,
        schema: &SystemStateSchema,
        _context: &InitializationContext,
    ) -> UnitResult<Self> {
        let mut state = schema.create_empty_state(StateTime::from_iteration(0));
        state.initialize_payload("population", constants.initial)?;
        Ok(Self { state, target: constants.steps })
    }

    fn member_count(&self) -> usize { 1 }

    fn member(&self, index: usize) -> Option<MemberView<'_>> {
        (index == 0).then(|| MemberView::new(
            "example",
            &self.state,
            (self.state.time().iteration() >= self.target)
                .then_some(MemberCompletion::without_reason()),
            Some(self.target),
        ))
    }

    fn step(&mut self) -> UnitResult {
        *self.state.payload_mut::<u64>("population")? += 1;
        self.state.advance_time(None)?;
        Ok(())
    }
}
```

The executable can call `scientific_workflow::run(Path::new("."))` directly.
Libraries should prefer narrow module imports when that makes ownership clearer.

## Not API

Individual `pub use` statements, the hidden sealed tuple implementation, the
macro-support registration type, and `scientific_workflow::__private` are
implementation details. Public ownership is defined by the crate root or the
symbol's module root, never by its prelude path.
