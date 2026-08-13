# scientific-workflow

This README is the public API documentation for the crates.io publication of
`scientific-workflow`.
For repository context, private crate boundaries, and runnable example guidance,
see [`../README.md`](../README.md).

`scientific-workflow` provides Rust primitives for representing scientific
system states and building reproducible simulation workflows.

The crate provides `SystemState`, a fixed-layout heterogeneous state container,
and `StateSeries`, an ordered growable collection of complete states for
in-memory analysis. Concrete payloads move through both layers without cloning,
making them suitable for large arrays and tensors.

## Features

- Standard `config/{fixed,sweep,paths}.json` task definition with either a
  project-owned or model-owned state schema.
- Deterministic Cartesian and correlated explicit-case task expansion.
- Lazy complete `TaskConfig` handles combining parameters and shared paths.
- Exact sweep-value filtering and ambiguity-safe unique task selection.
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
- Writer-owned typed sampling intervals with no payload access for skipped states.
- Human-friendly numeric sampling-interval decoding with tagged-format compatibility.
- Automatic exactly-once final-state sampling across sampling-interval boundaries.
- Exact finite-`f64` JSON reconstruction through Serde JSON's round-trip parser.
- Finite byte- and record-bounded asynchronous writers.
- Exact-byte automatic chunking with indivisible JSONL records.
- Durable chunk publication through open-file sync, incremental descriptor
  preparation, atomic lifecycle rename, and stream-directory sync.
- Explicit interrupted-run append and complete typed checkpoint recovery.
- RNG-agnostic method, version, key-encoding, key, and parameter provenance.
- SHA-256-verified eager reconstruction through per-key payload decoders.
- Efficient latest-state reconstruction without loading earlier chunks.
- Automatic UTC lifecycle timestamps and monotonic active durations.
- Collision-resistant generated or caller-named execution scopes.
- Atomic content-addressed input artifacts with verified execution-relative
  loading.
- Structurally separate terminal metadata and immutable completed-recording handles.
- Parameter-identified, parallel-safe centralized progress reporting.
- One exclusive terminal renderer with interactive, CI, and hidden modes.

## API Design Rules

- Add a public type or trait only when an existing Workflow owner cannot
  express the behavior.
- Extend the existing API instead of creating a parallel replacement whenever
  the concepts are the same.
- Optional behavior uses one interface with explicit defaults. This applies to
  stream limits, continuation reasons, and RNG-record parameters.
- Workflow records RNG provenance but never implements scientific randomness.

## Supported public API

This is the exhaustive supported API allowlist for `scientific-workflow`.
Users may rely on these items, their public enum variants, and their documented
public methods. Importing `scientific_workflow::prelude::*` brings the complete
allowlist into scope. Compiler-visible implementation paths not listed here are
not compatibility promises.

- Artifacts: `ArtifactDescriptor`, `ArtifactDisposition`, `ArtifactError`,
  `ArtifactLoadError`, `PersistedArtifact`, `VerifiedArtifact`,
  `persist_artifact`, and `load_verified_artifact`.
- Configuration: `ConfigurationError`, `MatchingTaskConfigIter`,
  `ParameterSpace`, `ProjectConfig`, `ProjectPaths`, `TaskConfig`,
  `TaskConfigIter`, `TaskParameters`, and `TaskParametersIter`.
- Projects and execution: `ScientificProject`, `ScientificProjectError`,
  `ExecutionScope`, and `ExecutionScopeError`.
- Phases and progress: `Phase`, `PhaseBuilder`, `PhaseId`, `Task`, `TaskId`,
  `TaskKey`, `TaskSelector`, `TaskDisplayKind`, `ProgressReporter`,
  `PhaseProgressReporterBuilder`, `ProgressReporterBuilder`,
  `ProgressSummary`, `ReportingError`, `TaskIdentity`, `TaskProgress`,
  `ActivityTask`, and `TaskStatus`.
- RNG provenance: `RNG_RECORDS_METADATA_KEY`, `RngRecord`, and
  `RngRecordError`.
- Persistent storage: `CompletedRecording`, `CompletedStreamSummary`,
  `JsonPayloadDecoder`, `JsonPayloadDecoderRegistry`, `JsonStringDecoder`,
  `JsonVecF64Decoder`, `RecordingTiming`, `SamplingInterval`,
  `StateStreamConfig`, `StorageError`, `StoredStateSeriesReader`,
  `SystemStateWriter`, `SystemStateWriterBuilder`, and `TimeAxisMetadata`.
