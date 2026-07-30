# SciTaskIO Clean-Slate Design

## Document Status

This is the living design record for the clean-slate SciTaskIO refactor.
It is updated throughout the architecture discussion.

Status labels used in this document:

- **Agreed**: explicitly requested or accepted by the project owner.
- **Proposed**: current recommendation, not yet accepted.
- **Open**: requires further discussion.
- **Deferred**: intentionally postponed.

No backward compatibility or legacy support is required. Existing code is
useful only as evidence about scientific workflows and failure modes; it does
not constrain the new API or persisted formats.

## Goals

### Agreed

SciTaskIO should naturally support dispatcher-like scientific projects around
three initial concepts:

1. `SystemState`: anything that describes a system at a particular time point.
2. `SSTS`: a time series of system states.
3. `Dispatcher`: accepts `fixed.json` and `sweep.json`, expands experiments,
   and provides automatic scoped execution, logging, and organization.

The design process must precede implementation. Source code must not be
refactored until the project owner explicitly permits it.

The current design scope is the Rust implementation only. Python support and
Rust/Python bridging will be designed later as a separate module and delivery
stage. The Rust ownership model must not be weakened or complicated in advance
to accommodate hypothetical Python object semantics.

## Findings From the Existing Repositories

The current responsibilities are fragmented:

- SciTaskIO models a `Trajectory` as independently timed signal tracks and
  mixes the data contract with JSON/NPY IO, checkpoint cleanup, asynchronous
  writing, memory monitoring, directory scanning, and model discovery.
- One example simulator owns a domain-specific `SystemState`: integer time,
  taxon counts, an optional lattice, cached mass, and activity status.
- The analysis project contains a separate SSTS abstraction and recovers
  parameters by decoding directory and file names.
- The dispatcher hardcodes mission types, constants, paths, sweep loops,
  Python environment details, workflow steps, and execution policy.
- Current numbered mission steps combine several distinct concepts:
  per-run computation, dependency ordering, batch optimization, and
  experiment-wide reduction.

The clean design should separate domain data, experiment specification,
workflow declaration, execution policy, and persistence.

## Conceptual Hierarchy

### Proposed

```text
Project
└── Experiment
    ├── fixed parameters
    ├── sweep definition
    ├── compiled run plan
    ├── experiment-scoped stages
    └── Runs
        ├── resolved immutable parameters
        ├── run-scoped stages
        └── Attempts
            ├── structured events and process logs
            ├── SSTS datasets
            ├── artifacts
            └── checkpoints
```

The boundaries are:

- `SystemState` and `SSTS` describe scientific data.
- `ExperimentSpec` describes what parameter space should be explored.
- `Workflow` describes computations and their dependencies.
- `Dispatcher` plans and schedules execution.
- Scoped contexts provide parameters, paths, logging, artifacts, and SSTS IO.
- Storage backends persist scientific data and orchestration records.

## Scientific Data Model

### SystemState

#### Agreed definition

A `SystemState` is anything that describes the system at a particular time
point.

#### Agreed requirements

- It is a general heterogeneous container with Python-dictionary-like
  insertion, lookup, mutation, removal, and iteration.
- It can own arbitrary scientific data rather than a fixed set of field types.
- Moving data into the state and taking data back out must transfer ownership
  without cloning or copying the underlying payload.
- Moving a state into an SSTS, between SSTS chunks, and into a writer must not
  clone its payloads.
- `SystemState` itself must implement `Clone`. An explicit state clone may
  clone payloads; zero-copy is required when loading/offloading payloads and
  moving states through SSTS/chunk/writer ownership boundaries.
- The set and order of state keys are defined by a JSON template before the
  first state is created. States cannot introduce undeclared keys.
- A state can create another empty state with the same declared keys/layout and
  no payloads.
- Public type and method names should remain concise.
- It must support efficient serialization through an extensible codec system.
- Its serialization design must admit a future Protocol Buffers encoding.

Zero-copy here means zero-copy ownership transfer within the process. Encoding
to JSON, Protocol Buffers, or another external byte representation inherently
writes bytes into an output buffer or stream; that encoding work is not
described as zero-copy.

#### Proposed logical structure

```text
SystemState
├── time: TimePoint
├── fields: StateMap<FieldName, OwnedValue>
```

### Revised recommended Rust shape

```rust
pub struct StateSpec {
    inner: Arc<StateLayout>,
}

pub struct SystemState {
    spec: StateSpec,
    time: TimePoint,
    values: Vec<Option<StateValue>>,
}

pub struct TimePoint {
    index: u64,
    physical: Option<f64>,
}

pub struct StateValue {
    // Opaque cloneable holder around an owned Rust value,
    // plus runtime type information for diagnostics/downcasting.
}
```

The JSON template is parsed and validated into `StateSpec`. Field order in the
template assigns compact field IDs. Each state shares the immutable spec
through `Arc` and stores only a slot vector. This is preferable to rebuilding
an `IndexMap<String, StateValue>` in every state:

- key strings and name-to-slot lookup are stored once;
- every state has deterministic field order;
- lookup by name resolves to an integer slot;
- serialization can use compact field IDs;
- an empty state is a vector of `None` slots;
- the declared key set cannot drift between states.

The `Arc` shares only immutable layout metadata. Payloads are still uniquely
owned and are deep-cloned when `SystemState::clone()` is explicitly called.

The time point is outside the value map because ordering, indexing, and
chunk-range metadata must not depend on a conventional string key. Units and
time-axis descriptions should normally live once in SSTS metadata rather than
being repeated in every state.

The state has no separate metadata or annotations map initially. Descriptive
values can be ordinary keyed entries when they genuinely belong to the state;
series-, run-, and experiment-level metadata belong to their corresponding
containers.

`SystemState` and `StateValue` implement `Clone`. Under the current
recommendation, cloning a state deep-clones every stored value. This preserves
independent-value semantics: mutating a clone does not affect the original.

This makes the ordinary inserted-value bound conceptually
`Any + Clone + Send + 'static`. Type erasure uses a cloneable trait-object
pattern whose `clone_box` operation calls the concrete value's `Clone`
implementation.

`StateValue` requires `Send`, but not `Sync`. This allows the entire state or
chunk to be moved to a writer thread without unnecessarily requiring values to
support concurrent shared access. The state owns values and therefore does not
accept non-`'static` borrowed references. An owned memory-map, buffer handle, or
other owner can still be stored.

Deep cloning is never part of ordinary IO flow. SSTS insertion, chunk rollover,
writer submission, removal, downcasting, draining, and consuming iteration all
move values and must not call `Clone` implicitly.

### Recommended method groups

#### Template and construction

```rust
let spec = StateSpec::load("state.json")?;
let state = spec.empty(time);
let next = state.empty(next_time);

state.time()
state.spec()
state.into_parts()
```

`state.empty(time)` is recommended over `clone_empty()`:

- it describes the result rather than the implementation;
- it makes the new time explicit;
- it does not imply that payloads are cloned;
- the returned state shares the same immutable `StateSpec` and contains no
  payloads.

Possible names considered:

- `empty(time)` — current recommendation;
- `empty_at(time)` — more explicit but slightly longer;
- `blank(time)` — concise but less conventional;
- `clone_empty()` — rejected because it obscures time and suggests cloning.

Time should be fixed when the state is constructed. Avoiding an ordinary public
time setter makes it harder to invalidate ordering after a state enters an
SSTS.

#### JSON loading boundary

The project owner proposed:

```rust
let state = SystemState::new(path)?;
```

The current architectural recommendation is instead:

```rust
let spec = StateSpec::load(path)?;
let state = spec.empty(time);
```

Reasons:

- `SystemState` remains an in-memory data object rather than a filesystem
  loader;
- the JSON template is parsed once and reused by every state;
- tests can construct a validated spec without temporary files;
- future protobuf or programmatic specs do not require changing
  `SystemState`;
- failure to read/parse a template is clearly separated from state creation;
- state creation still requires a template-derived `StateSpec`, so undeclared
  states cannot be created.

A concise convenience such as `SystemState::from_json(path, time)` may delegate
to `StateSpec::load(path)?.empty(time)`, but it should not be the only
architectural entry point. Calling fallible filesystem IO `new()` is also
unusual Rust API naming; `load` or `from_json` communicates the behavior more
accurately.

#### Typed dictionary operations

The Rust-facing API should behave approximately as follows:

```rust
state.set("population", population)?;
state.set("space", lattice)?;

let population = state.get::<Vec<u64>>("population")?;
let lattice = state.get_mut::<Lattice>("space")?;
let owned_lattice: Lattice = state.take("space")?;
```

`set` consumes `T`. `take` returns the same owned `T` and leaves the declared
slot empty. A large tensor,
vector, lattice, or domain object must not be cloned during either operation.
Calling `SystemState::clone()` is the explicit operation that may duplicate
those payloads.

Rust ownership transfer does not by itself promise address stability for a
large value stored inline inside `T`. Normal scientific containers such as
`Vec`, owned tensors, and memory-map handles move only a small descriptor while
their backing allocation remains in place.

Recommended operations:

```rust
state.set<T>(key, value) -> Result<Option<StateValue>, StateError>
state.try_set<T>(key, value) -> Result<(), OccupiedError<T>>
state.get<T>(key) -> Result<&T, StateTypeError>
state.get_mut<T>(key) -> Result<&mut T, StateTypeError>
state.take<T>(key) -> Result<Option<T>, StateTypeError>
state.has(key) -> Result<bool, UnknownField>
state.is<T>(key) -> Result<bool, UnknownField>
```

`has` means that a declared slot currently contains a payload. All states
already contain the same declared keys, so a `contains_key` method would add
little value. Unknown keys are errors.

Type mismatch errors should report the key, requested Rust type, and stored
Rust type. A failed typed `take` must leave the original entry in the state.

Returning an opaque `StateValue` from replacement permits the old value to have
a different Rust type. `StateValue` itself should provide:

```rust
value.is::<T>()
value.downcast_ref::<T>()
value.downcast_mut::<T>()
value.downcast::<T>() -> Result<T, StateValue>
value.type_id()
value.type_name()
```

The consuming `downcast` transfers the original value out of its box. It does
not clone the value or its buffers.

#### Untyped ownership operations

Untyped methods are necessary for generic routing, transformations, and
forwarding values whose concrete types are unknown:

```rust
state.value(key) -> Result<Option<&StateValue>, UnknownField>
state.value_mut(key) -> Result<Option<&mut StateValue>, UnknownField>
state.take_value(key) -> Result<Option<StateValue>, UnknownField>
state.set_value(key, value) -> Result<Option<StateValue>, UnknownField>
```

#### Collection operations

```rust
state.len()
state.is_empty()
state.keys()
state.values()
state.iter()
state.iter_mut()
state.clear()
state.into_iter()
```

Because the layout is fixed, `len()` should mean declared field count and
`is_empty()` should mean that no payload slots are filled; this asymmetry may
need reconsideration. A separate `filled()` count may be clearer.

Borrowed iteration exposes declared keys and optional opaque values. Consuming
iteration transfers ownership of filled values while retaining their field
identity.

### Serialization boundary

`SystemState` should not contain a codec registry and should not expose
format-specific conveniences such as `to_json_string()`. Those choices would
couple the ownership container to persistence and may allocate large temporary
buffers.

Instead, an external encoder receives:

```text
&SystemState + &CodecRegistry + streaming output
```

The registry resolves a value's Rust `TypeId` to a stable codec/type tag. The
decoder uses the stable serialized tag to find a codec and constructs a new
owned `StateValue`.

Conceptual APIs:

```rust
encoder.encoded_len(&state, &registry)
encoder.encode_into(&state, &registry, &mut writer)
decoder.decode_from(&registry, &mut reader) -> SystemState
```

Encoding borrows each payload and writes directly to the destination stream.
It does not clone the payload or first construct a second `SystemState`.
Format-specific framing, field tables, checksums, and protobuf support belong
to encoders and SSTS storage, not to `SystemState`.

### Clone semantics

#### Current recommendation

`SystemState::clone()` performs a deep logical clone by invoking `Clone` on
every stored concrete value. This keeps cloned states independent and preserves
unconditional mutable access and owned removal:

- `get_mut<T>` remains available without uniqueness checks;
- `remove<T>` always returns the owned `T`;
- mutating a cloned state cannot mutate the original.

The alternative is shallow cloning through `Arc`. That makes cloning cheap but
requires stored values to be `Sync` and complicates the API:

- mutable access works only for uniquely owned values or requires
  copy-on-write;
- returning an owned `T` works only when the reference count is one;
- otherwise removal must return `Arc<T>`.

Because zero-copy is required for loading/offloading rather than cloning, deep
explicit cloning is currently preferred. It means directly inserted values
must implement `Clone`.

#### Proposed owned-value representation

Each map entry is type-erased but owned:

```text
OwnedValue
├── stable type/codec tag
├── boxed owned value
└── codec reference or codec lookup information
```

The erased value is conceptually a boxed `Any + Send + 'static` plus downcast
and codec machinery. Boxing is important: when a `SystemState` or the
surrounding `Vec<SystemState>` reallocates, it moves only small ownership
handles rather than moving or copying large inline scientific values.

The container should accept any owned `Send + 'static` value. A value does not
have to be serializable merely to exist in a transient state. Persistence
requires a registered codec with a stable type tag. Attempting to persist an
entry without a codec is an explicit error.

This separates two capabilities:

- **containment**: generic ownership and typed downcasting;
- **persistence**: stable schema identity plus an encoder/decoder.

#### Why internal boxing remains necessary

Rust collections require elements to have a statically known size. Arbitrary
heterogeneous values such as `Vec<u64>`, a lattice, and a domain-specific
solver state have different concrete sizes and types. Type erasure therefore
needs an indirection with a known-sized handle, normally:

```text
Box<dyn CloneableAny + Send>
```

This internal box:

- gives every slot one known representation size;
- owns and drops the concrete value correctly;
- supports runtime downcasting;
- supports object-safe cloning through `clone_box`;
- lets states and slot vectors move without embedding large arbitrary values
  inline.

Dedicated public `set_boxed`/`take_boxed` methods are not necessary. `set<T>`
can perform the internal boxing, and `take<T>` can downcast the box and move
`T` back out. Typical large scientific types already own separate backing
allocations, so boxing moves only their small descriptor and does not copy the
backing data.

If a caller needs address stability for the top-level value itself, it can use
`Box<T>` or `Pin<Box<T>>` as the ordinary stored `T`:

```rust
state.set("solver", Box::new(solver))?;
let solver: Box<Solver> = state.take("solver")?.unwrap();
```

No specialized SystemState methods are needed for this case.

#### Proposed JSON template content

The template should define more than names. At minimum, every field needs a
stable codec/type tag so the state layout is useful for validation and
persistence:

```json
{
  "fields": [
    {"name": "population", "type": "vec.u64"},
    {"name": "space", "type": "example.lattice.v1"},
    {"name": "activity", "type": "example.activity.v1"}
  ]
}
```

Field order is meaningful and becomes the compact field-ID order. Optional
shape, dtype, unit, required/optional, and checkpoint requirements can be
added to individual field specifications.

#### Proposed codec contract

A codec should provide:

- a stable, language-neutral type tag;
- runtime compatibility checking/downcasting;
- encoding to a supplied streaming writer or output buffer;
- decoding into a newly owned value;
- encoded-length calculation when the format supports it;
- optional logical dtype, shape, and schema metadata;
- format-specific support such as JSON and future Protocol Buffers.

