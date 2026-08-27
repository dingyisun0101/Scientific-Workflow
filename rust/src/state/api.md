# State API

The `state` subsystem owns validated field layouts, heterogeneous in-memory
payloads, scientific time, and ordered in-memory state series. Its canonical
public scopes are `scientific_workflow::state::basic` and
`scientific_workflow::state::advanced`. `prelude::basic` and
`prelude::advanced` re-export the same symbols for convenience; they do not own
or wrap them.

State performs filesystem I/O only while loading a JSON schema. It does not
schedule tasks, infer recording paths, choose sampling policy, persist state,
or render progress. Payloads remain application-owned concrete Rust values.

## Basic API

### `state::basic::StateTime`

`StateTime` is a small `Copy` value describing one scientific coordinate. Its
mandatory `u64` iteration is the ordering authority; its physical coordinate
is optional. Physical time is unrelated to operational UTC timestamps.

- `StateTime::from_iteration(iteration)` constructs an iteration-only value.
- `StateTime::from_iteration_and_physical_time(iteration, physical_time)`
  returns `Some(StateTime)` for a finite physical value and `None` for NaN or
  infinity. Negative and zero finite coordinates are valid.
- `iteration()` returns the iteration by value.
- `physical_time()` returns the optional physical coordinate by value.
- `checked_advance(increment)` computes the next iteration without mutation.
  `None` preserves the optional physical coordinate. `Some(delta)` requires an
  existing physical coordinate and finite delta and sum.

`checked_advance` reports `IterationOverflow`, `MissingPhysicalTime`, or
`InvalidPhysicalAdvance`. It has no side effects and performs no allocation,
I/O, blocking, persistence, or cancellation work.

### `state::basic::SystemStateSchema`

`SystemStateSchema` is a cheap cloneable handle to one immutable schema
allocation. A schema fixes field names and deterministic field order, but it
does not store Rust payload types or storage codecs.

- `SystemStateSchema::load_json_template(path: &Path)` reads and strictly
  decodes a JSON document with a `fields` array. The borrowed `Path` is copied
  into the schema for provenance; the file is not kept open. Unknown JSON
  properties are rejected. Names and descriptions are trimmed, blank names
  and normalized duplicates fail, and blank descriptions become absent.
- `create_empty_state(time)` creates a `SystemState` sharing this schema. Every
  declared slot exists but starts uninitialized and empty.

Loading may return `TemplateRead`, `TemplateParse`, `EmptyFieldName`, or
`DuplicateField`. A failure returns no partially constructed schema. Cloning a
schema is an atomic reference-count increment and does not clone field names or
lookup tables. Immutable schema handles can be shared across threads.

Metadata inspection is deliberately advanced. Basic application code needs no
field-index API to construct or use a state.

### `state::basic::SystemState`

`SystemState` owns one fixed set of heterogeneous payload slots plus a
`StateTime`. Its only public construction path is
`SystemStateSchema::create_empty_state`. The schema fixes names and order; the
first successful initialization binds each slot to one exact concrete Rust
type for that state lineage.

Payloads accepted by `initialize_payload` and `insert_payload` implement
`Serialize + Clone + Send + 'static`. `Send` permits an owned state to move
between threads. `Sync` is not required, so shared cross-thread references are
available only when the concrete payload composition permits them. Explicit
state cloning invokes every populated payload's `Clone`; ordinary insertion,
borrowing, mutation, extraction, and series movement do not clone payloads.

The basic methods are:

- `time()` returns the current `StateTime` by value.
- `advance_time(increment)` validates a complete next `StateTime`, assigns it
  only after validation succeeds, and returns it. Failure is atomic and never
  touches payloads.
- `schema()` borrows the immutable shared schema handle.
- `contains_payload(field)` reports whether a declared slot currently owns a
  payload; an undeclared field returns `UnknownField`.
- `initialize_payload(field, payload)` is the preferred initial assembly API.
  It consumes and stores the payload only when the field is declared and has
  never been initialized. It establishes the retained concrete type contract
  and returns `()`. A second initialization returns
  `PayloadAlreadyInitialized` even when the same Rust type is supplied.
- `insert_payload(field, payload)` performs deliberate assignment. An
  uninitialized slot is initialized; a slot bound to the same exact type is
  replaced and returns the previous owner as `Some(T)`, or `None` when that
  typed slot is currently empty. Unknown fields and type mismatches leave the
  state unchanged and preserve the incoming owner in `PayloadInsertError<T>`.
- `payload::<T>(field)` returns `&T` after name, presence, and exact type
  validation.
- `payload_mut::<T>(field)` returns `&mut T` under the same validation and
  mutates the application payload in place.
- `borrow_payloads::<Q>(names)` borrows two through eight distinct populated
  fields in one checked operation. `Q` is the tuple of expected payload types;
  the names tuple has matching order and arity. All validation completes before
  references are returned.
- `borrow_payloads_mut::<Q>(names)` provides the corresponding disjoint mutable
  borrows. Repeated fields return `RepeatedPayloadBorrow`; failure grants no
  partial borrow.
