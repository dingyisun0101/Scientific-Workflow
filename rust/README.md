# Scientific Workflow Rust crate

Scientific Workflow is an inference-first library for typed scientific state,
configuration-driven model or program execution, and durable outputs.

> **Breaking 0.10 update:** Version 0.10.2 is the current patch release of the
> 0.10 API generation that intentionally removed the pre-0.10 orchestration,
> configuration, storage/writer, and study surfaces. Applications using 0.9.x
> or earlier must adopt registered `ScientificModel`
> implementations, `wf_configs/study.json`, `wf_configs/parameters.json`, named
> state schemas beneath `wf_configs/`, and the crate-level
> `run(&Path)` facade. Removed Rust APIs and legacy JSON fields have no
> compatibility aliases; neither root-level `study.json` nor the former
> `config/` directory is recognized as a project layout alias.

## Workflow at a glance

### Architecture and ownership

```text
Runtime (owns active execution, scheduling, cancellation, and coordination)
+-- consumes: immutable Study
|   +-- retains: Config
|   |   +-- owns: immutable parsed JSON snapshot
|   |   `-- sole reader/parser of: wf_configs/**/*.json
|   |
|   `-- owns: replicate policy + dependency-ordered phase graph
|       `-- phases own assembled tasks
|           +-- model task
|           |   +-- retains: resolved parameters
|           |   +-- retains: selected validated schema
|           |   +-- retains: bound observation plan
|           |   `-- creates: ScientificModel
|           |       `-- directly owns: SystemState
|           |
|           `-- program/Python task
|               `-- retains: resolved executable, arguments, and environment
|
+-- creates and coordinates: Persistence sessions
|   +-- model recording
|   |   +-- borrows SystemState at automatic observation boundaries
|   |   `-- owns bounded stream writers
|   |       `-- commit metadata + immutable chunks
|   |
|   `-- program workspace
|       `-- owns config snapshot + logs + status + artifacts directory
|
`-- publishes lifecycle/progress facts to: automatic terminal UI
```

Runtime consumes a completed Study but does not reinterpret it. Study retains
Config and the assembled phase/task graph. Each model task retains its own
selected, validated schema; the live model directly owns its `SystemState`.
Persistence—not the model—owns recording lifecycle, writer threads, durable
chunks, program workspaces, and reconstruction rules. External programs write
their own domain artifacts, while UI only receives Runtime-owned lifecycle and
progress facts.

### General project procedure

```text
[1. Implement/register models and prepare program or Python tasks]
                              |
                              v
[2. Author wf_configs/study.json, named state schemas,
    and wf_configs/parameters.json]
                              |
                              v
[3. Call scientific_workflow::run(project_root)]
                              |
                              v
[4. Config reads and strictly parses all project JSON once]
                              |
                              v
[5. Study performs effect-free assembly and preflight]
    - validate every named state schema
    - resolve model keys, programs, and Python environments
    - expand/decode parameters and bind observation plans
                              |
                              v
[6. Runtime creates execution/replicate output scopes]
                              |
                              v
[7. Dependency-ordered phases admit eligible tasks]
              |                               |
              v                               v
    [Model task]                    [Program/Python task]
    initialize model                launch resolved invocation
    observe initial state           capture config/dependencies
    step + observe                  capture logs/status/workspace
    observe final state                       |
              |                               |
              v                               |
    [Persistence writers commit]              |
    [verified metadata + chunks]              |
              |                               |
              +---------------+---------------+
                              |
                              v
[8. Return deterministic summaries and completed output]
```

Configuration or preflight failures stop before output exists. Once execution
starts, Runtime owns scheduling, cancellation, and task coordination;
successful model recordings are published only after Persistence finalizes
their metadata and writers.

## Installation

### Use the published crate

For application development, prefer the published release:

```toml
[dependencies]
scientific-workflow = "0.10.2"
serde = { version = "1", features = ["derive"] }
```

Or add the same dependencies from the command line:

```bash
cargo add scientific-workflow@0.10.2
cargo add serde --features derive
```

