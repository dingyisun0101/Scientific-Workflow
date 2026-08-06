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

The implemented crate exports `system_state` and `time_series`. Persistent JSON
storage is the next module stage.

## Architecture

The runtime and analysis paths are deliberately different:

    simulation-owned evolving full state
        -> cadence selects one logical output stream
        -> selected fields are serialized while borrowed
        -> one owned encoded record enters a bounded queue
        -> StateWriter appends complete records
        -> exact encoded-byte rollover commits immutable chunks

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
3. An owning simulation may mutate every payload and the complete TimePoint;
   StateSpec remains immutable.
4. Moving payloads into and out of SystemState does not clone them.
5. SystemState::clone creates a new erased box and calls T::clone for every
   populated payload; concrete Clone semantics remain T-defined.
6. Serialization borrows payloads but necessarily produces encoded bytes.
7. Each logical output stream has one StateSpec, cadence, path, writer, and
   independent chunk sequence.
8. One encoded partial state is one indivisible record.
9. Chunk boundaries are determined by exact encoded file bytes while records
   remain indivisible.
10. Every stream queue has finite record and encoded-byte capacity; submission
    blocks under backpressure instead of growing memory without bound.
11. No payload, field, or JSON record is split across chunks.
12. Structural metadata is stored once in output-root metadata.json.
13. Chunk files contain only minimally framed records.

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
    ├── time_series/
    │   ├── error.rs
    │   └── series.rs
    ├── storage.rs
    └── storage/
        ├── error.rs
        ├── format.rs
        ├── encoder.rs
        ├── writer.rs
        ├── decoder.rs
        └── reader.rs

Module-name files are preferred over mod.rs. Files named core.rs are avoided:
system_state::state and time_series::series are clearer in diagnostics and
documentation.

Module ownership is strict. `system_state` defines only the live in-memory
layout, time point, owned payload slots, typed access, cloning, and a private
format-agnostic erased view of each payload's own Serialize implementation. It
does not know about JSON, directories, metadata, streams, chunks, records, or
reconstruction. `time_series` owns only the in-memory StateSeries analysis
collection and its collection errors. It performs no serialization or IO.
`storage` owns JSON encoding, metadata, directory readers, StateDecoder,
DecodedRun, queues, chunks, and disk writers. Dependencies point from storage
to system_state and time_series, and from time_series to system_state; neither
lower-level module depends on storage.

### File responsibilities

- `src/lib.rs`: crate documentation and public module exports; it contains no
  implementation logic.
- `src/system_state.rs`: public system_state facade, workflow documentation,
  private submodule declarations, and public type re-exports.
- `src/system_state/spec.rs`: strict key-only JSON template parsing,
  normalization, immutable Arc-shared layouts, field lookup, and template
  round-trip output.
- `src/system_state/state.rs`: TimePoint and the public owned SystemState typed
  dictionary, time mutation, payload ownership operations, cloning, and the
  crate-private serialization-view boundary.
- `src/system_state/value.rs`: private boxed type erasure, downcasting,
  per-payload cloning, diagnostics, and erased-serde borrowing.
- `src/system_state/error.rs`: StateError and ownership-preserving SetError;
  no storage errors belong here.
- `src/time_series.rs`: public in-memory analysis facade only.
- `src/time_series/series.rs`: StateSeries and borrowed series views.
- `src/time_series/error.rs`: collection layout and ordering errors only.
- `src/storage.rs`: public JSON storage facade.
- `src/storage/format.rs`: metadata and JSONL record representations and
  validation.
- `src/storage/encoder.rs`: borrowed payload-to-EncodedRecord JSON encoding.
- `src/storage/writer.rs`: bounded queues, byte-targeted chunks, atomic files,
  RunOutput, and writer lifecycle.
- `src/storage/reader.rs`: metadata/chunk discovery and raw JSON record reading.
- `src/storage/decoder.rs`: key-typed StateDecoder and DecodedRun reconstruction.
- `src/storage/error.rs`: JSON, metadata, directory, chunk, queue, writer,
  reader, and decoder errors.

Tests mirror those production scopes under `tests/system_state/`,
`tests/time_series/`, and `tests/storage/`. Each top-level test file is the
Cargo integration entry point for its subdirectory; `tests/fixtures/state.json`
is the real key-only system-state template shared by public integration tests.

## System-state model

### Template

The first layout is loaded from a strict JSON template that describes only the
keys in the simulation-owned dictionary. Each field may carry an optional
human-facing description:

    {
      "fields": [
        {
          "name": "population",
          "description": "Population count for each simulated region"
        },
        {
          "name": "space",
          "description": "Current spatial lattice"
        }
      ]
    }

Array order defines compact slot indices. Field names are trimmed; empty and
duplicate names and unknown JSON properties are rejected. `description` may be
absent or null. Present descriptions are trimmed, and an empty or whitespace-
only description normalizes to absence. A description is documentation only:
it is never interpreted by the crate and cannot choose a decoder. `to_json`
omits absent descriptions, so round trips preserve normalized meaning rather
than insignificant nulls or whitespace. The state template deliberately
contains no Rust type, TypeId, codec, encoding, or persisted schema identifier.

JSON persistence uses Serde itself as the single universal codec. Every stored
payload satisfies Serialize, and the private erased-value wrapper exposes that
implementation through erased-serde. The writer can therefore borrow and
serialize any populated field without a registry, a schema ID, a per-type
adapter, or an intermediate serde_json::Value.

Deserialization is necessarily typed, but the type is supplied only at the
point where analysis requests a value:

    let lattice = record.decode::<SquareLattice<u64>>("space")?;

Serde constructs T directly from that field's JSON. The application does not
register T and does not implement a custom codec. A reader cannot automatically
reconstruct an entire heterogeneous SystemState because JSON and a natural-
language description do not identify Rust types. Analysis APIs must therefore
support typed field decoding or a caller-defined typed projection instead of
pretending the file contains enough information for automatic reconstruction.

The sole metadata file records field keys and descriptions, stream membership,
time information, chunk ordering, and the encoding/version. Lean chunk records
retain field keys but repeat no descriptions or type metadata.

### Ownership

SystemState stores heterogeneous values behind private boxes because a Vec
requires one statically sized element type. Boxing is not exposed publicly.
set consumes a concrete value; take returns the same value and backing
allocation. The box allocation and movement of a small owner are not copies of
the scientific buffer.

All stored values implement Serialize + Clone + Send + 'static. The 'static
bound supplies Any runtime identity. Serialize is the payload's own format-
agnostic implementation; SystemState itself does not choose JSON or implement
storage. Clone is required only so SystemState can clone every populated value.
The crate invokes T::clone for each payload and never shares the erased box;
types with internally shared owners such as Arc retain the semantics of their
own Clone implementation. Send allows an owned state to cross a thread
boundary. Sync is not required.

### Partial states

Empty slots are valid. A stream specification may describe only a subset of a
simulation's full state, and different streams may use different
specifications. The writer-side borrowed sampling API is not finalized; it
must serialize selected live fields without constructing an owned partial state
that copies data needed by the simulation.

### Simulation ownership model

The primary runtime model is that a simulation directly owns one evolving
SystemState containing its complete scientific state. Evolution mutates
payloads in place through get_mut, replaces or extracts owners through set and
take when required, and advances the state's coordinate through set_time or
advance. StateSpec remains fixed for the lifetime of that state.

    simulation
        -> owns mutable SystemState
        -> mutates payloads during evolution
        -> advances TimePoint
        -> lends selected fields to a cadence-specific JsonEncoder
        -> resumes mutation after synchronous encoding completes

StateSeries is not involved in this loop. It remains a decoded analysis
container. SystemState::clone is also not part of ordinary evolution or
sampling because it invokes Clone for all populated payloads.

Ephemeral solver caches, thread pools, random-number engines, and control
objects need not be stored in SystemState unless they are scientifically part
of the state or must be persisted. The fixed template describes the evolving
scientific data, not every implementation detail owned by the simulation.

### Runtime value access

