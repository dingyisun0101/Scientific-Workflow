# Observation API

The `observation` subsystem owns application-defined scientific observation: which
state fields belong to which logical stream, how often iteration-based streams
are sampled, the optional units attached to inferred time axes, and canonical
encoding of a borrowed observation. Its canonical scope is the
`scientific_workflow::observation` module root. The ordinary prelude re-exports
the downstream-facing declarations without wrapping them.

Observation is intentionally independent of persistence. It contains no output
path, task or replicate identity, chunk size, queue size, filename, checksum,
metadata lifecycle, recovery policy, or completed-recording handle. The
runtime and persistence backend infer and own those concerns.

An ordinary application returns its observation plan from
`ExecutionUnit::preflight(&Constants, &SystemStateSchema)`. The trait supplies
`ObservationPlan::all_fields()` by default, so an execution unit implements that method only when
field selection, named streams, cadence, or units carry scientific meaning.
Study calls it once during preflight, trusts the unit-owned domain validation,
binds the returned plan to the validated
state schema, and stores that exact bound plan in the compiled task. Runtime
never calls the execution unit method again.

## Public API

### `observation::ObservationPlan`

`ObservationPlan` is an immutable, owned definition of the scientific observation requested
by an application. It is `Clone + Debug + Eq`; cloning copies only small
definition metadata and never touches a state or payload. It is safe to move or
share according to its ordinary auto traits and performs no I/O, allocation of
scientific data, blocking, background work, persistence, or cancellation.

The constructors are:

- `ObservationPlan::all_fields()` creates the minimum-burden definition: one stream
  named `state`, every schema field in schema order, and sampling every
  iteration. This constructor cannot fail because all remaining facts are
  inferred when the definition binds to a schema.
- `ObservationPlan::fields(fields)` creates the same inferred `state` stream with an
  explicit nonempty field selection. It returns `ObservationError` for blank or
  duplicate field names. Field existence and canonical ordering are checked
  later, when a schema is available.
- `ObservationPlan::streams(streams)` accepts one or more already validated `ObservationStream`
  definitions. ObservationStream names must be unique after whitespace trimming. Input
  order is retained as deterministic stream order and later determines the
  backend's inferred stream-directory ordinals.
- `with_iteration_unit(unit)` attaches a nonempty unit to the axis whose name
  is always inferred as `iteration`.
- `with_physical_time_unit(unit)` attaches a nonempty unit to the axis whose
  name is always inferred as `physical_time`.

The two unit methods consume and return `ObservationPlan`, allowing fluent composition.
They trim surrounding whitespace and return `EmptyAxisUnit` without producing
a partially changed definition. Axis names are not parameters because they
already follow from `StateTime`.

`ObservationPlan` does not bind itself to a state schema and deliberately
exposes no getters. Application code declares intent; Study owns schema binding
and retains the private bound representation.

### `observation::ObservationStream`

`ObservationStream` is one owned scientific stream definition. A stream name is retained
because it distinguishes scientifically meaningful outputs; a filesystem
directory is not retained because it can be derived. `ObservationStream` is
`Clone + Debug + Eq` and owns only normalized names, field selections, and a
positive cadence.

- `ObservationStream::all_fields(name)` selects all fields of the future bound schema.
  It returns `EmptyStreamName` when trimming leaves no name.
- `ObservationStream::fields(name, fields)` selects a nonempty set of named fields. It
  trims all names and rejects an empty stream name, an empty field name, an
  empty selection, or a duplicate field. Selection order is not persisted:
  binding reorders fields into canonical schema order.
- `every_iterations(iterations)` changes the default cadence of one to the
  supplied positive value. Iteration zero is selected, followed by each
  iteration divisible by the value. Zero returns
  `InvalidSamplingInterval` and leaves the consumed original unavailable in
  the ordinary Rust builder style.

There is intentionally no public `Sampling`, field-selection, axis, or
checkpoint type. Selecting all fields makes complete state reconstruction
possible, but Observation does not classify or label a stream as a checkpoint.
The final state is a session concern and is offered once even when its
iteration does not align with a stream cadence.

### `observation::ObservationError`

`ObservationError` is the non-exhaustive error for definition, binding,
observation, and encoding. Ordinary users usually propagate it; specialized users
may inspect contextual variants. Every owned stream/field name remains valid
after temporary inputs and observations are dropped. Errors contain no
scientific payload.

Definition and binding variants are:

- `EmptyPlan`: `ObservationPlan::streams` received no streams.
- `EmptyStreamName`: a stream name was blank after trimming.
- `DuplicateStreamName { stream }`: two normalized logical names collide.
- `EmptyFieldName { stream }`: one selected field was blank.
- `EmptyFieldSelection { stream }`: a selected-field stream has no fields, or
  schema binding produced no selected fields.