Built-in codecs should cover scalar values, strings, bytes, and common numeric
vectors/arrays. Tensor libraries and application domain objects can register
extension codecs without making the core depend on those libraries.

Protocol Buffers cannot directly represent arbitrary process-local Rust
objects. The future protobuf envelope therefore needs a stable field name,
stable type tag/schema reference, and encoded payload. Known common types may
receive native protobuf messages; extension types use their registered codec
payload.

### Rust-only design boundary

#### Agreed

The current API is designed around Rust ownership and type erasure:

- inserted values are ordinary owned Rust values;
- type recovery uses Rust downcasting;
- zero-copy transfer means moving Rust ownership;
- thread-transfer requirements are expressed with Rust traits such as `Send`;
- codecs operate on Rust values.

The core does not currently design for:

- Python reference counting;
- Python dictionaries or arbitrary `PyObject` values;
- NumPy buffer ownership;
- PyO3 APIs;
- DLPack, Arrow, or another bridge protocol.

“Python-dictionary-like” describes the ergonomic map operations only. It does
not require Python-compatible in-memory representation.

Domain invariants remain outside the generic core. For example, a simulator
should validate population/lattice consistency, valid taxon IDs, and cached
mass.

### TimePoint

#### Proposed

```text
TimePoint
├── index: non-negative integer
├── value: optional physical-time number
└── unit: optional unit identifier
```

The index provides deterministic ordering and checkpoint identity. Physical
time is optional. An SSTS should not mix incomparable time domains such as
arbitrary strings and numbers.

### State Schema

#### Proposed

An SSTS carries a schema containing field descriptors:

```text
FieldSchema
├── name
├── value kind
├── dtype
├── shape constraints
├── unit
└── required-for-checkpoint
```

The schema validates generic representation constraints. Applications can add
domain-specific validation.

## SSTS

### Agreed representation

An SSTS is conceptually a growable array of system states:

```text
SSTS := Vec<SystemState>
```

Its fundamental public operations should mirror an owned growable sequence:

- `push(state)` consumes a `SystemState`;
- indexed and iterative borrowed access;
- mutable access;
- `pop()` and consuming iteration return owned states;
- reserving capacity;
- draining or splitting ranges without cloning payloads.

The canonical representation is state-major. Independent field channels are
not the defining data model.

### Open: completeness and multi-rate sampling

The simulator records small aggregate signals frequently and large lattice
states sparsely. Requiring every state to contain every field would duplicate
large data or force unrelated field series back into the old track model.

### Proposed multi-rate semantics

SSTS is an ordered stream of possibly partial states:

```text
t=0    {population, lattice, activity}
t=1    {population, activity}
t=2    {population, activity}
t=10   {population, lattice, activity}
```

The API may derive channel-like views without changing the owned state-major
representation:

- `states()` returns ordered state records;
- `channel(name)` iterates references to states that contain a field;
- `state_at(t)` returns fields recorded exactly at a time;
- `snapshot_at(t, fill=...)` explicitly reconstructs a state;
- `checkpoint_at(t)` requires all checkpoint fields.

### Proposed natural chunking

An SSTS chunk is itself an owned growable array:

```text
SSTSChunk
├── states: Vec<SystemState>
├── first/last time metadata
└── approximate or encoded byte count
```

The streaming writer owns one active `Vec<SystemState>`. When the configured
chunk policy triggers, it transfers the entire vector to the encoder or writer
using an ownership swap such as `mem::take`. It then continues with a new
vector. No state payload is cloned.

Chunk boundaries always occur between complete `SystemState` records. One
oversized state is allowed to occupy a chunk by itself.

Candidate chunk policies:

- maximum states per chunk;
- maximum estimated in-memory bytes;
- maximum encoded bytes;
- a combination of state count and bytes.

For arbitrary values, exact serialized size may be unavailable before
encoding. The codec can provide `encoded_len` where possible. Otherwise the
writer can use an estimate or stream length-delimited state records and treat
the byte target as a soft boundary. A chunk may exceed its target by at most
one state.

The storage format should be append-oriented and length-delimited so a writer
can:

- stream records without constructing a second full chunk in memory;
- detect truncated final records;
- resume at a valid state boundary;
- build indexes for time/range access;
- later encode the same logical records using Protocol Buffers.

Core SSTS owns representation, validation, streaming, slicing, chunking, and
persistence. Domain analysis, plotting, animation, and scientific
transformations live in higher-level packages.

### Proposed responsibility split for automatic IO

The requested behavior should be presented as one natural append workflow but
implemented with three focused ownership types:

```text
Ssts
├── spec: StateSpec
└── states: Vec<SystemState>

SstsWriter<S: ChunkSink>
├── buffer: Ssts
├── sink: S
├── policy: ChunkPolicy
├── estimated_bytes: usize
└── next_chunk: u64

SstsReader<S: ChunkSource>
├── manifest
├── source: S
└── codecs: CodecRegistry
```

`Ssts` is the predictable in-memory array. Borrowing or mutating it never
causes hidden filesystem IO. `SstsWriter::push(state)` is the simple public
entry point for automatic chunking: it validates and appends the state,
updates the active size, and flushes when its policy triggers.

This split preserves the requested user experience without coupling ordinary
array access to paths, filesystem failures, or writer lifetime state.

### Ownership rollover

Chunk rollover should not clone an empty series and then clear the populated
one element by element. It should replace the active owner:

```rust,ignore
let full = std::mem::replace(
    &mut self.buffer,
    Ssts::with_capacity(self.spec.clone(), self.policy.next_capacity()),
);
```

The old vector and every `SystemState` payload move into `full`. The new buffer
contains no payloads and cheaply shares only `StateSpec`. After the sink
successfully commits the chunk, `full` is dropped. Thus “erase itself” means
ownership transfer followed by destruction after commit, not cloning or
per-state clearing.

An IO failure must not silently discard the transferred chunk. The initial
synchronous sink should return the owned chunk with its error or allow the
writer to restore it before returning. Background writing can be added later,
but it must retain failed chunks and put the writer into an explicit failed
state.

### Chunk size accounting

“Current size” has three different meanings:

- number of complete states;
- estimated in-memory payload bytes;
- bytes produced by a particular encoder.

State count is exact and format-independent. Exact encoded JSON size is not
known without performing the encoding. Generic `Any` payloads also have no
built-in heap-size operation.

The proposed first policy combines an exact state limit with a codec-provided
memory estimate:

```text
ChunkPolicy
├── max_states: NonZeroUsize
└── max_bytes: Option<NonZeroUsize>
```

Each registered payload codec provides `estimate_bytes` for its concrete type.
For a dense tensor this can use tensor element count, scalar width, and shape
metadata without inspecting or copying the tensor buffer. The writer updates a
running total during `push`; it does not rescan the whole active series.

The writer flushes after appending the state that meets or exceeds either
threshold. A single oversized state forms a valid one-state chunk.

### Proposed JSON dataset layout

A single ever-growing JSON document is a poor chunk target: safely appending to
an open JSON array is awkward, a crash can leave the entire document invalid,
and finalization must maintain closing delimiters.

The proposed durable output is a dataset directory:

```text
output/
├── series.json
└── chunks/
    ├── 000000.json
    ├── 000001.json
    └── 000002.json
```

`series.json` is the authoritative manifest and contains:

```text
format name and version
one embedded StateSpec
time-axis metadata
ordered chunk descriptors
per-chunk state count
first and last TimePoint index
encoded byte length
optional checksum
```

The specification is stored once, not repeated in each state. Chunk files
contain ordered state records and payload values. Missing state fields remain
explicitly distinguishable from present payload data.

Each chunk is written to a temporary sibling path and atomically renamed before
the manifest references it. The manifest is likewise replaced atomically.
`finish()` flushes the final partial buffer and finalizes the manifest.

This format satisfies JSON write/read requirements while preserving real chunk
boundaries and crash recovery. A later one-file JSON export can be offered as
an interoperability operation, but it should not be the primary streaming
storage format.

### Payload codec requirement

`SystemState` cannot serialize arbitrary `Any` values by itself. Stable field
type tags must resolve through an IO-owned registry:

```text
CodecRegistry
└── type tag -> concrete Rust TypeId + encoder + decoder + size estimator
```

For writing, the codec borrows the erased payload internally, verifies its
concrete type, and emits JSON. For reading, it decodes the JSON value into the
registered concrete type and returns a new internally erased owner.

The JSON backend initially registers the selected
`physics_in_parallel::math::Tensor` combinations. The registry belongs to the
IO layer, not `SystemState`, so a future Protocol Buffers backend can reuse the
same stable field tags without making JSON part of the state container.

### Proposed read interfaces

`SstsReader::open(path, codecs)` loads and validates the manifest and embedded
specification. It should support:

- iterating chunks without loading the entire dataset;
- reading one chunk as an owned `Ssts`;
- iterating owned or borrowed states within a chunk;
- collecting all chunks into one in-memory `Ssts` when the dataset is small;
- selecting chunks by integer time range through manifest metadata.

Every decoded state in one reader shares the single `StateSpec` reconstructed
from the manifest.

### Decisions required before SSTS implementation

The recommended initial choices are:

1. Use both `max_states` and estimated tensor bytes for rollover.
2. Use a JSON manifest plus separate JSON chunk files rather than one growing
   JSON document.
3. Implement synchronous atomic chunk commits first; add background writing
   behind the same sink boundary later.
4. Require strictly increasing `TimePoint::index` values while allowing gaps.
5. Treat the integer index as authoritative; physical time may be absent and
   need not define chunk ordering.

### Proposed time-series source layout

The time-series implementation will follow the modern Rust module layout used
by SystemState:

```text
src/
├── time_series.rs
└── time_series/
    ├── error.rs
    ├── codec.rs
    ├── series.rs
    ├── format.rs
    ├── writer.rs
    └── reader.rs
```

There will be no `time_series/mod.rs`. The sibling
`src/time_series.rs` file is the small public facade.

#### `src/time_series/error.rs`

Owns the unified `SeriesError` surface:

- incompatible state specification;
- non-increasing time index;
- missing or duplicate payload codec;
- codec type mismatch;
- invalid manifest or chunk structure;
- truncated/missing chunk;
- filesystem and JSON failures;
- writer-finished or writer-failed state.

This file comes first so every later module reports failures consistently.

#### `src/time_series/codec.rs`

Owns JSON payload type registration:

```text
CodecRegistry
└── stable type tag -> concrete TypeId + JsonCodec + size estimator
```

A generic registered codec interacts with `SystemState` through its typed
public methods. It does not require exposing `StateValue` or the private
erasure module.

The core registry can register any `Serialize + DeserializeOwned` payload.
Tests and applications register concrete `physics_in_parallel` tensor types
with tensor-aware byte estimators. This keeps the time-series core generic and
avoids a mandatory production dependency on one tensor package.

#### `src/time_series/series.rs`

Owns both the in-memory `StateSeries` and its ownership-transfer unit,
`StateChunk`:

```text
StateSeries
├── spec: StateSpec
└── states: Vec<SystemState>

StateChunk
├── ordinal: u64
├── spec: StateSpec
├── states: Vec<SystemState>
└── estimated_bytes: usize
```

`StateSeries` provides growable-array operations, schema validation, strictly
increasing time checks with gaps allowed, indexing, iteration, reserve, pop,
and ownership-based draining/splitting. `StateChunk` validates complete chunk
metadata and exposes first/last integer time indices. Both are owned
state-array representations and perform no filesystem IO, so keeping them
together is cohesive.

#### `src/time_series/format.rs`

Owns the private versioned Serde representations for:

- `series.json`;
- embedded `StateSpec`;
- ordered chunk descriptors;
- chunk JSON headers;
- time-point records;
- state payload maps.

These are storage-format types, not public domain types. Keeping them separate
prevents serde annotations and format-version concerns from leaking into
`StateSeries`, `SystemState`, or the writer state machine.

#### `src/time_series/writer.rs`

Owns `ChunkPolicy`, `SeriesWriter`, and automatic chunking:

- validates nonzero state and optional byte thresholds;
- determines rollover and next-buffer capacity;
- accepts states through one `push` method;
- updates the running byte estimate;
- swaps out the active `StateSeries` on policy rollover;
- encodes and atomically commits chunk JSON;
- atomically updates the manifest;
- retains or restores ownership on failure;
- flushes the final partial chunk through `finish`.

The initial implementation is synchronous. Its ownership boundary will permit
a later background sink without changing `StateSeries`.

#### `src/time_series/reader.rs`

Owns `SeriesReader`:

- opens and validates `series.json`;
- reconstructs the single shared `StateSpec`;
- validates ordered chunk metadata;
- reads one owned `StateChunk`;
- iterates chunks;
- selects chunks by integer time range;
- collects all chunks into a `StateSeries` when requested.

It does not load every tensor payload merely to inspect the manifest.

#### `src/time_series.rs`

The public facade declares private implementation modules and re-exports only
the intended end-user types. The initial expected exports are:

```rust,ignore
pub use codec::CodecRegistry;
pub use error::SeriesError;
pub use reader::SeriesReader;
pub use series::StateSeries;
pub use writer::{ChunkPolicy, SeriesWriter};
```

`StateChunk` visibility remains an open API detail: it should be public only if
reader iteration returns owned chunks directly. Private disk-format structs
will never be re-exported.

### Prerequisite SystemState review units

Before implementing the time-series folder, two narrow SystemState additions
may be required:

1. A crate-private `StateSpec` layout-identity operation backed by
   `Arc::ptr_eq`, allowing constant-time append validation.
2. A crate-private way to reconstruct a `StateSpec` from the embedded manifest
   JSON while retaining the manifest path as provenance.

The codec design deliberately avoids any need to expose `StateValue` or add
raw payload-slot methods to `SystemState`.

### Proposed implementation order

Under the one-file review rule, the dependency-oriented order is:

1. required `StateSpec` additions in `system_state/spec.rs`;
2. `time_series/error.rs`;
3. `time_series/codec.rs`;
4. `time_series/series.rs`;
5. `time_series/format.rs`;
6. `time_series/writer.rs`;
7. `time_series/reader.rs`;
8. `time_series.rs` facade;
9. update `lib.rs` to expose `time_series`;
10. add mirrored private tests and one public tensor-backed integration suite.

The revised folder contains six implementation files rather than eight.
`StateChunk` belongs beside its underlying state-array owner, and
`ChunkPolicy` belongs beside the writer state machine that evaluates it.
`codec.rs` and `format.rs` remain separate because runtime payload typing and
versioned disk representation change for different reasons.

### First time-series review unit

The first implementation review should update
`src/system_state/spec.rs`, not create a time-series file yet.

It needs exactly two crate-internal capabilities:

1. `StateSpec::shares_layout(&self, other: &Self) -> bool`

   - implemented with `Arc::ptr_eq`;
   - constant-time and allocation-free;
   - used by `StateSeries::push` to reject states from another schema;
   - crate-private because pointer identity is a construction invariant, not a
     general public definition of schema equality.

2. `StateSpec::parse(source, json) -> Result<Self, StateError>`

   - crate-private;
   - parses and validates an in-memory JSON specification;
   - retains a supplied manifest path as provenance;
   - allows `SeriesReader` to reconstruct the spec embedded in `series.json`;
   - becomes the shared implementation used by public `StateSpec::load`, so
     filesystem and embedded parsing cannot diverge.