The owning simulation addresses a field by its template key and supplies the
exact concrete Rust type:

    let mass = state.get::<f64>("mass")?;

    *state.get_mut::<f64>("mass")? += mass_delta;

    let lattice = state.get_mut::<SquareLattice<u64>>("space")?;
    lattice.evolve(...);

get returns &T and get_mut returns &mut T pointing directly into the stored
payload. Neither operation clones, serializes, or replaces the value. set moves
in a new owner and returns any displaced same-typed owner; take moves the owner
out; clear drops it.

The exact type is required. An unknown key, empty slot, or different concrete
type returns StateError and leaves the state unchanged.

#### Sequential mutable field borrowing

The current get_mut API borrows the complete SystemState, so ordinary Rust
borrowing cannot retain mutable references to two independently stored fields
at once. This is the accepted public contract: callers mutate one field at a
time and allow that borrow to end before accessing another. Sequential mutation
is direct and efficient, and Copy values may be read out before mutating a
different field. Data that intrinsically requires simultaneous mutable access
belongs in one aggregate domain payload. SystemState will not add get2_mut or
other multi-field borrowing variants.

## Implementation-ready system-state refactor

This section is the execution contract for the next code stage. It supersedes
all transitional type-tag and codec assumptions still present in the current
source. No time_series or storage source is changed while these files are
reviewed one at a time.

### Final public contract

- The only public initial-construction path is
  `StateSpec::load(path) -> StateSpec::empty(time) -> SystemState`.
  `SystemState::new` remains crate-private.
- A template declares ordered keys and optional descriptions only. It never
  declares Rust types or storage codecs.
- Slots may be empty. Layout, ordering, names, descriptions, and lookup indices
  are immutable and shared through Arc.
- `set<T>` is the sole payload-entry boundary and requires
  `T: Serialize + Clone + Send + 'static`.
- `get`, `get_mut`, `is`, and `take` continue to use exact runtime type identity.
- `get_mut` permits one mutable field borrow at a time. No multi-field borrowing
  API is added.
- `set` and `take` preserve ownership and backing allocations; replacement
  returns the previous T. No hot-path method calls Clone.
- `SystemState::clone` allocates a new erased box and invokes Clone for each
  populated T. Callers are prominently warned about cost and about T-defined
  Clone semantics.
- SystemState does not implement JSON encoding. A crate-private borrowed erased
  Serialize view is its only storage-facing hook.
- StateError contains only state/template/time failures. JSON record encoding,
  directory, chunk, and decoder failures belong to storage errors.

### Template normalization

The accepted declaration is:

    {
      "name": "space",
      "description": "Current spatial lattice"
    }

`name` is required, trimmed, and must be non-empty and unique after trimming.
`description` is optional; absent, null, empty, and whitespace-only values all
normalize to None. A non-empty value is trimmed and stored as Box<str>.
Unknown properties at the document and field levels are rejected. Field array
order determines indices. Empty field arrays remain valid. `to_json` emits the
normalized strict representation, omits absent descriptions, indices, and
source paths, and preserves declaration order.

### Erased payload contract

The private ErasedValue blanket implementation has the exact concrete bounds:

    T: Serialize + Clone + Send + 'static

It supplies clone_box, Any borrowed/mutable/owned views, diagnostic type name,
and as_serialize. `as_serialize` performs an explicit coercion from concrete T
to `&dyn erased_serde::Serialize`; it does not rely on trait-object upcasting.
StateValue forwards this through `serializable`. SystemState forwards it through
the crate-private `serializable(key)` accessor after ordinary unknown/missing
field validation. No public erased type escapes the module.

### Error cleanup

`EmptyTypeTag` is removed after spec.rs stops referencing it.
`FieldCountMismatch` is removed because no public or crate-private constructor
accepts a caller-provided slot vector: StateDecoder reconstructs through
StateSpec::empty and SystemState::set. No description-specific error is needed
because empty descriptions normalize to absence.

### Test contract

No test is placed inside a production module. Focused suites remain under
`tests/system_state/` and mirror the production filename. The top-level
`tests/system_state.rs` integration target includes those focused suites so
Cargo, not direct rustc commands, supplies serde and erased-serde dependencies.

The completed refactor must test:

- actual fixture loading with descriptions and without type tags;
- absent, null, empty, whitespace-only, trimmed, duplicate-name, empty-name,
  unknown-property, malformed-JSON, and empty-template behavior;
- normalized `to_json` output and explicit semantic round-trip equality;
- deterministic indices/lookups and Arc identity versus independent loads;
- erased serialization of scalar, collection, custom, and tensor payloads
  without Clone calls or serde_json::Value;
- insertion, same-type replacement, mismatch rejection, get/get_mut, take,
  clear, clear_all, empty derivation, time mutation, and error transactionality;
- pointer/capacity preservation across set/take and rejected ownership recovery;
- one-per-payload Clone invocation for explicit SystemState cloning;
- bounded Debug output and precise Error::source behavior;
- the public downstream workflow using the actual JSON fixture and a
  physics_in_parallel tensor that satisfies the new Serialize bound.

### One-file review order

The implementation proceeds in this exact order. A file is reviewed before the
next item begins; required later changes remain in todo.md.

1. `src/system_state/spec.rs` — remove type tags, add descriptions, normalize
   templates, and update internal documentation.
2. `tests/fixtures/state.json` — convert the real fixture to keys plus useful
   descriptions.
3. `tests/system_state/spec.rs` — replace type-tag assertions with the complete
   template-normalization and round-trip matrix.
4. `src/system_state/error.rs` — remove obsolete EmptyTypeTag and
   FieldCountMismatch while preserving SetError and time errors.
5. `tests/system_state/error.rs` — align the focused error contract.
6. `src/system_state/value.rs` — add the Serialize bound and borrowed erased
   serialization view without altering ownership/downcast behavior.
7. `tests/system_state/value.rs` — prove erased JSON serialization borrows the
   original value and never invokes Clone.
8. `src/system_state/state.rs` — enforce Serialize in set and add the
   crate-private serializable accessor; keep the public mutation API unchanged.
9. `tests/system_state/state.rs` — update test payloads to Serialize and cover
   the new internal borrowing hook alongside all ownership/time contracts.
10. `src/system_state.rs` — rewrite facade documentation and examples around
    key-only templates and serializable payloads.
11. `tests/system_state.rs` — include the focused suites and update the public
    real-tensor integration workflow.
12. `src/lib.rs` — update crate-level module boundaries and examples without
    exporting unfinished time_series or storage modules.

Breaking edits can temporarily invalidate later tests or the transitional
time_series codec. This is expected under the one-file rule and is recorded in
todo.md; no downstream source is patched early merely to keep a staged
intermediate tree green.

### System-state verification gate

After item 12, run from `dev/`:

    cargo fmt --check
    cargo check --lib
    cargo test --test system_state
    cargo test --doc
    cargo clippy --lib --test system_state -- -D warnings

The full `cargo test` gate resumes only after the obsolete time_series codec
and its direct tests are removed in their own stage. Until then, system-state
verification deliberately selects the system_state integration target so the
downstream-file rule remains intact.

### Verified implementation status

The complete system_state refactor now implements this contract. Focused Cargo
test results are: 6 specification tests, 4 error tests, 7 erased-value tests,
and 15 state tests. The joint `tests/system_state.rs` target runs those 32 tests
plus one public physics_in_parallel tensor workflow, for 33 passing tests.
Three Rust doctests also pass. `cargo fmt --check`, `cargo check --lib`, and
Clippy over the library and joint system_state target with warnings denied all
pass.

The crate-private serialization view carries a temporary justified dead-code
allowance because the storage module is intentionally not implemented early.
That allowance is removed when JsonEncoder becomes its first production caller.

### SystemState completion boundary

No functional work remains inside `src/system_state/`. Its specification,
errors, erased payload ownership, mutable typed access, time handling, borrowed
serialization boundary, public facade, fixtures, and focused/joint tests are
complete. The only deferred source cleanup is removal of three justified
`dead_code` allowances on the serialization boundary; that happens when
`storage::JsonEncoder` becomes their production caller and is therefore storage
work, not another SystemState redesign. Reader reconstruction through the
crate-private `StateSpec::parse` is likewise implemented later by
`storage::reader` without changing the SystemState contract.

