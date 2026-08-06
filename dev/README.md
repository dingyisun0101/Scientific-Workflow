# scientific-workflow

`scientific-workflow` provides Rust primitives for representing scientific
system states and building reproducible simulation workflows.

The crate provides `SystemState`, a fixed-layout heterogeneous state container,
and `StateSeries`, an ordered growable collection of complete states for
in-memory analysis. Concrete payloads move through both layers without cloning,
making them suitable for large arrays and tensors.

## Features

- JSON-defined state fields with deterministic order and optional descriptions.
- Dictionary-like typed access to heterogeneous Rust payloads.
- Clone-free payload insertion, in-place mutation, and owned extraction.
- Explicit deep cloning of complete states.
- Shared immutable state specifications.
- Integer and optional finite physical time coordinates.
- Strict template validation and semantic JSON round trips.
- Compatibility with owned scientific payloads such as
  `physics_in_parallel` tensors.
- Ordered state-series collection with strict shared-layout identity.
- Lightweight copyable series views and field-level analysis mutation.
- Borrowed JSON encoding without payload cloning.
- Finite byte- and record-bounded asynchronous writers.
- Exact-byte automatic chunking with indivisible JSONL records.
- SHA-256-verified eager reconstruction through per-key payload decoders.

The storage implementation is verified internally but remains staged outside
the public crate API until the next run-level facade owns metadata and writer
lifecycle. Workflow dispatch remains a later development stage.

## Installation

Add the crate to a Rust project:

```toml
[dependencies]
scientific-workflow = "0.1"
```

The crate uses Rust edition 2024 and requires Rust 1.85 or newer.

## State Template

A program begins with a JSON template that declares every state key and may
document its payload in natural language:

```json
{
  "fields": [
    {
      "name": "population",
      "description": "Population count at each modeled location"
    },
    {
      "name": "space"
    }
  ]
}
```

Field order defines the compact runtime slot order. The template contains no
Rust type or storage codec information. Payload types are checked at runtime
through typed access, and descriptions are documentation only.

## Basic Usage

```rust,no_run
use scientific_workflow::system_state::{StateSpec, TimePoint};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let spec = StateSpec::load("state.json")?;
    let mut state = spec.empty(TimePoint::new(0));

    drop(state.set("population", vec![10_u64, 20, 30])?);

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
creates a new erased box and calls `Clone` for every populated payload; the
semantic depth is defined by each concrete type's `Clone` implementation.

## In-Memory Time Series

`StateSeries` owns complete states for analysis. Appending validates that every
state shares the series' exact specification allocation and that simulation
indices increase strictly. Index gaps are allowed.

```rust,no_run
use scientific_workflow::system_state::{StateSpec, TimePoint};
use scientific_workflow::time_series::StateSeries;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let spec = StateSpec::load("state.json")?;
    let mut state = spec.empty(TimePoint::new(0));
    drop(state.set("population", vec![10_u64, 20, 30])?);

    let mut series = StateSeries::new(spec);
    series.push(state)?;
    series
        .field_mut::<Vec<u64>>(0, "population")?
        .push(40);

    let view = series.view();
    assert_eq!(view.len(), 1);
    Ok(())
}
```

The collection never returns `&mut SystemState`, because changing a stored
state's time would invalidate ordering. `field_mut` permits one typed payload
mutation at a time. `push`, `pop`, and `into_states` move ownership without
cloning. Explicit `StateSeries::clone` deep-clones all populated payloads; use
`view` or `Arc<StateSeries>` for lightweight sharing.

`StateSeries` performs no serialization, chunking, queueing, or disk IO. Those
responsibilities belong to the separate storage layer.

## Tensor Payloads

Any concrete type satisfying `Serialize + Clone + Send + 'static` can be
stored. For example, an application can use a dense `physics_in_parallel`
tensor:

```rust,ignore
use physics_in_parallel::math::{Dense, Tensor};
use scientific_workflow::system_state::{StateSpec, TimePoint};

let spec = StateSpec::load("state.json")?;
let mut state = spec.empty(TimePoint::new(0));

let mut population = Tensor::<u64, Dense>::zeros(&[3]);
population.set(&[0], 10);
population.set(&[1], 20);
population.set(&[2], 30);

drop(state.set("population", population)?);
let population = state.take::<Tensor<u64, Dense>>("population")?;
```

The tensor crate is not a required runtime dependency of
`scientific-workflow`; applications use their own concrete serializable
scientific payload types without registering codecs.

## Testing

From the package directory:

```bash
cargo test
```

The system-state integration target loads an actual JSON template, runs all
focused module suites, and exercises the complete state lifecycle using
`physics_in_parallel` tensor payloads:

```bash
cargo test --test system_state
```

The time-series target runs its focused error and collection suites plus a
public cross-module ownership workflow:

```bash
cargo test --test time_series
```

The staged storage target runs focused format, encoder, writer, decoder, and
reader contracts. Its logged workflow uses real tensor payloads, two sampling
cadences, JSON encoding, bounded writers, automatic chunking, one metadata
file, integrity verification, default String/Vec<f64> decoder round trips, and
custom per-key tensor reconstruction. Pass `--nocapture` to display bounded
phase, sample, chunk, readback, and result summaries:

```bash
cargo test --test storage -- --nocapture
```

## License

Licensed under the MIT License.