The public template-first construction rule remains intact:

- applications create their initial specification through
  `StateSpec::load(path)`;
- only crate-internal persistence code uses `StateSpec::parse` when restoring a
  specification from a series manifest.

After that reviewed change, the first new time-series file will be
`src/time_series/error.rs`, followed by `codec.rs`, `series.rs`, `format.rs`,
`writer.rs`, and `reader.rs`.

No source file was edited while confirming this order.

### Crates.io publication cleanup

Publication preparation temporarily precedes the time-series implementation.
The legacy README and manifest were inspected for useful package structure,
license identity, repository metadata, keywords, categories, and user-facing
API organization. Legacy trajectory, Python, NPY, model-discovery, checkpoint,
and cross-language claims will not be carried into the clean crate.

Cargo's package conventions require the crate README to live in the package
root beside `Cargo.toml`. Because the Rust package root is currently `dev/`,
the publishable README is `dev/README.md`, not the repository-level README.

The one-file publication sequence is:

1. create the accurate package-root `dev/README.md`;
2. restore the MIT text as `dev/LICENSE`;
3. complete crates.io metadata and package file selection in
   `dev/Cargo.toml`;
4. replace the stale repository-root README with a concise repository landing
   page that points to the package;
5. remove the placeholder binary;
6. run formatting, checks, tests, Clippy, rustdoc, `cargo package --list`, and
   `cargo publish --dry-run`.

#### Package README implemented

- Added `dev/README.md`.
- Described only the currently implemented public `system_state` module.
- Documented template-defined fields, typed heterogeneous payload access,
  clone-free insertion and extraction, explicit deep state cloning, time
  coordinates, and JSON specification round trips.
- Marked time-series storage, chunking, and dispatch as under development
  rather than presenting planned APIs as published features.
- Added installation metadata, a canonical JSON template, a complete
  `SystemState` example, tensor-payload guidance, standard test instructions,
  the Rust 1.85 minimum version, and the MIT license declaration.
- Reused the useful explanatory organization of the legacy README without
  copying obsolete absolute paths, Python APIs, trajectory contracts, or
  compatibility promises.
- Kept `physics_in_parallel` as an application-selected payload dependency
  rather than claiming it is a required runtime dependency.
- The package README contains no Rust structs or methods and therefore adds no
  method-level `Reference` sections.
- No manifest, license, source, or repository-root README file was changed in
  this review unit.

#### crates.io transient publish failure

- Recorded a failed publish request at `2026-07-30 22:55:12 UTC`.
- Classified the response as registry infrastructure failure rather than
  manifest validation or authentication failure:

  ```text
  HTTP 503
  server: Varnish
  body: backend write error
  ```

- Queried the official crates.io status API. It reported
  `All Systems Operational`, indicating either a short-lived or region-specific
  write-path failure not reflected as a status-page incident.
- Queried the crates.io crate API with an explicit user agent. It returned
  `404` for `scientific-workflow`, confirming that version `0.1.0` was not
  registered despite the failed request.
- Confirmed `cargo search scientific-workflow` returned no published match.
- Therefore:

  - do not increment the package version;
  - do not rotate the API token;
  - do not interpret the response as a source or metadata defect;
  - do not repeatedly retry in a tight loop.

- Publication should resume only after the local license and manifest cleanup
  is complete. The intended sequence is:

  1. `cargo publish --dry-run`;
  2. inspect `cargo package --list`;
  3. wait briefly if the registry write path is unstable;
  4. retry `cargo publish` once;
  5. verify the exact crate and version through the registry API.

- No publish retry or project-file edit was performed while diagnosing this
  external service failure.

#### Repository ignore policy configured

- Added the repository-root `.gitignore`.
- Ignored Cargo build and package output through `**/target/`, covering the
  active crate and future workspace members.
- Ignored `dev/Cargo.lock` while the package is a reusable library. This rule
  must be revisited if the repository later ships a deployable binary whose
  exact dependency resolution should be committed.
- Ignored runtime-generated scientific output directories, including output,
  results, runs, checkpoints, logs, and explicitly generated-data roots.
- Ignored common generated numerical and columnar formats:
  NPY, NPZ, HDF5, Parquet, Arrow, Feather, and future SSTS artifacts.
- Ignored partial/temporary atomic-write files, tool caches, local environment
  files, editor state, and operating-system metadata.
- Deliberately did not ignore JSON globally. State templates, manifests used as
  fixtures, and configuration files remain trackable outside ignored runtime
  output directories.
- Deliberately preserved `dev/tests/fixtures/state.json`; verified it is not
  matched by any ignore rule.
- Verified representative Cargo targets, lockfiles, generated dataset paths,
  numerical artifacts, and partial files resolve to their intended rules with
  `git check-ignore`.
- This file defines repository hygiene only and introduces no Rust structs or
  methods requiring `Reference` sections.

## Experiment Configuration

### Agreed

The dispatcher accepts:

- `fixed.json` for constants that remain fixed throughout an experiment;
- `sweep.json` for parameters varied by the experiment.

### Proposed `fixed.json`

```json
{
  "model": {
    "family": "PAF",
    "b": 3,
    "lattice_shape": [1000, 1000]
  },
  "solver": {
    "steps": 160000,
    "signal_interval": 1,
    "space_interval": 10000
  }
}
```

### Proposed `sweep.json`

```json
{
  "strategy": "product",
  "axes": [
    {
      "path": "model.K",
      "values": [100, 300, 600]
    },
    {
      "path": "kernel.mu",
      "values": [0.2, 0.4, 0.6]
    }
  ],
  "replicates": 3
}
```

Sweep axes are explicit so constant JSON arrays, such as lattice shapes and
initial vectors, are not mistaken for sweep dimensions.

### Proposed initial rules

- Initial sweep strategy is Cartesian product.
- Replicates are supported explicitly.
- A sweep path may not conflict with a fixed parameter path.
- The compiler produces a complete immutable resolved parameter document for
  every run.
- Canonical resolved parameters determine a stable run ID.
- Sweep ordering is not part of permanent run identity.
- Replicate identity or seed participates in the run ID.

Zipped axes, explicit cases, sampled distributions, conditional axes, and
derived parameters are possible later extensions.

## Workflow Model

### Proposed

Workflows are dependency graphs of named stages, not numbered functions.

One representative scientific workflow roughly corresponds to:

```text
interaction_matrix
        │
        ├── well_mixed_simulation ── fixed_point
        │
        └── initial_lattice
                    │
                    ├── spatial_differential_equation
                    └── lattice_simulation
                                │
                                ▼
                    experiment_postprocessing
```

A stage declares:

- name;
- execution scope;
- dependencies;
- expected input artifacts;
- declared output artifacts;
- executor/resource requirements;
- implementation entry point.

### Proposed scopes

- `project`: once for the scientific project;
- `experiment`: once for one fixed/sweep specification;
- `run`: once for each resolved parameter combination;
- `attempt`: one execution attempt of a stage.

Batching is orthogonal to scope. Compatible run-scoped jobs may execute as a
batch while retaining independent run identities and output namespaces.
Experiment-wide post-processing is an experiment-scoped reduction.

## Scoped Execution

### Proposed

Task implementations receive a context rather than reconstructing paths and
environments:

```rust
fn simulate(ctx: &RunContext) -> Result<()> {
    let params: SimulationParams = ctx.params()?;
    let mut states = ctx.ssts_writer("trajectory")?;

    ctx.log().info("simulation started");
    // scientific computation
    states.append(state)?;
    ctx.artifacts().write_json("outcome", &outcome)?;
    Ok(())
}
```

A corresponding Python API should expose the same concepts.

The context supplies:

- resolved typed parameters;
- project, experiment, run, stage, and attempt IDs;
- private working and output directories;
- artifact lookup and writing;
- SSTS readers and writers;
- structured logging and stdout/stderr capture;
- cancellation;
- checkpoint access;
- subprocess execution.

Scientific tasks should not know the global output root or manually encode
parameter values into paths.

## Logging and Lifecycle

### Proposed

Every structured event contains:

- timestamp;
- project, experiment, run, stage, and attempt IDs;
- severity;
- message;
- optional structured fields.

JSON Lines is the canonical event stream, with a human terminal renderer.

Run-stage lifecycle:

```text
planned -> queued -> running -> succeeded
                            ├-> failed
                            └-> cancelled
```

Retries create new attempts rather than overwriting history. Successful
completion is represented by an atomically committed manifest listing declared
outputs; the dispatcher does not infer success from arbitrary file presence.

## Proposed Storage Layout

Stable IDs and manifests are authoritative. Human-readable parameter paths may
be generated as aliases but are not the database.

```text
project-output/
└── experiments/
    └── <experiment-id>/
        ├── experiment.json
        ├── fixed.json
        ├── sweep.json
        ├── plan.json
        ├── events.jsonl
        └── runs/
            └── <run-id>/
                ├── run.json
                └── stages/
                    └── <stage-name>/
                        └── attempts/
                            └── 0001/
                                ├── attempt.json
                                ├── events.jsonl
                                ├── stdout.log
                                ├── stderr.log
                                ├── ssts/
                                └── artifacts/
```

## Proposed Modules

- `scitask-core`
  - SystemState, TimePoint, field values, schemas, SSTS, validation
- `scitask-storage`
  - storage traits, chunked writers, manifests, atomic commits, filesystem
- `scitask-config`
  - fixed/sweep parsing, parameter paths, canonicalization, run planning
- `scitask-project`
  - project discovery, experiment/run records, artifacts, catalog queries
- `scitask-workflow`
  - stages, dependency graph, scopes, input/output contracts, batching policy
- `scitask-runtime`
  - contexts, workers, subprocesses, resources, cancellation, retry, logging
- `scitask-dispatch`
  - planning, scheduling, reconciliation, CLI

### Deferred module

- `scitask-python` or a more explicitly named Rust/Python bridge
  - Python task API
  - Python ownership and lifetime policy
  - NumPy/tensor buffer interoperability
  - analysis-friendly readers

This bridge is not part of the current Rust core design.

Systemd resource scopes should become one runtime backend rather than a
hardcoded dispatcher property.

## Proposed Delivery Stages

1. Define vocabulary, contracts, example manifests, and schemas.
2. Implement SystemState, TimePoint, field types, validation, and in-memory
   SSTS with Rust contract tests.
3. Implement streaming/chunked SSTS persistence and checkpoint lookup.
4. Implement fixed/sweep compilation, deterministic plans, and stable IDs.
5. Implement the project store, manifests, artifacts, and parameter queries.
6. Implement workflow graphs, scopes, dependencies, and declared artifacts.
7. Implement the local dispatcher, contexts, logs, retries, cancellation, and
   resource backend interface.
8. Express one representative scientific mission as the reference project
   without legacy adapters.
9. Separately design and implement the Python/bridging module.
10. Add Python analysis selection APIs and optional
    NumPy/dataframe/xarray adapters.

## Rust Crate and SystemState File Layout

### Agreed constraints

- The repository root is renamed from `SciTaskIO` to
  `scientific-workflow`.
- The clean implementation lives under `scientific-workflow/dev`.
- The current implementation target is `SystemState`.
- Internal boxing must not appear in the minimal end-user API.
- The crate uses Rust edition 2024 and current Rust module layout conventions.

### Proposed modern crate layout

```text
dev/
├── Cargo.toml
├── src/
│   ├── lib.rs
│   ├── system_state.rs
│   └── system_state/
│       ├── error.rs
│       ├── spec.rs
│       ├── state.rs
│       └── value.rs
└── tests/
    ├── system_state.rs
    └── fixtures/
        └── state.json
```

Rust 2018 and later do not require `mod.rs`. The current convention used here
is:

- `src/system_state.rs` declares the module's private submodules and public
  re-exports;
- `src/system_state/state.rs`, `spec.rs`, `value.rs`, and `error.rs` contain
  implementation details;
- `src/lib.rs` exposes `pub mod system_state`.

The existing `src/main.rs` represents a binary target. SciTaskIO's core should
be a library target. A CLI can later live explicitly under `src/bin/` without
mixing command-line concerns into the library.

The Cargo package name should follow lowercase package naming. The current
`SciTaskHelper` package name is not recommended. See the naming decision below.

### Naming decision

#### Agreed

Use **Scientific Workflow** as the project name and `scientific-workflow` as
the Cargo package:

```toml
[package]
name = "scientific-workflow"
```

```rust
use scientific_workflow::system_state::{StateSpec, SystemState};
```

Why `scientific-workflow`:

- it directly communicates that the crate organizes and executes scientific
  work;
- workflow is broad enough to include scientific states, time series,
  parameter sweeps, scoped stages, persistence, logging, and dispatch;
- it is descriptive rather than a brand-like contraction;
- it avoids the narrower `IO` label while remaining more action-oriented than
  `scientific-project`.

If the project later becomes a Cargo workspace, use:

```text
scientific-workflow             facade and primary user API
scientific-workflow-core        SystemState and SSTS
scientific-workflow-storage     persistence
scientific-workflow-dispatch    planning and execution
```

The initial single crate should be `scientific-workflow`, without creating
workspace packages prematurely.

Other considered names:

- `scientific-experiment`: more specific, but the framework also owns
  project-level organization and may contain multiple experiments;
- `scientific-project`: broad and accurate, but less explicit about execution;
- `experiment-runtime`: describes execution but not persisted scientific data
  or project structure;
- `scitask`: concise, but rejected because it is a brand-like contraction that
  does not explain the crate's purpose;
- `sciflow`: similarly compact but overemphasizes workflow;
- `scistate`: too narrow once dispatch is added;
- `sciforge`: already used by an existing Rust crate family;
- `sci-task-io`: retains the scope problem being removed.

Registry search did not return an exact `scientific-workflow` package at the
time of this discussion, but crates.io availability and reservation must be
verified again immediately before publication.

## Implementation Review Protocol

### Agreed

- Implementation proceeds exactly one file at a time.
- After creating or changing one implementation file, work stops for project
  owner review.
- The next file is not touched until the project owner approves moving on.
- Each file must contain thorough internal documentation and annotations.
- Documentation must be informative, accurate, and idiomatic under Rust
  conventions rather than comments that merely restate syntax.
- `design.md` remains the living discussion record and is updated as required;
  it is not counted as an implementation file.

### Documentation standard

Every implementation file should use the applicable Rust documentation forms:

- `//!` module- or crate-level documentation describing purpose, invariants,
  ownership, error behavior, and relationships to neighboring modules;
- `///` documentation for public items and behaviorally important internal
  abstractions;
- ordinary comments only for non-obvious implementation reasoning, safety
  constraints, or performance decisions;
- examples where they clarify ownership or error semantics;
- explicit documentation of clone behavior, zero-copy movement guarantees,
  type erasure, and template/layout invariants.

Comments should explain why an implementation exists and what callers may rely
on. They should not narrate self-evident statements line by line.

### Design reference standard

`design.md` is also the exhaustive architectural reference for the Rust
implementation:

- every struct, enum, trait, and other behaviorally significant type receives
  its own explanation;
- every method and trait operation receives its own subsection;
- every method subsection contains a subsection titled exactly `Reference`;
- `Reference` lists every production and test call site known at that revision;
- references use an explicit caller-to-callee mapping;
- indirect calls, such as `Vec::clone` dispatching to `StateValue::clone`, are
  labeled as indirect;