## Public system-state API

The method catalog covers every explicitly declared production method in the
current or target source. Compiler-generated derives such as Debug, Clone,
Serialize, and Default are recorded as type properties unless their cost or
behavior changes an architectural contract.

### FieldSpec

One immutable normalized field declaration. Fields are stored in template
order and carry a compact slot index, human-facing key, and optional
human-facing description. They carry no persisted or Rust type information.

#### FieldSpec::new

Private constructor that stores a validated normalized name, optional
description, and template-order index.

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
    JSON encoder -> output field key
    stream definition -> state-key selection

#### FieldSpec::description

Returns the optional natural-language payload description. The return value is
documentation only and has no effect on runtime typing or serialization.

##### Reference

    schema inspection and generated documentation -> FieldSpec::description
    metadata writer -> optional human-facing field documentation

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

#### StateSpec::parse

Crate-private parser for a borrowed JSON template document. It applies the same
strict parsing and semantic validation as load while recording a caller-supplied
metadata path as provenance. This is the persistence-reader reconstruction
boundary; public application initialization remains path-only.

##### Reference

    StateSpec::load -> StateSpec::parse
    storage reader embedded template -> StateSpec::parse
    focused specification tests -> StateSpec::parse

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
    storage exact-spec sampling validation -> StateSpec::shares_layout
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
    storage reader and encoder setup -> share state layout

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

Returns the current TimePoint by value.

##### Reference

    StateSeries ordering and errors -> SystemState::time
    encoder -> record time
    analysis -> state coordinate

#### SystemState::set_time

Replaces the complete TimePoint and returns the previous value. Replacing one
validated TimePoint at once keeps physical-time validation inside TimePoint
construction and makes ownership-side time changes explicit.

##### Reference

    simulation initialization or discontinuous time change
        -> SystemState::set_time
        -> previous TimePoint

    checkpoint restore -> SystemState::set_time

#### SystemState::advance

Atomically increments the integer index by one and optionally adds a finite
delta to an existing physical coordinate. The proposed signature is:

    advance(physical_delta: Option<f64>) -> Result<TimePoint, StateError>

Success returns the new TimePoint. None increments only the integer index and
preserves physical time. Some(delta) requires delta and the resulting physical
coordinate to be finite. Integer overflow or invalid physical arithmetic leaves
the original TimePoint unchanged.

The recommended meaning of Some(delta) when physical time is currently absent
is an error, not an implicit zero origin. A caller that knows the physical
origin must establish it explicitly through set_time before advancing it.
Negative finite deltas remain valid because the integer index, not physical
time, defines ordering.

##### Reference

    simulation step without physical coordinate
        -> SystemState::advance(None)

    simulation step with physical timestep
        -> SystemState::advance(Some(dt))
        -> updated TimePoint

    failed checked arithmetic
        -> SystemState::advance
        -> StateError
        -> original TimePoint unchanged

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
remain prominent in API documentation. T must implement Serialize + Clone +
Send + 'static. This is the only public operation that introduces a new
concrete payload, so it is the single enforcement point for the complete stored
payload contract.

##### Reference

    simulation and analysis -> SystemState::set
    tests -> insertion, replacement, rejection, and ownership recovery

#### SystemState::get<T>

Borrows a populated field as exactly T.

##### Reference

    application and analysis -> SystemState::get
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

#### SystemState::serializable

Crate-private, format-agnostic accessor returning a populated field as
`&dyn erased_serde::Serialize`. It delegates lookup and missing-value errors to
SystemState::value. It does not allocate, clone, encode, or expose StateValue.

##### Reference

    storage::JsonEncoder field traversal -> SystemState::serializable
        -> StateValue::serializable

#### SystemState::clone

Shares StateSpec and invokes T::clone once for every populated payload, creating
a new erased box for each. It never aliases StateValue boxes. The semantic depth
of a concrete payload clone is defined by that type's Clone implementation.
This operation is potentially extremely expensive and is never the normal
persistence path.

##### Reference

    explicit caller request for independent state -> SystemState::clone
    StateSeries::clone -> SystemState::clone for every state
    clone contract tests

#### SystemState::fmt

Debug output contains only time, template source, field count, and loaded count.

##### Reference

    diagnostics and assertion failures -> Debug::fmt(SystemState)

### StateError

Non-exhaustive public error enum for template IO/parsing, empty or duplicate
field names, unknown or empty fields, and Rust type mismatches. It also
represents checked time-advance failures:
TimeIndexOverflow records the current u64 index, MissingPhysicalTime rejects an
implicit physical origin, and InvalidPhysicalAdvance records the current
coordinate and rejected delta. Time errors are detected before mutation.
Wrapped IO and JSON errors remain available through Error::source.

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
template encoding. FieldDeclaration contains `name: String` and
`description: Option<String>` with unknown properties denied. StateTemplateRef
borrows normalized FieldSpec values and omits absent descriptions. Their
Serialize and Deserialize behavior is derived rather than manually implemented.

### ErasedValue

Private object-safe trait implemented for every Serialize + Clone + Send +
'static payload.

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

#### ErasedValue::as_serialize

Returns the concrete payload coerced to `&dyn erased_serde::Serialize` without
trait-object upcasting. This explicit method remains compatible with the
crate's Rust 1.85 MSRV.

##### Reference

    StateValue::serializable -> ErasedValue::as_serialize

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

#### StateValue::serializable

Returns a borrowed erased-serde view of the original concrete payload. It does
not serialize by itself and introduces no intermediate value.

##### Reference

    SystemState::serializable -> StateValue::serializable

#### StateValue::clone

Deep-clones the erased payload.

##### Reference

    SystemState::clone -> Vec<Option<StateValue>>::clone

#### StateValue::fmt

Formats only the concrete type name.

##### Reference

    internal diagnostics -> Debug::fmt(StateValue)

## Payload serialization

### Erased serialization

StateValue privately combines runtime type erasure, per-payload cloning, and a
borrowed erased-serde view. SystemState::set accepts any concrete T satisfying
Serialize + Clone + Send + 'static. JsonEncoder asks each populated value for
that view while borrowing the original payload. This is one internal blanket
mechanism, not a user-visible codec.

The method chain is:

    JsonEncoder -> SystemState::serializable
        -> StateValue::serializable
        -> ErasedValue::as_serialize

JsonEncoder, not the erased-value method, supplies the JSON serializer.

### Typed decoding

JSON readers preserve record boundaries and field names. They deserialize a
field only when the analysis caller supplies T. T must implement
DeserializeOwned; it need not be registered globally. A wrong requested type
returns the ordinary Serde decoding error with stream, state, and field context.

#### SerializedRecord::decode<T>

Deserializes one named JSON payload directly into caller-selected T. The exact
record representation and final method name are deferred until the reader file
is designed; this entry defines the required behavior, not a committed public
type name.

##### Reference

    analysis field access -> SerializedRecord::decode
    typed analysis projection -> SerializedRecord::decode

Only JSON encoding is in scope. No alternate binary encoding or related
extension point is part of the present architecture.

### End-to-end JSON workflow

The simulation loads one key-only template, creates its live state, and moves
ordinary Serde payloads into it. The following uses target writer/reader names
to demonstrate responsibilities; their exact signatures are finalized only
when their source files are reviewed.

    let spec = StateSpec::load("state.json")?;
    let mut state = spec.empty(TimePoint::from_physical(0, 0.0).unwrap());

    state.set("signal", Signal::initial())?;
    state.set("space", Space::initial())?;

The run output is configured once with named streams, selected keys, maximum
chunk file sizes, and per-stream encoded-byte queue limits. The library fixes
the independent record-count bound at 1,024. Construction validates all keys
and writes the sole metadata.json before any record is accepted.

    let mut output = RunOutput::builder("output/run-001", state.spec())
        .stream("signal", ["signal"], ByteSize::mib(64))?
        .stream("space", ["space"], ByteSize::mib(512))?
        .build()?;

