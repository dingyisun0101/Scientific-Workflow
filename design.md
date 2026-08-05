# Scientific Workflow: Rust Architecture

## Status and scope

This document is the current source of truth for the clean-slate Rust design of
the scientific-workflow crate. It replaces the former chronological discussion
log. Superseded alternatives are omitted; source code that has not yet caught
up with the agreed design is listed explicitly under Implementation delta.

Current scope:

- Rust data structures and persistence only;
- no Python API or language bridge yet;
- no backward compatibility or legacy-format support;
- generic serializable payloads, including scientific tensors;
- one-file-at-a-time implementation and review;
- tests live under tests, never inside production modules.

The implemented crate currently exports system_state. Time-series source files
are staged but not yet exported from lib.rs.

## Architecture

The runtime and analysis paths are deliberately different:

    simulation-owned evolving full state
        -> cadence selects one logical output stream
        -> selected fields are serialized while borrowed
        -> one owned encoded record enters a bounded queue
        -> StateWriter appends complete records
        -> count-based rollover commits immutable chunks

    metadata.json plus committed chunks
        -> SeriesReader selects a stream and range
        -> records decode lazily
        -> optional collection into StateSeries
        -> analysis

The persisted stream is the authoritative system-state time series. StateSeries
is an in-memory analysis working set; it is not a writer buffer.

## Core invariants

1. A SystemState has a fixed template-defined layout.
2. StateSpec clones share one immutable Arc allocation.
3. Moving payloads into and out of SystemState does not clone them.
4. SystemState::clone is a deliberate deep clone of every populated payload.
5. Serialization borrows payloads but necessarily produces encoded bytes.
6. Each logical output stream has one StateSpec, cadence, path, writer, and
   independent chunk sequence.
7. One encoded partial state is one indivisible record.
8. Chunk boundaries are determined only by states_per_chunk.
9. Byte counts may bound queues and describe files, but never determine
   chunk boundaries.
10. No payload, field, JSON object, or protobuf message is split across chunks.
11. Structural metadata is stored once in output-root metadata.json.
12. Chunk files contain only minimally framed records.

## Crate layout

Target Rust 2024 layout:

    src/
    ├── lib.rs
    ├── system_state.rs
    ├── system_state/
    │   ├── error.rs
    │   ├── spec.rs
    │   ├── state.rs
    │   └── value.rs
    ├── time_series.rs
    └── time_series/
        ├── error.rs
        ├── codec.rs
        ├── series.rs
        ├── format.rs
        ├── writer.rs
        └── reader.rs

Module-name files are preferred over mod.rs. Files named core.rs are avoided:
system_state::state and time_series::series are clearer in diagnostics and
documentation.

## System-state model

### Template

The first layout is loaded from a strict JSON template:

    {
      "fields": [
        {"name": "population", "type": "vec.u64"},
        {"name": "space", "type": "square-lattice.u64.v1"}
      ]
    }

Array order defines compact slot indices. Field names and stable type tags are
trimmed. Empty names, duplicate names, empty tags, and unknown JSON properties
are rejected. A type tag identifies persisted meaning; it is not a Rust type
name.

### Ownership

SystemState stores heterogeneous values behind private boxes because a Vec
requires one statically sized element type. Boxing is not exposed publicly.
set consumes a concrete value; take returns the same value and backing
allocation. The box allocation and movement of a small owner are not copies of
the scientific buffer.

All stored values implement Any + Clone + Send + 'static. Clone is required
only so SystemState can honor its public deep-clone contract. Send allows an
owned state to cross a thread boundary. Sync is not required.

### Partial states

Empty slots are valid. A stream specification may describe only a subset of a
simulation's full state, and different streams may use different
specifications. The writer-side borrowed sampling API is not finalized; it
must serialize selected live fields without constructing an owned partial state
that copies data needed by the simulation.

## Public system-state API

The method catalog covers every explicitly declared production method in the
current or target source. Compiler-generated derives such as Debug, Clone,
Serialize, and Default are recorded as type properties unless their cost or
behavior changes an architectural contract.

### FieldSpec

One immutable normalized field declaration. Fields are stored in template
order and carry a compact slot index, human-facing key, and stable codec tag.

#### FieldSpec::new

Private constructor that trims a validated name and type tag and assigns the
template-order index.

##### Reference

    StateSpec::from_template -> FieldSpec::new

#### FieldSpec::index

Returns the zero-based slot index.

##### Reference

    StateSpec template validation -> FieldSpec::index
    state encoder field traversal -> FieldSpec::index -> payload slot
    tests -> verify deterministic template order

#### FieldSpec::name

Returns the normalized dictionary key.

##### Reference

    SystemState typed access -> FieldSpec::name
    CodecRegistry -> TypedCodec::payload -> FieldSpec::name
    JSON encoder -> output field key
    metadata writer -> embedded stream schema

#### FieldSpec::type_tag

Returns the stable serialization tag.

##### Reference

    CodecRegistry lookup -> FieldSpec::type_tag
    metadata writer -> embedded stream schema
    reader -> codec selection

### StateSpec

A cheap cloneable handle around Arc<StateLayout>. Cloning shares field
definitions and the name lookup map. Exact shared identity is intentionally
distinct from structural equality.

#### StateSpec::load