- proposed call sites are labeled `planned` until their implementation exists;
- references are updated whenever a new call site is added, removed, or
  renamed.

Required mapping format:

```text
Caller::method -> Callee::method
```

When a method has no call site yet:

```text
No implemented call sites.
Planned: Caller::method -> Callee::method
```

The source documentation and the design reference have different purposes:
source documentation explains the API locally to Rust readers, while
`design.md` explains how every part participates in the complete system.

### Approved implementation sequence pending review

Only one item is handled per review cycle:

1. `dev/Cargo.toml` — rename the package, declare the library dependencies and
   package metadata, and retain Rust edition 2024.
2. `dev/src/system_state/error.rs` — define the unified error model first so
   every following module can use consistent failures.
3. `dev/src/system_state/value.rs` — implement private cloneable type erasure,
   internal boxing, runtime type information, and downcasting.
4. `dev/src/system_state/spec.rs` — implement JSON template parsing,
   validation, immutable field layout, stable field IDs, and name lookup.
5. `dev/src/system_state/state.rs` — implement `TimePoint`, `SystemState`,
   cloning, empty-state derivation, and concise typed payload operations.
6. `dev/src/system_state.rs` — create the public module facade, declare private
   submodules, and re-export only the intended end-user API.
7. `dev/src/lib.rs` — create the documented library root and expose
   `system_state`.
8. `dev/tests/fixtures/state.json` — add the canonical valid template used by
   public contract tests.
9. `dev/tests/system_state.rs` — add integration tests through the public API.
10. `dev/src/main.rs` — remove the placeholder binary after the library target
    is complete and verified.

This is dependency-oriented: errors precede erased values, erased values
precede state storage, and public facades are created after their internal
contracts are known. The crate may not compile at every intermediate review
point because modern Rust module declarations are deliberately wired only
after the reviewed implementation files exist. Compilation, formatting, Clippy,
and tests become mandatory once the facade and library root are connected.

### Proposed file responsibilities

#### `src/system_state.rs`

Small public facade only:

- declares private submodules;
- re-exports the minimal API;
- contains no substantial implementation.

Proposed public exports:

```rust
pub use error::StateError;
pub use spec::{FieldSpec, StateSpec};
pub use state::{SystemState, TimePoint};
```

`FieldSpec` must be public because the public `StateSpec::fields`,
`StateSpec::get`, and `SystemState::fields` methods return it. `StateValue`,
clone-erasure machinery, compact lookup indices, and internal template
representations remain private.

#### `src/system_state/spec.rs`

Owns the immutable template-derived layout:

- `StateSpec`;
- JSON template deserialization;
- field-name and stable type-tag validation;
- duplicate-name detection;
- deterministic template order and field IDs;
- name-to-slot lookup;
- creation of empty states.

#### `src/system_state/state.rs`

Owns the end-user state behavior:

- `SystemState`;
- `TimePoint`;
- `Clone`;
- `empty(time)`;
- concise `set`, `get`, `get_mut`, `take`, `has`, and `is` methods;
- key/value iteration and clearing;
- move-based ownership semantics.

#### `src/system_state/value.rs`

Owns the private type-erasure boundary:

- cloneable erased-value trait;
- internal boxing;
- type name and `TypeId`;
- borrowed and consuming downcasts;
- deep cloning for explicit `SystemState::clone()`.

Keeping this isolated makes the most subtle generic/object-safe code auditable
without exposing boxing to users.

### Downstream access to stored values

#### Agreed boundary

Downstream application code does not import or call `value.rs`. The module is
declared privately by the `system_state` facade:

```rust
// src/system_state.rs
mod value;
mod state;

pub use state::{SystemState, TimePoint};
```

The sibling state implementation uses the erased wrapper internally:

```rust
// src/system_state/state.rs
use super::value::StateValue;
```

External crates use only typed `SystemState` methods:

```rust
use scientific_workflow::system_state::SystemState;

state.set("space", lattice)?;
let space = state.get::<Lattice>("space")?;
let space = state.take::<Lattice>("space")?;
```

The flow is:

```text
downstream T
   -> SystemState::set<T>
   -> private StateValue::new<T>
   -> state slot

state slot
   -> private StateValue downcast
   -> SystemState::get<T> / get_mut<T> / take<T>
   -> downstream T
```

Future storage and serialization code should also avoid depending broadly on
the private erased-value implementation. It should consume a narrow
crate-internal visitor or encoding interface supplied by `SystemState`. This
keeps storage formats from becoming coupled to the concrete boxing strategy.

The mirrored `tests/system_state/value.rs` file is an implementation test, not
an example of downstream usage. It includes the private source directly only
while the public `SystemState` facade is not yet implemented.

### Concrete calls into `value.rs`

The future `state.rs` implementation owns the call sites. The following
examples are representative of the intended implementation.

#### `set` calls `StateValue::new`

```rust
pub fn set<T>(&mut self, key: &str, payload: T) -> Result<(), StateError>
where
    T: Any + Clone + Send,
{
    let index = self.spec.index_of(key)?;
    self.values[index] = Some(StateValue::new(payload));
    Ok(())
}
```

`payload` moves into `StateValue::new`; no clone occurs.

#### `get` calls `downcast_ref`

```rust
pub fn get<T>(&self, key: &str) -> Result<&T, StateError>
where
    T: Any,
{
    let value = self.value(key)?;
    let actual = value.type_name();

    value
        .downcast_ref::<T>()
        .ok_or_else(|| StateError::TypeMismatch {
            field: key.to_owned(),
            expected: type_name::<T>(),
            actual,
        })
}
```

The returned reference points into the original boxed payload.

#### `get_mut` calls `downcast_mut`

```rust
pub fn get_mut<T>(&mut self, key: &str) -> Result<&mut T, StateError>
where
    T: Any,
{
    let value = self.value_mut(key)?;
    let actual = value.type_name();

    value
        .downcast_mut::<T>()
        .ok_or_else(|| StateError::TypeMismatch {
            field: key.to_owned(),
            expected: type_name::<T>(),
            actual,
        })
}
```

The returned mutable reference modifies the original payload in place.

#### `is` calls `StateValue::is`

```rust
pub fn is<T>(&self, key: &str) -> Result<bool, StateError>
where
    T: Any,
{
    let index = self.spec.index_of(key)?;
    Ok(self.values[index]
        .as_ref()
        .is_some_and(StateValue::is::<T>))
}
```

An empty declared field returns `false`; an undeclared key returns
`StateError::UnknownField`.

#### `take` calls consuming `downcast`

```rust
pub fn take<T>(&mut self, key: &str) -> Result<T, StateError>
where
    T: Any + Send,
{
    let index = self.spec.index_of(key)?;
    let value = self.values[index]
        .take()
        .ok_or_else(|| StateError::MissingValue {
            field: key.to_owned(),
        })?;
    let actual = value.type_name();

    match value.downcast::<T>() {
        Ok(payload) => Ok(payload),
        Err(value) => {
            // A failed typed extraction must not discard the payload.
            self.values[index] = Some(value);
            Err(StateError::TypeMismatch {
                field: key.to_owned(),
                expected: type_name::<T>(),
                actual,
            })
        }
    }
}
```

On success, `take` moves `T` out and leaves the slot empty. On mismatch, the
unchanged `StateValue` is restored before returning the error.

#### `SystemState::clone` calls `StateValue::clone`

```rust
impl Clone for SystemState {
    fn clone(&self) -> Self {
        Self {
            spec: self.spec.clone(),
            time: self.time,
            values: self.values.clone(),
        }
    }
}
```

Cloning the `Vec<Option<StateValue>>` calls `StateValue::clone` for every filled
slot. The immutable spec is shared, while every payload is deep-cloned.

The mapping is therefore:

```text
SystemState::set      -> StateValue::new
SystemState::get      -> StateValue::type_name + downcast_ref
SystemState::get_mut  -> StateValue::type_name + downcast_mut
SystemState::is       -> StateValue::is
SystemState::take     -> StateValue::type_name + downcast
SystemState::clone    -> StateValue::clone
```

### Implemented Rust API reference: erased values

This section is exhaustive for `src/system_state/value.rs` at the current
revision.

#### Struct: `StateValue`

`StateValue` is the crate-private, owned, type-erased payload stored in each
filled SystemState slot.

```text
StateValue
└── inner: Box<dyn ErasedValue>
```

The box gives heterogeneous concrete values a uniform representation.
`StateValue` is `Send` because `ErasedValue` requires `Send`, but it does not
require `Sync`. It is not part of the end-user API.

#### Trait: `ErasedValue`

`ErasedValue` is the private object-safe trait implemented for every
`T: Any + Clone + Send`. It provides the operations that cannot be invoked
directly through `Any` after type erasure.

#### Method: `ErasedValue::clone_box`

Deep-clones the concrete payload and returns a newly allocated erased owner.
This is the object-safe foundation for `StateValue::clone`.

##### Reference

```text
StateValue::clone -> ErasedValue::clone_box
```

#### Method: `ErasedValue::as_any`

Returns an immutable `Any` view of the concrete payload. It neither allocates
nor changes ownership.

##### Reference

```text
StateValue::type_id      -> ErasedValue::as_any
StateValue::downcast_ref -> ErasedValue::as_any
```

#### Method: `ErasedValue::as_any_mut`

Returns a mutable `Any` view of the concrete payload. The exclusive borrow of
`StateValue` guarantees exclusive access to the payload.

##### Reference

```text
StateValue::downcast_mut -> ErasedValue::as_any_mut
```

#### Method: `ErasedValue::into_any`

Consumes the erased box and converts it into `Box<dyn Any + Send>` so the
standard library can perform an owned downcast.

##### Reference

```text
StateValue::downcast -> ErasedValue::into_any
```

#### Method: `ErasedValue::concrete_type_name`

Returns `std::any::type_name::<T>()` for diagnostics. The returned spelling is
not a stable serialization tag.

##### Reference

```text
StateValue::type_name -> ErasedValue::concrete_type_name
```

#### Method: `StateValue::new`

Consumes a concrete `T: Any + Clone + Send` and stores it behind the internal
erased box. It never invokes `Clone`.

##### Reference

```text
SystemState::set -> StateValue::new

tests/system_state/value.rs::borrowed_downcasts_access_the_original_payload
    -> StateValue::new
tests/system_state/value.rs::clone_deep_clones_the_concrete_payload
    -> StateValue::new
tests/system_state/value.rs::consuming_downcast_preserves_a_vec_backing_allocation
    -> StateValue::new
tests/system_state/value.rs::failed_consuming_downcast_returns_the_original_value
    -> StateValue::new
tests/system_state/value.rs::debug_output_describes_the_type_without_printing_the_payload
    -> StateValue::new
```

#### Method: `StateValue::type_id`

Returns the runtime `TypeId` of the stored concrete payload.

##### Reference

```text
StateValue::is -> StateValue::type_id
```

#### Method: `StateValue::type_name`

Returns the fully qualified Rust type name of the stored concrete payload for
error reporting and bounded debug output.

##### Reference

```text
StateValue::fmt -> StateValue::type_name

SystemState::get     -> StateValue::type_name
SystemState::get_mut -> StateValue::type_name
SystemState::take    -> StateValue::type_name
```

#### Method: `StateValue::is`

Compares the stored `TypeId` with `TypeId::of::<T>()`.

##### Reference

```text
StateValue::downcast -> StateValue::is

SystemState::is -> StateValue::is

tests/system_state/value.rs::borrowed_downcasts_access_the_original_payload
    -> StateValue::is
tests/system_state/value.rs::failed_consuming_downcast_returns_the_original_value
    -> StateValue::is
```

#### Method: `StateValue::downcast_ref`

Returns `Some(&T)` when the requested type matches and `None` otherwise. The
reference points into the original stored allocation.

##### Reference

```text
SystemState::get -> StateValue::downcast_ref

tests/system_state/value.rs::borrowed_downcasts_access_the_original_payload
    -> StateValue::downcast_ref
tests/system_state/value.rs::clone_deep_clones_the_concrete_payload
    -> StateValue::downcast_ref
tests/system_state/value.rs::failed_consuming_downcast_returns_the_original_value
    -> StateValue::downcast_ref
```

#### Method: `StateValue::downcast_mut`

Returns `Some(&mut T)` when the requested type matches and `None` otherwise.
Mutation occurs in place.

##### Reference

```text
SystemState::get_mut -> StateValue::downcast_mut

tests/system_state/value.rs::borrowed_downcasts_access_the_original_payload
    -> StateValue::downcast_mut
tests/system_state/value.rs::clone_deep_clones_the_concrete_payload
    -> StateValue::downcast_mut
```

#### Method: `StateValue::downcast`

Consumes `StateValue`. A matching type returns the owned `T` without cloning;
a mismatch returns the original `StateValue` unchanged.

##### Reference

```text
SystemState::take -> StateValue::downcast

tests/system_state/value.rs::consuming_downcast_preserves_a_vec_backing_allocation
    -> StateValue::downcast
tests/system_state/value.rs::failed_consuming_downcast_returns_the_original_value
    -> StateValue::downcast
```

#### Method: `StateValue::clone`

Calls the concrete payload's `Clone` implementation through
`ErasedValue::clone_box`. The clone is independent of the original payload.

##### Reference

```text
Indirect:
SystemState::clone
    -> Vec<Option<StateValue>>::clone
    -> Option<StateValue>::clone
    -> StateValue::clone

tests/system_state/value.rs::clone_deep_clones_the_concrete_payload
    -> StateValue::clone
```

#### Method: `StateValue::fmt`

Implements `Debug` using only the wrapper name and concrete type name. It does
not recursively format potentially enormous scientific payloads.

##### Reference

```text
tests/system_state/value.rs::debug_output_describes_the_type_without_printing_the_payload
    -> format!("{value:?}")
    -> StateValue::fmt

tests/system_state/value.rs::failed_consuming_downcast_returns_the_original_value
    -> Result::expect_err
    -> StateValue::fmt (available to the assertion failure path)
```

### Implemented Rust API reference: state specifications

This section is exhaustive for `src/system_state/spec.rs` at the current
revision.

#### Struct: `FieldSpec`

`FieldSpec` describes one immutable declared state field:

```text
FieldSpec
├── index: usize
├── name: Box<str>
└── type_tag: Box<str>
```

The index is assigned by JSON array order. The name is the dictionary key. The
type tag is a stable serialization/codec identifier rather than a Rust type
name.

#### Method: `FieldSpec::new`

Constructs one private field specification and trims its name and type tag.
Semantic validation occurs in `StateSpec::from_template` before this constructor
is called.

##### Reference

```text
StateSpec::from_template -> FieldSpec::new
```

#### Method: `FieldSpec::index`

Returns the field's zero-based payload-slot index.

##### Reference

```text
tests/system_state/spec.rs::loads_and_round_trips_an_actual_example_template
    -> FieldSpec::index
tests/system_state.rs::tensor_state_round_trip_integrates_public_modules
    -> FieldSpec::index

Planned: downstream field inspection -> FieldSpec::index
```

#### Method: `FieldSpec::name`

Returns the normalized field name.

##### Reference

```text
tests/system_state/spec.rs::loads_and_round_trips_an_actual_example_template
    -> FieldSpec::name
tests/system_state.rs::tensor_state_round_trip_integrates_public_modules
    -> FieldSpec::name

Planned: downstream field inspection -> FieldSpec::name
```

