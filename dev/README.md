# scientific-workflow

`scientific-workflow` provides Rust primitives for representing scientific
system states and building reproducible simulation workflows.

The crate provides `SystemState`, a fixed-layout heterogeneous state container,
and `StateSeries`, an ordered growable collection of complete states for
in-memory analysis. Concrete payloads move through both layers without cloning,
making them suitable for large arrays and tensors.

## Features

- Standard `config/{fixed,sweep,paths}.json` project configuration.
- Deterministic Cartesian and correlated explicit-case task expansion.
- Clone-free dict-like resolved task views over shared JSON values.
- Named project-root-relative path resolution and byte-exact source export.
- JSON-defined state fields with deterministic order and optional descriptions.
- Dictionary-like typed access to heterogeneous Rust payloads.
- Coordinated immutable and mutable tuple borrowing for coupled kernels.
- Assembly-established field types retained across extraction and blank-state
  derivation.
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
- Writer-owned per-stream step cadence with no payload access for skipped states.
- Automatic exactly-once final-state sampling across cadence boundaries.
- Exact finite-`f64` JSON reconstruction through Serde JSON's round-trip parser.
- Finite byte- and record-bounded asynchronous writers.
- Exact-byte automatic chunking with indivisible JSONL records.
- Durable chunk publication through open-file sync, incremental descriptor
  preparation, atomic lifecycle rename, and stream-directory sync.
- Explicit interrupted-run append and complete typed checkpoint recovery.
- SHA-256-verified eager reconstruction through per-key payload decoders.

The public `SystemStateWriter` facade owns multi-stream metadata, one bounded
queue and worker, and the recording's completion or failure lifecycle.
Workflow dispatch remains a later
development stage.

## Installation

Add the crate to a Rust project:

```toml
[dependencies]
scientific-workflow = "0.1"
```

The crate uses Rust edition 2024 and requires Rust 1.85 or newer.

## Complete Project Example

The source repository includes `examples/attractor_2d`, a standalone
downstream application that exercises configuration loading, Cartesian task
expansion, directly owned mutable states, tuple payload borrowing, independent
sample streams, bounded asynchronous recording, automatic chunking, and
explicit completion. It then reconstructs all streams with typed payload
decoders, calculates numerical summaries, renders a terminal phase portrait,
and verifies the final live-to-stored round trip exactly. From the repository
root, run:

```bash
cargo run --manifest-path examples/attractor_2d/Cargo.toml
```

The example is intentionally outside this crate directory and therefore is
not part of the crates.io package. Its generated recordings remain under its
ignored `target/recordings` directory.

## Project Configuration

A standard project keeps three files together:

```text
project-root/
└── config/
    ├── fixed.json
    ├── sweep.json
    └── paths.json
```

`fixed.json` contains values shared by every task:

```json
{
  "time_step": 0.125,
  "lattice_shape": [4, 8]
}
```

`sweep.json` supports ordered Cartesian axes:

```json
{
  "mode": "cartesian",
  "axes": [
    {"name": "temperature", "values": [280.0, 300.0]},
    {"name": "seed", "values": [7, 11, 13]}
  ]
}
```

or correlated explicit cases:

```json
{
  "mode": "cases",
  "cases": [
    {"temperature": 280.0, "time_step": 0.1},
    {"temperature": 300.0, "time_step": 0.05}
  ]
}
```

`paths.json` contains shared path strings resolved relative to the project root:

```json
{
  "input_data": "data/input.json",
  "output_root": "results"
}
```

Load the project and consume exact JSON names through each resolved task's
read-only dictionary:

```rust,no_run
use scientific_workflow::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let project = ProjectConfig::load("project-root")?;
    let output_root = project.paths().resolve_path("output_root")?;

    for task in project.parameters().tasks() {
        let time_step = task.decode_value::<f64>("time_step")?;
        let temperature = task.decode_value::<f64>("temperature")?;
        let seed = task.decode_value::<u64>("seed")?;
        println!(
            "task={} dt={time_step} temperature={temperature} seed={seed} output={}",
            task.task_index(),
            output_root.display()
        );
    }
    Ok(())
}
```

Task handles share the parsed source allocation and do not clone values or
construct merged maps. `value` and `require_value` borrow raw JSON;
`decode_value` explicitly constructs one requested Rust value. The final sweep
axis changes fastest. Fixed and swept names must be disjoint.

`ProjectConfig::write_source_config(destination)` reproduces all three original
files byte for byte beneath a new destination project. It never overwrites an
existing `config/` directory. `TaskParameters::to_json` instead serializes one
deterministic derived fixed-plus-sweep dictionary.