Reads, parses, validates, and records the source path of a JSON template.

##### Reference

    program initialization -> StateSpec::load
    stream configuration -> StateSpec::load
    tests -> fixture loading and round trip

#### StateSpec::empty

Creates a blank SystemState at a TimePoint using this shared layout.

##### Reference

    initial state construction -> StateSpec::empty
    decoder -> StateSpec::empty -> populate decoded fields
    analysis -> create derived state

#### StateSpec::to_json

Pretty-serializes the strict template structure without runtime indices or the
source path.

##### Reference

    template round-trip test -> StateSpec::to_json -> StateSpec::load
    metadata writer -> serialize stream schema
    diagnostics/export tools -> emit normalized template

#### StateSpec::source

Returns the original, non-canonicalized template path.

##### Reference

    bounded Debug for SystemState and StateSeries -> StateSpec::source
    diagnostics -> StateSpec::source

#### StateSpec::fields

Returns all fields in deterministic template order.

##### Reference

    SystemState::fields -> StateSpec::fields
    state encoder and decoder -> ordered field traversal
    StateSpec::to_json -> StateTemplateRef

#### StateSpec::len

Returns the number of declared fields.

##### Reference

    SystemState::new -> allocate one slot per field
    collection and metadata inspection -> StateSpec::len

#### StateSpec::is_empty

Reports whether the layout declares no fields.

##### Reference

    initialization validation and inspection -> StateSpec::is_empty
    tests -> empty-template contract

#### StateSpec::get

Looks up a FieldSpec by normalized name.

##### Reference

    borrowed partial-state validation -> StateSpec::get
    callers inspecting stream schemas -> StateSpec::get

#### StateSpec::contains

Reports whether a normalized field name is declared.

##### Reference

    configuration validation -> StateSpec::contains
    analysis field discovery -> StateSpec::contains

#### StateSpec::shares_layout

Performs constant-time Arc identity comparison.

##### Reference

    StateSeries::push -> StateSpec::shares_layout
    writer exact-spec submission path -> StateSpec::shares_layout
    tests -> cloned versus independently loaded specifications

#### StateSpec::index_of

Crate-private lookup from name to compact slot index, returning UnknownField.

##### Reference

    SystemState::has, is, set, take, clear, value, value_mut
        -> StateSpec::index_of

#### StateSpec::from_template

Private constructor that validates parsed declarations and builds StateLayout.

##### Reference

    StateSpec::load -> StateSpec::from_template

#### StateSpec::clone

Shares the immutable StateLayout through Arc.

##### Reference

    StateSpec::empty -> StateSpec::clone
    SystemState::empty and SystemState::clone -> StateSpec::clone
    StateSeries ownership -> retain canonical layout
    writer/reader setup -> share stream schema

### TimePoint

The time coordinate of one state. index is a required u64 ordering key;
physical is an optional finite f64 domain coordinate. Units are stream metadata,
not repeated schema metadata.

#### TimePoint::new

Constructs an index-only time point.

##### Reference

    simulation sampling -> TimePoint::new
    tests and decoded index-only records -> TimePoint::new

#### TimePoint::from_physical

Constructs a time point with a finite physical coordinate; NaN and infinities
return None.

##### Reference

    simulation with physical time -> TimePoint::from_physical
    reader -> validate decoded physical time

#### TimePoint::index

Returns the authoritative integer index.

##### Reference

    StateSeries ordering checks -> TimePoint::index
    writer stream ordering and chunk descriptors -> TimePoint::index
    reader range selection -> TimePoint::index

#### TimePoint::physical

Returns the optional physical coordinate.

##### Reference

    encoder -> TimePoint::physical
    analysis and plotting adapters -> TimePoint::physical

### SystemState

An owned fixed-layout heterogeneous dictionary at one time point:

    SystemState
    ├── spec: StateSpec
    ├── time: TimePoint
    └── values: Vec<Option<StateValue>>

#### SystemState::new

Crate-private constructor allocating empty slots for a validated StateSpec.

##### Reference

    StateSpec::empty -> SystemState::new
    SystemState::empty -> SystemState::new

#### SystemState::empty

Creates a blank state at a new time while sharing this state's layout.

##### Reference

    simulation or analysis -> previous_state.empty(next_time)
    tests -> verify no payload clone

#### SystemState::time

Returns the immutable TimePoint.

##### Reference

    StateSeries ordering and errors -> SystemState::time
    encoder -> record time
    analysis -> state coordinate

#### SystemState::spec

Returns the shared StateSpec.

##### Reference

    StateSeries::push -> SystemState::spec -> StateSpec::shares_layout
    writer validation and encoder -> SystemState::spec
    callers -> schema inspection

#### SystemState::len

Returns declared slot count, including empty slots.

##### Reference

    SystemState Debug -> SystemState::len
    structural inspection and tests -> SystemState::len

#### SystemState::is_empty

Reports whether the specification declares zero fields.

##### Reference

    structural inspection and tests -> SystemState::is_empty

#### SystemState::loaded

Counts populated slots.

##### Reference

    SystemState Debug -> SystemState::loaded
    completeness checks and tests -> SystemState::loaded

#### SystemState::is_blank

Reports whether every declared slot is empty.

##### Reference

    decoder initialization and analysis -> SystemState::is_blank
    tests -> empty-state contract