#### Method: `FieldSpec::type_tag`

Returns the stable codec tag declared by the JSON template.

##### Reference

```text
tests/system_state/spec.rs::loads_and_round_trips_an_actual_example_template
    -> FieldSpec::type_tag
tests/system_state.rs::tensor_state_round_trip_integrates_public_modules
    -> FieldSpec::type_tag

Planned: storage codec resolution -> FieldSpec::type_tag
Planned: downstream field inspection -> FieldSpec::type_tag
```

#### Method: `FieldSpec::serialize`

Compiler-derived Serde serialization writes only `name` and `type`; the
runtime-only `index` field is skipped. `type_tag` is emitted under the JSON key
`type`.

##### Reference

```text
StateSpec::to_json
    -> serde_json::to_string_pretty
    -> StateTemplateRef::serialize
    -> FieldSpec::serialize
```

#### Struct: `StateSpec`

`StateSpec` is the public, cheaply cloneable handle to one immutable state
layout:

```text
StateSpec
└── inner: Arc<StateLayout>
```

Every state derived from the same specification shares its field definitions,
name lookup table, and source path.

#### Method: `StateSpec::load`

Reads a JSON template as bytes, deserializes `StateTemplate`, validates its
semantics through `from_template`, and returns a shared specification.

##### Reference

```text
tests/system_state/spec.rs::loads_and_round_trips_an_actual_example_template
    -> StateSpec::load
tests/system_state.rs::tensor_state_round_trip_integrates_public_modules
    -> StateSpec::load

Planned: application startup -> StateSpec::load
```

#### Method: `StateSpec::empty`

Creates a SystemState with the same shared specification, the supplied time,
and one empty payload slot per declared field.

##### Reference

```text
StateSpec::empty -> StateSpec::clone
StateSpec::empty -> SystemState::new
tests/system_state/spec.rs::loads_and_round_trips_an_actual_example_template
    -> StateSpec::empty
tests/system_state.rs::tensor_state_round_trip_integrates_public_modules
    -> StateSpec::empty

Planned: application initialization -> StateSpec::empty
```

#### Method: `StateSpec::to_json`

Serializes the validated specification into a pretty-printed JSON string with
the same strict `fields` shape accepted by `StateSpec::load`. It excludes the
source path and runtime field indices. It borrows field names and type tags
without cloning them.

##### Reference

```text
StateSpec::to_json -> StateSpec::fields
StateSpec::to_json -> serde_json::to_string_pretty

tests/system_state/spec.rs::loads_and_round_trips_an_actual_example_template
    -> StateSpec::to_json
tests/system_state.rs::tensor_state_round_trip_integrates_public_modules
    -> StateSpec::to_json
```

#### Method: `StateSpec::source`

Returns the original, non-canonicalized JSON template path retained by the
shared layout.

##### Reference

```text
tests/system_state/spec.rs::loads_and_round_trips_an_actual_example_template
    -> StateSpec::source
tests/system_state.rs::tensor_state_round_trip_integrates_public_modules
    -> StateSpec::source

Planned: downstream provenance inspection -> StateSpec::source
```

#### Method: `StateSpec::fields`

Returns all field specifications as a borrowed slice in deterministic template
order.

##### Reference

```text
StateSpec::to_json -> StateSpec::fields

tests/system_state/spec.rs::loads_and_round_trips_an_actual_example_template
    -> StateSpec::fields
tests/system_state.rs::tensor_state_round_trip_integrates_public_modules
    -> StateSpec::fields

Planned: downstream schema inspection -> StateSpec::fields
```

#### Method: `StateSpec::len`

Returns the number of declared fields.

##### Reference

```text
tests/system_state/spec.rs::loads_and_round_trips_an_actual_example_template
    -> StateSpec::len
tests/system_state.rs::tensor_state_round_trip_integrates_public_modules
    -> StateSpec::len

SystemState::new -> StateSpec::len
Planned: downstream schema inspection -> StateSpec::len
```

#### Method: `StateSpec::is_empty`

Reports whether the JSON template declares zero fields. Empty templates are
currently valid.

##### Reference

```text
tests/system_state/spec.rs::loads_and_round_trips_an_actual_example_template
    -> StateSpec::is_empty
tests/system_state.rs::tensor_state_round_trip_integrates_public_modules
    -> StateSpec::is_empty

Planned: downstream schema inspection -> StateSpec::is_empty
```

#### Method: `StateSpec::get`

Resolves a normalized field name and returns its `FieldSpec`, or `None` when
the name is undeclared.

##### Reference

```text
tests/system_state/spec.rs::loads_and_round_trips_an_actual_example_template
    -> StateSpec::get
tests/system_state.rs::tensor_state_round_trip_integrates_public_modules
    -> StateSpec::get

Planned: downstream schema inspection -> StateSpec::get
```

#### Method: `StateSpec::contains`

Checks whether the name lookup table contains a declared field.

##### Reference

```text
tests/system_state/spec.rs::loads_and_round_trips_an_actual_example_template
    -> StateSpec::contains
tests/system_state.rs::tensor_state_round_trip_integrates_public_modules
    -> StateSpec::contains

Planned: downstream schema inspection -> StateSpec::contains
```

#### Method: `StateSpec::index_of`

Resolves a field name to its compact slot index. It is crate-private and
returns `StateError::UnknownField` rather than exposing missing-index handling
to each SystemState method.

##### Reference

```text
SystemState::set     -> StateSpec::index_of
SystemState::get     -> StateSpec::index_of
SystemState::get_mut -> StateSpec::index_of
SystemState::take    -> StateSpec::index_of
SystemState::has     -> StateSpec::index_of
SystemState::is      -> StateSpec::index_of
```

#### Method: `StateSpec::from_template`

Validates normalized names and type tags, rejects duplicate names, assigns
field indices, and constructs the immutable `StateLayout`.

##### Reference

```text
StateSpec::load -> StateSpec::from_template
StateSpec::from_template -> FieldSpec::new
StateSpec::from_template -> StateLayout construction
```

#### Method: `StateSpec::clone`

Compiler-derived `Clone` increments the `Arc<StateLayout>` reference count. It
does not duplicate field definitions, strings, paths, or lookup tables.

##### Reference

```text
StateSpec::empty -> StateSpec::clone

SystemState::empty -> StateSpec::clone or owned StateSpec move
SystemState::clone -> StateSpec::clone
```

#### Struct: `StateLayout`

`StateLayout` is the private immutable allocation shared by every `StateSpec`
clone:

```text
StateLayout
├── source: PathBuf
├── fields: Vec<FieldSpec>
└── by_name: HashMap<Box<str>, usize>
```

It has no methods. `StateSpec` is its only access boundary.

#### Struct: `StateTemplate`

`StateTemplate` is the private Serde representation of the top-level JSON
object. It contains the ordered `Vec<FieldDeclaration>` and rejects unknown
properties.

It has no methods.

#### Struct: `FieldDeclaration`

`FieldDeclaration` is the private Serde representation of one JSON field. Its
`type` JSON property is deserialized into `type_tag` because `type` is a Rust
keyword.

It has no methods.

#### Struct: `StateTemplateRef`

`StateTemplateRef` is the private borrowed serialization view used by
`StateSpec::to_json`:

```text
StateTemplateRef<'a>
└── fields: &'a [FieldSpec]
```

It prevents conversion back to JSON from allocating owned copies of field names
and type tags.

#### Method: `StateTemplateRef::serialize`

Compiler-derived Serde serialization emits the top-level `fields` object.

##### Reference

```text
StateSpec::to_json
    -> serde_json::to_string_pretty
    -> StateTemplateRef::serialize
```

#### Test-only struct: `tests/system_state/spec.rs::SystemState`

This narrow specification-test boundary owns the `StateSpec` received from
`StateSpec::empty`. It is not a behavioral substitute for the production
`SystemState`; it lets a single-source specification test verify construction
and shared layout ownership without importing the state implementation.

#### Test-only method: `tests/system_state/spec.rs::SystemState::new`

Accepts the cheaply cloned production `StateSpec` and placeholder `TimePoint`,
then retains the specification in the test boundary.

##### Reference

```text
StateSpec::empty -> tests/system_state/spec.rs::SystemState::new
```

#### Test-only method: `tests/system_state/spec.rs::SystemState::spec`

Returns the retained specification so the test can compare its field-slice
address with the original specification.

##### Reference

```text
tests/system_state/spec.rs::loads_and_round_trips_an_actual_example_template
    -> tests/system_state/spec.rs::SystemState::spec
```

#### Test-only struct: `tests/system_state/spec.rs::TimePoint`

This zero-sized placeholder satisfies the argument type expected by
`StateSpec::empty` in this isolated specification test. It has no methods.

##### Reference

```text
tests/system_state/spec.rs::loads_and_round_trips_an_actual_example_template
    -> tests/system_state/spec.rs::TimePoint construction
```

### Implemented Rust API reference: system states

This section is exhaustive for `src/system_state/state.rs` at the current
revision.

#### Struct: `TimePoint`

`TimePoint` identifies one state on the SSTS time axis:

```text
TimePoint
├── index: u64
└── physical: Option<f64>
```

The mandatory index provides deterministic ordering. The optional physical
coordinate is finite when present. Time units belong to SSTS metadata.

#### Method: `TimePoint::new`

Constructs an index-only time point with no physical coordinate.

##### Reference

```text
No implemented call sites.
Planned: downstream state creation -> TimePoint::new
tests/system_state/state.rs::blank_state -> TimePoint::new
tests/system_state/state.rs::time_points_preserve_valid_coordinates_and_reject_non_finite_values
    -> TimePoint::new
tests/system_state/state.rs::blank_state_has_fixed_shape_and_deterministic_fields
    -> TimePoint::new
tests/system_state/state.rs::derived_empty_state_shares_layout_without_cloning_payloads
    -> TimePoint::new
tests/system_state.rs::tensor_state_round_trip_integrates_public_modules
    -> TimePoint::new
```

#### Method: `TimePoint::from_physical`

Returns a time point for a finite physical coordinate and `None` for `NaN` or
infinity.

##### Reference

```text
No implemented call sites.
Planned: downstream state creation -> TimePoint::from_physical
tests/system_state/state.rs::time_points_preserve_valid_coordinates_and_reject_non_finite_values
    -> TimePoint::from_physical
tests/system_state.rs::tensor_state_round_trip_integrates_public_modules
    -> TimePoint::from_physical
```

#### Method: `TimePoint::index`

Returns the mandatory deterministic index.

##### Reference

```text
No implemented call sites.
Planned: SSTS ordering and chunk indexing -> TimePoint::index
tests/system_state/state.rs::time_points_preserve_valid_coordinates_and_reject_non_finite_values
    -> TimePoint::index
tests/system_state.rs::tensor_state_round_trip_integrates_public_modules
    -> TimePoint::index
```

#### Method: `TimePoint::physical`

Returns the optional finite physical coordinate.

##### Reference

```text
No implemented call sites.
Planned: downstream time inspection -> TimePoint::physical
tests/system_state/state.rs::time_points_preserve_valid_coordinates_and_reject_non_finite_values
    -> TimePoint::physical
tests/system_state.rs::tensor_state_round_trip_integrates_public_modules
    -> TimePoint::physical
```

#### Struct: `SystemState`

`SystemState` owns the payloads describing one time point:

```text
SystemState
├── spec: StateSpec
├── time: TimePoint
└── values: Vec<Option<StateValue>>
```

`StateSpec` cheaply shares immutable layout metadata. `values` always has one
slot per declared field. Filled slots uniquely own their erased payloads.

#### Method: `SystemState::new`

Crate-private constructor that consumes a validated `StateSpec`, records the
time, and allocates the exact number of empty payload slots.

##### Reference

```text
SystemState::empty -> SystemState::new
tests/system_state/state.rs::blank_state -> SystemState::new
StateSpec::empty -> SystemState::new
```

#### Method: `SystemState::empty`

Creates another blank state at a supplied time by cheaply cloning the
specification and allocating empty slots. It never clones payloads.

##### Reference

```text
SystemState::empty -> StateSpec::clone
SystemState::empty -> SystemState::new

No implemented external call sites.
Planned: downstream simulation loop -> SystemState::empty
tests/system_state/state.rs::derived_empty_state_shares_layout_without_cloning_payloads
    -> SystemState::empty
tests/system_state.rs::tensor_state_round_trip_integrates_public_modules
    -> SystemState::empty
```

#### Method: `SystemState::time`

Returns the copyable `TimePoint`.

##### Reference

```text
No implemented call sites.
Planned: SSTS insertion -> SystemState::time
tests/system_state/state.rs::blank_state_has_fixed_shape_and_deterministic_fields
    -> SystemState::time
tests/system_state/state.rs::explicit_clone_deep_clones_payloads_but_shares_the_layout
    -> SystemState::time
tests/system_state/state.rs::derived_empty_state_shares_layout_without_cloning_payloads
    -> SystemState::time
tests/system_state.rs::tensor_state_round_trip_integrates_public_modules
    -> SystemState::time
```

#### Method: `SystemState::spec`

Returns a borrowed reference to the shared immutable specification.

##### Reference

```text
No implemented call sites.
Planned: storage/schema inspection -> SystemState::spec
tests/system_state/state.rs::explicit_clone_deep_clones_payloads_but_shares_the_layout
    -> SystemState::spec
tests/system_state/state.rs::derived_empty_state_shares_layout_without_cloning_payloads
    -> SystemState::spec
tests/system_state.rs::tensor_state_round_trip_integrates_public_modules
    -> SystemState::spec
```

#### Method: `SystemState::len`

Returns the structural slot count, including empty slots.

##### Reference

```text
SystemState::fmt -> SystemState::len

Planned: downstream state inspection -> SystemState::len
tests/system_state/state.rs::blank_state_has_fixed_shape_and_deterministic_fields
    -> SystemState::len
tests/system_state/state.rs::derived_empty_state_shares_layout_without_cloning_payloads
    -> SystemState::len
tests/system_state/state.rs::clear_operations_drop_payloads_without_changing_shape
    -> SystemState::len
tests/system_state.rs::tensor_state_round_trip_integrates_public_modules
    -> SystemState::len
```

#### Method: `SystemState::is_empty`

Reports whether the specification declares zero fields. It is consistent with
`len`; it does not mean that all payload slots are empty.

##### Reference

```text
No implemented call sites.
Planned: downstream state inspection -> SystemState::is_empty
tests/system_state/state.rs::blank_state_has_fixed_shape_and_deterministic_fields
    -> SystemState::is_empty
```

#### Method: `SystemState::loaded`

Counts currently populated slots.

##### Reference

```text
SystemState::fmt -> SystemState::loaded

Planned: downstream state inspection -> SystemState::loaded
tests/system_state/state.rs::blank_state_has_fixed_shape_and_deterministic_fields
    -> SystemState::loaded
tests/system_state/state.rs::set_mutate_and_take_transfer_payload_without_cloning
    -> SystemState::loaded
tests/system_state/state.rs::derived_empty_state_shares_layout_without_cloning_payloads
    -> SystemState::loaded
tests/system_state/state.rs::clear_operations_drop_payloads_without_changing_shape
    -> SystemState::loaded
tests/system_state.rs::tensor_state_round_trip_integrates_public_modules
    -> SystemState::loaded
```

