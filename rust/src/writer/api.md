# Writer API

The `writer` subsystem owns application-defined scientific observation: which
state fields belong to which logical stream, how often iteration-based streams
are sampled, the optional units attached to inferred time axes, and canonical
encoding of a borrowed observation. Its canonical scopes are
`scientific_workflow::writer::basic` and
`scientific_workflow::writer::advanced`. The central preludes re-export these
same symbols without wrapping them.

Writer is intentionally independent of persistence. It contains no output
path, task or replicate identity, chunk size, queue size, filename, checksum,
metadata lifecycle, recovery policy, or completed-recording handle. The
runtime and record backend infer and own those concerns.

An ordinary application returns its writer from
`ScientificModel::writer(&Constants)`. The trait supplies
`Writer::all_fields()` by default, so a model implements that method only when
field selection, named streams, cadence, or units carry scientific meaning.
Because Study calls it during preflight and task calls it again at execution,
the function must be deterministic and externally side-effect-free.

## Basic API

### `writer::basic::Writer`

`Writer` is an immutable, owned definition of the scientific output requested
by an application. It is `Clone + Debug + Eq`; cloning copies only small
definition metadata and never touches a state or payload. It is safe to move or
share according to its ordinary auto traits and performs no I/O, allocation of
scientific data, blocking, background work, persistence, or cancellation.

The constructors are:

- `Writer::all_fields()` creates the minimum-burden definition: one stream
  named `state`, every schema field in schema order, and sampling every
  iteration. This constructor cannot fail because all remaining facts are
  inferred when the definition binds to a schema.
- `Writer::fields(fields)` creates the same inferred `state` stream with an
  explicit nonempty field selection. It returns `WriterError` for blank or
  duplicate field names. Field existence and canonical ordering are checked
  later, when a schema is available.
- `Writer::streams(streams)` accepts one or more already validated `Stream`
  definitions. Stream names must be unique after whitespace trimming. Input
  order is retained as deterministic stream order and later determines the
  backend's inferred stream-directory ordinals.
- `with_iteration_unit(unit)` attaches a nonempty unit to the axis whose name
  is always inferred as `iteration`.
- `with_physical_time_unit(unit)` attaches a nonempty unit to the axis whose
  name is always inferred as `physical_time`.

The two unit methods consume and return `Writer`, allowing fluent composition.
They trim surrounding whitespace and return `EmptyAxisUnit` without producing
a partially changed definition. Axis names are not parameters because they
already follow from `StateTime`.

`Writer` does not bind itself to a state schema and deliberately exposes no
basic getters. Ordinary application code declares intent and supplies the
definition to a task/runtime integration. Advanced code may validate it once
with `WriterDescriptor::bind`.

### `writer::basic::Stream`

`Stream` is one owned scientific stream definition. A stream name is retained
because it distinguishes scientifically meaningful outputs; a filesystem
directory is not retained because it can be derived. `Stream` is
`Clone + Debug + Eq` and owns only normalized names, field selections, and a
positive cadence.

- `Stream::all_fields(name)` selects all fields of the future bound schema.
  It returns `EmptyStreamName` when trimming leaves no name.
- `Stream::fields(name, fields)` selects a nonempty set of named fields. It
  trims all names and rejects an empty stream name, an empty field name, an
  empty selection, or a duplicate field. Selection order is not persisted:
  binding reorders fields into canonical schema order.
- `every_iterations(iterations)` changes the default cadence of one to the
  supplied positive value. Iteration zero is selected, followed by each
  iteration divisible by the value. Zero returns
  `InvalidSamplingInterval` and leaves the consumed original unavailable in
  the ordinary Rust builder style.

There is intentionally no public `Sampling`, field-selection, axis, or
checkpoint type. A full-state stream is inferred as checkpoint-capable after
schema binding. The final state is a session concern and is offered once even
when its iteration does not align with a stream cadence.

### `writer::basic::WriterError`

`WriterError` is the non-exhaustive error for definition, binding,
observation, and encoding. Basic users usually propagate it; advanced users
may inspect contextual variants. Every owned stream/field name remains valid
after temporary inputs and observations are dropped. Errors contain no
scientific payload.

Definition and binding variants are:

- `EmptyWriter`: `Writer::streams` received no streams.
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
  immutable schema allocation used by the bound writer.
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

## Advanced API

`writer::advanced` re-exports every Basic API symbol above. It adds the
supported boundary used by Workflow runtime and replaceable record backends.
It does not expose the concrete runtime session.

### `writer::advanced::WriterDescriptor`

`WriterDescriptor` is an immutable `Clone + Debug` writer definition validated
against one `SystemStateSchema`. `WriterDescriptor::bind(writer, schema)`
consumes the definition, retains a cheap clone of the schema handle, verifies
every selected field, canonicalizes field order, and computes complete-state
coverage. It allocates only definition metadata and performs no payload access,
I/O, persistence, blocking, or background work.

- `schema()` returns the exact retained schema handle.
- `streams()` returns descriptors in definition order.
- `iteration_unit()` and `physical_time_unit()` return borrowed optional
  normalized units. Axis names remain inferred and therefore have no getters.

A runtime binds once before it creates output. Backends use the result to
derive metadata and deterministic layout; application code normally does not.

### `writer::advanced::StreamDescriptor`

`StreamDescriptor` is an immutable `Clone + Debug + Eq` description obtained
only through `WriterDescriptor::bind`.

- `name()` returns the normalized logical stream name.
- `fields()` returns `StateFieldSchema` values in canonical state-schema
  order, independent of the application's selection order.
- `every_iterations()` returns the positive cadence as `u64`; no sampling
  implementation type crosses the boundary.