#### SystemState::fields

Returns field definitions in specification order.

##### Reference

    caller schema inspection -> SystemState::fields
    encoder -> deterministic traversal

#### SystemState::has

Reports whether a declared key currently contains a payload.

##### Reference

    partial-record encoder -> omit empty optional fields
    analysis -> SystemState::has
    tests -> insertion, take, and clear transitions

#### SystemState::is<T>

Reports whether a populated field contains exactly T.

##### Reference

    caller type probing -> SystemState::is
    tests -> exact runtime type identity

#### SystemState::set<T>

Target ownership-preserving assignment:

    empty slot -> Ok(None)
    occupied by T -> replace and return Ok(Some(previous_T))
    unknown key -> Err(SetError containing incoming T)
    occupied by another type -> leave old value and return incoming T in SetError

No payload is cloned. Returning the displaced payload is intentional and must
remain prominent in API documentation.

##### Reference

    simulation and analysis -> SystemState::set
    CodecRegistry decode -> SystemState::set
    tests -> insertion, replacement, rejection, and ownership recovery

#### SystemState::get<T>

Borrows a populated field as exactly T.

##### Reference

    application and analysis -> SystemState::get
    TypedCodec::payload -> SystemState::get
    tests -> typed immutable access

#### SystemState::get_mut<T>

Mutably borrows a populated field as exactly T without cloning.

##### Reference

    simulation evolution -> SystemState::get_mut
    analysis transformation -> StateSeries::get_mut -> SystemState::get_mut
    tests -> in-place payload mutation

#### SystemState::take<T>

Moves T out of a slot. A type mismatch restores the original erased value.

##### Reference

    ownership handoff -> SystemState::take
    tests -> pointer-preserving extraction and mismatch restoration

#### SystemState::clear

Drops one declared payload and reports whether one existed.

##### Reference

    selective reset -> SystemState::clear
    tests -> populated and already-empty slots

#### SystemState::clear_all

Drops all payloads while retaining layout and time.

##### Reference

    explicit state reset -> SystemState::clear_all
    tests -> blank-state transition

#### SystemState::value

Private helper returning one populated StateValue.

##### Reference

    SystemState::get -> SystemState::value

#### SystemState::value_mut

Private helper returning one populated mutable StateValue.

##### Reference

    SystemState::get_mut -> SystemState::value_mut

#### SystemState::clone

Shares StateSpec and deep-clones every populated payload. This operation is
potentially extremely expensive and is never the normal persistence path.

##### Reference

    explicit caller request for independent state -> SystemState::clone
    StateSeries::clone -> SystemState::clone for every state
    clone contract tests

#### SystemState::fmt

Debug output contains only time, schema source, field count, and loaded count.

##### Reference

    diagnostics and assertion failures -> Debug::fmt(SystemState)

### StateError

Non-exhaustive public error enum for template IO/parsing, semantic template
validation, unknown or empty fields, Rust type mismatches, and reconstructed
slot-count mismatch. Wrapped IO and JSON errors remain available through
Error::source.

### SetError<T>

Ownership-preserving SystemState::set rejection containing StateError and the
unchanged incoming T. Formatting never traverses or requires Debug on T.

#### SetError::new

Crate-private constructor for a rejected insertion.

##### Reference

    SystemState::set unknown key -> SetError::new
    SystemState::set occupied different type -> SetError::new

#### SetError::error

Borrows the rejection reason.

##### Reference

    caller error inspection -> SetError::error
    tests -> reason verification

#### SetError::payload

Borrows the unchanged rejected payload.

##### Reference

    caller inspection before recovery -> SetError::payload
    tests -> identity and no-clone verification

#### SetError::into_parts

Consumes the error and returns StateError plus the original T.

##### Reference

    caller recovery -> SetError::into_parts
    tests -> recover original allocation

#### SetError Debug::fmt

Formats only StateError and the compile-time name of T.

##### Reference

    diagnostics and assertion failures -> Debug::fmt(SetError)

#### SetError Display::fmt

Delegates to the contained StateError.

##### Reference

    logs and user-facing errors -> Display::fmt(SetError)

#### SetError Error::source

Returns the contained StateError.

##### Reference

    standard error-chain traversal -> Error::source(SetError)

## Private system-state representation

### StateLayout

Arc-owned immutable source path, ordered FieldSpec vector, and name-to-index
hash map. It has no methods.

### StateTemplate, FieldDeclaration, and StateTemplateRef

Private Serde representations for strict template decoding and borrowed
template encoding. Their Serialize and Deserialize behavior is derived rather
than manually implemented.

### ErasedValue

Private object-safe trait implemented for every Any + Clone + Send payload.

#### ErasedValue::clone_box

Deep-clones the concrete payload into a new erased box.

##### Reference

    StateValue::clone -> ErasedValue::clone_box

#### ErasedValue::as_any

Returns an immutable Any view.

##### Reference

    StateValue::type_id and StateValue::downcast_ref -> ErasedValue::as_any

#### ErasedValue::as_any_mut

Returns a mutable Any view.

##### Reference

    StateValue::downcast_mut -> ErasedValue::as_any_mut

#### ErasedValue::into_any

Converts the erased owner into Box<dyn Any + Send>.

##### Reference

    StateValue::downcast -> ErasedValue::into_any