#### Method: `SystemState::is_blank`

Reports whether every payload slot is empty.

##### Reference

```text
No implemented call sites.
Planned: downstream state inspection -> SystemState::is_blank
tests/system_state/state.rs::blank_state_has_fixed_shape_and_deterministic_fields
    -> SystemState::is_blank
tests/system_state/state.rs::set_mutate_and_take_transfer_payload_without_cloning
    -> SystemState::is_blank
tests/system_state/state.rs::derived_empty_state_shares_layout_without_cloning_payloads
    -> SystemState::is_blank
tests/system_state/state.rs::clear_operations_drop_payloads_without_changing_shape
    -> SystemState::is_blank
tests/system_state.rs::tensor_state_round_trip_integrates_public_modules
    -> SystemState::is_blank
```

#### Method: `SystemState::fields`

Returns field specifications in deterministic template order.

##### Reference

```text
SystemState::fields -> StateSpec::fields

No implemented external call sites.
Planned: downstream schema inspection -> SystemState::fields
tests/system_state/state.rs::blank_state_has_fixed_shape_and_deterministic_fields
    -> SystemState::fields
tests/system_state.rs::tensor_state_round_trip_integrates_public_modules
    -> SystemState::fields
```

#### Method: `SystemState::has`

Reports whether one declared field currently contains a payload.

##### Reference

```text
SystemState::has -> StateSpec::index_of

No implemented external call sites.
Planned: downstream state access -> SystemState::has
tests/system_state/state.rs::blank_state_has_fixed_shape_and_deterministic_fields
    -> SystemState::has
tests/system_state/state.rs::set_mutate_and_take_transfer_payload_without_cloning
    -> SystemState::has
tests/system_state.rs::tensor_state_round_trip_integrates_public_modules
    -> SystemState::has
```

#### Method: `SystemState::is`

Reports whether a filled field stores the exact Rust type `T`; empty fields
return `false`.

##### Reference

```text
SystemState::is -> StateSpec::index_of
SystemState::is -> StateValue::is

No implemented external call sites.
Planned: downstream typed inspection -> SystemState::is
tests/system_state/state.rs::set_mutate_and_take_transfer_payload_without_cloning
    -> SystemState::is
tests/system_state.rs::tensor_state_round_trip_integrates_public_modules
    -> SystemState::is
```

#### Method: `SystemState::set`

Consumes `T: Any + Clone + Send`, erases it, and fills or replaces a declared
slot without cloning.

##### Reference

```text
SystemState::set -> StateSpec::index_of
SystemState::set -> StateValue::new

No implemented external call sites.
Planned: downstream simulation output -> SystemState::set
tests/system_state/state.rs::set_mutate_and_take_transfer_payload_without_cloning
    -> SystemState::set
tests/system_state/state.rs::failed_take_restores_the_original_payload
    -> SystemState::set
tests/system_state/state.rs::explicit_clone_deep_clones_payloads_but_shares_the_layout
    -> SystemState::set
tests/system_state/state.rs::derived_empty_state_shares_layout_without_cloning_payloads
    -> SystemState::set
tests/system_state/state.rs::access_errors_distinguish_unknown_missing_and_mismatched_fields
    -> SystemState::set
tests/system_state/state.rs::clear_operations_drop_payloads_without_changing_shape
    -> SystemState::set
tests/system_state/state.rs::debug_output_reports_structure_without_formatting_payloads
    -> SystemState::set
tests/system_state.rs::tensor_state_round_trip_integrates_public_modules
    -> SystemState::set
```

#### Method: `SystemState::get`

Borrows a filled slot as `&T`, reporting unknown, missing, and mismatched
fields distinctly.

##### Reference

```text
SystemState::get -> SystemState::value
SystemState::get -> StateValue::type_name
SystemState::get -> StateValue::downcast_ref

No implemented external call sites.
Planned: downstream state access -> SystemState::get
tests/system_state/state.rs::set_mutate_and_take_transfer_payload_without_cloning
    -> SystemState::get
tests/system_state/state.rs::failed_take_restores_the_original_payload
    -> SystemState::get
tests/system_state/state.rs::explicit_clone_deep_clones_payloads_but_shares_the_layout
    -> SystemState::get
tests/system_state/state.rs::access_errors_distinguish_unknown_missing_and_mismatched_fields
    -> SystemState::get
tests/system_state.rs::tensor_state_round_trip_integrates_public_modules
    -> SystemState::get
```

#### Method: `SystemState::get_mut`

Borrows a filled slot as `&mut T` and mutates the original payload in place.

##### Reference

```text
SystemState::get_mut -> SystemState::value_mut
SystemState::get_mut -> StateValue::type_name
SystemState::get_mut -> StateValue::downcast_mut

No implemented external call sites.
Planned: downstream state mutation -> SystemState::get_mut
tests/system_state/state.rs::set_mutate_and_take_transfer_payload_without_cloning
    -> SystemState::get_mut
tests/system_state/state.rs::explicit_clone_deep_clones_payloads_but_shares_the_layout
    -> SystemState::get_mut
tests/system_state.rs::tensor_state_round_trip_integrates_public_modules
    -> SystemState::get_mut
```

#### Method: `SystemState::take`

Removes a filled `StateValue` and consumes its owned downcast. Success returns
the original `T`; mismatch restores the unchanged erased value before returning
`StateError::TypeMismatch`.

##### Reference

```text
SystemState::take -> StateSpec::index_of
SystemState::take -> StateValue::type_name
SystemState::take -> StateValue::downcast

No implemented external call sites.
Planned: downstream payload offload -> SystemState::take
tests/system_state/state.rs::set_mutate_and_take_transfer_payload_without_cloning
    -> SystemState::take
tests/system_state/state.rs::failed_take_restores_the_original_payload
    -> SystemState::take
tests/system_state.rs::tensor_state_round_trip_integrates_public_modules
    -> SystemState::take
```

#### Method: `SystemState::clear`

Drops one filled payload and returns whether the declared slot was populated.

##### Reference

```text
SystemState::clear -> StateSpec::index_of

No implemented external call sites.
Planned: downstream state reset -> SystemState::clear
tests/system_state/state.rs::clear_operations_drop_payloads_without_changing_shape
    -> SystemState::clear
```

#### Method: `SystemState::clear_all`

Drops all payloads while preserving the specification and time.

##### Reference

```text
No implemented call sites.
Planned: downstream state reset -> SystemState::clear_all
tests/system_state/state.rs::clear_operations_drop_payloads_without_changing_shape
    -> SystemState::clear_all
```

#### Method: `SystemState::value`

Private helper that resolves a key and returns its filled erased value.

##### Reference

```text
SystemState::get -> SystemState::value
SystemState::value -> StateSpec::index_of
```

#### Method: `SystemState::value_mut`

Private helper that resolves a key and returns its filled mutable erased value.

##### Reference

```text
SystemState::get_mut -> SystemState::value_mut
SystemState::value_mut -> StateSpec::index_of
```

#### Method: `SystemState::clone`

Cheaply clones `StateSpec`, copies `TimePoint`, and deep-clones every populated
`StateValue`.

##### Reference

```text
SystemState::clone -> StateSpec::clone
SystemState::clone
    -> Vec<Option<StateValue>>::clone
    -> Option<StateValue>::clone
    -> StateValue::clone

No implemented external call sites.
Planned: downstream state branching -> SystemState::clone
tests/system_state/state.rs::explicit_clone_deep_clones_payloads_but_shares_the_layout
    -> SystemState::clone
tests/system_state.rs::tensor_state_round_trip_integrates_public_modules
    -> SystemState::clone
```

#### Method: `SystemState::fmt`

Implements bounded `Debug` output containing time, template source, field
count, and loaded count without formatting payload contents.

##### Reference

```text
SystemState::fmt -> StateSpec::source
SystemState::fmt -> SystemState::len
SystemState::fmt -> SystemState::loaded

No implemented external call sites.
Planned: diagnostics and assertion failure paths -> SystemState::fmt
tests/system_state/state.rs::debug_output_reports_structure_without_formatting_payloads
    -> SystemState::fmt
```

### State contract-test support reference

This section records the test-only support types in
`tests/system_state/state.rs`. They exist solely to isolate `state.rs` before
the production module facade is connected.

#### Test-only struct: `tests/system_state/state.rs::FieldSpec`

The stub stores one static field name. It supplies only the metadata operation
that the state contract tests inspect.

#### Method: `tests/system_state/state.rs::FieldSpec::name`

Returns the declared test field name.

##### Reference

```text
tests/system_state/state.rs::blank_state_has_fixed_shape_and_deterministic_fields
    -> tests/system_state/state.rs::FieldSpec::name
```

#### Test-only struct: `tests/system_state/state.rs::Layout`

The stub layout owns a synthetic source path and an ordered field vector behind
an `Arc`. It has no methods.

#### Test-only struct: `tests/system_state/state.rs::StateSpec`

The stub is a cheaply cloneable `Arc<Layout>` handle that reproduces only the
sharing and lookup behavior required by the real `SystemState`.

#### Method: `tests/system_state/state.rs::StateSpec::fixture`

Constructs the deterministic `population`, `space`, and `status` test layout.

##### Reference

```text
tests/system_state/state.rs::blank_state
    -> tests/system_state/state.rs::StateSpec::fixture
```

#### Method: `tests/system_state/state.rs::StateSpec::len`

Returns the number of fields in the test layout.

##### Reference

```text
SystemState::new -> tests/system_state/state.rs::StateSpec::len
```

#### Method: `tests/system_state/state.rs::StateSpec::fields`

Returns the ordered test field slice.

##### Reference

```text
SystemState::fields -> tests/system_state/state.rs::StateSpec::fields
```

#### Method: `tests/system_state/state.rs::StateSpec::source`

Returns the synthetic provenance path used by structural debug output.

##### Reference

```text
SystemState::fmt -> tests/system_state/state.rs::StateSpec::source
```

#### Method: `tests/system_state/state.rs::StateSpec::index_of`

Resolves a declared test name or returns `StateError::UnknownField`.

##### Reference

```text
SystemState::has -> tests/system_state/state.rs::StateSpec::index_of
SystemState::is -> tests/system_state/state.rs::StateSpec::index_of
SystemState::set -> tests/system_state/state.rs::StateSpec::index_of
SystemState::take -> tests/system_state/state.rs::StateSpec::index_of
SystemState::clear -> tests/system_state/state.rs::StateSpec::index_of
SystemState::value -> tests/system_state/state.rs::StateSpec::index_of
SystemState::value_mut -> tests/system_state/state.rs::StateSpec::index_of
```

#### Method: `tests/system_state/state.rs::StateSpec::shares_layout_with`

Uses `Arc::ptr_eq` to verify that derived and cloned states share the exact
immutable layout allocation.

##### Reference

```text
tests/system_state/state.rs::explicit_clone_deep_clones_payloads_but_shares_the_layout
    -> tests/system_state/state.rs::StateSpec::shares_layout_with
tests/system_state/state.rs::derived_empty_state_shares_layout_without_cloning_payloads
    -> tests/system_state/state.rs::StateSpec::shares_layout_with
```

#### Test-only struct: `tests/system_state/state.rs::CloneTracked`

The ownership-test payload contains a `Vec<u64>` and a shared atomic clone
counter. Its manual `Clone` implementation makes every payload clone
observable.

#### Method: `tests/system_state/state.rs::CloneTracked::new`

Creates a payload together with the external handle to its zeroed clone
counter.

##### Reference

```text
tests/system_state/state.rs::set_mutate_and_take_transfer_payload_without_cloning
    -> tests/system_state/state.rs::CloneTracked::new
tests/system_state/state.rs::failed_take_restores_the_original_payload
    -> tests/system_state/state.rs::CloneTracked::new
tests/system_state/state.rs::explicit_clone_deep_clones_payloads_but_shares_the_layout
    -> tests/system_state/state.rs::CloneTracked::new
tests/system_state/state.rs::derived_empty_state_shares_layout_without_cloning_payloads
    -> tests/system_state/state.rs::CloneTracked::new
```

#### Method: `tests/system_state/state.rs::CloneTracked::clone`

Increments the shared counter and deeply clones the vector. It detects both
unexpected clones on ownership-transfer paths and the required clone during
explicit `SystemState::clone`.

##### Reference

```text
StateValue::clone
    -> ErasedValue::clone_box
    -> tests/system_state/state.rs::CloneTracked::clone
```

#### Test helper: `tests/system_state/state.rs::blank_state`

Builds a blank `SystemState` at a supplied index from the deterministic test
specification.

##### Reference

```text
tests/system_state/state.rs::blank_state
    -> tests/system_state/state.rs::StateSpec::fixture
tests/system_state/state.rs::blank_state -> TimePoint::new
tests/system_state/state.rs::blank_state -> SystemState::new

Every state contract test except
tests/system_state/state.rs::time_points_preserve_valid_coordinates_and_reject_non_finite_values
    -> tests/system_state/state.rs::blank_state
```

#### `src/system_state/error.rs`

Owns one concise public error type covering:

- template IO and JSON parsing;
- invalid/duplicate fields and type tags;
- unknown keys;
- empty slots;
- stored/requested type mismatches.

If this remains very small after implementation, it may later be folded into
`system_state.rs`; it is separated initially to keep state operations readable.

#### `tests/system_state.rs`

Integration tests exercise only the public API:

- loading a valid JSON template;
- rejecting malformed and duplicate fields;
- empty-state creation;
- deep state cloning;
- typed set/get/mutation/take;
- payload ownership transfer without calling `Clone`;
- unknown-key and type-mismatch behavior;
- derived empty states sharing their immutable spec.

All tests must live under the dedicated `dev/tests/` tree. Source modules must
never contain inline `#[cfg(test)]` modules or test functions. Private
implementation behavior is tested from mirrored dedicated test modules or
through the public `SystemState` contract; internal types are not made public
solely to make them testable.

Test files mirror the source organization:

- tests focused on one source file use the same filename, for example
  `src/system_state/value.rs` maps to `tests/system_state/value.rs`;
- tests spanning multiple source files use a concise filename describing the
  behavior or contract under test;
- nested test files are collected by a top-level Cargo integration-test
  harness once the public module facade is available.

## Current Recommendations Awaiting Agreement

- Use a template-defined immutable field layout shared by every state.
- Represent missing payloads as empty declared slots, naturally permitting
  partial/multi-rate states.
- Require an integer time index and make physical time optional.
- Store each state's payloads in compact field-ID-indexed slots with concise
  typed `set`/`get`/`take` operations.
- Deep-clone payloads only when `SystemState::clone()` is explicitly called;
  keep loading, offloading, and pipeline movement clone-free.
- Keep internal boxing for heterogeneous type erasure, but omit specialized
  public boxed methods.
- Allow arbitrary transient values but require registered codecs for persisted
  values.
- Make chunks owned `Vec<SystemState>` values transferred to writers without
  cloning payloads.
- Use length-delimited state records and state-boundary chunking.
- Use explicit sweep paths and begin with Cartesian sweeps plus replicates.
- Model workflows as dependency graphs.
- Make experiment and run scopes first-class.
- Keep stable IDs/manifests authoritative and readable paths optional.
- Keep domain validation outside the generic SSTS core.
- Implement the current core and runtime as Rust-native components.
- Begin with a local filesystem storage backend behind a storage interface.
- Use a representative scientific project as the first clean client rather
  than as a compatibility target.

