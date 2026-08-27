# Persistence API

The `persistence` subsystem owns automatic durable task recordings and
verified reconstruction. Config parses optional operational sizing, Study
retains the effective private plan, and Runtime constructs and drives one
private session for every task. Models and applications never receive a
writer, destination, queue, flush operation, or lifecycle handle.

Observation decides which scientific fields and cadences matter. Persistence
only stores the resulting encoded observations, lifecycle metadata, and
Workflow provenance. It does not parse project JSON, bind tasks, or schedule
execution.

## Basic API

`scientific_workflow::persistence::basic` intentionally exports no Rust
symbols. Its user-facing surface is the optional root object in `study.json`:

```json
"persistence": {
  "chunk_target_bytes": 67108864,
  "queue_capacity_bytes": 67108864
}
```

Both fields are optional positive integers and default independently to 64
MiB. The first is an approximate encoded-byte chunk rollover target; the
second is the per-stream queued-byte backpressure capacity. The backend and
all destinations are inferred. Invalid settings fail during Config/Study
loading before output exists.

## Advanced API

`persistence::advanced` is the strict superset of Basic. It exposes only
completed-recording readers, payload decoder contracts, operational timing,
and `PersistenceError`. It exposes no plan or write construction.

### `StoredStateSeriesReader`

`StoredStateSeriesReader::open_completed_recording(root: &Path, decoders)`
consumes a `JsonPayloadDecoderRegistry`, reads `metadata.json`, validates the
format and successful terminal status, and retains one immutable metadata
snapshot. The reader is intentionally non-`Clone` because custom decoders may
not have meaningful clone semantics.

Inspection and reconstruction methods:

- `recording_directory() -> &Path` returns the retained typed path;
- `stream_names()` iterates logical streams in metadata order;
- `format_version()` returns the validated wire-format version;
- `user_metadata()` borrows creation-time model constants and Workflow
  provenance;
- `terminal_metadata()` borrows completion-time metadata;
- `recording_timing() -> &RecordingTiming` returns verified host timing;
- `stream_record_count(stream)` and `stream_encoded_bytes(stream)` compute
  checked metadata aggregates;
- `read_stream_as_state_series(stream)` verifies and reconstructs one complete
  ordered `StateSeries`;
- `read_all_streams_as_state_series()` reconstructs every stream in metadata
  order and returns no partial vector; and
- `read_latest_state_from_stream(stream)` verifies the newest chunk and
  reconstructs its final state.

Reads verify metadata invariants, declared file sizes, SHA-256 checksums, JSONL
framing, record counts, iteration ordering, schema shape, decoder coverage,
payload conversion, and StateSeries invariants. They perform synchronous
filesystem IO and allocate owned payloads, but start no worker and never mutate
the recording. Any failure returns no partial state/series.

### `RecordingTiming`

`RecordingTiming` is `Clone + Debug + Eq` operational provenance acquired
from a verified reader:

- `created_at_utc()` and `finalized_at_utc()` borrow RFC 3339 UTC strings;
- `active_duration_ns()` returns exact accumulated nanoseconds;
- `active_duration()` returns the same value as `Duration`; and
- `continuation_count()` returns the persisted format field. New Workflow
  recordings are single-lifecycle and therefore record zero.

Timing is host execution metadata, not scientific `StateTime`.

### Payload decoder contracts

`JsonPayloadDecoder<T>: Send + Sync + 'static` converts one borrowed complete
raw JSON value into an owned `T`. Its associated `Error` must implement
`Error + Send + Sync + 'static`. Compatible closures implement the trait.
The reader adds stream, iteration, and field context around decoder failures.

`JsonPayloadDecoderRegistry` owns at most one decoder per exact field name:

- `new()` and `with_capacity(capacity)` create empty registries;
- `with_json_field::<T>(field)` consumes and returns the registry with a
  Serde-based decoder;
- `register_for_field::<T, D>(field, decoder)` mutates the registry with a
  custom decoder;
- `len()`, `is_empty()`, and `has_decoder_for_field(field)` inspect it;
  and
- `registered_field_names()` iterates keys in unspecified hash-map order.

Field keys must be nonempty and unique. A selected stream requires decoders for
all of its fields; unrelated extra registrations are allowed.
`JsonStringDecoder` and `JsonVecF64Decoder` are zero-sized
`Copy + Clone + Debug + Default` built-ins for `String` and `Vec<f64>`.

### `PersistenceError`

`PersistenceError` is non-exhaustive and owns its paths and contextual names.
Its complete current variant set is:

- declaration/lifecycle: `Observation`, `RecordingDirectoryExists`,
  `InvalidConfiguration`, `DuplicateStateStream`, `UnknownStateStream`,
  `StateWriterClosed`, `RecordingFinished`, `RecordingDirectoryInUse`, and
  `NoRecordedState`;
- operational metadata: `OperationalTimestamp`,
  `OperationalDurationOverflow`, `UnsupportedVersion`, `InvalidMetadata`, and
  `RecordingNotComplete`;
- integrity/framing: `MissingChunk`, `ChunkSizeMismatch`,
  `ChecksumMismatch`, and `InvalidRecord`;
- decoding/state assembly: `DuplicateDecoder`, `MissingDecoder`,
  `DecodeField`, and `StateSeriesInvariant`;
- filesystem/serialization/accounting: `Io`, `Json`, `ByteCountOverflow`,
  `RecordTooLarge`, and `OutOfOrderIteration`; and
- worker lifecycle: `WriterQueueDisconnected`, `StateWriterTerminated`, and
  `StateWriterPanicked`.

Reader code normally observes only the format, integrity, decoding, and
filesystem families. The same enum retains exact private write-path failures
so Runtime can preserve their source chains through `RuntimeError`; those
variants do not authorize application write control. Variants with a `source`
preserve observation, timestamp, IO, JSON, decoder, StateSeries, or shared
worker errors.

## Example

Ordinary persistence is automatic:

```rust,no_run
fn main() -> Result<(), scientific_workflow::WorkflowError> {
    scientific_workflow::run(std::path::Path::new("."))
}
```

An analysis process can reconstruct a completed stream:

```rust,no_run
use std::path::Path;
use scientific_workflow::persistence::advanced::{
    JsonPayloadDecoderRegistry, StoredStateSeriesReader,
};

# fn read() -> Result<(), Box<dyn std::error::Error>> {
let decoders = JsonPayloadDecoderRegistry::new()
    .with_json_field::<Vec<f64>>("position")?;
let reader = StoredStateSeriesReader::open_completed_recording(
    Path::new("output/execution-123-0/replicate-000000/task-000000"),
    decoders,
)?;
let trajectory = reader.read_stream_as_state_series("trajectory")?;
println!("{}", trajectory.len());
# Ok(())
# }
```

The example path is discovered programmatically from a successful
`RunSummary` in production; it is not supplied to model or Study code.

## Not API

The effective persistence plan, backend selection, `PersistenceSession`,
`SystemStateWriter`, stream storage/layout values, queue worker, chunk
publisher, metadata mutation, directory lease, filenames, and atomic temporary
files are private. There is one internal constructor consuming Study's
already-bound observation plan, provenance, destination, and shared storage
policy. There is no builder, resume/continuation API, per-stream storage
override, explicit flush, or public completion result.

Future local or remote adapters belong behind `PersistenceSession`; adding
one must not broaden the model/task API. A replacement backend must preserve
observation order, bounded backpressure, failure evidence, terminal metadata,
effective-setting provenance, and the verified reader contract.