- State: `PayloadInsertError`, `SimulationTime`, `StateError`,
  `StateFieldSchema`, `SystemState`, and `SystemStateSchema`.
- In-memory series: `StateSeries`, `StateSeriesError`,
  `StateSeriesPushError`, and `StateSeriesView`.

The sections below document the supported constructors and operations by
workflow responsibility. Generated crate documentation is the exact signature
reference for every item in this list.

## Parallel Progress Reporting

First-class tasks belong to a phase before reporting begins. Parameterized
tasks are generated directly from the existing configuration manager, retain
all fixed and selected sweep values without cloning their shared JSON owners,
and receive automatic labels from the parameters that vary:

```rust,no_run
use scientific_workflow::prelude::*;

# fn example(project: &ScientificProject) -> Result<(), Box<dyn std::error::Error>> {
let phase = Phase::builder(2, "simulation")
    .progress_tasks_from_project(project, "simulation")
    .display_tasks_by("simulation", ["temperature", "seed"])
    .max_concurrent_workloads(4)
    .queue_capacity(8)
    .build()?;

let selected = phase.unique_task_matching(
    &TaskSelector::new()
        .kind("simulation")
        .parameter("temperature", serde_json::json!(300.0))
        .parameter("seed", serde_json::json!(11)),
)?;
let selected_key = selected.key().clone();

let reporter = ProgressReporter::for_phases([phase]).hidden().start()?;
let progress = reporter.start_progress(&selected_key, 0, Some(1_000))?;
progress.set_iteration(1_000)?;
progress.complete(None)?;
reporter.fail("other declared tasks intentionally not run")?;
# Ok(())
# }
```

The reporter observes phase/task identities supplied by the work-management
layer; it does not construct them. A phase heading becomes the display
separator for its task rows. Lifecycle-only `ActivityTask` handles provide
status, detail, messages, cancellation, completion, and failure without an
iteration API.

`ProgressReporter` derives human-facing identity from task parameters and uses
the automatically assigned task ordinal only for stable ordering. With no
explicit identity selection, all sweep keys form the identity. Applications may
choose any smaller parameter combination that remains unique:

```rust,no_run
use scientific_workflow::prelude::*;

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let project = ScientificProject::load("project-root")?;
let reporter = ProgressReporter::for_project(&project)
    .identify_tasks_by(["temperature", "seed"])
    .start()?;

for task in project.task_configs() {
    let progress = reporter.start_task(&task, 0, Some(1_000))?;
    // Workflow owns the generic target decision.
    assert!(!progress.should_continue(1_000)?);
    progress.complete(None)?;
}

let summary = reporter.complete("all scientific tasks completed")?;
assert!(summary.is_success());
# Ok(())
# }
```

Iteration updates are atomic and allocation-free. One renderer thread polls all
tasks at a bounded frequency and is the only component permitted to write
human-facing terminal output during the session. Interactive stderr is cleared
once at renderer startup and then receives one row for every configured task,
including tasks still waiting for a worker. Known-target rows show elapsed task
execution time and ETA; redirected stderr receives line-oriented lifecycle
events without being cleared. Dropping an unfinished `TaskProgress` marks that
task failed.

Progress is not scientific state. Callers set it from the authoritative
`SystemState::simulation_time()` after a successful transition. Known targets
use absolute iterations, while `None` supports convergence-driven or otherwise
open-ended work.

## RNG Records

`RngRecord` persists application-owned RNG identity without providing
any RNG behavior. Workflow does not generate keys, derive streams, choose
algorithms or distributions, sample values, or maintain cursors.

```rust
use scientific_workflow::prelude::*;
use serde_json::{Map, json};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let record = RngRecord::new(
    "simulation.noise",
    "chacha12+standard_normal",
    "rand_chacha-0.10+rand_distr-0.6",
    "u64_be_hex",
    "000000000000002a",
    Some(Map::from_iter([("lanes".to_owned(), json!(2))])),
)?;
let mut user_metadata = Map::new();
record.insert_into_metadata(&mut user_metadata)?;

assert_eq!(
    RngRecord::from_metadata(&user_metadata, "simulation.noise")?,
    Some(record),
);
# Ok(())
# }
```

Records are indexed by namespace beneath the reserved `rng_records`
user-metadata key. Duplicate namespaces and malformed records are rejected.
Because recording continuation already requires exact user-metadata equality,
a changed RNG method, version, or key prevents continuation. Keys are persisted
as plain text reproducibility material and must not contain secrets.