Rust 1.97 or newer is required. Application executables should commit
`Cargo.lock`; libraries should normally leave final version selection to their
downstream application. The model attribute is re-exported by
`scientific-workflow`; no separate procedural-macro dependency is needed.

The published crate is the right choice when the supported workflow matches
the project:

- Rust models can implement `ScientificModel`, directly own `SystemState`, and
  use JSON-deserializable constants;
- work can be expressed as dependency-ordered model, executable, or Python
  tasks in `wf_configs/study.json`;
- named JSON state schemas and the central `wf_configs/parameters.json` are suitable
  configuration boundaries;
- automatic local recordings, program workspaces, and terminal UI are desired;
  and
- completed recordings can be analyzed through `StoredStateSeriesReader` and
  field decoders.

In that case, cloning the repository adds no capability. Depend on the release
and use only the public API below.

### Clone a local copy

Clone the complete repository when evaluating unreleased work, running the
bundled example and full Rust/Python validation suites, contributing upstream,
or temporarily consuming a local revision before it is published:

```bash
git clone https://github.com/dingyisun0101/Scientific-Workflow.git
```

Point an application at the cloned crate directory:

```toml
[dependencies]
scientific-workflow = { path = "../Scientific-Workflow/rust" }
serde = { version = "1", features = ["derive"] }
```

Adjust the relative path to match the checkout location. Clone the whole
repository rather than copying `rust/` alone because the crate uses the sibling
procedural-macro package and the repository also contains its Python reader,
example, architecture, and conformance tests. A path dependency follows local
edits immediately; commit the repository revision separately because
`Cargo.lock` cannot reproduce uncommitted path contents.

### Fork instead of only cloning

Create a fork when the required behavior deliberately exceeds the supported
replacement boundaries or must be maintained independently. Typical reasons
include:

- exposing a different public orchestration or model contract;
- implementing a custom persistence backend, writer lifecycle, or incompatible
  recording format;
- replacing scheduling, cancellation, output-layout, or UI policy;
- carrying organization-specific changes that cannot be contributed upstream;
  or
- preparing a pull request through a personal remote.

Do not fork merely to define new models, payload types, observation plans,
field decoders, executable tasks, Python tasks, or project configuration; those
are supported extension points. Before changing internals, read the subsystem
`api.md` replacement contract and pin dependent applications to a known fork
commit. This crate is pre-1.0, so review breaking notes before upgrading either
the release or a fork.

## Public API reference

The canonical ordinary import is `scientific_workflow::prelude::basic::*`.
Advanced readers and embedding hosts use
`scientific_workflow::prelude::advanced::*`, which is a strict superset.
The tables below list every supported public symbol and callable operation.
Documentation-hidden `__private`, `ModelRegistration`, and `PayloadTuple`
exports exist only for procedural-macro expansion or sealed tuple bounds and
are not supported application APIs.

The complete supported symbol inventory is:

- Basic: `run`, `model`, `WorkflowError`, `ScientificModel`, `TaskResult`,
  `ObservationPlan`, `ObservationStream`, `ObservationError`, `StateTime`,
  `SystemStateSchema`, `SystemState`, `StateSeries`, `StateSeriesPushError`,
  `PayloadInsertError`, `StateError`, and `StateSeriesError`.
- Advanced-only additions: `ConfigError`, `Study`, `StudyError`, `execute`,
  `RunSummary`, `ReplicateRunSummary`, `PhaseRunSummary`, `TaskRunSummary`,
  `TaskRunKind`, `RuntimeError`, `StateFieldSchema`, `StateSchemaAccess`,
  `StateMaintenance`, `JsonPayloadDecoder`, `JsonPayloadDecoderRegistry`,
  `JsonStringDecoder`, `JsonVecF64Decoder`, `StoredStateSeriesReader`,
  `RecordingTiming`, and `PersistenceError`.

### Crate root, errors, and import scopes