#### ErasedValue::concrete_type_name

Returns the concrete Rust type name for diagnostics.

##### Reference

    StateValue::type_name -> ErasedValue::concrete_type_name

### StateValue

Private Box<dyn ErasedValue> wrapper. It is the only type-erased value stored
in SystemState slots.

#### StateValue::new

Consumes and erases a concrete payload without cloning it.

##### Reference

    SystemState::set success -> StateValue::new

#### StateValue::type_id

Returns the concrete TypeId.

##### Reference

    StateValue::is -> StateValue::type_id

#### StateValue::type_name

Returns the concrete Rust type name for diagnostics.

##### Reference

    SystemState::get, get_mut, and take mismatch reporting
    StateValue Debug -> StateValue::type_name

#### StateValue::is<T>

Tests exact concrete type identity.

##### Reference

    SystemState::is -> StateValue::is
    StateValue::downcast -> StateValue::is
    SystemState::set replacement validation

#### StateValue::downcast_ref<T>

Returns an immutable typed payload reference.

##### Reference

    SystemState::get -> StateValue::downcast_ref

#### StateValue::downcast_mut<T>

Returns a mutable typed payload reference.

##### Reference

    SystemState::get_mut -> StateValue::downcast_mut

#### StateValue::downcast<T>

Consumes the wrapper and moves out T, or returns the wrapper unchanged.

##### Reference

    SystemState::take -> StateValue::downcast
    SystemState::set same-type replacement -> recover previous T

#### StateValue::clone

Deep-clones the erased payload.

##### Reference

    SystemState::clone -> Vec<Option<StateValue>>::clone

#### StateValue::fmt

Formats only the concrete type name.

##### Reference

    internal diagnostics -> Debug::fmt(StateValue)

## Payload codecs

### CodecRegistry

Maps stable StateSpec type tags to concrete Serde-compatible Rust types. The
registry is open to arbitrary application types. Encoding borrows values and
does not create serde_json::Value. A payload's own Serialize implementation may
still allocate or copy internally.

The target registry does not estimate sizes because chunking is count-based.
Actual encoded length is known after serialization and may be used for queue
backpressure and file metadata.

#### CodecRegistry::new

Creates an empty registry.

##### Reference

    program initialization -> CodecRegistry::new
    tests -> isolated registry

#### CodecRegistry::register<T>

Associates one unused stable tag with T, requiring Serialize,
DeserializeOwned, Clone, Send, and 'static.

##### Reference

    application startup -> CodecRegistry::register
    stream initialization -> verify every StateSpec tag is registered
    tests -> scalar, collection, custom, and tensor codecs

#### CodecRegistry::contains

Reports whether a tag is registered.

##### Reference

    stream configuration validation -> CodecRegistry::contains
    callers and tests -> registry inspection

#### CodecRegistry::len

Returns registered-tag count.

##### Reference

    CodecRegistry Debug -> CodecRegistry::len
    diagnostics and tests -> CodecRegistry::len

#### CodecRegistry::is_empty

Reports whether no codecs are registered.

##### Reference

    initialization validation and tests -> CodecRegistry::is_empty

#### CodecRegistry::value

Crate-private lookup returning a borrowed erased Serialize view for a populated
SystemState field.

##### Reference

    JSON record encoder -> CodecRegistry::value

#### CodecRegistry::decode

Crate-private dynamic decode followed by ownership transfer into SystemState.

##### Reference

    JSON record decoder -> CodecRegistry::decode -> SystemState::set

#### CodecRegistry::insert

Private duplicate-checking typed codec insertion.

##### Reference

    CodecRegistry::register -> CodecRegistry::insert

#### CodecRegistry::get

Private exact-tag lookup returning MissingCodec when absent.

##### Reference

    CodecRegistry::value and decode -> CodecRegistry::get

#### CodecRegistry::fmt

Formats only the number of registered codecs.

##### Reference

    diagnostics and assertion failures -> Debug::fmt(CodecRegistry)

### ErasedCodec

Private object-safe codec interface behind CodecRegistry.

#### ErasedCodec::value

Validates the concrete payload type and returns a borrowed Serialize view.

##### Reference

    CodecRegistry::value -> ErasedCodec::value

#### ErasedCodec::decode

Deserializes one owned concrete value and inserts it into a state.

##### Reference

    CodecRegistry::decode -> ErasedCodec::decode

### TypedCodec<T>

Private implementation of ErasedCodec for one registered T.

#### TypedCodec::payload

Borrows T from a named field and converts a mismatch into CodecTypeMismatch.

##### Reference

    TypedCodec ErasedCodec::value -> TypedCodec::payload

#### TypedCodec ErasedCodec::value

Coerces a validated borrowed T into erased_serde::Serialize.

##### Reference

    CodecRegistry::value -> ErasedCodec::value -> TypedCodec::value

#### TypedCodec ErasedCodec::decode

Deserializes T and transfers it into SystemState.

##### Reference

    CodecRegistry::decode -> ErasedCodec::decode -> TypedCodec::decode

## StateSeries analysis interface

StateSeries is an owned growable array of decoded states for analysis:

    StateSeries
    ├── spec: StateSpec
    └── states: Vec<SystemState>

All states share the exact layout Arc and have strictly increasing indices.
Gaps are valid. StateSeries never performs IO, manages writer chunks, or
estimates encoded sizes.