The simulation mutates one live field at a time. It decides sampling cadence;
the writer does not control evolution.

    for _ in 0..steps {
        state.get_mut::<Space>("space")?.evolve();
        state.get_mut::<Signal>("signal")?.measure();
        state.advance(Some(dt))?;

        if state.time().index() % 10 == 0 {
            output.sample("signal", &state)?;
        }
        if state.time().index() % 100 == 0 {
            output.sample("space", &state)?;
        }
    }
    output.finish()?;

sample asks the stream's JsonEncoder to borrow and encode only the configured
fields, then hands the complete EncodedRecord to StateWriter. After sample
returns, the state is no longer borrowed and simulation may continue.
StateWriter never sees a payload type and never invokes Serialize; it only
queues and appends encoded bytes. The background worker seals a chunk only
after its configured number of records and never splits a state.

Analysis opens metadata and chunk files without a type registry. The concrete
type is supplied only for a requested payload:

    let run = RunReader::open("output/run-001")?;
    for record in run.stream("space")? {
        let time = record.time();
        let space = record.decode::<Space>("space")?;
        analyze(time, space);
    }

decode invokes Space's existing Deserialize implementation directly. A wrong
T is a contextual JSON decoding error. Reading timestamps, inspecting keys, or
skipping records requires no payload deserialization.

## State decoder module

`storage::decoder` provides one public facade for turning an output
directory back into in-memory states. StateDecoder is simply a collection of
field decoders keyed by the same names used by SystemState. The user declares
that mapping once; StateDecoder handles metadata loading, stream discovery,
chunk ordering, record parsing, decoder selection, and state construction.
It is not part of `system_state` or `time_series`; no decoder type is
re-exported from either in-memory module.

    let decoder = StateDecoder::new()
        .field::<Signal>("signal")?
        .field::<Space>("space")?;

    let run = decoder.decode("output/run-001")?;
    let spaces: &StateSeries = run.stream("space")?;

`field::<T>` creates the ordinary decoder automatically from T's Deserialize
implementation. It is an explicit key-to-type declaration, not a custom codec
implementation. A custom `field_with` closure remains available only for data
migration or domain validation.

Before reading chunk payloads, decode loads metadata and checks that every key
actually emitted by the directory has a decoder. Missing decoders, duplicate
decoder declarations, corrupt metadata, unknown chunk fields, and JSON type
errors are reported with directory, stream, chunk, record, and field context.
Extra decoder declarations are allowed so one StateDecoder can be reused for
multiple compatible datasets that contain different subsets of known keys.

Each output stream becomes a separate StateSeries because streams may have
different cadences and time points. StateDecoder must not guess how to merge
them. Every reconstructed SystemState shares the StateSpec recovered from the
directory metadata, uses the record's TimePoint, populates the fields contained
in that stream, and leaves other declared fields empty.

The initial decode method eagerly loads all streams because its result is an
in-memory decoded run. Large-dataset streaming can later be exposed as a
separate method using the same decoder collection; it must not silently change
decode into a lazy result. Restoration of each record is transactional: a
partially decoded state is discarded on failure.

Internally, decoder entries are heterogeneous object-safe functions stored in
a HashMap keyed by field name. Each function borrows raw JSON, deserializes its
concrete T directly, and moves T into the destination SystemState. No
serde_json::Value, payload clone, stable type tag, or global codec registry is
used.

### StateDecoder

Owns the reusable key-to-decoder collection. It is independent of a particular
directory until decode is called.

#### StateDecoder::new

Creates an empty decoder collection.

##### Reference

    analysis initialization -> StateDecoder::new
    tests -> isolated decoder construction

#### StateDecoder::field<T>

Adds the default Serde JSON decoder for one field key and returns self for
chaining. T must implement DeserializeOwned, Serialize, Clone, Send, and
'static so the reconstructed payload satisfies SystemState's full contract.

##### Reference

    application decoder declaration -> StateDecoder::field
    tests -> scalar, collection, custom struct, and tensor decoding

#### StateDecoder::field_with<T, F>

Adds an application-supplied raw-JSON-to-T conversion closure for one key. This
is intended for migration and validation, not ordinary payload decoding.

##### Reference

    persisted-version migration -> StateDecoder::field_with
    domain validation during load -> StateDecoder::field_with

#### StateDecoder::decode

Accepts an output directory, validates complete decoder coverage from its sole
metadata file, reads every declared stream and chunk in deterministic order,
and returns a DecodedRun containing one StateSeries per stream.

##### Reference

    analysis load -> StateDecoder::decode -> metadata and chunk readers
    tests -> complete multi-stream directory reconstruction

### DecodedRun

Owns decoded StateSeries values indexed by stream name plus the validated run
metadata required for inspection. It never merges streams implicitly.

#### DecodedRun::stream

Returns one decoded StateSeries by stream name.

##### Reference

    analysis -> StateDecoder::decode -> DecodedRun::stream

### FieldDecoder

Private object-safe interface behind each StateDecoder entry.

#### FieldDecoder::decode_into

Borrows one raw JSON field, constructs its concrete T, and transfers T into the
matching destination SystemState slot.

##### Reference

    StateDecoder::decode record reconstruction
        -> FieldDecoder::decode_into
        -> SystemState::set

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
    analysis and storage convenience validation -> StateSeries::spec

#### StateSeries::view

Creates a lightweight Copy read-only SeriesRef.

##### Reference

    analysis helper arguments -> StateSeries::view
    optional RunOutput::sample_series -> StateSeries::view
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

#### StateSeries::field_mut

Mutably borrows one named payload of concrete type `T` at a zero-based series
position. It returns `PositionOutOfBounds` when the position is absent and wraps
unknown, missing, or mismatched SystemState fields in position-aware
`FieldAccess`. StateSeries never returns `&mut SystemState`, because mutable
time access could invalidate series ordering.

##### Reference

    analysis transform
        -> StateSeries::field_mut
        -> SystemState::get_mut
        -> mutate payload without exposing set_time or advance

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

Private constructor combining SeriesError and a failure-only boxed rejected
SystemState. Boxing keeps the successful `Result` representation small; it does
not clone the state or its payloads.

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

#### PushError Debug::fmt

Formats the rejection reason and bounded structural SystemState diagnostics.
Payload values are never formatted.

##### Reference

    diagnostics and assertion failures -> Debug::fmt(PushError)

#### PushError Display::fmt

Delegates to SeriesError.

##### Reference

    logs and user-facing diagnostics -> Display::fmt(PushError)

#### PushError Error::source

Returns the contained SeriesError.

##### Reference

    standard error-chain traversal -> Error::source(PushError)

## Streaming persistence

### Storage module file boundaries

The storage stage uses one public facade plus six focused implementation files:

```text
src/
├── storage.rs
└── storage/
    ├── error.rs
    ├── format.rs
    ├── encoder.rs
    ├── writer.rs
    ├── reader.rs
    └── decoder.rs
```

- `storage.rs` is documentation and public re-exports only. It explains the
  sampling, writing, and reconstruction workflows but contains no IO logic.
- `error.rs` owns storage-specific failures and context: output paths, stream
  names, record indices, chunk ordinals, metadata validation, JSON errors,
  queue shutdown, writer termination, and decoder failures. SystemState and
  StateSeries retain their own errors.
- `format.rs` owns the versioned persisted contract and lean runtime envelopes:
  metadata, logical stream declarations, embedded key/description schemas,
  chunk descriptors, completion state, raw JSON record framing, and
  `EncodedRecord`. It validates data structures but never opens files or starts
  threads.
- `encoder.rs` owns `JsonEncoder`. It synchronously borrows a live
  `SystemState`, selects the fields declared for one logical stream, invokes
  each payload's existing `Serialize` implementation, and returns one complete
  owned `EncodedRecord`. It does not clone or take payloads, retain state
  borrows, decide chunk boundaries, or perform disk IO.
- `writer.rs` owns `StateWriter`, its worker lifecycle, bounded record and byte
  backpressure, exact-byte chunk rollover, atomic chunk commit, atomic updates
  of the single `metadata.json`, and the multi-stream `RunOutput` coordinator.
  It accepts only completed `EncodedRecord` values and never sees concrete
  payload types or calls `Serialize`.