## Open Questions

1. Should the canonical constructor be
   `StateSpec::load(path)?.empty(time)`, as recommended, or must
   `SystemState::new(path, time)` directly perform JSON filesystem IO?
2. Does the JSON template declare only field names, or field names plus stable
   codec/type tags as recommended?
3. Is a transient non-serializable value allowed in `SystemState`, with
   persistence failing until a codec is registered, or must every insertion
   require a codec immediately?
4. Should `SystemState::clone()` deep-clone independent payloads, as currently
   recommended, or shallow-clone shared `Arc` payloads?
5. Should `len()` report declared slots or filled slots, and should
   `is_empty()` mean no declared fields or no loaded payloads?
6. Should the initial dispatcher support only in-process Rust stages, or must
   arbitrary external commands be first-class immediately?
7. Which execution backends are initial requirements: local processes,
   systemd scopes, Slurm, containers, or others?
8. Should a workflow be declared in code, in a project manifest, or through a
   hybrid where manifests reference registered code entry points?

## Discussion Log

### 2026-07-30 — Initial architecture review

- Established the clean-slate, no-compatibility constraint.
- Identified responsibility mixing across SciTaskIO, dispatcher, simulator,
  and analysis.
- Proposed the Project/Experiment/Run/Stage/Attempt hierarchy.
- Proposed partial multi-rate states, explicit sweep axes, workflow graphs,
  scoped contexts, stable IDs, structured logs, and modular implementation.
- Identified SystemState completeness as the first major semantic decision.

### 2026-07-30 — Living design record requested

- `design.md` became the living design record.
- Future discussion turns will update this file.
- Added explicit status labels so proposals remain distinguishable from agreed
  decisions.

### 2026-07-30 — SystemState and SSTS performance requirements

- Established SystemState as a heterogeneous dictionary-like owned container.
- Made zero-copy transfer into/out of states and between SSTS/chunk/writer
  ownership boundaries a hard requirement.
- Separated generic containment from codec-backed persistence.
- Established SSTS as a growable state-major array.
- Proposed whole-vector ownership transfer for automatic chunking.
- Required serialization architecture to admit a future protobuf encoding.

### 2026-07-30 — Rust-only current scope

- Restricted the current design effort to the Rust implementation.
- Clarified that dictionary-like means Rust map ergonomics, not Python object
  compatibility.
- Deferred Python ownership, NumPy buffers, PyO3, and bridge protocols to a
  separate module and delivery stage.

### 2026-07-30 — Concrete SystemState surface

- Recommended `SystemState { time, values }` with insertion-ordered keys.
- Recommended opaque cloneable boxed `Any + Clone + Send + 'static` values and
  no `Sync` requirement.
- Distinguished ordinary move-based insertion from boxed address-stable
  insertion for strict no-relocation requirements.
- Initially proposed non-cloneable states; this was superseded by the explicit
  cloning decision below.
- Defined typed, untyped, collection, downcast, and consuming ownership
  operations.
- Kept codec registries and format-specific serialization outside
  `SystemState`.
- Kept time immutable through the ordinary API and outside the value map.

### 2026-07-30 — SystemState cloning

- Made `SystemState` and `StateValue` cloneable.
- Clarified that zero-copy is mandatory for payload insertion, removal, and
  movement through SSTS/chunk/writer boundaries, not for explicit cloning.
- Recommended deep cloning on explicit `clone()` to preserve independent
  mutation and unconditional owned removal.
- Recorded shallow `Arc` cloning as an alternative with uniqueness,
  copy-on-write, `Sync`, and removal tradeoffs.

### 2026-07-30 — Template-defined states and concise API

- Required the first state layout to originate from a JSON template and fixed
  the declared key set for all derived states.
- Replaced per-state string maps with a shared immutable `StateSpec` and compact
  optional value slots.
- Recommended `state.empty(time)` instead of `clone_empty()`.
- Recommended separating `StateSpec::load(path)` from state creation rather
  than making filesystem IO the sole `SystemState::new` behavior.
- Shortened payload operations to `set`, `get`, `get_mut`, `take`, `has`, and
  `is`.
- Retained boxing internally for heterogeneous type erasure but removed
  dedicated public boxed methods from the recommendation.

### 2026-07-30 — Proposed Rust implementation layout

- Confirmed `dev` is currently a blank Rust 2024 binary package and the new
  `system_state` directory has no implementation.
- Proposed converting the core to a conventionally named library crate.
- Proposed the modern `system_state.rs` plus `system_state/*.rs` module layout,
  without `mod.rs`.
- Split template/specification, state behavior, private type erasure, and
  errors into focused files behind one minimal facade.
- Deferred implementation until this layout is approved.

### 2026-07-30 — Crate naming

- Initially recommended `scitask`, then rejected it as insufficiently
  descriptive.
- Subsequently considered `scientific-project`.
- Finalized `scientific-workflow` as the accepted project and crate name.
- Rejected an `IO` suffix because the intended scope includes state, time
  series, experiments, and dispatch.
- Reserved corresponding `scientific-workflow-*` names as possible future
  workspace packages rather than creating them prematurely.

### 2026-07-30 — One-file implementation protocol

- Authorized implementation after the architecture discussion.
- Required exactly one implementation file per review cycle.
- Required thorough idiomatic Rust documentation in every file.
- Established the dependency-oriented implementation order from Cargo metadata
  through internal modules, facade, library root, fixtures, tests, and removal
  of the placeholder binary.

### 2026-07-30 — Implementation begins

- Interpreted the requested folder rename as changing the repository root
  directory from `SciTaskIO` to `scientific-workflow`.
- Authorized one initial review unit containing the Cargo manifest update plus
  the first Rust implementation file, `system_state/error.rs`.
- Preserved the existing dirty working tree created by moving old code into
  `legacy/`.

### 2026-07-30 — Erased state values implemented

- Added the crate-private `system_state/value.rs` implementation.
- Used an object-safe erased-value trait with internal boxing and no public
  boxed API.
- Required directly stored payloads to implement `Any + Clone + Send`.
- Implemented borrowed, mutable, and consuming typed downcasts.
- Made explicit value cloning deep-clone the concrete payload.
- Initially added inline unit tests, then removed them under the dedicated-test
  policy below.

### 2026-07-30 — Dedicated tests only

- Prohibited tests inside all source module files.
- Required every test to live under `dev/tests/`.
- Required private implementation tests to remain outside the source module
  without expanding the production API.

### 2026-07-30 — Mirrored test organization

- Required single-source-file tests to mirror the source filename under the
  corresponding `tests/` subdirectory.
- Required cross-module test files to use concise behavior-oriented names.
- Added `tests/system_state/value.rs` for the erased-value implementation.
- Verified six value tests independently while the Cargo module facade remains
  intentionally unwired.

### 2026-07-30 — Test instructions

- Created the repository-root `README.md`.
- Added only the initial test-instruction section.
- Documented the direct `rustc --test` command required while the nested test
  suite is not yet connected to a top-level Cargo integration-test harness.
- Documented `cargo test` as the standard command after the facade and
  `tests/system_state.rs` suite entry point are created.

### 2026-07-30 — StateValue privacy boundary

- Clarified that downstream code never imports or calls `value.rs`.
- Restricted erased values to the SystemState implementation and narrow
  crate-internal persistence interfaces.
- Defined `set`, `get`, `get_mut`, and `take` as the complete typed downstream
  ownership boundary.

### 2026-07-30 — StateValue call demonstration

- Documented concrete `state.rs` call sites for every `StateValue` operation.
- Demonstrated clone-free insertion, borrowing, mutation, and successful
  extraction.
- Demonstrated restoration of the erased payload after a failed consuming
  downcast.

### 2026-07-30 — Exhaustive design references

- Made `design.md` the exhaustive reference for every Rust type and method.
- Required every method to contain a `Reference` subsection listing all known
  production and test callers.
- Standardized references as explicit caller-to-callee mappings.
- Added the complete initial reference for `StateValue` and `ErasedValue`.

### 2026-07-30 — State specifications implemented

- Added `system_state/spec.rs`.
- Implemented strict JSON template loading and source-preserving errors.
- Assigned compact field indices from deterministic template order.
- Normalized field names and stable type tags and rejected empty or duplicate
  declarations.
- Represented `StateSpec` as a cheap cloneable handle around
  `Arc<StateLayout>`.
- Added the exhaustive design reference for every specification struct and
  method.
- Added `StateSpec::to_json` using a borrowed serialization view that omits
  runtime indices and source paths.
- Added an explicit filesystem round-trip test:
  load original JSON -> serialize -> compare parsed JSON values -> write
  serialized JSON -> reload -> compare every reconstructed `FieldSpec`.

### 2026-07-30 — System states implemented

- Added `system_state/state.rs` as the typed public ownership boundary over
  the private `StateValue` erasure layer.
- Added `TimePoint` with a mandatory deterministic index and an optional
  validated finite physical coordinate.
- Stored one optional payload slot per template field and preserved the fixed
  `StateSpec` layout across derived empty states.
- Implemented concise typed operations: `set`, `get`, `get_mut`, `take`,
  `has`, `is`, `clear`, and `clear_all`.
- Kept insertion, in-place mutation, successful extraction, and empty-state
  derivation clone-free for payloads.
- Made explicit `SystemState::clone` share immutable specification metadata
  while deeply cloning populated payloads.
- Restored the original payload after a failed typed `take`, preventing a
  mistaken type request from discarding scientific data.
- Added bounded `Debug` output that reports structural metadata without
  traversing or formatting potentially large payloads.
- Added the exhaustive design reference for every `TimePoint` and
  `SystemState` method.
- Deferred the one-line `StateSpec::empty` integration correction to a
  subsequent review unit.

### 2026-07-30 — System state contract tests

- Added the dedicated mirrored `tests/system_state/state.rs` file without
  placing tests inside any source module.
- Used the real `state.rs`, `value.rs`, and `error.rs` implementations with a
  narrow test-only specification stub while the public facade remains
  intentionally unwired.
- Verified finite and non-finite `TimePoint` behavior, fixed layout shape,
  typed access, precise failures, clearing, bounded debug output, and shared
  specification ownership.
- Used an atomic clone counter to prove that `set`, `get_mut`, `take`, and
  derived empty-state creation do not clone payloads.
- Compared the `Vec` data pointer and capacity before insertion and after
  extraction to explicitly verify allocation-preserving ownership transfer.
- Verified that a failed typed `take` restores the exact original payload.
- Verified that explicit `SystemState::clone` performs exactly one deep
  payload clone and permits independent mutation afterward.
- Ran all nine state contract tests successfully with the temporary Cargo test
  harness. The only compiler warning concerns unused `StateError` variants
  because this isolated suite does not exercise template loading.
- Replaced planned state-test references with exact caller-to-callee mappings
  and documented every test-only support struct and method.

### 2026-07-30 — State construction boundary integrated

- Updated `StateSpec::empty` to call the crate-private
  `SystemState::new(spec, time)` constructor.
- Established `StateSpec::empty(time)` as the production path for creating the
  first blank state from a JSON-derived layout.
- Retained `SystemState::empty(time)` as the payload-free derivation path from
  an existing state.
- Removed the pending-integration marker from the `SystemState::new` reference;
  `StateSpec::empty -> SystemState::new` is now an implemented production call.
- Compiled the real `error.rs`, `value.rs`, `spec.rs`, and `state.rs` modules
  together in a temporary integration harness and verified the complete path:
  JSON load -> `StateSpec::empty` -> typed `SystemState::set` -> typed
  `SystemState::take`.
- Kept this review unit limited to `spec.rs`; its isolated test stub still
  follows the previous constructor signature and will be updated in the next
  dedicated test-file review.

### 2026-07-30 — Specification construction test integrated

- Updated only `tests/system_state/spec.rs` to match the finalized
  crate-private `SystemState::new(spec, time)` boundary.
- Made the test-only state retain its received `StateSpec` and expose a narrow
  borrowed `spec` accessor.
- Extended the real-template round-trip test to call `StateSpec::empty`.
- Used field-slice pointer equality to verify that empty-state construction
  shares the original immutable layout allocation rather than copying field
  metadata.
- Re-ran the specification test successfully. The warnings are limited to
  state-access errors and lookup methods that this isolated specification test
  intentionally does not exercise.
- Updated the exact method references for the revised test-only constructor,
  accessor, placeholder time-point construction, and `StateSpec::empty`.

### 2026-07-30 — Next review unit confirmed

- Confirmed `dev/src/system_state.rs` as the next file.
- Limited the facade to module declarations, module-level documentation, and
  intentional public re-exports; it will contain no state implementation.
- Finalized the facade exports as `StateError`, `FieldSpec`, `StateSpec`,
  `SystemState`, and `TimePoint`.
- Added `FieldSpec` to the earlier proposed export list because several public
  inspection methods return that type; leaving it trapped behind private
  `spec` would make the public API incomplete.
- Kept `StateValue`, `ErasedValue`, template parsing representations, and
  compact name-to-slot lookup details private.
- No Rust source was edited while clarifying this next step.

### 2026-07-30 — Public system-state facade implemented

- Added `dev/src/system_state.rs` using the Rust 2018-and-later module layout:
  the facade file sits beside the `system_state/` implementation directory,
  with no `mod.rs`.
- Declared `error`, `spec`, `state`, and `value` as private submodules.
- Re-exported exactly `StateError`, `FieldSpec`, `StateSpec`, `SystemState`,
  and `TimePoint`.
- Kept `StateValue`, `ErasedValue`, Serde-only template types, layout storage,
  and compact name lookup inaccessible to downstream crates.
- Added module-level documentation covering the template-first workflow,
  fixed-layout invariant, move-based payload ownership, deep explicit cloning,
  and encapsulation boundary.
- Kept the facade free of structs, methods, and substantive implementation;
  therefore it introduces no additional method-level `Reference` sections.
- Verified all five public exports using a temporary library harness that
  mirrors the final `src/system_state.rs` plus `src/system_state/` directory
  shape.
- Confirmed the facade test and documentation test phases complete
  successfully.
- Identified `dev/src/lib.rs` as the next one-file review unit.

### 2026-07-30 — Library-root responsibility clarified

- Defined `dev/src/lib.rs` as Cargo's library entry point and the root of the
  downstream `scientific_workflow` Rust namespace.
- Kept the proposed implementation intentionally small:

  ```rust
  //! Scientific state, time-series, and execution primitives.

  pub mod system_state;
  ```

- Confirmed that the package name `scientific-workflow` becomes the Rust import
  name `scientific_workflow`, so downstream paths begin with
  `scientific_workflow::system_state`.
- Chose not to flatten all system-state types into the crate root. Callers will
  use the explicit namespace:

  ```rust
  use scientific_workflow::system_state::{
      FieldSpec, StateError, StateSpec, SystemState, TimePoint,
  };
  ```

- Assigned crate-level documentation and top-level module selection to
  `lib.rs`; parsing, state storage, ownership transfer, and other substantive
  behavior remain in their focused modules.
- Noted that adding `lib.rs` makes Cargo compile and document the real module
  tree normally, allowing standard `cargo check`, `cargo test`, and rustdoc
  validation without temporary source-inclusion harnesses.
- Deferred crate-wide lint attributes until the complete public surface can be
  checked together; the library root should not introduce policy unrelated to
  its namespace role.