| API | Parameters | Purpose |
| --- | --- | --- |
| `scientific_workflow::run(project_root)` | `project_root: &Path` | Loads and preflights a project, executes it, and returns `Result<(), WorkflowError>`. This is the ordinary entry point. |
| `#[scientific_workflow::model("key")]` | One nonempty, whitespace-exact string literal on a `ScientificModel` impl | Registers the model implementation under the stable key selected by `wf_configs/study.json` and `wf_configs/parameters.json`. |
| `WorkflowError` | Variants `Study(StudyError)` and `Runtime(RuntimeError)` | Distinguishes effect-free loading/preflight failure from active execution failure. |
| `prelude::basic` | Glob import; no parameters | Re-exports `run`, `model`, ordinary state, observation, task, and workflow-error APIs. |
| `prelude::advanced` | Glob import; no parameters | Re-exports Basic plus Study loading, Runtime execution/summaries, verified persistence reading, Config errors, and advanced State traits. |

`config::basic`, `study::basic`, `persistence::basic`, `runtime::basic`, and
both UI tiers intentionally export no callable objects. Their ordinary behavior
is declarative or automatic. `error::advanced`, `observation::advanced`, and
`task::advanced` currently add no supported symbols beyond their Basic tiers.

### Model and task API

`task::basic::TaskResult<T = ()>` is
`Result<T, Box<dyn Error + Send + Sync + 'static>>` and is the common model
error boundary.

`task::basic::ScientificModel` is `Send + Sized + 'static` and has one
associated type, `Constants: DeserializeOwned + Send + Sync + 'static`:

| Method | Parameters | Purpose |
| --- | --- | --- |
| `observation_plan(constants)` | `constants: &Self::Constants` | Optional side-effect-free declaration hook. The default returns `ObservationPlan::all_fields()`; overrides may select streams, fields, cadence, and units. |
| `initialize(constants, schema)` | owned `Self::Constants`; `schema: &SystemStateSchema` | Required constructor. Returns a fully initialized model whose directly owned state was created from the supplied task-bound schema. |
| `state()` | `&self` | Required stable borrow of the model's directly owned `SystemState`. |
| `is_complete()` | `&self` | Required side-effect-free completion predicate. |
| `step()` | `&mut self` | Required single scientific transition returning `TaskResult`; every success must strictly advance state iteration. |
| `target_iteration()` | `&self` | Optional progress estimate. Defaults to `None`; once present it cannot decrease or disappear. |

Models receive no paths, writers, persistence sessions, progress callbacks, or
UI handles. Runtime checks cancellation between steps and automatically
observes initial, successful-step, and final states.

### Observation API

| Type or method | Parameters | Purpose |
| --- | --- | --- |
| `ObservationPlan::all_fields()` | None | Creates one inferred stream named `state` containing every schema field. |
| `ObservationPlan::fields(fields)` | `fields: IntoIterator<Item = impl Into<String>>` | Creates the inferred `state` stream with a nonempty unique field selection. |
| `ObservationPlan::streams(streams)` | `streams: IntoIterator<Item = ObservationStream>` | Creates a nonempty multi-stream plan; normalized stream names must be unique. |
| `ObservationPlan::with_iteration_unit(unit)` | consumes `self`; `unit: impl Into<String>` | Adds a nonblank unit for the iteration axis. |
| `ObservationPlan::with_physical_time_unit(unit)` | consumes `self`; `unit: impl Into<String>` | Adds a nonblank unit for the physical-time axis. |
| `ObservationStream::all_fields(name)` | `name: impl Into<String>` | Creates a named every-iteration stream containing all fields. |
| `ObservationStream::fields(name, fields)` | name plus `IntoIterator<Item = impl Into<String>>` | Creates a named stream with nonempty unique selected fields. |
| `ObservationStream::every_iterations(iterations)` | consumes `self`; positive `iterations: u64` | Changes cadence to iteration zero and iterations divisible by the interval; final observation is handled automatically. |
| `ObservationError` | Non-exhaustive error enum | Reports `EmptyPlan`, `EmptyStreamName`, `DuplicateStreamName`, `EmptyFieldName`, `EmptyFieldSelection`, `DuplicateField`, `UnknownField`, `InvalidSamplingInterval`, `EmptyAxisUnit`, `SchemaMismatch`, `StateAccess`, `EncodeField`, or `NonIncreasingObservation`. |