- `take_payload::<T>(field)` validates first, then moves the original payload
  out without cloning. The now-empty slot retains its type contract.

Typed access may return `UnknownField`, `MissingPayload`, `TypeMismatch`, or
`RepeatedPayloadBorrow`. All rejected operations are failure-atomic. State has
no internal locks, background work, persistence, blocking calls, or
cancellation behavior.

### `state::basic::StateSeries`

`StateSeries` owns complete `SystemState` snapshots for in-memory analysis. A
series retains one canonical schema handle even when empty. Every accepted
state must share that exact schema allocation and have an iteration strictly
greater than the current last state. Gaps are valid; optional physical time is
not an ordering key.

- `new(schema)` creates an empty series without reserving state capacity.
- `with_capacity(schema, capacity)` also reserves owner slots in the backing
  vector; it creates no states or payloads.
- `schema()`, `len()`, `is_empty()`, and `capacity()` inspect the collection.
- `reserve(additional)` reserves backing-vector capacity.
- `state_at(position)`, `first_state()`, `last_state()`,
  `as_state_slice()`, and `iter()` expose immutable state borrows.
- `payload_mut_at::<T>(position, field)` mutates one payload without exposing a
  mutable `SystemState`, preserving schema and time-order invariants.
- `push_state(state)` moves a validated state into the series. Rejection
  preserves the unchanged state in `StateSeriesPushError`.
- `pop_state()` moves out the latest state. `clear_states()` drops all states
  while retaining schema and vector capacity. `into_states()` consumes the
  series and returns its backing state vector.
- `IntoIterator for &StateSeries` yields immutable states. Owned iteration
  moves states out.

Borrowing `&StateSeries` is the canonical lightweight view; there is no
separate view type. Cloning a series deep-clones every populated payload and
should be reserved for analysis that truly needs independent owners. Series
operations perform no I/O or background work.

### Basic error types

`state::basic::StateError` is the non-exhaustive state/schema/time error enum.
Callers should match variants of interest and retain a fallback arm.

- `TemplateRead { path, source }`: filesystem read failed.
- `TemplateParse { path, source }`: JSON syntax, shape, or strict-field decode
  failed.
- `EmptyFieldName { index }`: a normalized declaration name is blank.
- `DuplicateField { field }`: two normalized field names collide.
- `UnknownField { field }`: a requested name is not declared.
- `RepeatedPayloadBorrow { field }`: one tuple borrow names a field twice.
- `MissingPayload { field }`: a declared slot currently has no value.
- `TypeMismatch { field, expected, actual }`: an exact Rust type request does
  not match the retained slot contract.
- `PayloadAlreadyInitialized { field }`: initial assembly tried to initialize
  an already bound slot.
- `IterationOverflow { iteration }`: the next iteration exceeds `u64::MAX`.
- `MissingPhysicalTime { iteration }`: a physical delta was requested without
  an established physical coordinate.
- `InvalidPhysicalAdvance { current, delta }`: the delta or sum is non-finite.

`state::basic::PayloadInsertError<T>` owns a rejected incoming payload and its
`StateError`. `error()` and `payload()` borrow the two components;
`into_parts()` returns `(StateError, T)` without cloning. `Display` delegates to
the state error, while `Debug` intentionally omits payload contents.

`state::basic::StateSeriesError` is non-exhaustive:

- `SchemaMismatch { iteration }` rejects a state from a different schema
  allocation.
- `NonIncreasingIteration { previous, next }` rejects equal or decreasing
  iterations.
- `PositionOutOfBounds { position, len }` contextualizes failed indexed
  mutation.
- `PayloadAccess { position, source }` wraps the originating `StateError` from
  indexed typed access.

`state::basic::StateSeriesPushError` owns an unchanged rejected `SystemState`
and the `StateSeriesError`. `error()` and `state()` borrow them; `into_parts()`
moves out `(StateSeriesError, SystemState)`. Formatting never traverses payload
values.

## Advanced API

`state::advanced` re-exports every Basic API symbol above. It adds the
following stable contracts for advanced users, Workflow internals, and peer
subsystems. These are extension and inspection seams, not access to private
storage representation.

### `state::advanced::StateFieldSchema`

`StateFieldSchema` is immutable metadata for one validated declaration.
Applications acquire it through `StateSchemaAccess`; it has no public
constructor.

- `position()` returns deterministic zero-based template order. It is metadata,
  not an unchecked state-index API.
- `name()` borrows the normalized name used by typed state operations.
- `description()` borrows the optional normalized scientific description.

The type is `Clone + Debug + Eq + Serialize`. Serialization writes the field
name and optional description, not its private compact position.

### `state::advanced::StateSchemaAccess`

This extension trait is implemented for `SystemStateSchema`. Importing it adds:

- `shares_schema_instance(other)` for allocation identity rather than
  structural equality;