- No Rust source was edited during this clarification.

### 2026-07-30 — Library root implemented

- Added `dev/src/lib.rs` as Cargo's library entry point.
- Exposed only the top-level `system_state` module, preserving the explicit
  `scientific_workflow::system_state::*` downstream namespace.
- Added crate-level documentation describing the modular scientific-workflow
  scope, the currently implemented state capabilities, ownership guarantees,
  and the intended relationship to future SSTS and dispatcher modules.
- Added a compile-only rustdoc example covering JSON specification loading,
  initial state construction, payload insertion, in-place mutation, and
  clone-free extraction through the public API.
- Kept the library root free of structs and methods, so it introduces no new
  method-level `Reference` sections.
- Ran `cargo check --all-targets` successfully against the real library and
  binary targets.
- Ran the crate documentation tests successfully; the public example compiled
  as written.
- Removed the automatically generated untracked `Cargo.lock` after validation
  to preserve the one-file review boundary. Lockfile policy will be decided
  when the final package target structure is established.
- Identified the canonical JSON test fixture as the next one-file review unit.

### 2026-07-30 — Remaining SystemState bootstrap sequence

- Confirmed the next review unit as
  `dev/tests/fixtures/state.json`, containing one strict, representative,
  domain-neutral state template.
- Planned the fixture fields as an ordered population vector, spatial grid, and
  activity/status payload, each with a stable versioned scientific type tag.
- The fixture will contain only valid template data. Invalid-template cases
  will be expressed directly by individual tests so the canonical fixture
  remains an unambiguous reusable baseline.
- Planned `dev/tests/system_state.rs` immediately after the fixture. It will
  exercise the installed public path
  `scientific_workflow::system_state::*`, rather than including private source
  files or using test-only stubs.
- The public integration suite will cover:

  1. loading the canonical fixture;
  2. deterministic field order and tags;
  3. initial construction through `StateSpec::empty`;
  4. derived construction through `SystemState::empty`;
  5. typed set, borrow, mutation, and take;
  6. explicit deep cloning;
  7. precise unknown, missing, and mismatched-field errors;
  8. JSON serialization and semantic round-trip equality.

- Planned removal of the placeholder `dev/src/main.rs` only after the public
  library integration suite passes, leaving a library-only package.
- Planned a final README test-command update after the real Cargo test layout
  is established.
- Retained the one-file-at-a-time review boundary throughout this sequence.
- No source or fixture file was edited during this planning step.

### 2026-07-30 — Canonical tensor payload selected

- Selected tensors from the `physics_in_parallel` crate as the canonical
  scientific payloads used by public SystemState integration tests.
- Inspected the locally available `physics_in_parallel` 3.0.3 implementation.
  Its public generic tensor is
  `physics_in_parallel::math::Tensor<T, B>`, with owned dense and sparse
  backends.
- Confirmed compatibility with the current SystemState insertion bound:
  tensor storage implements `Clone + Send + Sync`, scalar values are
  `'static + Send + Sync`, and therefore concrete tensors satisfy
  `Any + Clone + Send`.
- Confirmed that dense tensor cloning is deep at this version because the
  backend owns `Vec` shape and data allocations and derives `Clone`. Sparse
  tensor storage likewise owns and clones its storage. Consequently, explicit
  `SystemState::clone` preserves the agreed independent-mutation behavior.
- Kept `SystemState` generic rather than making its core API tensor-specific.
  Applications may still store any compatible Rust type, while tensor payloads
  provide the canonical performance and integration contract.
- Planned the canonical fixture tags around payload meaning and codec schema,
  for example:

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
      },
      {
        "name": "activity",
        "type": "physics_in_parallel.tensor.dense.u8.v1"
      }
    ]
  }
  ```

- Treated the final `.v1` component as the scientific-workflow codec schema
  version, not the dependency's crate release number.
- Planned public integration tests using
  `physics_in_parallel::math::{Dense, Tensor}` and concrete
  `Tensor<u64, Dense>` / `Tensor<u8, Dense>` payloads.
- Noted an important serialization limitation: the inspected tensor JSON
  implementation constructs a flat payload using `data().to_vec()`, which
  copies the complete dense buffer before encoding.
- Therefore, the existing tensor Serde JSON path is acceptable for small
  correctness tests but is not the planned high-performance SSTS codec. The
  later persistence stage must use a borrowed/streaming tensor encoder or an
  owned-parts transfer API that avoids a full intermediate tensor copy.
- Planned `physics_in_parallel = "3.0.3"` initially as a development dependency
  for public payload contract tests. A production dependency should be added
  only when a tensor-specific codec or adapter becomes part of the library.
- Updated the next-file sequence: revise `dev/Cargo.toml` with the development
  dependency first, then create the canonical fixture, then implement public
  tensor-backed integration tests.
- No project file other than this living design was edited while recording the
  tensor decision.

### 2026-07-30 — All-modules integration test begins

- Accepted the requirement for one public integration test backed by an actual
  JSON fixture and actual `physics_in_parallel` tensor payloads.
- Split the work into three mandatory one-file review units:

  1. `dev/Cargo.toml` — tensor development dependency;
  2. `dev/tests/fixtures/state.json` — canonical template;
  3. `dev/tests/system_state.rs` — public all-modules integration test.

- Completed only the first review unit by adding:

  ```toml
  [dev-dependencies]
  physics_in_parallel = "3.0.3"
  ```

- Kept this as a development dependency because the generic production
  SystemState API does not name tensor types. The integration contract may use
  the tensor crate without forcing every downstream library user to compile it.
- Planned the final integration test to import only
  `scientific_workflow::system_state` public exports. It will exercise:

  ```text
  JSON fixture
      -> StateSpec::load
      -> StateSpec::empty
      -> SystemState::set<Tensor>
      -> SystemState::get<Tensor>
      -> SystemState::get_mut<Tensor>
      -> SystemState::clone
      -> SystemState::take<Tensor>
      -> StateSpec::to_json
      -> StateSpec::load
  ```

- This path covers `spec.rs`, `state.rs`, and `error.rs` directly through their
  public behavior and `value.rs` indirectly through typed tensor erasure,
  cloning, borrowing, mutation, and owned extraction.
- Validated the revised manifest with `cargo metadata --no-deps`; it parsed
  successfully without generating a lockfile.
- Deferred the fixture and test source until their respective reviews, in
  accordance with the one-file-at-a-time rule.

### 2026-07-30 — Canonical tensor fixture implemented

- Added `dev/tests/fixtures/state.json` as the single valid template baseline
  for public integration testing.
- Declared exactly three fields in deterministic order:

  ```text
  0 -> population -> physics_in_parallel.tensor.dense.u64.v1
  1 -> space      -> physics_in_parallel.tensor.dense.u64.v1
  2 -> activity   -> physics_in_parallel.tensor.dense.u8.v1
  ```

- Used `u64` dense tensors for population and spatial data and a compact `u8`
  dense tensor for activity data.
- Kept the field tags explicit about provider, tensor storage backend, scalar
  type, and codec schema version.
- Added no fixture-only shape constraint because tensor shapes are runtime
  state properties, while the current template contract defines stable field
  identity and serialization meaning.
- Kept the fixture strictly valid and free of invalid cases; malformed,
  duplicate, and empty-field templates remain local to their individual error
  tests.
- Verified JSON syntax, the exact top-level and field property sets, field
  count, declaration order, names, and type tags with `jq`.
- The fixture defines data only and introduces no structs or methods requiring
  method-level `Reference` sections.
- Identified `dev/tests/system_state.rs` as the next review unit. It will load
  this fixture through the public library API and exercise every current
  SystemState implementation module with concrete tensor payloads.

### 2026-07-30 — Public all-modules test specified

- Confirmed `dev/tests/system_state.rs` as the next and only file in its review
  unit.
- Planned one end-to-end public test named
  `tensor_state_round_trip_integrates_public_modules`.
- Restricted its imports to:

  ```rust
  use physics_in_parallel::math::{Dense, Tensor};
  use scientific_workflow::system_state::{
      FieldSpec, StateError, StateSpec, SystemState, TimePoint,
  };
  ```

- The test will resolve `tests/fixtures/state.json` relative to
  `env!("CARGO_MANIFEST_DIR")`, avoiding assumptions about the process working
  directory.
- Planned the execution and assertion sequence:

  1. Load the real fixture through `StateSpec::load`.
  2. Verify source provenance, field count, deterministic order, compact
     indices, names, and all versioned tensor tags.
  3. Serialize the specification with `StateSpec::to_json` and compare parsed
     JSON values to verify semantic equality independent of formatting.
  4. Write the serialized template to a unique temporary path, reload it, and
     verify reconstructed field equality.
  5. Construct the first blank state through `StateSpec::empty` with a physical
     `TimePoint`.
  6. Create real dense `Tensor<u64, Dense>` population and spatial payloads and
     a real dense `Tensor<u8, Dense>` activity payload.
  7. Move all three tensors into the state with `SystemState::set`.
  8. Borrow tensors with `get`, mutate original tensor storage in place with
     `get_mut`, and verify shape and scalar contents.
  9. Verify unknown-field, missing-value, and exact-type-mismatch errors through
     the public `StateError` variants without losing stored data.
  10. Explicitly clone the populated state, mutate a cloned tensor, and verify
      the original tensor remains independent.
  11. Move tensors back out through `take`, validate their shapes and values,
      and verify the corresponding slots become empty.
  12. Derive a later blank state through `SystemState::empty` and verify it
      shares the same specification while containing no payloads.
  13. Remove the temporary round-trip directory.

- Mapped module coverage as follows:

  ```text
  system_state.rs -> public import boundary
  spec.rs         -> fixture load, validation, lookup, serialization, reload
  state.rs        -> time, construction, access, mutation, clone, extraction
  value.rs        -> indirect tensor erase/borrow/mutate/clone/downcast paths
  error.rs        -> public unknown/missing/mismatch variants
  lib.rs          -> external crate namespace
  ```

- Distinguished template round-trip serialization from payload persistence.
  This test validates JSON serialization of `StateSpec`; tensor payload
  serialization belongs to the later SSTS codec stage and will not be implied
  by this test.
- Noted that the existing `CloneTracked` contract test proves absence of clone
  calls during ownership transfer. The tensor integration test validates that
  the real canonical tensor types traverse the same public ownership paths;
  their private backing pointers are not exposed for direct identity checks.
- No source or test file was edited during this specification step.

### 2026-07-30 — Readiness for state time series

- Determined that the SystemState ownership model is sufficient to begin the
  in-memory SSTS stage after the public all-modules integration test passes.
- Identified the capabilities already available to SSTS:

  ```text
  SystemState uniquely owns each payload
  Vec<SystemState> moves states without cloning payloads
  TimePoint provides a mandatory deterministic index
  SystemState::time exposes ordering metadata
  SystemState::spec exposes immutable schema metadata
  SystemState::empty creates later states without payload copies
  ```

- Defined the immediate post-test completion checks:

  1. run the public integration test;
  2. run the complete Cargo test and documentation-test suite;
  3. remove the placeholder binary entry point;
  4. update the README with the final standard test commands.

- Treated steps 3 and 4 as packaging cleanup rather than blockers for starting
  SSTS architecture discussion.
- Identified one likely SSTS prerequisite: appending a state must efficiently
  prove that its `StateSpec` is the same immutable layout as the series schema.
  Comparing every field on every append would be unnecessary.
- Planned a narrow `StateSpec` identity operation backed by `Arc::ptr_eq`,
  likely crate-private unless downstream schema comparison has a demonstrated
  public use. Its final name and visibility will be decided with the SSTS
  append contract.
- Established that initial SSTS ordering uses `TimePoint::index`, not the
  optional floating-point physical coordinate. The integer index supports
  exact monotonicity checks and chunk boundaries; physical time remains
  descriptive metadata.
- Deferred tensor payload encoding, protobuf, disk persistence, and streaming
  IO to the later SSTS codec/persistence stage. The first SSTS implementation
  will be an in-memory growable owner of states and chunks.
- Required chunk extraction to move contiguous `SystemState` ranges rather
  than clone states or payloads.
- No source or test file was edited while recording this readiness decision.

### 2026-07-30 — Public tensor integration test implemented

- Added `dev/tests/system_state.rs` as the public end-to-end SystemState
  contract test.
- Imported all production types through
  `scientific_workflow::system_state`, verifying the library root and facade
  rather than including private implementation files.
- Loaded `tests/fixtures/state.json` using a path derived from
  `CARGO_MANIFEST_DIR`.
- Verified all field indices, names, tensor codec tags, lookups, and source
  provenance.
- Verified semantic JSON equality, wrote the generated template to a unique
  temporary directory, reloaded it, and compared the reconstructed field
  specifications.
- Used concrete `physics_in_parallel::math::Tensor<u64, Dense>` and
  `Tensor<u8, Dense>` payloads throughout the state lifecycle.
- Verified blank construction, physical and integer time metadata, tensor
  insertion, typed borrowing, in-place tensor mutation, exact runtime type
  checks, deep state cloning, independent branch mutation, and owned tensor
  extraction.
- Verified `StateError::UnknownField`, `StateError::MissingValue`, and
  `StateError::TypeMismatch` through the public API and confirmed failed typed
  access does not alter the stored tensor.
- Verified later blank-state derivation and field-slice pointer equality,
  proving immutable specification storage remains shared.
- The test function
  `tests/system_state.rs::tensor_state_round_trip_integrates_public_modules`
  is now recorded in every production method `Reference` section it calls.
- Ran the standard real-crate suite successfully:

  ```text
  public tensor integration tests: 1 passed
  crate unit-test targets:          0 failed
  rustdoc examples:                 1 passed
  ```

- Re-ran every isolated private contract suite successfully:

  ```text
  StateSpec contracts:  1 passed
  SystemState contracts: 9 passed
  StateValue contracts: 6 passed
  ```

- Removed the test-generated untracked `Cargo.lock` after validation to retain
  the one-file review boundary.
- Declared the SystemState implementation and verification stage ready for the
  requested SSTS architecture discussion.

### 2026-07-30 — Example-project references generalized

- Removed all references to the surrounding example project from this design.
- Replaced example-specific simulator and workflow wording with
  domain-neutral scientific-project descriptions.
- Replaced example-specific serialization tags with neutral `example.*` tags.
- Simplified the rename history to repository-directory names without
  embedding a machine-specific parent path.
- Kept the example project's behavior only where it illustrates a generic
  architectural requirement.

### 2026-07-30 — Repository publication target updated

- Changed the Git `origin` target to
  `git@github.com:dingyisun0101/Scientific-Workflow.git`.
- Added `/legacy/` to the repository ignore policy.
- Removed already tracked legacy paths from the Git index while retaining the
  local archive on disk.
- Scoped the publication commit to the clean-slate scientific-workflow files
  and the removal of legacy content from version control.
- The first push was rejected by GitHub because prior history tracked
  `dev/target/package/scientific-workflow-0.1.0.crate`, whose generated package
  archive exceeded GitHub's 100 MiB per-file limit.
- Identified 1,779 tracked files under `dev/target`; ignore rules alone cannot
  remove already committed build output from history.
- Preserved the pre-cleanup commit graph on a local backup branch, removed
  `dev/target` from the Git index without deleting local build files, and
  prepared a parentless clean snapshot for the empty destination repository.