Plans and streams are immutable, cloneable declarations. They perform no IO;
Study binds them to the model task's selected schema during preflight.

### State Basic API

#### `StateTime`

| Method | Parameters | Purpose |
| --- | --- | --- |
| `from_iteration(iteration)` | `iteration: u64` | Creates an iteration-only coordinate. |
| `from_iteration_and_physical_time(iteration, physical_time)` | `u64`, finite `f64` | Returns `Some(StateTime)` for a finite physical coordinate, otherwise `None`. |
| `iteration()` | `self` | Returns the iteration. |
| `physical_time()` | `self` | Returns `Option<f64>`. |
| `checked_advance(increment)` | `self`; `increment: Option<f64>` | Computes the next iteration and optional physical-time advance without mutation. |

#### `SystemStateSchema` and `SystemState`

| Method | Parameters | Purpose |
| --- | --- | --- |
| `SystemStateSchema::load_json_template(path)` | `path: &Path` | Reads and strictly validates a standalone JSON state schema. Composed Workflow instead supplies Config's already parsed named documents internally. |
| `schema.create_empty_state(time)` | `time: StateTime` | Creates an empty `SystemState` sharing the schema. |
| `state.time()` | `&self` | Returns the current `StateTime` by value. |
| `state.advance_time(increment)` | `&mut self`; `Option<f64>` | Atomically advances iteration and, when supplied, an existing physical coordinate. |
| `state.schema()` | `&self` | Borrows the shared `SystemStateSchema`. |
| `state.contains_payload(key)` | `key: &str` | Reports whether a declared field is populated. |
| `state.initialize_payload(key, payload)` | `key: &str`; owned `T: Serialize + Clone + Send + 'static` | Initializes a field exactly once and establishes its retained concrete type. Failure returns the unchanged payload in `PayloadInsertError<T>`. |
| `state.insert_payload(key, payload)` | same key and `T` bounds | Initializes or replaces a same-typed payload, returning `Option<T>` for the previous owner. |
| `state.payload::<T>(key)` | `key: &str`; `T: Any` | Returns `&T` after field, presence, and exact-type validation. |
| `state.payload_mut::<T>(key)` | `key: &str`; `T: Any` | Returns `&mut T` under the same validation. |
| `state.borrow_payloads::<Q>(keys)` | tuple type `Q`; matching tuple of 2-8 field names | Returns checked immutable references to distinct typed fields. |
| `state.borrow_payloads_mut::<Q>(keys)` | tuple type `Q`; matching tuple of 2-8 field names | Returns checked disjoint mutable references after complete validation. |
| `state.take_payload::<T>(key)` | `key: &str`; `T: Any + Send` | Moves a payload out without cloning while retaining the field's type contract. |

`SystemState` is `Clone`; cloning deep-clones every populated payload. It is
`Send` but not `Sync`, so shared cross-thread mutation requires external
synchronization.

#### `StateSeries` and ownership-preserving errors