### StateSeries

#### StateSeries::new

Creates an empty series with no reserved state capacity.

##### Reference

    analysis construction -> StateSeries::new
    SeriesReader collection -> StateSeries::new

#### StateSeries::with_capacity

Creates an empty series with reserved state-owner capacity.

##### Reference

    reader with known selected count -> StateSeries::with_capacity
    analysis batch construction -> StateSeries::with_capacity

#### StateSeries::spec

Returns the canonical shared specification.

##### Reference

    SeriesRef construction -> StateSeries::spec
    analysis and writer convenience validation -> StateSeries::spec

#### StateSeries::view

Creates a lightweight Copy read-only SeriesRef.

##### Reference

    analysis helper arguments -> StateSeries::view
    optional StateWriter::submit_series -> StateSeries::view
    callers avoiding deep clone -> StateSeries::view

#### StateSeries::len

Returns the state count.

##### Reference

    capacity/analysis checks and Debug -> StateSeries::len
    tests -> collection transitions

#### StateSeries::is_empty

Reports whether no states are stored.

##### Reference

    analysis control flow and tests -> StateSeries::is_empty

#### StateSeries::capacity

Returns Vec capacity.

##### Reference

    allocation tuning and tests -> StateSeries::capacity

#### StateSeries::reserve

Reserves space for additional state owners.

##### Reference

    reader and analysis preallocation -> StateSeries::reserve

#### StateSeries::get

Returns one immutable state by position.

##### Reference

    indexed analysis and tests -> StateSeries::get

#### StateSeries::get_mut

Planned narrow mutable access to one state. It permits payload mutation but not
slice reordering; SystemState time and spec remain immutable.

##### Reference

    analysis transform -> StateSeries::get_mut -> SystemState::get_mut

#### StateSeries::first

Returns the earliest state.

##### Reference

    ordering validation, range inspection, and Debug -> StateSeries::first

#### StateSeries::last

Returns the latest state.

##### Reference

    StateSeries::push ordering check -> StateSeries::last
    range inspection and Debug -> StateSeries::last

#### StateSeries::states

Returns an immutable contiguous slice.

##### Reference

    SeriesRef construction and analysis adapters -> StateSeries::states

#### StateSeries::iter

Returns an ordered borrowed iterator.

##### Reference

    borrowed IntoIterator -> StateSeries::iter
    analysis loops and writer convenience submission -> StateSeries::iter

#### StateSeries::push

Moves in one state after exact-layout and increasing-time validation. Failure
returns PushError with the unchanged state.

##### Reference

    SeriesReader eager collection -> StateSeries::push
    analysis series assembly -> StateSeries::push
    tests -> ownership and invariant checks

#### StateSeries::pop

Moves out the latest state.

##### Reference

    analysis stack-like processing and tests -> StateSeries::pop

#### StateSeries::clear

Drops all states while retaining Vec capacity and canonical specification.

##### Reference

    explicit analysis working-set reuse -> StateSeries::clear
    tests -> clear behavior

#### StateSeries::into_states

Consumes the series and returns its Vec without cloning payloads.

##### Reference

    ownership handoff to application analysis -> StateSeries::into_states
    tests -> allocation-preserving extraction

#### StateSeries::clone

Deep-clones every SystemState and payload. Use SeriesRef or Arc for lightweight
reference semantics.

##### Reference

    explicit independent analysis copy -> StateSeries::clone
    clone-cost tests

#### StateSeries::fmt

Formats schema source, state count, and index range without payload traversal.

##### Reference

    diagnostics and assertion failures -> Debug::fmt(StateSeries)

#### borrowed StateSeries::into_iter

Iterates immutable state references.

##### Reference

    for state in &series -> borrowed IntoIterator

#### owned StateSeries::into_iter

Consumes the series and moves out states.

##### Reference

    for state in series -> owned IntoIterator

### SeriesRef

Copy borrowed pair of StateSpec and state slice. It never clones payloads.

#### SeriesRef::new

Private invariant-preserving constructor.

##### Reference

    StateSeries::view -> SeriesRef::new

#### SeriesRef::spec

Returns the canonical borrowed specification.

##### Reference

    analysis helpers and writer convenience validation -> SeriesRef::spec

#### SeriesRef::len

Returns borrowed state count.

##### Reference

    analysis, Debug, and tests -> SeriesRef::len

#### SeriesRef::is_empty

Reports whether the view has no states.

##### Reference

    analysis control flow and tests -> SeriesRef::is_empty

#### SeriesRef::get

Returns one borrowed state by position.

##### Reference

    indexed read-only analysis and tests -> SeriesRef::get

#### SeriesRef::first

Returns the earliest borrowed state.

##### Reference

    range inspection and Debug -> SeriesRef::first

#### SeriesRef::last

Returns the latest borrowed state.

##### Reference

    range inspection and Debug -> SeriesRef::last

#### SeriesRef::states

Returns the complete borrowed slice.

##### Reference

    slice-based analysis adapters -> SeriesRef::states

#### SeriesRef::iter

Returns an ordered borrowed iterator.

##### Reference

    SeriesRef IntoIterator -> SeriesRef::iter
    analysis and writer convenience loops -> SeriesRef::iter

#### SeriesRef::fmt