- `covers_complete_state()` reports whether the selected fields exactly cover
  the bound schema. A record backend may use this fact to infer eligible
  checkpoint streams.

Descriptors own their small metadata and borrow nothing. Consumers must not
derive filesystem paths by reimplementing a backend's private layout rule;
stream ordering is the portable contract.

### `writer::advanced::Observation<'a>`

`Observation<'a>` is a checked, copyable borrowed pair of one descriptor and
one live `SystemState`. `Observation::new(writer, state)` performs an exact
schema-allocation identity check and returns `SchemaMismatch` on failure. It
does not inspect payload slots. The check uses the supported
`state::advanced::StateSchemaAccess` contract, so writer does not depend on the
state module's allocation internals.

- `time()` returns the state's `StateTime` by value.
- `state()` returns the original borrowed state.
- `writer()` returns the validating descriptor.
- `encode_stream(stream)` synchronously borrows and serializes the supplied
  stream's selected payloads into one owned `EncodedObservation`.

The `StreamDescriptor` passed to `encode_stream` is expected to come from the
observation's `writer().streams()` slice. Encoding performs no state clone and
retains no payload reference after return. It may allocate the output bytes and
temporary erased-Serde references. It performs no filesystem I/O or blocking
other than work performed by user serializers.

### `writer::advanced::EncodedObservation`

`EncodedObservation` is the owned handoff from writer to backend. It is
intentionally non-`Clone`, so one encoded allocation moves into exactly one
sink or storage queue.

- `stream()` borrows the logical stream name.
- `time()` returns the scientific coordinate by value.
- `bytes()` borrows the complete unframed canonical JSON object.
- `into_bytes()` consumes the observation and returns its allocation.

`Debug` prints the stream, time, and byte length but never payload bytes. The
current canonical document contains `iteration`, optional `physical_time`, and
a positional `values` array in descriptor field order. Newline framing,
checksums, chunks, and files remain backend responsibilities.

### `writer::advanced::ObservationSink`

`ObservationSink` is the replaceable persistence port. Implementations are
`Send` because a runtime may own or move a sink across its execution boundary.
Its associated `Error` must implement `Error + Send + Sync + 'static` so the
runtime can retain and compose failures.

- `submit(&mut self, observation)` takes ownership of one complete encoded
  observation. It may synchronously apply backpressure. On failure, the
  observation is consumed; implementations must not publish partial framing.
- `finish(&mut self, outcome)` receives the runtime-owned terminal outcome.
  Implementations atomically make success, failure, or cancellation durable
  according to their own documented contract.

The runtime, not task code, calls `finish` exactly once. A backend must retain
stream order, preserve observation bytes, reject invalid order, and ensure a
failed `submit` cannot be mistaken for successful completion.

### `writer::advanced::SessionOutcome`

`SessionOutcome` is `Clone + Debug + Eq` and describes the single terminal
verdict passed to an `ObservationSink`:

- `Complete` means task work and required final observation succeeded.
- `Failed { reason }` owns the stable failure reason.
- `Cancelled { reason }` distinguishes cancellation and optionally owns an
  explanatory reason.

It is a backend request, not proof that finalization succeeded. The sink's
`finish` result remains authoritative for persistence failure.

## Example

The ordinary flow loads state, initializes it, and declares only irreducible
scientific output intent:

```rust,no_run
use std::path::Path;

use scientific_workflow::state::basic::{StateTime, SystemStateSchema};
use scientific_workflow::writer::basic::{Stream, Writer, WriterError};

# fn example() -> Result<(), Box<dyn std::error::Error>> {
let schema = SystemStateSchema::load_json_template(Path::new("config/state.json"))?;
let mut state = schema.create_empty_state(StateTime::from_iteration(0));
state.initialize_payload("position", vec![0.0_f64, 1.0])?;
state.initialize_payload("energy", 0.5_f64)?;

let writer = Writer::streams([
    Stream::fields("trajectory", ["position"])?.every_iterations(10)?,
    Stream::all_fields("checkpoint")?.every_iterations(100)?,
])?
.with_physical_time_unit("s")?;

// ScientificModel::writer returns `writer`; Workflow infers paths, schema
// metadata, checkpoint eligibility, lifecycle, and persistence policy.
# let _ = (state, writer);
# Ok(())
# }
```

A record integration binds and encodes through only the advanced boundary:

```rust,no_run
use scientific_workflow::state::basic::SystemState;
use scientific_workflow::writer::advanced::{
    EncodedObservation, Observation, Writer, WriterDescriptor, WriterError,
};

fn encode_default(state: &SystemState) -> Result<EncodedObservation, WriterError> {
    let descriptor = WriterDescriptor::bind(Writer::all_fields(), state.schema())?;
    let observation = Observation::new(&descriptor, state)?;
    observation.encode_stream(&descriptor.streams()[0])
}
```

## Not API

The following remain private and may change during a writer replacement:

- `WriterSession`, its accepted-iteration vector, due-stream selection, and
  final-observation deduplication;
- `IterationSampling`, its `NonZeroU64` representation, and divisibility
  implementation;
- the internal field-selection enum and descriptor constructors;
- erased-Serde reference adapters, active-field tracking, and JSON serializer
  structs;
- allocation choices such as `Box<str>`, boxed slices, and temporary vectors;
  and
- the adapter from `EncodedObservation` to the transitional storage JSONL
  record.

There is no public session constructor, sampling type, time-axis metadata
type, encoder implementation, directory field, storage configuration, queue,
chunk, checkpoint flag, path, metadata map, lifecycle guard, or backend worker
in this subsystem. Replacement implementations may change internal sampling
and encoding machinery while preserving the documented definition,
descriptor, observation, owned handoff, error, and sink contracts.