- `reader.rs` owns read-only filesystem mechanics: opening and validating
  `metadata.json`, deterministic stream/chunk discovery, byte-length and
  integrity checks, range selection, and lazy raw-record iteration. It returns
  borrowed or owned raw JSON record material and never chooses concrete payload
  types.
- `decoder.rs` owns `StateDecoder`, its explicit key-to-concrete-type decoder
  declarations, typed payload reconstruction, and `DecodedRun`. It applies the
  right decoder to each field, builds SystemState values through ownership
  transfer, and optionally collects each logical stream into `StateSeries`.

Dependency direction is one-way:

```text
SystemState ──> JsonEncoder ──> EncodedRecord ──> StateWriter ──> disk
disk ──> raw reader ──> StateDecoder ──> SystemState ──> StateSeries
```

The writer never depends on StateSeries. The reader never depends on payload
types. The decoder never writes files. `format.rs` and `error.rs` are shared
support layers and do not depend on writer or reader lifecycle objects.

The output directory contains exactly one metadata file. It is created
atomically with structural run and stream declarations before sampling begins,
then atomically replaced as committed chunk descriptors and final completion
state become authoritative. Chunk files contain only complete JSONL records;
there are no per-chunk metadata sidecars. One record is never split. Chunk
rollover uses the exact framed byte length, and an oversized record becomes the
sole record in its chunk.

Backpressure is finite in two independent dimensions: an internal fixed limit
of 1,024 accepted but uncommitted records prevents pathological tiny-record
backlogs, and a caller-configured byte budget bounds encoded payload memory.
The record-count limit is a library safety policy, not user configuration and
not persisted metadata. Submission blocks until both constraints are satisfied.
Writer termination wakes blocked submitters and returns the terminal writer
error rather than deadlocking or growing memory without bound.

### Logical streams

Different partial-state cadences or output paths are different streams. A
stream declares:

- unique name;
- StateSpec for its partial records;
- cadence description;
- relative output directory or filename prefix;
- max_chunk_bytes: NonZeroU64;
- caller-configured queue byte limit;
- encoding and framing;
- independent monotonically increasing chunk ordinal.

Names such as signal and space are ordinary configured stream names, not
special concepts in the crate.

All streams must be declared before the first submission so metadata.json can
be written before simulation output begins. Paths are validated as relative,
non-colliding destinations beneath the run output root.

### Sampling boundary

The simulation owns and mutates its complete state. At a cadence boundary, a
stream's JsonEncoder selects and serializes only its fields. Encoding is
synchronous with respect to that borrowed data; RunOutput::sample returns only
after the EncodedRecord no longer borrows the simulation. StateWriter receives
only that owned encoded record.

The exact borrowed-field API remains the next design decision. It must:

- accept arbitrary Serde-serializable values by reference;
- validate selected field names and populated slots;
- encode in deterministic template order;
- avoid constructing an owned partial SystemState;
- produce exactly one complete encoded record or no record on failure.

An already-owned partial SystemState may be supported as a convenience source,
but it is not the required hot path.

### EncodedRecord

Private, non-Clone queue message containing one TimePoint and one complete
encoded field-record buffer. It is transport data, not a second public state
model.

EncodedRecord is indivisible. The queue moves it to one stream worker. Queue
capacity is bounded by both encoded bytes and record slots. A record larger
than the queue byte budget is admitted only when it is the sole outstanding
record, preventing permanent deadlock while preserving the no-split rule.

### JsonEncoder

Private storage component that supplies serde_json's Serializer and record
framing while borrowing the selected payloads' own Serialize implementations.
It contains no payload-specific conversion logic.

#### JsonEncoder::encode

Borrows a SystemState and configured field selection, invokes each payload's
erased Serialize implementation, and returns one owned EncodedRecord.

##### Reference

    RunOutput::sample -> JsonEncoder::encode -> EncodedRecord

### Byte-targeted chunks

Each StateWriter tracks the exact bytes already appended to its active JSONL
file, including each newline. Before appending a record, it evaluates the
complete encoded record length:

1. if the active chunk is non-empty and appending would exceed
   max_chunk_bytes, seal the active chunk first;
2. append the complete record to the current or newly opened chunk;
3. if the resulting size equals or exceeds max_chunk_bytes, seal that chunk.

The maximum is exact for ordinary records but necessarily soft for one record
larger than max_chunk_bytes because records may not be split. The recommended
policy is to accept such a record as the sole record in an oversized chunk and
record its exact size in metadata. Rejecting it would be the only way to make
the maximum strict; silently splitting it is forbidden.

Chunk record counts therefore vary with payload size. Different streams track
bytes and roll over independently. A durability operation may seal a chunk
below the target only if its contract explicitly says so.

The framing is compact JSON Lines: one complete object plus one newline per
record. Chunk files contain no schema header and no repeated metadata.

### StateWriter

One non-Clone authority for one logical stream. It owns the strict byte limit,
shared bounded FIFO, worker lifecycle, admission ordering, terminal error, and
successful summary transition. It receives only complete EncodedRecord values
and never accesses SystemState, payload types, or Serde.

#### StateWriter::start

Creates a previously absent stream directory and starts one named worker
thread. Existing output is rejected rather than overwritten. Chunk files are
created lazily, so finishing an empty writer leaves an empty stream directory.

##### Reference

    RunOutput construction -> WriterConfig::new -> StateWriter::start
    focused writer setup -> StateWriter::start

#### StateWriter::submit

Consumes one already complete EncodedRecord. It immediately rejects a record
larger than the strict byte budget or an index not greater than the last
accepted index. Otherwise it waits until both the fixed 1,024-record limit and
configured byte limit admit the record, then moves it into the FIFO. Success
means accepted, not durable. Worker failure wakes waiters with the shared
terminal cause.

##### Reference

    RunOutput::sample -> JsonEncoder::encode
        -> StateWriter::submit(EncodedRecord)
        -> bounded queue
        -> active chunk

#### StateWriter::finish

Consumes the exclusive writer, rejects further admission, drains accepted
records, seals the final non-empty chunk, joins the worker, and returns
WriterSummary. A worker failure is returned as WriterTerminated; a panic is
returned as WriterPanicked.

##### Reference

    successful run termination -> each StateWriter::finish
    run-level coordinator -> collect stream completion statistics

#### StateWriter::close_admission

Private lifecycle transition that closes the queue and wakes both the worker
and capacity waiters.

##### Reference

    StateWriter::finish -> StateWriter::close_admission
    StateWriter::drop -> StateWriter::close_admission

#### StateWriter::join_worker

Private one-shot join operation that consumes the stored JoinHandle and maps an
unexpected panic into WriterPanicked.

##### Reference

    StateWriter::finish -> StateWriter::join_worker
    StateWriter::drop -> StateWriter::join_worker

#### StateWriter::drop

Performs best-effort close, drain, and join when the caller omits finish. Drop
cannot report a storage failure, so explicit finish remains required for a
successful run.

##### Reference

    abandoned StateWriter -> Drop::drop(StateWriter)

### WriterConfig

Cloneable, payload-free startup values for one stream: diagnostic stream name,
new output directory, nonzero chunk target, and nonzero strict queue-byte
budget. There is deliberately no user-configurable record limit.

#### WriterConfig::new

Constructs configuration without filesystem access and rejects empty stream or
directory values. NonZeroU64 enforces positive byte settings.

##### Reference

    RunOutput construction -> WriterConfig::new -> StateWriter::start

#### WriterConfig::stream

Returns the logical stream name.

##### Reference

    configuration inspection and tests -> WriterConfig::stream

#### WriterConfig::directory

Returns the target stream directory.

##### Reference

    configuration inspection and tests -> WriterConfig::directory

#### WriterConfig::max_chunk_bytes

Returns the nonzero soft chunk rollover target.

##### Reference

    configuration inspection and tests -> WriterConfig::max_chunk_bytes

#### WriterConfig::queue_bytes

Returns the nonzero strict outstanding-byte budget.

##### Reference

    configuration inspection and tests -> WriterConfig::queue_bytes

### WriterSummary

Small cloneable result containing stream identity, committed ChunkMetadata in
ordinal order, total record count, and exact total encoded bytes. It contains
no payload bytes.