Formats schema source, count, and index range without payload traversal.

##### Reference

    diagnostics and assertion failures -> Debug::fmt(SeriesRef)

#### SeriesRef::into_iter

Iterates borrowed states.

##### Reference

    for state in series.view() -> SeriesRef::into_iter

### PushError

Ownership-preserving StateSeries::push failure.

#### PushError::new

Private constructor combining SeriesError and rejected SystemState.

##### Reference

    StateSeries::push spec mismatch -> PushError::new
    StateSeries::push time-order failure -> PushError::new

#### PushError::error

Borrows the rejection reason.

##### Reference

    caller and tests -> PushError::error

#### PushError::state

Borrows the unchanged rejected state.

##### Reference

    caller and tests -> PushError::state

#### PushError::into_parts

Consumes the error and recovers SeriesError plus SystemState.

##### Reference

    caller recovery and tests -> PushError::into_parts

#### PushError Display::fmt

Delegates to SeriesError.

##### Reference

    logs and user-facing diagnostics -> Display::fmt(PushError)

#### PushError Error::source

Returns the contained SeriesError.

##### Reference

    standard error-chain traversal -> Error::source(PushError)

## Streaming persistence

### Logical streams

Different partial-state cadences or output paths are different streams. A
stream declares:

- unique name;
- StateSpec for its partial records;
- cadence description;
- relative output directory or filename prefix;
- states_per_chunk: NonZeroUsize;
- encoding and framing;
- independent monotonically increasing chunk ordinal.

Names such as signal and space are ordinary configured stream names, not
special concepts in the crate.

All streams must be declared before the first submission so metadata.json can
be written before simulation output begins. Paths are validated as relative,
non-colliding destinations beneath the run output root.

### Sampling boundary

The simulation owns and mutates its complete state. At a cadence boundary, a
stream selects and serializes only its fields. Serialization is synchronous
with respect to that borrowed data; submit returns only after the encoded
record no longer borrows the simulation.

The exact borrowed-field API remains the next design decision. It must:

- accept arbitrary registered serializable values by reference;
- validate field names, stable tags, and concrete Rust types;
- encode in deterministic schema order;
- avoid constructing an owned partial SystemState;
- produce exactly one complete encoded record or no record on failure.

An already-owned partial SystemState may be supported as a convenience source,
but it is not the required hot path.

### EncodedRecord

Private, non-Clone queue message containing one TimePoint and one complete
encoded field-record buffer. It is transport data, not a second public state
model.

EncodedRecord is indivisible. The queue moves it to one stream worker. Queue
capacity is byte-weighted, optionally with a record-count cap. A record larger
than the byte budget is accepted only when it is the sole pending record.

### Count-based chunks

Each StateWriter has states_per_chunk. Appending one complete EncodedRecord
increments the active record count once. The chunk is sealed when the count
reaches the configured limit.

All ordinary non-final chunks contain exactly states_per_chunk records. The
final chunk may be shorter. A durability operation may seal an underfilled
chunk only if its contract explicitly says so. Different streams count
independently.

For JSON, the initial framing is compact JSON Lines: one complete object plus
one newline per record. A future protobuf codec uses length-delimited messages.
Chunk files contain no schema header and no repeated metadata.

### StateWriter

One non-Clone authority for one logical stream. It owns configuration, queue
sender, worker lifecycle, chunk ordinal, active temporary file state, ordering
state, and terminal error state. Committed chunks are immutable.

#### StateWriter::submit

Borrows one partial sample, validates and serializes it, waits for queue
capacity, and moves one EncodedRecord to the worker. Success means accepted,
not durable.

##### Reference

    simulation cadence -> stream StateWriter::submit
        -> borrowed field encoding
        -> EncodedRecord
        -> bounded queue
        -> active chunk

#### StateWriter::submit_series

Optional convenience that borrows SeriesRef and submits every state through the
ordinary single-state path. It does not preserve the analysis vector as a
physical chunk.

##### Reference

    persist analysis result -> StateSeries::view
        -> StateWriter::submit_series
        -> StateWriter::submit for each state

#### StateWriter::flush

Waits for all records accepted before the call to satisfy the defined
durability boundary. Whether flush seals an underfilled chunk must be fixed
before implementation; it cannot be ambiguous.

##### Reference

    checkpoint or explicit durability boundary -> StateWriter::flush

#### StateWriter::finish

Rejects new submissions, drains the queue, commits the final non-empty chunk,
joins the worker, and returns final stream statistics or the terminal failure.

##### Reference

    successful run termination -> each StateWriter::finish
    run-level coordinator -> collect stream completion statistics

### Run-level writer coordinator

A run-level coordinator creates all StateWriter instances and owns the single
metadata.json lifecycle. Its final public name is not yet fixed; RunWriter is
used descriptively here.

#### RunWriter::new

Validates all stream definitions and paths, verifies required codecs, creates
the output directory, and atomically writes metadata.json before submissions.

##### Reference

    simulation initialization -> RunWriter::new

#### RunWriter::writer

Returns the StateWriter for an exact configured stream name.

##### Reference

    cadence setup -> RunWriter::writer("signal")
    cadence setup -> RunWriter::writer("space")

#### RunWriter::finish

Finishes every stream and performs the sole run-level completion transition.