- `DuplicateField { stream, field }`: a stream selected one field twice.
- `UnknownField { stream, field }`: schema binding found no declaration for a
  selected name.
- `InvalidSamplingInterval { stream }`: a cadence was zero.
- `EmptyAxisUnit { axis }`: an optional unit was blank.

Observation and encoding variants are:

- `SchemaMismatch { iteration }`: the observed state does not share the exact
  immutable schema allocation used by the bound observation plan.
- `StateAccess { stream, iteration, field, source }`: a due selected field is
  absent or otherwise cannot be borrowed. The originating `StateError` is
  preserved as the error source.
- `EncodeField { stream, iteration, field, source }`: a payload's Serde
  implementation rejected canonical JSON encoding. The `serde_json::Error`
  source is preserved.
- `NonIncreasingObservation { stream, previous, next }`: an observation moved
  backward relative to the last accepted iteration for a stream. Repeating an
  equal iteration is idempotently skipped by the private session, so this
  variant currently reports a decreasing coordinate.

Construction, binding, and encoding failures are atomic. A failed definition
is not produced; a failed bind yields no descriptor; a failed session
observation advances no stream's accepted-iteration marker and submits no
encoded observation.

## Crate-visible peer API

Workflow peers use these crate-visible contracts:

- `BoundObservationPlan` is the one-time, schema-checked plan retained by
  Study. Its stream and axis accessors supply immutable scientific descriptors.
- `BoundObservationStream` exposes one normalized stream name, canonical field
  order, cadence, and bound schema metadata to Persistence.
- `ObservationSession` owns accepted-iteration markers. `new`, `observe`, and
  `observe_final` select due streams, enforce order/schema identity, and return
  canonical encoded records failure-atomically.
- `EncodedObservation::into_parts()` returns
  `(stream_name, StateTime, Vec<u8>)`: owned canonical JSON bytes that can cross
  into an asynchronous writer without retaining a borrow of mutable state.

The owned byte handoff is an intentional efficiency boundary. Scientific state
payloads are not required to be `Sync`, Runtime mutates them between steps, and
the persistence worker is asynchronous; Observation therefore encodes while it
holds a valid synchronous borrow. Observation owns selection and deterministic
scientific-record encoding. Persistence owns queueing, framing, chunk files,
checksums, and durable lifecycle. A backend replacement consumes this same
handoff unless it deliberately coordinates a new canonical encoding contract
with Observation.

## Example

The ordinary flow loads state, initializes it, and declares only irreducible
scientific observation intent:

```rust,no_run
use std::path::Path;

use scientific_workflow::state::{StateTime, SystemStateSchema};
use scientific_workflow::observation::{
    ObservationError, ObservationPlan, ObservationStream,
};

# fn example() -> Result<(), Box<dyn std::error::Error>> {
let schema = SystemStateSchema::load_json_template(Path::new("wf_configs/states/state.json"))?;
let mut state = schema.create_empty_state(StateTime::from_iteration(0));
state.initialize_payload("position", vec![0.0_f64, 1.0])?;
state.initialize_payload("energy", 0.5_f64)?;

let plan = ObservationPlan::streams([
    ObservationStream::fields("trajectory", ["position"])?.every_iterations(10)?,
    ObservationStream::all_fields("checkpoint")?.every_iterations(100)?,
])?
.with_physical_time_unit("s")?;

// ExecutionUnit::preflight returns `plan`; Workflow infers paths,
// schema metadata, lifecycle, and persistence policy. The all-fields stream
// retains every field needed for complete state reconstruction.
# let _ = (state, plan);
# Ok(())
# }
```

## Not API

The following implementation details remain private and may change during an
observation replacement:

- accepted-iteration storage, due-stream algorithms, and final-observation
  deduplication behind `ObservationSession`;
- concrete storage behind `BoundObservationPlan`, `BoundObservationStream`,
  `StateObservation`, and `EncodedObservation`;
- `IterationSampling`, its `NonZeroU64` representation, and divisibility
  implementation;
- the internal field-selection enum and descriptor constructors;
- erased-Serde reference adapters, active-field tracking, and JSON serializer
  structs;
- allocation choices such as `Box<str>`, boxed slices, and temporary vectors;
  and
- the persistence adapter after the documented `EncodedObservation` handoff.

There is no public session constructor, sampling type, time-axis metadata
type, encoder implementation, directory field, persistence configuration, queue,
chunk, checkpoint flag, path, metadata map, lifecycle guard, or backend worker
in this subsystem. Replacement implementations may change internal sampling
and encoding machinery while preserving the documented declaration and error
contracts plus the crate-visible peer API above. Persistence owns backend
construction and lifecycle.
