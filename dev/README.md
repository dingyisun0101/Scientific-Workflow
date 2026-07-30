# scientific-workflow

`scientific-workflow` provides Rust primitives for representing scientific
system states and building reproducible simulation workflows.

The crate currently focuses on `SystemState`: a fixed-layout, heterogeneous
state container whose schema is loaded from JSON. Concrete payloads move into
and out of a state without cloning, making the container suitable for large
arrays and tensors used in scientific calculations.

## Features

- JSON-defined state fields with deterministic order and stable type tags.
- Dictionary-like typed access to heterogeneous Rust payloads.
- Clone-free payload insertion, in-place mutation, and owned extraction.
- Explicit deep cloning of complete states.
- Shared immutable state specifications.
- Integer and optional finite physical time coordinates.
- Strict template validation and semantic JSON round trips.
- Compatibility with owned scientific payloads such as
  `physics_in_parallel` tensors.

Time-series storage, automatic chunking, and workflow dispatch are under active
development and are not part of the published API described below.

## Installation

Add the crate to a Rust project:

```toml
[dependencies]
scientific-workflow = "0.1"
```

The crate uses Rust edition 2024 and requires Rust 1.85 or newer.

## State Template

A program begins with a JSON template that declares every state field:

```json
{
  "fields": [
    {
      "name": "population",
      "type": "physics_in_parallel.tensor.dense.u64.v1"
    },
    {
      "name": "space",
      "type": "physics_in_parallel.tensor.dense.u64.v1"
    }
  ]
}
```

Field order defines the compact runtime slot order. The `type` value is a
stable serialization tag rather than a Rust type name.

## Basic Usage

```rust,no_run
use scientific_workflow::system_state::{StateSpec, TimePoint};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let spec = StateSpec::load("state.json")?;
    let mut state = spec.empty(TimePoint::new(0));

    state.set("population", vec![10_u64, 20, 30])?;

    state
        .get_mut::<Vec<u64>>("population")?
        .push(40);

    let population = state.take::<Vec<u64>>("population")?;
    assert_eq!(population, vec![10, 20, 30, 40]);
    assert!(state.is_blank());

    Ok(())
}
```

`set` consumes the supplied payload, and `take` returns that same owned
payload. Neither operation calls `Clone`. Calling `SystemState::clone`
explicitly deep-clones populated payloads so the two states can be mutated
independently.

## Tensor Payloads

Any concrete type satisfying `Any + Clone + Send` can be stored. For example,
an application can use a dense `physics_in_parallel` tensor:

```rust,ignore
use physics_in_parallel::math::{Dense, Tensor};
use scientific_workflow::system_state::{StateSpec, TimePoint};

let spec = StateSpec::load("state.json")?;
let mut state = spec.empty(TimePoint::new(0));

let mut population = Tensor::<u64, Dense>::zeros(&[3]);
population.set(&[0], 10);
population.set(&[1], 20);
population.set(&[2], 30);

state.set("population", population)?;
let population = state.take::<Tensor<u64, Dense>>("population")?;
```

The tensor crate is not a required runtime dependency of
`scientific-workflow`; applications select and register their own concrete
scientific payload types.

## Testing

From the package directory:

```bash
cargo test
```

The public integration suite loads an actual JSON template and exercises the
complete state lifecycle using `physics_in_parallel` tensor payloads.

## License

Licensed under the MIT License.