##### Reference

    successful simulation termination -> RunWriter::finish

## Storage format

### Directory

    output/
    ├── metadata.json
    ├── signal/
    │   ├── chunk-000000.jsonl
    │   └── chunk-000001.jsonl
    └── space/
        ├── chunk-000000.jsonl
        └── chunk-000001.jsonl

Temporary chunk and metadata files use non-committed names and become visible
through atomic rename. Generated data and Cargo target directories are ignored
by Git.

### metadata.json

Before accepting records, the writer records all information already known:

- format name and version;
- time-axis definition and units;
- resolved run metadata supplied by the caller;
- every stream name and complete StateSpec;
- cadence description;
- relative path and deterministic chunk naming;
- encoding and framing;
- states_per_chunk.

No other metadata file exists in the output directory. Per-record field keys
remain in JSON for readability.

Final chunk count, exact lengths, checksums, final time, completion, and failure
are not knowable at startup. The sole-file rule allows metadata.json to be
atomically replaced later, but the exact dynamic metadata policy remains open.
If the file is immutable after startup, readers must discover deterministic
chunk names and cannot infer authoritative successful completion.

### JSON record

Conceptual compact record:

    {"time":{"index":42,"physical":0.25},"values":{"population":[1,2,3]}}

Only populated fields appear. The stream schema in metadata defines valid keys
and codec tags. One newline terminates the complete object.

## Reading and analysis

### SeriesReader

A reader opens metadata.json, validates its version and stream declarations,
discovers/selects chunks, and decodes complete records lazily. It never requires
the full persisted series in memory.

#### SeriesReader::open

Loads and validates metadata.json and prepares codec-backed stream access.

##### Reference

    analysis program startup -> SeriesReader::open

#### SeriesReader::states

Returns a lazy decoded stream for one logical stream and optional time range.

##### Reference

    streaming analysis -> SeriesReader::states -> one SystemState at a time
    eager analysis -> SeriesReader::states -> collect into StateSeries

#### SeriesReader::read_series

Optional eager convenience that collects a selected range into StateSeries.

##### Reference

    bounded analysis request -> SeriesReader::read_series
        -> SeriesReader::states
        -> StateSeries::push

### SeriesError

Non-exhaustive public error enum spanning series invariants, codec registration
and type mismatches, record encoding/decoding, format version, metadata and
chunk validation, missing files, byte-length mismatch, filesystem/JSON errors,
wrapped StateError, and terminal writer lifecycle.

All metadata-related variants refer to metadata.json. Chunk byte length is an
integrity fact, not a chunking policy.

## Clone and concurrency policy

- FieldSpec: cheap metadata clone.
- StateSpec: cheap Arc clone.
- TimePoint: Copy.
- SystemState: deep payload clone; avoid on hot paths.
- StateSeries: deep clone of every state; use SeriesRef or Arc instead.
- SeriesRef: Copy borrowed view.
- CodecRegistry: normally built once and shared by Arc.
- EncodedRecord: moved, not cloned.
- StateWriter and RunWriter: non-Clone exclusive lifecycle authorities.
- Metadata transaction and active chunk: non-Clone.

The normal persistence path borrows live values during encoding and then moves
encoded buffers. No deep state clone is used.

## Scientific tensor compatibility

The core API is tensor-library agnostic. Any type satisfying the state and codec
bounds can be stored and registered.

The current physics-in-parallel tensor and SquareLattice serialization creates
intermediate owned payload data through to_vec. CodecRegistry borrowing cannot
remove a copy performed inside that external Serialize implementation. A later
integration stage must add a borrowed serializer or update that crate before
claiming end-to-end copy-free tensor encoding.

Zero-copy guarantees therefore apply precisely to:

- moving owned payloads into and out of SystemState;
- borrowing payloads through get and get_mut;
- moving SystemState values into analysis collections;
- borrowing payloads at the registry boundary.

They do not claim that JSON or every third-party Serialize implementation can
encode without allocating or copying.

## Future dispatcher layer

Dispatcher work follows persistence and remains a separate module. It will:

- read fixed.json constants;
- expand sweep.json parameter products;
- create deterministic run and task identities;
- provide scoped paths, logging, and execution contexts;
- initialize run writers with resolved parameters and declared streams;
- record lifecycle without embedding domain-specific mission enums.

The dispatcher does not own scientific payloads and does not alter SystemState
or StateWriter ownership rules.

## Implementation delta

The following source changes are required to match this document. They are not
authorized by this documentation cleanup and must be reviewed one production
file at a time.

1. system_state/error.rs contains SetError, but system_state.rs does not export
   it and state.rs does not yet use it.
2. SystemState::set still returns Result<(), StateError>, drops a rejected
   incoming payload, and drops a displaced payload. It must adopt the documented
   Result<Option<T>, SetError<T>> contract.
3. time_series is not exported from lib.rs.
4. StateSeries lacks get_mut.
5. StateSeries len, is_empty, and capacity currently use const Vec methods newer
   than the crate's Rust 1.85 MSRV; const must be removed or MSRV reconsidered.
6. PushError stores SystemState inline and currently triggers Clippy's
   result_large_err diagnostic.
7. StateChunk and StateSeries::into_chunk implement the rejected writer-buffer
   architecture and must be removed.