#### WriterSummary::stream

Returns the completed logical stream name.

##### Reference

    run-level completion bookkeeping -> WriterSummary::stream

#### WriterSummary::chunks

Borrows the committed chunk inventory for insertion into metadata.json.

##### Reference

    RunOutput metadata commit -> WriterSummary::chunks

#### WriterSummary::records

Returns total committed records.

##### Reference

    completion diagnostics and tests -> WriterSummary::records

#### WriterSummary::bytes

Returns exact bytes across committed chunks.

##### Reference

    completion diagnostics and tests -> WriterSummary::bytes

### Shared and QueueState

Private synchronization state implemented with one Mutex and two Condvars.
QueueState owns the FIFO, outstanding record and byte counts, last accepted
index, admission flag, authoritative terminal error, and final summary.

#### Shared::new

Creates one open, empty coordination state.

##### Reference

    StateWriter::start -> Shared::new

### ActiveChunk

Worker-only non-Clone state for a temporary chunk: paths, file handle, SHA-256
hasher, exact counts, and first/last indices.

#### ActiveChunk::create

Creates a deterministic temporary file with create_new and refuses an existing
committed filename.

##### Reference

    writer append loop -> ActiveChunk::create

#### ActiveChunk::append

Writes one complete framed record, updates SHA-256 from the same byte slice,
and advances checked record, byte, and time statistics.

##### Reference

    writer append loop -> ActiveChunk::append

#### ActiveChunk::seal

Synchronizes the temporary file, atomically renames it to its deterministic
committed name, finalizes the lowercase SHA-256 digest, and returns
ChunkMetadata.

##### Reference

    byte rollover or writer drain -> ActiveChunk::seal -> ChunkMetadata

The current writer intentionally has no flush method. `submit` is an admission
boundary and `finish` is the durability/completion boundary. A future explicit
checkpoint API may seal an underfilled chunk, but that policy is not implied by
the current interface.

### RunOutput

Run-level storage facade owning field selections, one JsonEncoder and
StateWriter per stream, and the sole metadata.json lifecycle. It coordinates
encoding and writing but preserves their internal separation.

#### RunOutput::builder

Starts configuration for an output directory and StateSpec.

##### Reference

    simulation initialization -> RunOutput::builder

#### RunOutput::sample

Selects one configured stream, asks its JsonEncoder to produce an
EncodedRecord, then passes that record to its StateWriter.

##### Reference

    simulation cadence -> RunOutput::sample
        -> JsonEncoder::encode
        -> StateWriter::submit

#### RunOutput::sample_series

Optional convenience that borrows SeriesRef and routes each state through
RunOutput::sample. It does not make StateSeries responsible for serialization.

##### Reference

    persist analysis result -> StateSeries::view
        -> RunOutput::sample_series
        -> RunOutput::sample for each state

#### RunOutput::finish

Finishes every StateWriter and performs the sole run-level completion
transition.

##### Reference

    successful simulation termination -> RunOutput::finish

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
- max_chunk_bytes and the configured queue byte limit. The fixed 1,024-record
  safety bound is runtime policy and is not repeated in metadata.

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

Only populated fields selected for that stream appear. The state layout and
stream field selection in metadata define valid keys. No type or codec tag is
stored. One newline terminates the complete object.

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

Non-exhaustive public error enum limited to StateSeries layout identity,
increasing-index ordering, analysis position bounds, and position-aware typed
field access.

### StorageError

Non-exhaustive public error enum for persistent storage only. Its variants are
grouped into configuration/lifecycle, persisted format and integrity,
state-borrowing and field processing, filesystem/JSON mechanics, and bounded
writer-worker lifecycle. IO, JSON, StateError, SeriesError, custom decoder, and
terminal worker errors preserve their source chains. No variant owns a
scientific payload.

`WriterTerminated` shares one authoritative terminal `StorageError` through an
Arc so blocked submitters can wake with the same cause without making the enum
Clone. `SeriesInvariant` retains only the SeriesError after dropping the
newly-decoded rejected state, preventing corrupt input from being retained as a
potentially huge error payload.

`StorageError` declares no inherent methods. Each detecting boundary constructs
the precise variant directly.

The dedicated `tests/storage/error.rs` suite covers every variant in seven
tests. It verifies exact context, typed and object-safe sources, shared terminal
error Arc identity, non-exhaustive matching, and `Send + Sync` without mutating
the filesystem.

#### Reference

    format validation -> UnsupportedVersion / InvalidMetadata / integrity variants
    JsonEncoder -> StateAccess / EncodeField
    StateWriter and RunOutput -> configuration / queue / worker variants
    raw reader -> IO / JSON / chunk integrity variants
    StateDecoder -> decoder registration / DecodeField / SeriesInvariant

### Persisted format types

`format.rs` defines crate-private Serde representations and validates them
without filesystem access.

#### RunMetadata

The complete single-file metadata document: format/version, lifecycle status,
record format, time-axis description, arbitrary JSON run attributes, and
ordered stream declarations.

#### RunMetadata::running

Builds the initial running snapshot using the supported format constants.

##### Reference

    RunOutput initialization -> RunMetadata::running

#### RunMetadata::validate

Validates the complete in-memory document using a supplied provenance path.

##### Reference

    writer before atomic metadata commit -> RunMetadata::validate
    reader after JSON parsing -> RunMetadata::validate

#### RunMetadata::stream

Finds an immutable stream declaration by configured name.

##### Reference

    reader stream selection -> RunMetadata::stream

#### RunMetadata::stream_mut

Finds a mutable stream declaration so a writer can append committed chunk
descriptors before revalidation and atomic metadata replacement.

##### Reference

    writer commit bookkeeping -> RunMetadata::stream_mut

#### RunStatus

Tagged `running`, `complete`, or `failed { message }` lifecycle state. Its
private validation rejects an empty failure explanation.

##### Reference

    writer lifecycle updates -> RunStatus
    reader completion policy -> RunStatus

#### RunStatus::validate

Rejects an empty message on failed lifecycle state.

##### Reference

    RunMetadata::validate -> RunStatus::validate

#### RecordFormat

Declares JSON payload encoding and JSON Lines framing once per run. Its private
constructor and validator enforce the version-one constants.

##### Reference

    RunMetadata::running -> RecordFormat::json_lines
    RunMetadata::validate -> RecordFormat::validate

#### RecordFormat::json_lines

Constructs the supported JSON and JSON Lines declaration.

##### Reference

    RunMetadata::running -> RecordFormat::json_lines

#### RecordFormat::validate

Rejects encoding or framing labels unsupported by this version.

##### Reference

    RunMetadata::validate -> RecordFormat::validate

#### TimeAxis

Names the mandatory integer coordinate and optional physical coordinate with
optional units. Private validation rejects empty labels and a physical unit
without a physical coordinate name.

##### Reference

    RunMetadata configuration -> TimeAxis
    RunMetadata::validate -> TimeAxis::validate

#### TimeAxis::validate

Rejects empty present labels and a physical unit without a physical name.

##### Reference

    RunMetadata::validate -> TimeAxis::validate

#### StreamMetadata

One logical stream's name, safe relative directory, cadence description,
ordered key/description schema, chunk and queue byte limits, and committed chunk
inventory. The fixed internal record-count limit is intentionally absent from
metadata. Private validation checks byte limits, unique fields, safe paths,
contiguous chunk ordinals, and increasing cross-chunk indices.

##### Reference

    RunOutput configuration -> StreamMetadata
    StateWriter commit -> StreamMetadata::chunks
    RunMetadata::validate -> StreamMetadata::validate

#### StreamMetadata::validate

Checks the safe directory, nonzero limits, normalized unique fields, contiguous
chunk ordinals, and increasing cross-chunk index ranges.

##### Reference

    RunMetadata::validate -> StreamMetadata::validate

#### FieldMetadata

One persisted key and optional natural-language description. It contains no
Rust type or decoder tag. Private validation rejects empty normalized content.

##### Reference

    stream field selection -> FieldMetadata
    decoder declaration validation -> FieldMetadata

#### FieldMetadata::validate

Rejects an empty field name or empty present description.