| Method | Parameters | Purpose |
| --- | --- | --- |
| `StateSeries::new(schema)` | owned `SystemStateSchema` | Creates an empty ordered series. |
| `StateSeries::with_capacity(schema, capacity)` | schema; `capacity: usize` | Creates an empty series with owner capacity reserved. |
| `schema()`, `len()`, `is_empty()`, `capacity()` | `&self` | Inspect collection identity and size. |
| `reserve(additional)` | `&mut self`; `additional: usize` | Reserves state-owner capacity. |
| `state_at(position)` | `position: usize` | Borrows a state by collection position, not scientific iteration. |
| `payload_mut_at::<T>(position, key)` | `usize`, `&str`; `T: Any` | Mutably borrows one payload without exposing structural state mutation. |
| `first_state()`, `last_state()` | `&self` | Borrow boundary states, or `None` when empty. |
| `as_state_slice()`, `iter()` | `&self` | Borrow all states in increasing iteration order. |
| `push_state(state)` | owned `SystemState` | Appends only when schema identity matches and iteration strictly increases. Rejection preserves the state. |
| `pop_state()` | `&mut self` | Moves out the last state. |
| `clear_states()` | `&mut self` | Drops states but retains schema and allocation capacity. |
| `into_states()` | owned `self` | Moves out the complete backing vector. |
| `StateSeriesPushError::error()` | `&self` | Borrows the `StateSeriesError`. |
| `StateSeriesPushError::state()` | `&self` | Borrows the unchanged rejected state. |
| `StateSeriesPushError::into_parts()` | owned `self` | Returns `(StateSeriesError, SystemState)`. |
| `PayloadInsertError<T>::error()` / `payload()` | `&self` | Borrow the rejection reason or unchanged incoming payload. |
| `PayloadInsertError<T>::into_parts()` | owned `self` | Returns `(StateError, T)` without cloning. |

`StateSeries` supports `IntoIterator` for owned and borrowed series. Explicit
`Clone` deep-clones its states and payloads. `StateError` is non-exhaustive and
currently exposes `TemplateRead`, `TemplateParse`, `EmptyFieldName`,
`DuplicateField`, `UnknownField`, `RepeatedPayloadBorrow`, `MissingPayload`,
`TypeMismatch`, `PayloadAlreadyInitialized`, `IterationOverflow`,
`MissingPhysicalTime`, and `InvalidPhysicalAdvance`. `StateSeriesError` is
non-exhaustive and exposes `SchemaMismatch`, `NonIncreasingIteration`,
`PositionOutOfBounds`, and `PayloadAccess`.

### State Advanced API

Import the extension traits from `state::advanced` or `prelude::advanced`:

| Type or method | Parameters | Purpose |
| --- | --- | --- |
| `StateFieldSchema::position()` | `&self` | Returns declaration-order slot position. |
| `StateFieldSchema::name()` | `&self` | Borrows the normalized field name. |
| `StateFieldSchema::description()` | `&self` | Borrows the optional normalized description. |
| `StateSchemaAccess::shares_schema_instance(other)` | `other: &SystemStateSchema` | Tests shared allocation identity, not structural equality. |
| `template_path()` | `&self` | Borrows retained schema provenance path. |
| `field_schemas()` | `&self` | Borrows fields in declaration order. |
| `field_schema(name)` / `contains_field(name)` | `name: &str` | Looks up or tests a field. |
| `len()` / `is_empty()` | `&self` | Inspects schema field count. |
| `to_json_template()` | `&self` | Returns strict pretty JSON for the schema. |
| `StateMaintenance::clone_structure_without_payloads(time)` | `time: StateTime` | Creates an empty related state retaining schema and field type contracts. |
| `replace_time(time)` | `&mut self`; `StateTime` | Replaces time and returns the previous value. |
| `populated_field_count()` | `&self` | Counts populated slots. |
| `payload_has_type::<T>(key)` | `key: &str`; `T: Any` | Tests the exact concrete type of a populated field. |
| `clear_payload(key)` | `key: &str` | Drops one payload and returns whether one was present. |
| `clear_all_payloads()` | `&mut self` | Drops every payload while retaining structure and type contracts. |

### Config and Study Advanced API