- `template_path()` for borrowed provenance as `&Path`;
- `field_schemas()` for deterministic immutable field traversal;
- `field_schema(name)` for optional name lookup;
- `contains_field(name)` for declaration membership;
- `len()` and `is_empty()` for layout size; and
- `to_json_template()` for strict pretty-printed schema JSON.

`shares_schema_instance` is constant-time. It returns `true` only when both
handles refer to the same immutable schema allocation; independently loading
identical JSON returns `false`. Observation, task, and series integrations use this
to establish one field-order authority without publishing compact indices.

Inspection does not mutate or allocate except `to_json_template`, which
allocates its returned `String` and may return `serde_json::Error`. The trait is
the supported metadata boundary for observation and persistence implementations; they
must not import `state/schema.rs` internals.

### `state::advanced::StateMaintenance`

This extension trait is implemented for `SystemState` and makes explicit the
operations needed by specialized state lifecycle and analysis code:

- `clone_structure_without_payloads(time)` creates a new empty state sharing
  the schema and copying retained type contracts without cloning payloads.
- `replace_time(time)` atomically replaces the complete coordinate and returns
  the previous `StateTime`. Callers must not use it on a state already stored
  in a `StateSeries`.
- `populated_field_count()` counts currently owned payloads.
- `payload_has_type::<T>(field)` reports exact populated payload type and
  returns `UnknownField` for undeclared names.
- `clear_payload(field)` drops one payload, returns whether one existed, and
  retains its concrete type contract.
- `clear_all_payloads()` drops every payload while retaining schema, time, and
  type contracts.

Dropping or clearing invokes ordinary payload destructors but performs no I/O.
Name-validation failures leave state unchanged.

### `state::advanced::PayloadTuple`

`PayloadTuple` is a documentation-hidden, sealed trait exported because it is
the generic bound of the basic tuple-borrow methods. Workflow provides
implementations for tuple arities two through eight. Downstream crates cannot
implement it and should never name its associated types or methods directly;
they select an implementation by writing a tuple type such as `(Position,
Velocity)`.

Compatibility for a replacement state subsystem therefore requires preserving
the public tuple-borrow call shape, supported arities, validation-before-borrow
behavior, and exact-type semantics—not exposing the current sealing or macro
implementation.

## Example

Given `config/state.json`:

```json
{
  "fields": [
    {"name": "position", "description": "Particle positions"},
    {"name": "velocity", "description": "Particle velocities"}
  ]
}
```

ordinary application code loads the schema with a typed path, initializes its
state once, performs typed work, advances time, and collects a snapshot:

```rust
use std::path::Path;

use scientific_workflow::state::basic::{StateSeries, StateTime, SystemStateSchema};

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let schema =
        SystemStateSchema::load_json_template(Path::new("config/state.json"))?;
    let time = StateTime::from_iteration_and_physical_time(0, 0.0)
        .expect("the physical coordinate is finite");
    let mut state = schema.create_empty_state(time);

    state.initialize_payload("position", vec![0.0_f64, 1.0])?;
    state.initialize_payload("velocity", vec![0.25_f64, -0.5])?;

    {
        let (positions, velocities) =
            state.borrow_payloads_mut::<(Vec<f64>, Vec<f64>)>(
                ("position", "velocity"),
            )?;
        for (position, velocity) in positions.iter_mut().zip(velocities) {
            *position += *velocity;
        }
    }

    state.advance_time(Some(1.0))?;
    let mut series = StateSeries::new(schema);
    series.push_state(state)?;

    let final_positions = series
        .last_state()
        .expect("one snapshot was collected")
        .payload::<Vec<f64>>("position")?;
    assert_eq!(final_positions, &[0.25, 0.5]);
    Ok(())
}
```

An integration that needs field metadata imports the advanced trait without
changing the owning types:

```rust,no_run
use scientific_workflow::state::advanced::{StateSchemaAccess, SystemStateSchema};

fn field_names(schema: &SystemStateSchema) -> Vec<&str> {
    schema.field_schemas().iter().map(|field| field.name()).collect()
}
```

## Not API

The following mechanisms are intentionally private and may change during a
subsystem replacement:

- `StateValue`, `ErasedValue`, erased Serde objects, boxes, and downcast
  implementation;
- `StateSlot`, retained `ValueType`, compact field indices, lookup maps, and
  schema `Arc` layout;
- JSON deserialization-only template structs and the standalone basic loader's
  byte parser;
- the schema `Arc` and pointer-comparison implementation behind the supported
  `shares_schema_instance` result;
- tuple-sealing types, tuple-generation macros, and disjoint-slot algorithms;
- the observation-only serializable payload accessor; and
- constructors for field descriptors and ownership-preserving error wrappers.

There is no public raw erased payload, unchecked field-index accessor, mutable
schema, state-series mutable-state accessor, `StateSeriesView`, stringly typed
path loader, persistence codec, task context, or scheduler hook in this
subsystem. Replacement implementations may change internal allocation and
erasure strategies while preserving the documented basic and advanced
contracts.