8. CodecRegistry::register_with_size, CodecRegistry::estimate, SizeEstimator,
   and ErasedCodec::estimate exist for rejected byte-based chunking and must be
   removed unless a separate non-chunking use case is approved.
9. SeriesError documentation and variants still refer to series.json rather
   than metadata.json.
10. format.rs, writer.rs, reader.rs, borrowed partial-state encoding,
    count-based chunks, queue backpressure, and metadata lifecycle are not yet
    implemented.

### Transitional APIs scheduled for removal

These source items are documented here only so their current references are not
ambiguous. They are not part of the target architecture.

### SizeEstimator<T>

Obsolete private callback alias returning an estimated byte count from a
borrowed T.

It has no methods. CodecRegistry::register_with_size stores it and
TypedCodec::estimate invokes it. Count-based chunking removes both uses.

#### CodecRegistry::register_with_size

Registers a codec plus a size estimator.

##### Reference

    current codec tests -> CodecRegistry::register_with_size
    current CodecRegistry::insert
    target architecture -> remove

#### CodecRegistry::estimate

Runs a registered estimate for a field.

##### Reference

    current codec tests -> CodecRegistry::estimate
    target architecture -> remove

#### ErasedCodec::estimate

Object-safe estimator dispatch.

##### Reference

    CodecRegistry::estimate -> ErasedCodec::estimate
    target architecture -> remove

#### TypedCodec ErasedCodec::estimate

Validates T and runs the optional estimator.

##### Reference

    ErasedCodec::estimate -> TypedCodec::estimate
    target architecture -> remove

#### StateSeries::into_chunk

Moves an analysis series into the obsolete StateChunk wrapper.

##### Reference

    current series tests and future-unused writer hook -> StateSeries::into_chunk
    target architecture -> remove

### StateChunk

Obsolete owner of ordinal, StateSeries, and estimated bytes. Streaming writers
queue EncodedRecord values instead.

#### StateChunk::new

Constructs the obsolete wrapper.

##### Reference

    StateSeries::into_chunk -> StateChunk::new
    target architecture -> remove

#### StateChunk::ordinal

Returns its ordinal.

##### Reference

    current series tests -> StateChunk::ordinal
    target architecture -> remove

#### StateChunk::spec

Delegates to the wrapped series.

##### Reference

    current series tests -> StateChunk::spec
    target architecture -> remove

#### StateChunk::view

Delegates to StateSeries::view.

##### Reference

    current series tests -> StateChunk::view
    target architecture -> remove

#### StateChunk::len

Delegates to StateSeries::len.

##### Reference

    current tests and Debug -> StateChunk::len
    target architecture -> remove

#### StateChunk::is_empty

Delegates to StateSeries::is_empty.

##### Reference

    current series tests -> StateChunk::is_empty
    target architecture -> remove

#### StateChunk::estimated_bytes

Returns the obsolete rollover estimate.

##### Reference

    current series tests and Debug -> StateChunk::estimated_bytes
    target architecture -> remove

#### StateChunk::first_index

Returns the first wrapped state index.

##### Reference

    current series tests and Debug -> StateChunk::first_index
    target architecture -> remove

#### StateChunk::last_index

Returns the last wrapped state index.

##### Reference

    current series tests and Debug -> StateChunk::last_index
    target architecture -> remove

#### StateChunk::get

Delegates indexed access to StateSeries.

##### Reference

    current series tests -> StateChunk::get
    target architecture -> remove

#### StateChunk::states

Delegates slice access to StateSeries.

##### Reference

    current series tests -> StateChunk::states
    target architecture -> remove

#### StateChunk::iter

Delegates borrowed iteration to StateSeries.

##### Reference

    current borrowed IntoIterator and tests -> StateChunk::iter
    target architecture -> remove

#### StateChunk::into_series

Recovers the wrapped analysis series.

##### Reference

    current series tests -> StateChunk::into_series
    target architecture -> remove

#### StateChunk::fmt

Formats obsolete chunk context.

##### Reference

    current diagnostics and tests -> Debug::fmt(StateChunk)
    target architecture -> remove

#### borrowed StateChunk::into_iter

Iterates wrapped states.

##### Reference

    current for state in &chunk -> borrowed IntoIterator
    target architecture -> remove

## Verification and review

Source files contain thorough Rustdoc and no embedded test modules. Test layout:

    tests/
    ├── fixtures/
    ├── system_state.rs
    ├── system_state/
    │   ├── error.rs
    │   ├── spec.rs
    │   ├── state.rs
    │   └── value.rs
    ├── time_series.rs
    └── time_series/
        ├── error.rs
        ├── codec.rs
        └── series.rs

Single-module tests mirror the source filename. Tests spanning modules use a
concise integration name. The package README owns user-facing test commands.

Before each reviewed source change:

1. update this document with the intended contract;
2. edit one production file;
3. add or update the corresponding dedicated tests;
4. run focused tests, full tests, rustfmt checks, and Clippy proportionate to
   the change;
5. wait for review before moving to the next production file.

## Next design decision

Before implementing format.rs or writer.rs, define the borrowed partial-state
submission surface. It is the boundary that determines whether a simulation can
serialize selected live fields without copying them into an owned snapshot.
After that decision, reconcile the staged SystemState::set and StateSeries APIs,
then implement record format, writer, and reader in that order.