| API | Parameters | Purpose |
| --- | --- | --- |
| `ConfigError` | Non-exhaustive enum | Reports `Read`, `Parse`, `DuplicateKey`, `InvalidDocument`, `PathOutsideConfig`, `InvalidProgram`, `UnknownDependency`, `UnknownState`, `ExpansionOverflow`, or `DecodeModelConstants`. Context fields retain paths, JSON pointers, model/state/phase keys, ordinals, and sources as applicable. |
| `Study::load(project_root)` | `project_root: &Path` | Performs complete effect-free Config loading, state validation, model discovery, parameter decode, program/Python resolution, and observation preflight. |
| `study.project_root()` | `&self` | Borrows Config's canonical project root. |
| `study.output_root()` | `&self` | Borrows the inferred `<project-root>/output` path; it need not exist yet. |
| `StudyError` | Non-exhaustive enum | Reports `Config`, `State`, `InvalidModelRegistration`, `UnknownModel`, `ModelPreflight`, or `TaskIdentityOverflow`. Every variant occurs before Runtime creates output. |

`Study` is immutable and clone-cheap through shared ownership. Its phase graph,
tasks, selected schemas, resolved constants, and policies are intentionally not
publicly mutable or inspectable.

### Runtime Advanced API

| API | Parameters | Purpose |
| --- | --- | --- |
| `runtime::advanced::execute(study)` | owned `Study` | Executes validated intent and returns `Result<RunSummary, RuntimeError>`. |
| `RunSummary::output_directory()` | `&self` | Borrows the unique execution directory. |
| `RunSummary::replicates()` | `&self` | Borrows successful replicate summaries in ascending index order. |
| `ReplicateRunSummary::index()` | `&self` | Returns the zero-based replicate index. |
| `ReplicateRunSummary::output_directory()` | `&self` | Borrows the replicate directory. |
| `ReplicateRunSummary::phases()` | `&self` | Borrows phases in dependency execution order. |
| `PhaseRunSummary::name()` | `&self` | Borrows the manifest phase key. |
| `PhaseRunSummary::tasks()` | `&self` | Borrows task summaries in deterministic Study order. |
| `TaskRunSummary::identity()` | `&self` | Borrows the Study-inferred identity. |
| `TaskRunSummary::kind()` | `&self` | Returns non-exhaustive `TaskRunKind::Model` or `TaskRunKind::Program`. |
| `TaskRunSummary::model()` | `&self` | Returns the model key for model tasks. |
| `TaskRunSummary::program()` | `&self` | Returns the resolved executable/interpreter/manager path for program tasks. |
| `TaskRunSummary::final_iteration()` | `&self` | Returns the final iteration for model tasks. |
| `TaskRunSummary::output_directory()` | `&self` | Borrows the completed recording or program-workspace directory. |
| `RuntimeError` | Non-exhaustive enum | Reports `ExecutionCancelled`, `OutputScope`, `Task`, `TaskPanicked`, `TaskTimedOut`, `TaskCancelled`, `PhaseTimedOut`, `StartWorker`, `ReplicatePanicked`, or contextual `Replicate` failure. |

Summary objects are cloneable, read-only owned values and perform no IO.

### Persistence Advanced read API

Persistence writing is automatic and has no public writer constructor. The
public surface reconstructs completed model recordings:

| Type or method | Parameters | Purpose |
| --- | --- | --- |
| `JsonPayloadDecoder<T>::decode_json_payload(raw_json)` | `raw_json: &str` containing exactly one JSON value | Converts borrowed encoded input into one owned typed field payload. The associated `Error` must be thread-safe and `'static`; compatible closures implement the trait. |
| `JsonPayloadDecoderRegistry::new()` | None | Creates an empty exact-field decoder registry. |
| `with_capacity(capacity)` | `capacity: usize` | Creates an empty registry with key capacity. |
| `with_json_field::<T>(key)` | consumes registry; `key: impl Into<String>`; `T: DeserializeOwned + Serialize + Clone + Send + 'static` | Adds a Serde JSON decoder and returns the registry. |
| `register_for_field::<T, D>(key, decoder)` | `&mut self`; exact key; `D: JsonPayloadDecoder<T>` | Registers one custom typed decoder. Empty or duplicate keys fail. |
| `len()`, `is_empty()` | `&self` | Inspect registry size. |
| `has_decoder_for_field(key)` | `key: &str` | Tests exact registration. |
| `registered_field_names()` | `&self` | Iterates keys in unspecified hash-map order. |
| `JsonStringDecoder` | Unit value; no parameters | Built-in decoder for `String`. |
| `JsonVecF64Decoder` | Unit value; no parameters | Built-in decoder for `Vec<f64>`. |
| `StoredStateSeriesReader::open_completed_recording(root, decoders)` | `root: &Path`; owned registry | Synchronously validates completed `metadata.json` and retains the decoder registry. |
| `recording_directory()` | `&self` | Borrows the supplied recording root. |
| `stream_names()` / `format_version()` | `&self` | Iterates stream names in metadata order or returns the format version. |
| `user_metadata()` / `terminal_metadata()` | `&self` | Borrows creation-time or completion-time JSON metadata maps. |
| `recording_timing()` | `&self` | Borrows verified `RecordingTiming`. |
| `stream_record_count(stream)` | `stream: &str` | Returns checked declared record count. |
| `stream_encoded_bytes(stream)` | `stream: &str` | Returns checked declared encoded bytes. |
| `read_stream_as_state_series(stream)` | `stream: &str` | Verifies and reconstructs one complete ordered `StateSeries`. |
| `read_all_streams_as_state_series()` | `&self` | Returns every stream as `Vec<(String, StateSeries)>`, with no partial success. |
| `read_latest_state_from_stream(stream)` | `stream: &str` | Verifies the newest chunk and reconstructs its final `SystemState`. |
| `RecordingTiming::created_at_utc()` / `finalized_at_utc()` | `&self` | Borrows RFC 3339 UTC timestamps. |
| `active_duration_ns()` / `active_duration()` | `&self` | Returns exact nanoseconds or `Duration`. |
| `continuation_count()` | `&self` | Returns the persisted continuation count; new Workflow recordings use zero. |

`PersistenceError` is non-exhaustive. Its current variants are
`Observation`, `RecordingDirectoryExists`, `InvalidConfiguration`,
`DuplicateStateStream`, `UnknownStateStream`, `StateWriterClosed`,
`RecordingFinished`, `RecordingDirectoryInUse`, `NoRecordedState`,
`OperationalTimestamp`, `OperationalDurationOverflow`, `UnsupportedVersion`,
`InvalidMetadata`, `RecordingNotComplete`, `MissingChunk`,
`ChunkSizeMismatch`, `ChecksumMismatch`, `InvalidRecord`, `DuplicateDecoder`,
`MissingDecoder`, `DecodeField`, `StateSeriesInvariant`, `Io`, `Json`,
`ByteCountOverflow`, `RecordTooLarge`, `OutOfOrderIteration`,
`WriterQueueDisconnected`, `StateWriterTerminated`, and `StateWriterPanicked`.
Reader users normally encounter the metadata, integrity, decoder, state-series,
IO, and JSON families; write-only variants remain public so Runtime can retain
complete source chains, not to expose writer control.

For exhaustive invariants, failure atomicity, thread-safety, and examples, use
the subsystem contracts linked at the end of this guide.

## Complete user procedure

### 1. Define a model

The model directly owns its canonical `SystemState`. Its associated constants
type is the complete typed form of one expanded section selected from
`wf_configs/parameters.json` by the model's registration key.
In the fixed-duration example below, `StateTime::iteration()` is the current
completed iteration, while `target_iteration` retains the configured stopping
target; it is not a second progress counter.

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
    target_iteration: u64,
}

#[scientific_workflow::model("population")]
impl ScientificModel for PopulationModel {
    type Constants = Constants;

    fn initialize(constants: Constants, schema: &SystemStateSchema) -> TaskResult<Self> {
        let mut state = schema.create_empty_state(StateTime::from_iteration(0));
        state.initialize_payload("population", constants.initial_population)?;
        state.initialize_payload("cumulative_births", 0_u64)?;
        Ok(Self {
            state,
            target_iteration: constants.steps,
        })
    }