##### Reference

    StreamMetadata::validate -> FieldMetadata::validate

#### ChunkMetadata

One immutable committed chunk's ordinal, deterministic filename, record count,
exact bytes, algorithm-prefixed checksum, and first/last integer indices.
Private validation rejects empty chunks, reversed ranges, invalid checksums, and
non-deterministic names.

##### Reference

    StateWriter chunk seal -> ChunkMetadata
    raw reader integrity check -> ChunkMetadata

#### ChunkMetadata::validate

Checks ordinal, deterministic filename, nonzero records and bytes, ordered
index range, and algorithm-prefixed lowercase hexadecimal checksum.

##### Reference

    StreamMetadata::validate -> ChunkMetadata::validate

#### RawRecord

Parsed but untyped JSON record containing integer time, optional physical time,
and a key-to-JSON-value map.

#### RawRecord::time

Reconstructs `TimePoint`; JSON numeric parsing already excludes non-finite
physical values.

##### Reference

    StateDecoder record reconstruction -> RawRecord::time

#### RawRecord::into_values

Moves the JSON field map to decoder dispatch without cloning its value tree.

##### Reference

    StateDecoder field loop -> RawRecord::into_values

#### EncodedRecord

Non-Clone queue message owning a `TimePoint` and one complete framed JSONL byte
buffer.

#### EncodedRecord::new

Accepts compact object bytes and appends the sole framing newline.

##### Reference

    JsonEncoder successful encoding -> EncodedRecord::new

#### EncodedRecord::time

Returns the complete temporal coordinate used for writer ordering.

##### Reference

    StateWriter ordering -> EncodedRecord::time

#### EncodedRecord::len

Returns exact framed bytes, including the newline.

##### Reference

    queue budget and chunk rollover -> EncodedRecord::len

#### EncodedRecord::bytes

Borrows the indivisible record for checksumming and append.

##### Reference

    checksum and append -> EncodedRecord::bytes

#### EncodedRecord::into_bytes

Moves out the framed allocation for a consuming writer adapter.

##### Reference

    consuming writer adapter -> EncodedRecord::into_bytes

#### EncodedRecord Debug::fmt

Formats only time and byte length, never payload bytes.

##### Reference

    writer diagnostics and tests -> Debug::fmt(EncodedRecord)

#### chunk_filename

Returns `chunk-{ordinal:06}.jsonl`, expanding naturally beyond six digits.

##### Reference

    StateWriter chunk creation -> chunk_filename
    ChunkMetadata validation -> chunk_filename

The dedicated `tests/storage/format.rs` suite covers semantic JSON round trips,
stream lookup and mutation, every metadata validation class, deterministic
chunk names, raw time/value reconstruction, allocation-moving encoded framing,
bounded Debug output, and validation without filesystem access.
All 11 focused tests pass when compiled with warnings denied; the staged test
harness uses the crate's real public state and series types while including only
the not-yet-exported storage files under review.

All metadata-related variants refer to metadata.json. Chunk byte length is an
integrity fact, not a chunking policy.

## Clone and concurrency policy

- FieldSpec: cheap metadata clone.
- StateSpec: cheap Arc clone.
- TimePoint: Copy.
- SystemState: deep payload clone; avoid on hot paths.
- StateSeries: deep clone of every state; use SeriesRef or Arc instead.
- SeriesRef: Copy borrowed view.
- EncodedRecord: moved, not cloned.
- StateWriter and RunOutput: non-Clone exclusive lifecycle authorities.
- Metadata transaction and active chunk: non-Clone.

The normal persistence path borrows live values during encoding and then moves
encoded buffers. No deep state clone is used.

The handoff into a stream writer is exactly one complete `EncodedRecord` per
sample. A writer is already bound to its logical output stream, so the record
does not repeat a stream name or schema. The writer may inspect the record's
time and framed byte length for ordering, queue accounting, and chunk rollover,
but it treats the encoded bytes as opaque. It never receives a `SystemState`,
borrows a scientific payload, or invokes `Serialize`.

### Writer backpressure

Each stream writer has two finite admission limits: a library-defined maximum
of 1,024 outstanding records and a caller-configured maximum of outstanding
encoded bytes. `StateWriter::submit` examines the incoming
`EncodedRecord::len` and waits on a condition variable while accepting it would
exceed either limit. Once admitted, the call transfers the record into the
writer's FIFO and returns; the simulation may then continue.

An admitted record retains one record permit and its exact number of byte
permits until the worker has committed it to the chunk file, not merely until
the worker removes it from the in-memory queue. This bounds all accepted but
uncommitted data, including the record currently being written. After commit,
the worker releases both permits and wakes blocked submitters. The queue
preserves admission order. Callers producing one logical stream concurrently
must establish sampling order before submission; the writer validates ordering
but does not guess the intended order of racing samples.

A record larger than the writer's entire byte budget cannot ever satisfy the
admission condition and must fail immediately with a size-limit error rather
than deadlock. It may still exceed the preferred chunk target: because records
are indivisible, such a record occupies a one-record oversized chunk. Chunk
size is therefore a rollover target, whereas queue byte capacity is a strict
memory bound.

If the worker fails, it stores one terminal error and wakes every blocked
submitter. Current and future submissions then return `WriterTerminated`
instead of waiting indefinitely. Writer shutdown similarly stops admission,
drains already accepted records, commits metadata, and joins the worker.

## Scientific tensor compatibility

The core API is tensor-library agnostic. Any type satisfying the SystemState
and Serde bounds can be stored and written.

The pinned development dependency `physics_in_parallel` 3.0.4 implements
Serialize for Tensor<T, Dense>, Tensor<T, Sparse>, and SquareLattice<T> under
their documented scalar bounds. The new SystemState::set bound therefore does
not require an upstream crate change merely to store these payloads; the public
integration test remains the compile-time gate. During coordinated local
development, Cargo resolves version 3.0.4 from `../../pip`; the version remains
specified so the path can be removed after that release is published.

Until that release, the local `../../pip` checkout is the authoritative
development dependency. Changes to scientific payload contracts may update
`physics_in_parallel` and `scientific-workflow` together, with tests in both
crates serving as the joint compatibility gate. Migration back to the registry
is a separate release step: publish the coordinated PiP version, replace the
path dependency with its crates.io version, refresh the lockfile, and rerun both
test suites before publishing Scientific Workflow.

Current readiness: local integration and Cargo packaging are ready. All 152 PiP
tests, the 33-test SystemState integration target, `cargo package`, and
`cargo publish --dry-run` pass. Registry publication remains intentionally
deferred. Before that release, commit the 3.0.4 changes and resolve the crate's
pre-existing MSRV inconsistency: its manifest declares Rust 1.85, while existing
engine code uses `slice::get_disjoint_mut`, stabilized in Rust 1.86. Existing
Clippy warnings should also be triaged as release hygiene, although they do not
block packaging or local use.

The upstream compatibility and optimization requirements are distinct:

- Compatibility requires no change. The existing implementations satisfy
  SystemState::set and StateDecoder's future typed Deserialize requirements.
- Copy-efficient dense encoding required an upstream change. That change is
  implemented in the local 3.0.4 source and preserves the existing JSON schema.

physics_in_parallel contains `FlatPayloadRef<'a, T>` with borrowed `kind`,
`shape`, and `data` fields. Version 3.0.4 now uses it as follows:

1. Dense tensor storage serializes a borrowed shape and data slice. The public
   dense tensor facade delegates directly to storage instead of constructing a
   `serde_json::Value`.
2. Square lattices and vector lists serialize borrowed buffers and retain their
   existing kind tags and flat schemas.
3. Matrix backends can expose optional contiguous row-major storage. Dense
   matrices use that borrowed slice; sparse and structured backends retain the
   logical-entry fallback.
4. Convenience methods explicitly returning an owned `Value`, `String`, or
   `FlatPayload` still allocate because ownership is their stated contract.
5. Sparse tensor storage remains a separate decision. Its current Serialize calls `to_dense()`,
   allocating the full logical tensor. Either stream the existing dense JSON
   sequence through a custom borrowed Serialize wrapper for format stability,
   or introduce a genuinely sparse JSON representation. The latter is more
   efficient but is a persisted-format change and requires matching Deserialize
   plus deterministic entry ordering.