The state template is a separate `SystemStateSchema` input, not a fourth file
owned or exported by `ProjectConfig`. A project may keep it beside the other
inputs as `config/state.json` and name that location in `paths.json`; the
standalone attractor example uses this arrangement.

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
Rust type or storage codec information. The first payload inserted into a field
establishes its concrete runtime type; that contract remains after
`take_payload` or `clear_payload` and is copied into blank states derived with
`SystemState::clone_structure_without_payloads`.
Descriptions remain documentation only.

## Basic Usage

```rust,no_run
use scientific_workflow::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let spec = SystemStateSchema::load_json_template("state.json")?;
    std::fs::create_dir_all("output")?;
    let mut state = spec.create_empty_state(SimulationTime::from_step(0));

    drop(state.insert_payload("population", vec![10_u64, 20, 30])?);

    state
        .payload_mut::<Vec<u64>>("population")?
        .push(40);

    let population = state.take_payload::<Vec<u64>>("population")?;
    assert_eq!(population, vec![10, 20, 30, 40]);
    assert!(state.has_no_payloads());

    Ok(())
}
```

`insert_payload` consumes the supplied payload, and `take_payload` returns that same owned
payload. Neither operation calls `Clone`. Calling `SystemState::clone`
creates a new erased box and calls `Clone` for every populated payload; the
semantic depth is defined by each concrete type's `Clone` implementation.

## Coupled Payload Access

Scientific kernels can borrow several distinct fields without payload copies,
temporary extraction, locks, or application-side selector structures. Supply
the expected concrete types and field names in matching tuple order:

```rust,no_run
use scientific_workflow::prelude::*;

# fn evolve(position: &mut Vec<f64>, velocity: &mut Vec<f64>) {
#     position[0] += velocity[0];
# }
# fn main() -> Result<(), Box<dyn std::error::Error>> {
let spec = SystemStateSchema::load_json_template("state.json")?;
let mut state = spec.create_empty_state(SimulationTime::from_step(0));
drop(state.insert_payload("position", vec![0.0_f64])?);
drop(state.insert_payload("velocity", vec![1.0_f64])?);

let (position, velocity) = state
    .borrow_payloads_mut::<(Vec<f64>, Vec<f64>)>(("position", "velocity"))?;
evolve(position, velocity);
# Ok(())
# }
```

Supported tuple arities are two through eight. The complete request is
validated before any reference is returned, and repeating a field is rejected.
Use `payload` or `payload_mut` for one field. Name lookup and type validation occur once
per tuple borrow, so the returned references should normally surround the full
kernel or simulation sweep.

## In-Memory Time Series

`StateSeries` owns complete states for analysis. Appending validates that every
state shares the series' exact specification allocation and that simulation
indices increase strictly. Index gaps are allowed.

```rust,no_run
use scientific_workflow::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let spec = SystemStateSchema::load_json_template("state.json")?;
    let mut state = spec.create_empty_state(SimulationTime::from_step(0));
    drop(state.insert_payload("population", vec![10_u64, 20, 30])?);

    let mut series = StateSeries::new(spec);
    series.push_state(state)?;
    series
        .payload_mut_at::<Vec<u64>>(0, "population")?
        .push(40);

    let view = series.as_view();
    assert_eq!(view.len(), 1);
    Ok(())
}
```

The collection never returns `&mut SystemState`, because changing a stored
state's time would invalidate ordering. `payload_mut_at` permits one typed payload
mutation at a time. `push_state`, `pop_state`, and `into_states` move ownership without
cloning. Explicit `StateSeries::clone` deep-clones all populated payloads; use
`as_view` or `Arc<StateSeries>` for lightweight sharing.

`StateSeries` performs no serialization, chunking, queueing, or disk IO. Those
responsibilities belong to the separate storage layer.

## Tensor Payloads

Any concrete type satisfying `Serialize + Clone + Send + 'static` can be
stored. For example, an application can use a dense `physics_in_parallel`
tensor:

```rust,ignore
use physics_in_parallel::math::{Dense, Tensor};
use scientific_workflow::prelude::*;

let spec = SystemStateSchema::load_json_template("state.json")?;
let mut state = spec.create_empty_state(SimulationTime::from_step(0));

let mut population = Tensor::<u64, Dense>::zeros(&[3]);
population.set(&[0], 10);
population.set(&[1], 20);
population.set(&[2], 30);

drop(state.insert_payload("population", population)?);
let population = state.take_payload::<Tensor<u64, Dense>>("population")?;
```

The tensor crate is not a required runtime dependency of
`scientific-workflow`; applications use their own concrete serializable
scientific payload types without registering codecs.

The PiP 3.0.4 integration uses versioned Serde schemas for dense and
sparse tensors, matrices, vector lists, square lattices, and heterogeneous
`PhysObj` values. They reconstruct through the same generic registry path:

```rust,ignore
let decoders = JsonPayloadDecoderRegistry::new()
    .with_json_field::<Tensor<f64, Dense>>("population")?
    .with_json_field::<PhysObj>("particles")?;
```

Sparse PiP records contain only sorted nonzero indices and values; Scientific
Workflow does not densify them during encoding or reconstruction.

## Persistent State Recording

One import brings the complete supported state, analysis, storage, reader, and
decoder API into scope:

```rust,no_run
use std::num::NonZeroU64;
use scientific_workflow::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let spec = SystemStateSchema::load_json_template("state.json")?;
    let mut writer = SystemStateWriter::builder("output/recording-001", &spec)
        .with_time_axis_metadata(TimeAxisMetadata::new("step").with_physical_axis("time", "s"))
        .with_shared_stream_limits(
            NonZeroU64::new(64 * 1024 * 1024).unwrap(),
            NonZeroU64::new(256 * 1024 * 1024).unwrap(),
        )
        .add_periodic_state_stream(
            "signal",
            ["population"],
            NonZeroU64::new(1).unwrap(),
        )
        .create_new_recording()?;

    let mut state = spec.create_empty_state(SimulationTime::from_step_and_physical_time(0, 0.0).unwrap());
    drop(state.insert_payload("population", vec![10.0_f64, 20.0, 30.0])?);
    writer.observe_state(&state)?;
    writer.complete_recording_with_final_state(&state)?;

    let decoders = JsonPayloadDecoderRegistry::new()
        .with_json_field::<Vec<f64>>("population")?;
    let series = StoredStateSeriesReader::open_completed_recording("output/recording-001", decoders)?
        .read_stream_as_state_series("signal")?;
    assert_eq!(series.len(), 1);
    Ok(())
}
```

`observe_state` checks each stream's typed step cadence before accessing any
payload. Non-due streams perform no serialization or queue work. Due streams
resolve each selected key once and borrow payloads only while producing owned
encoded bytes, after which bounded blocking backpressure applies through the
recording's single queue and worker. `complete_recording_with_final_state`
records a non-aligned endpoint exactly once per stream before completion. Each
chunk is synchronized, described in the sole
metadata file, atomically renamed from `.jsonl.tmp` to `.jsonl`, and followed by
a stream-directory sync. `flush_stream_to_storage(stream)` exposes this as an
ordered durability barrier. `continue_existing_recording` recovers append
position without reconstructing state, while
`continue_recording_from_latest_checkpoint` also returns a complete typed
checkpoint through registered decoders.

### Custom Payload Decoders

Each decoder is registered for one exact state key and returns that key's
concrete payload type. A closure is sufficient for stateless conversion; a
named decoder can carry configuration or shared resources:

```rust
use scientific_workflow::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize, Serialize)]
struct ParticleBlock {
    positions: Vec<[f64; 3]>,
}

struct ParticleBlockDecoder;

impl JsonPayloadDecoder<ParticleBlock> for ParticleBlockDecoder {
    type Error = serde_json::Error;

    fn decode_json_payload(&self, raw_json: &str) -> Result<ParticleBlock, Self::Error> {
        serde_json::from_str(raw_json)
    }
}

fn configure() -> Result<JsonPayloadDecoderRegistry, StorageError> {
    let mut decoders = JsonPayloadDecoderRegistry::new();
    decoders.register_for_field("particles", ParticleBlockDecoder)?;
    decoders.register_for_field::<Vec<u64>, _>("counts", |raw_json: &str| {
        serde_json::from_str(raw_json)
    })?;
    Ok(decoders)
}
```

The reader performs record parsing and key lookup, passes only the matching raw
JSON value to each decoder, and moves the returned payload into the reconstructed
state. Custom decoders do not handle chunks, metadata, sibling fields, or state
assembly.

## Testing

From the package directory:

```bash
cargo test --all-targets --no-fail-fast --locked
```

The permanent suite contains six logged integration workflows. Run each with
`--nocapture` to display its stable semantic report.

Project configuration and task expansion:

```bash
cargo test --test configuration_workflow -- --nocapture
```

Simulation-owned state:

```bash
cargo test --test state_workflow -- --nocapture
```

In-memory analysis series:

```bash
cargo test --test analysis_workflow -- --nocapture
```

Successful storage and typed reconstruction:

```bash
cargo test --test storage_workflow -- --nocapture
```

Storage failure and corruption handling:

```bash
cargo test --test storage_resilience -- --nocapture
```

Interrupted-run recovery, checkpoint reconstruction, and append:

```bash
cargo test --test resume_workflow -- --nocapture
```

Doctests and lint gate:

```bash
cargo test --doc --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
```

The repository-level `tests.md` documents complete method allocation, indirect
private coverage, logging rules, and completion criteria.

## License

Licensed under the MIT License.