    fn state(&self) -> &SystemState { &self.state }
    fn is_complete(&self) -> bool {
        self.state.time().iteration() >= self.target_iteration
    }
    fn target_iteration(&self) -> Option<u64> { Some(self.target_iteration) }

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

`ScientificModel::observation_plan` defaults to
`ObservationPlan::all_fields()`. Override it only
when selected fields, named streams, cadence, or units carry scientific
meaning. The observation-plan function may inspect constants but must be
deterministic and side-effect-free.

The model is an ordinary Rust owner: `state()` returns a direct borrow of its
`SystemState`, and coupled field access uses a typed tuple expansion. The
attribute does not define the model or generate field access. Its stable key is
only the automatic bridge from `wf_configs/study.json` to compiled Rust behavior, so there
is no separate registry list in `main`.

Standalone executable and Python tasks require no Rust trait or wrapper.
Declare them in `wf_configs/study.json`; they receive the same captured central project
configuration and dependency results at runtime. A Python task declares its
environment locally inside its nested `python` object—there is no global
environment registry.

### 2. Write project JSON

```text
project/
├── scripts/
│   └── plot.py
└── wf_configs/
    ├── study.json
    ├── parameters.json
    └── states/
        └── population.json
```

The `wf_configs/` directory is required and identifies the project root passed
to Workflow. Both `wf_configs/study.json` and
`wf_configs/parameters.json` are required. The `states/` directory is the
recommended organization, not a requirement: schemas may live anywhere beneath
`wf_configs/` when their exact project-root-relative paths are supplied in
`study.json.paths.states`. Paths outside `wf_configs/` are rejected.

`wf_configs/states/population.json`:

```json
{
  "fields": [
    {"name":"population"},
    {"name":"cumulative_births"}
  ]
}
```

`wf_configs/parameters.json`:

```json
{
  "population": {
    "initial_population": 10,
    "steps": {"$sweep":[100, 200]}
  },
  "plot": {
    "output_directory": "output/plots",
    "dpi": 180
  }
}
```

`wf_configs/study.json`:

```json
{
  "paths": {
    "states": {
      "population": "wf_configs/states/population.json"
    }
  },
  "phases": {
    "simulate": {
      "tasks": [
        {"model":"population","state":"population"}
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
local backend with a 64 MB decimal chunk target and queue capacity.
Projects that need explicit operational sizing may add:

```json
"persistence": {
  "chunk_target_mb": 64,
  "queue_capacity_mb": 64
}
```

All JSON persistence sizes are authored in decimal megabytes; one MB is
exactly 1,000,000 bytes. Config converts both settings into exact internal byte
counts.

Users never provide Rust recording paths, construct a backend, submit
observations, or finalize a recording. External programs may own a
project-relative domain output such as the Python plotter's `output/plots`.

UI is also automatic. No `ui` object or model display fields are required.
Interactive stdin and stderr select the Ratatui dashboard with inferred task
rows for only the current phase, progress, timing, lifecycle messages, and the
`exit` command. The task-panel title carries the replicate and phase once.
Redirected execution uses stable plain lifecycle lines.

Config alone reads `wf_configs/study.json`, every named state document, and the complete
arbitrary `wf_configs/parameters.json` namespace once. `$sweep` creates independent Cartesian
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
binds each observation plan to its model task's explicitly selected state
schema, resolves program paths and Python
environments, creates immutable generic tasks and phases, compiles the
effective persistence plan, infers identities/output paths, schedules work,
and persists every task
automatically while publishing inferred terminal progress.

Each program or Python script receives absolute `WORKFLOW_CONFIG_PATH` and
`WORKFLOW_DEPENDENCIES_PATH` snapshot files plus project, execution, replicate,
and task-output paths through `WORKFLOW_*` environment variables. It runs in an
isolated `artifacts/` working directory; Workflow captures stdout, stderr, and
terminal status. A program may instead write domain outputs to a safe
project-relative destination it reads from `wf_configs/parameters.json`. Editing project
JSON after `Study::load` does not alter that execution.

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
Python plotting phase that opens the verified recordings and emits an SVG in
the configured `output/plots` directory.

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