Concretely, the current sparse tensor wire object is
`{"kind":"tensor_sparse","shape":[...],"data":[...]}`. `data` contains every
logical element in row-major order, including implicit zeros. Serialization
materializes a temporary dense tensor and dense data vector; deserialization
first owns that dense vector and then builds sparse storage from it. The public
`Tensor<T, Sparse>` facade now avoids an additional `serde_json::Value`, but it
delegates to this still-densifying storage serializer. SystemState itself moves
sparse tensor ownership without copying on `set` and `take`; the extra
allocation occurs only when JSON serialization begins. Sparse matrices likewise
retain their logical-dense JSON fallback.

The detailed sparse-format task list is maintained locally in `pip/todo.md`.
PiP intentionally ignores that coordination file through `/todo.md` in its
`.gitignore`; this design document remains the tracked cross-crate architectural
record.

Deserialization may allocate the final owned Vec<T>; that allocation becomes
the reconstructed tensor backing store and is not an avoidable intermediate
copy. `FlatPayload<T>` already transfers its owned vectors into the tensor or
lattice constructors on the read path.

The 3.0.4 tests explicitly preserve dense and sparse tensor schemas, round-trip
through `serde_json::to_writer` and `from_slice`, preserve lattice and matrix
schemas, and exercise vector-list serialization through engine attributes. All
152 upstream tests pass. A future allocation benchmark should quantify the
removed dense intermediates and the remaining sparse cost.

Zero-copy guarantees therefore apply precisely to:

- moving owned payloads into and out of SystemState;
- borrowing payloads through get and get_mut;
- moving SystemState values into analysis collections;
- borrowing payloads at the erased-serialization boundary.

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

The `system_state` and `time_series` sources now match their contracts above.
The remaining implementation delta is the complete `storage` module: format,
encoder, writer, decoder, reader, borrowed partial-state encoding, byte-targeted
chunks, bounded blocking backpressure, metadata lifecycle, and storage-specific
errors are not yet implemented.

### Completed module: time_series

The time-series stage produces a deliberately small in-memory analysis module.
Its final production surface contains `time_series.rs`, `time_series/error.rs`,
and `time_series/series.rs`; `codec.rs` is deleted rather than replaced.

The work is performed in these review units:

1. Refactor `time_series/error.rs` down to collection errors: incompatible
   shared layout, non-increasing index, analysis position out of bounds, and a
   position-aware wrapper around `StateError` for field mutation. Persistence,
   codec, filesystem, JSON, chunk, and writer errors move to the future storage
   module instead of remaining here.
2. Rewrite `tests/time_series/error.rs` to mirror that reduced contract.
3. Refactor `time_series/series.rs`: retain owned `StateSeries`, borrowed Copy
   `SeriesRef`, strict shared-layout identity, increasing integer indices,
   ownership-moving push/pop/consumption, and explicitly expensive deep Clone.
   Remove `StateChunk` and `into_chunk`; remove unsupported const qualifiers;
   box the rejected state inside `PushError` to keep `Result` small while still
   returning ownership without cloning.
4. Add `StateSeries::field_mut<T>(position, key)` as the only mutable analysis
   boundary. It delegates to `SystemState::get_mut` but never exposes
   `&mut SystemState`, so callers cannot invalidate series time ordering or
   replace a state with a foreign layout.
5. Rewrite `tests/time_series/series.rs` around the final ownership, ordering,
   field-mutation, view, iteration, allocation-reuse, and bounded-Debug
   contracts. Remove every chunk assertion.
6. Delete `time_series/codec.rs` and its dedicated test. Payload reconstruction
   belongs to `storage::StateDecoder`; time_series knows nothing about JSON or
   concrete payload decoders.
7. Create `time_series.rs` as the documented public facade and export only
   `SeriesError`, `PushError`, `SeriesRef`, and `StateSeries`.
8. Rewrite `tests/time_series.rs` as a public, unified in-memory workflow using
   the real JSON StateSpec fixture and serializable payloads, without codecs,
   chunks, files, or writer behavior.
9. Export `time_series` from `lib.rs`, update README test instructions, and run
   formatting, library checks, focused tests, doctests, Clippy with warnings
   denied, and the complete test suite.

This stage does not implement sampling, encoding, decoding, queues, file-size
chunking, metadata, or disk IO. Those are all owned by the subsequent `storage`
module.

`time_series/codec.rs` has now been deleted. Its former registry, erased codec,
stable-tag lookup, JSON value conversion, decoding, and size-estimation APIs
have no successor within time_series.

The obsolete `tests/time_series/codec.rs` suite has also been deleted. The
unified public target now includes only the focused error and series suites plus
one cross-module ownership workflow.

#### `time_series/error.rs` implementation decision

`SeriesError` is now a non-exhaustive four-variant analysis error:

- `SpecMismatch { index }` reports rejected shared-layout identity;
- `NonIncreasingTime { previous, next }` reports append ordering;
- `PositionOutOfBounds { position, len }` reports a missing zero-based analysis
  position; and
- `FieldAccess { position, source: StateError }` adds series position while
  preserving the typed state-access source.

The file imports no filesystem, path, JSON, codec, chunk, or writer types. It
declares no methods; downstream construction occurs directly at
`StateSeries::push` and `StateSeries::field_mut`.

#### `time_series/series.rs` implementation decision

The production collection now matches the analysis-only contract. `StateChunk`
and `StateSeries::into_chunk` are removed. `StateSeries::field_mut<T>` first
validates the zero-based collection position, then delegates to
`SystemState::get_mut`, wrapping its typed error with the position. No complete
mutable state is exposed. StateSeries collection accessors are ordinary methods
rather than relying on newer const `Vec` APIs. `PushError` stores the rejected
state in a failure-only `Box<SystemState>` so the common `Result<(), PushError>`
representation stays small; `into_parts` moves the original state back out and
does not clone its payloads.

The dedicated `tests/time_series/series.rs` suite now exercises every retained
collection, view, and push-error method. Its nine tests cover shared-layout and
ordering rejection, typed field mutation and all error classes, payload pointer
preservation, clone counts, vector allocation preservation, capacity reuse,
borrowed and owned iteration, bounded Debug output, and thread transferability.
It contains no codec, chunk, or persistence fixture.

The focused error suite contains five tests covering exact variant context,
source preservation, non-exhaustive matching, and `Send + Sync`. The focused
series suite contains nine tests covering every retained collection, view, and
push-error method. The unified target adds one public SystemState-to-StateSeries
ownership and mutation workflow.

The completed module is exported from `lib.rs` and documented in the crate and
repository READMEs. Verification passes for formatting, all-target checks,
15 time-series tests, 33 system-state tests, the complete all-target test suite,
four doctests, and Clippy across all targets with warnings denied. Cargo package
verification is intentionally deferred only because the coordinated local
`physics_in_parallel` 3.0.4 dev dependency is not yet published on crates.io;
the registry currently offers 3.0.3. This does not affect local compilation or
the storage-stage dependency graph.

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
    ├── time_series/
    │   ├── error.rs
    │   └── series.rs
    ├── storage.rs
    └── storage/
        ├── decoder.rs
        ├── encoder.rs
        ├── error.rs
        ├── format.rs
        ├── reader.rs
        └── writer.rs

Single-module tests mirror the source filename. Tests spanning modules use a
concise integration name. The package README owns user-facing test commands.

For each review unit:

1. update this document with the intended contract;
2. edit exactly the one scheduled file;
3. run whatever focused checks are valid at that staged boundary;
4. wait for review before moving to the next scheduled file, including its
   separate dedicated-test review unit.

Run the complete stage verification only after all scheduled system_state files
and tests have been reviewed. Transitional downstream failures are not repaired
out of order.

This ordering rule applies across the entire project. When a reviewed file
requires changes in a downstream file, that downstream file remains untouched
until its own review unit. The required change is recorded in both todo.md and
the relevant design section. Documentation files may be updated alongside the
single active production file because they record, rather than implement, the
dependency.