When a scientific crate resolves optional RNG settings, record the resolved
values—not the original request. For example, PiP exposes one `RngConfig`
input across its stochastic APIs and returns or retains its resolved form. The
mapping into Workflow remains explicit and lightweight:

```rust,ignore
use physics_in_parallel::prelude::*;
use scientific_workflow::prelude::*;
use serde_json::{Map, json};

let resolved = generator.rng_config();
let method = resolved.method().expect("a PiP component resolves its method");
let parameters = resolved.parallel_streams().map(|streams| {
    Map::from_iter([("parallel_streams".to_owned(), json!(streams.get()))])
});

let record = RngRecord::new(
    "simulation.noise",
    method.name(),
    method.version(),
    method.seed_encoding(),
    resolved.encode_seed().expect("a PiP component resolves its seed"),
    parameters,
)?;
```

Workflow deliberately does not depend on PiP, interpret `RngConfig`, or expose
a second RNG configuration API. The application chooses a stable namespace and
copies the upstream generator's resolved identity into `RngRecord`.

## Immutable Input Artifacts

Workflow owns the generic mechanics for persisting immutable bytes inside an
execution scope. Scientific crates keep ownership of their formats and domain
metadata: they serialize their document, call `persist_artifact`, and embed the
returned descriptor in each recording that consumed it.

```rust,no_run
use scientific_workflow::prelude::*;

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let scope = ExecutionScope::create_named("recordings", "example")?;
let persisted = persist_artifact(&scope, "initial-space", "json", br#"{"sites":[0,1]}"#)?;
let verified = load_verified_artifact(scope.directory(), persisted.descriptor())?;

assert_eq!(verified.bytes(), br#"{"sites":[0,1]}"#);
assert_eq!(persisted.descriptor().sha256().len(), 64);
# Ok(())
# }
```

The filename is derived from SHA-256, publication is atomic, and identical
bytes in one scope are reused. Loading rejects malformed or escaping relative
paths and verifies the digest before returning bytes. Workflow deliberately
does not interpret JSON, matrices, lattices, or other scientific encodings.

## Mandatory Chunk Integrity

Every sealed JSONL chunk is described by an exact byte count and SHA-256 digest
in `metadata.json`. Verification is mandatory whenever a chunk is validated or
reconstructed. The public reader has no unchecked mode, checksum opt-out,
feature switch, or performance flag: corruption produces `StorageError` rather
than partially trusted scientific data.

Parsing alone is not validation. Skipping a chunk because an operation does not
need its contents is permitted, but that chunk is then unexamined—not verified.
Any chunk actually used to reconstruct scientific state must cross the checksum
boundary first. This integrity guarantee detects accidental corruption; it is
not a substitute for provenance, signatures, or validation of the scientific
model itself.

The public `SystemStateWriter` facade owns multi-stream metadata, one bounded
queue and worker, and the recording's completion or failure lifecycle.
Workflow orchestration-layer support remains a later
development stage.

## Installation

Add the crate to a Rust project:

```toml
[dependencies]
scientific-workflow = "0.2"
```

The crate uses Rust edition 2024 and requires Rust 1.97 or newer.

## Complete Project Example

The source repository includes `examples/attractor_2d`, a standalone
consumer application that exercises configuration loading, Cartesian task
expansion, directly owned mutable states, tuple payload borrowing, independent
sample streams, bounded asynchronous recording, automatic chunking, and
explicit completion. Its lazy `TaskConfig` iterator feeds Rayon's bounded
work-stealing pool, while stable task indices keep recording paths deterministic
regardless of completion order. It then reads the complete checkpoint's latest state with typed payload
decoders and verifies the final live-to-stored round trip exactly. From the
repository root, run:

```bash
cargo run --manifest-path examples/attractor_2d/Cargo.toml
```

The example is intentionally outside this crate directory and therefore is
not part of the crates.io package. Its generated recordings remain under its
ignored `target/recordings` directory.

## Project Configuration

A project with a project-owned state contract keeps four files together:

```text
project-root/
└── config/
    ├── fixed.json
    ├── sweep.json
    ├── paths.json
    └── state.json
```

`fixed.json` contains values shared by every task:

```json
{
  "physical_time_increment": 0.125,
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
    {"temperature": 280.0, "physical_time_increment": 0.1},
    {"temperature": 300.0, "physical_time_increment": 0.05}
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
    let project = ScientificProject::load("project-root")?;
    for task in project.task_configs() {
        let physical_time_increment =
            task.decode_value::<f64>("physical_time_increment")?;
        let temperature = task.decode_value::<f64>("temperature")?;
        let seed = task.decode_value::<u64>("seed")?;
        let output_root = task.resolve_path("output_root")?;
        println!(
            "task={} dt={physical_time_increment} temperature={temperature} seed={seed} output={}",
            task.task_ordinal(),
            output_root.display()
        );
    }
    Ok(())
}
```

`task_configs()` lazily emits the complete Cartesian product—or exactly the
declared correlated cases—in stable task-ordinal order. Each item is a cheap
owned handle over shared fixed, sweep, and path storage, so it can move into a
worker queue without cloning merged JSON dictionaries:

```rust,no_run
# use scientific_workflow::prelude::*;
# fn submit(_: TaskConfig) -> Result<(), Box<dyn std::error::Error>> { Ok(()) }
# fn main() -> Result<(), Box<dyn std::error::Error>> {
let project = ScientificProject::load("project-root")?;

for task in project.task_configs_matching("temperature", 300.0)? {
    submit(task)?;
}
# Ok(())
# }
```

Matching constrains only the named sweep dimension; every combination of the
remaining axes is retained. Unique selection returns an error when no task or
more than one task matches, rather than silently choosing the first. Fixed keys
and path keys cannot be used as sweep selectors. Use
`unique_task_config_matching(key, value)` only when that one sweep dimension is
known to identify exactly one task, as is common for an explicit case ID.

Task handles share the parsed source allocations and do not clone values or
construct merged maps. `value` and `require_value` borrow raw JSON;
`decode_value` explicitly constructs one requested Rust value. The final sweep
axis changes fastest. Fixed and swept names must be disjoint.

`ProjectConfig::write_source_config(destination)` reproduces the three
parameter/path files byte for byte beneath a new destination project. It never overwrites an
existing `config/` directory. `TaskParameters::to_json` instead serializes one
deterministic derived fixed-plus-sweep dictionary.

`ScientificProject::load` requires `config/state.json` and exposes its shared
schema through `state_schema()`. This is appropriate when the project itself
defines its state contract.

A fixed-model crate should instead load its one canonical schema and pass it to
`ScientificProject::load_with_state_schema`:

```rust,no_run
use scientific_workflow::prelude::*;

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let schema = SystemStateSchema::load_json_template("model/schemas/state.json")?;
let project = ScientificProject::load_with_state_schema("project-root", schema)?;
assert!(!project.state_schema().is_empty());
# Ok(())
}
```

Such projects contain only `fixed.json`, `sweep.json`, and `paths.json`; the
model dependency supplies the schema. Workflow does not choose a fallback
schema or mutate the configuration directory. The lower-level `ProjectConfig`
remains available when an application intentionally needs only parameter and
path configuration.

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
    let mut state = spec.create_empty_state(SimulationTime::from_iteration(0));

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
let mut state = spec.create_empty_state(SimulationTime::from_iteration(0));
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
    let mut state = spec.create_empty_state(SimulationTime::from_iteration(0));
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
let mut state = spec.create_empty_state(SimulationTime::from_iteration(0));

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
        .with_time_axis_metadata(
            TimeAxisMetadata::new("iteration")
                .with_iteration_unit("iteration")
                .with_physical_axis("physical_time", "s"),
        )
        .with_shared_stream_limits(
            NonZeroU64::new(64 * 1024 * 1024).unwrap(),
            NonZeroU64::new(256 * 1024 * 1024).unwrap(),
        )
        .add_state_stream(StateStreamConfig::new(
            "signal",
            ["population"],
            SamplingInterval::iterations(1).unwrap(),
            None,
        ))
        .create_new_recording()?;

    let mut state = spec.create_empty_state(
        SimulationTime::from_iteration_and_physical_time(0, 0.0).unwrap(),
    );
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

`observe_state` checks each stream's typed sampling interval before accessing any
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
checkpoint through registered decoders. When the selected checkpoint is in a
sealed chunk, Workflow verifies that chunk's declared byte count and SHA-256
checksum before decoding the final record or returning an append-capable
writer. A recovery-selected open tail is decoded only after the recovery scan
has validated its complete JSONL prefix.

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

The permanent suite contains seven logged integration workflows. Run each with
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
