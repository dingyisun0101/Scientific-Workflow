# Scientific Workflow: Rust Architecture

## Scope and status

This is the authoritative clean-slate design for the Rust crate. Superseded
alternatives are intentionally absent.

Current scope:

- Rust only; Python and bridging are later modules.
- JSON persistence only; protobuf is out of scope.
- No backward compatibility or legacy support.
- Payloads may be any `Serialize + Clone + Send + 'static` Rust value.
- Production tests live under `tests/`, never inside module files.
- Additional built-in payload decoders are deferred until core development is
  complete.

Implemented and verified:

- `system_state`: mutable simulation-owned heterogeneous state;
- `time_series`: eager in-memory analysis collection;
- storage format, borrowed encoder, bounded writer, decoder registry, two
  default decoders, and eager reader.

The next stage is the run-level storage facade. It will connect existing
encoders and writers, own the sole `metadata.json` lifecycle, and expose the
storage module from `lib.rs`.

## End-to-end workflow

    simulation owns and mutates one complete SystemState
        -> simulation cadence selects a logical stream
        -> JsonEncoder borrows only that stream's selected fields
        -> one owned EncodedRecord is produced
        -> StateWriter::submit applies bounded backpressure
        -> worker appends indivisible records to byte-targeted chunks
        -> run facade commits chunk descriptors and completion metadata

    completed metadata.json and immutable chunks
        -> SeriesReader validates metadata and selects a stream
        -> each JSONL record is read into borrowed raw field slices
        -> reader looks up the decoder registered for each exact key
        -> decoder converts only that raw field into its concrete payload
        -> reader assembles SystemState values
        -> reader returns one complete StateSeries

`StateSeries` is an analysis result, not a runtime writer buffer. The persisted
stream is the authoritative sampled history.

## Core invariants

1. `StateSpec` fixes keys and order but never persists Rust type names.
2. All states derived from one spec share its immutable allocation through
   `Arc`.
3. The simulation owns and mutates its live `SystemState`, one mutable payload
   borrow at a time.
4. `set`, `take`, writer submission, and decoded insertion transfer ownership;
   they do not clone payloads.
5. `SystemState::clone` and `StateSeries::clone` are explicit deep clones and
   should be avoided in performance-sensitive paths.
6. Encoding borrows payloads but necessarily allocates the owned serialized
   record bytes.
7. Each stream has independent selected keys, cadence, directory, queue,
   chunk sequence, and analysis series.
8. One sampled partial state is one indivisible JSONL record.
9. Chunk rollover uses exact framed bytes; no record is split.
10. Writer admission is bounded by a user byte budget and an internal maximum
    of 1,024 accepted but uncommitted records.
11. Full queues block the simulation until capacity becomes available.
12. A run directory contains exactly one structural metadata file.
13. Chunk files contain only compact records with readable field keys.
14. A reader returns a complete series or an error, never a partial series.
15. Reader key lookup and decoder conversion are separate responsibilities.

## Current file tree

    scientific-workflow/
    ├── design.md                 authoritative architecture and references
    ├── tests.md                  integration-test architecture and coverage
    ├── todo.md                   next-stage and deferred work only
    ├── README.md                 repository test entry points
    └── dev/
        ├── Cargo.toml            publishable Rust package manifest
        ├── README.md             crate-facing usage and test documentation
        ├── src/
        │   ├── lib.rs            crate documentation and public module exports
        │   ├── system_state.rs   system-state facade and re-exports
        │   ├── system_state/
        │   │   ├── error.rs      state and ownership-preserving set errors
        │   │   ├── spec.rs       JSON template and shared layout
        │   │   ├── state.rs      TimePoint and SystemState
        │   │   └── value.rs      private boxed type erasure
        │   ├── time_series.rs    analysis-series facade and re-exports
        │   ├── time_series/
        │   │   ├── error.rs      collection/access errors
        │   │   └── series.rs     StateSeries, SeriesRef, and PushError
        │   └── storage/
        │       ├── error.rs      complete storage error vocabulary
        │       ├── format.rs     metadata and encoded-record contract
        │       ├── encoder.rs    borrowed SystemState-to-JSON encoding
        │       ├── writer.rs     bounded queue and chunk persistence
        │       ├── decoder.rs    decoder trait, registry, and re-exports
        │       ├── decoder/
        │       │   ├── string.rs String default decoder
        │       │   └── vec_f64.rs Vec<f64> default decoder
        │       └── reader.rs     verified eager reconstruction
        └── tests/
            ├── fixtures/state.json
            ├── state_workflow.rs
            ├── analysis_workflow.rs
            ├── storage_workflow.rs
            └── storage_resilience.rs

Rust 2024 split-module layout is used. There are no `mod.rs` files. Storage is
intentionally not exported from `lib.rs` until the next run-level facade makes
its lifecycle safe as a public API.

## System state

### FieldSpec

Immutable normalized field declaration: compact index, exact key, and optional
natural-language description. It contains no type or codec tag.

#### FieldSpec::new

Private normalized construction during template validation.

##### Reference

    StateSpec::from_template -> FieldSpec::new

#### FieldSpec::index

Returns template-order slot index.

##### Reference

    SystemState compact slot lookup and metadata inspection

#### FieldSpec::name

Returns the exact normalized key.

##### Reference

    encoder field selection and dictionary-style access

#### FieldSpec::description

Returns optional documentation without affecting behavior.

##### Reference

    run metadata construction -> FieldMetadata

### StateSpec

Cheap cloneable handle to one immutable `Arc`-shared layout containing ordered
fields, key lookup, and template provenance.

#### StateSpec::load

Loads the first layout from the required JSON template path.

##### Reference

    program initialization -> StateSpec::load

#### StateSpec::parse

Crate-private reconstruction from metadata bytes using identical validation.

##### Reference

    SeriesReader stream schema -> StateSpec::parse

#### StateSpec::empty

Creates a blank state sharing the layout.

##### Reference

    simulation initialization and SeriesReader state assembly

#### StateSpec::to_json

Returns normalized pretty JSON for semantic round trips.

##### Reference

    template inspection and tests -> StateSpec::to_json

#### StateSpec::source

Returns retained template or metadata provenance.

##### Reference

    diagnostics and tests -> StateSpec::source

#### StateSpec::fields

Returns declarations in deterministic template order.

##### Reference

    encoder canonicalization and metadata construction

#### StateSpec::len

Returns declared field count.

##### Reference

    SystemState allocation -> StateSpec::len

#### StateSpec::is_empty

Reports whether no fields are declared.

##### Reference

    time-only state inspection and tests

#### StateSpec::get

Looks up one declaration by exact key.

##### Reference

    JsonEncoder configuration -> StateSpec::get

#### StateSpec::contains

Reports whether a key is declared.

##### Reference

    configuration inspection and tests

#### StateSpec::shares_layout

Performs constant-time `Arc` identity comparison.

##### Reference

    StateSeries::push -> StateSpec::shares_layout

#### StateSpec::index_of

Crate-private exact key-to-slot resolution.

##### Reference

    SystemState accessors -> StateSpec::index_of

#### StateSpec::from_template

Private semantic validation and layout construction.

##### Reference

    StateSpec::load/parse -> StateSpec::from_template

### TimePoint

Small `Copy` coordinate with mandatory `u64` simulation index and optional
finite physical time.

#### TimePoint::new

Creates an index-only coordinate.

##### Reference

    simulation or reader without physical time -> TimePoint::new

#### TimePoint::from_physical

Creates a coordinate only when physical time is finite.

##### Reference

    simulation initialization and SeriesReader record reconstruction

#### TimePoint::index

Returns the authoritative ordering index.

##### Reference

    StateSeries ordering, writer ordering, chunk descriptors

#### TimePoint::physical

Returns optional physical time.

##### Reference

    JsonEncoder record header and analysis

### SystemState

Simulation-owned fixed-layout dictionary of optional heterogeneous concrete
payloads plus one mutable `TimePoint`.

#### SystemState::new

Crate-private allocation from a validated spec.

##### Reference

    StateSpec::empty and SystemState::empty -> SystemState::new

#### SystemState::empty

Creates another blank state sharing the same layout without cloning payloads.

##### Reference

    simulation scratch states and tests

#### SystemState::time

Returns the complete `Copy` coordinate.

##### Reference

    encoder, writer, series, simulation

#### SystemState::set_time

Replaces time and returns the previous coordinate.

##### Reference

    simulation-owned explicit time reset -> SystemState::set_time

#### SystemState::advance

Increments index by one and optionally adds a physical-time delta
transactionally.

##### Reference

    simulation evolution loop -> SystemState::advance

#### SystemState::spec

Returns the shared immutable spec.

##### Reference

    StateSeries::push and JsonEncoder compatibility checks

#### SystemState::len

Returns structural slot count.

##### Reference

    state inspection and tests

#### SystemState::is_empty

Reports whether the layout declares no slots.

##### Reference

    time-only state inspection

#### SystemState::loaded

Counts populated slots.

##### Reference

    simulation diagnostics

#### SystemState::is_blank

Reports whether all declared slots are empty.

##### Reference

    empty-state lifecycle checks

#### SystemState::fields

Returns ordered field declarations.

##### Reference

    dictionary inspection

#### SystemState::has

Checks whether an exact declared key has a payload.

##### Reference

    simulation conditional access and decoder tests

#### SystemState::is<T>

Checks the concrete type in one populated slot.

##### Reference

    runtime type inspection

#### SystemState::set<T>

Moves a payload into a slot; same-type replacement returns the previous
payload, while rejection returns the incoming payload through `SetError<T>`.

##### Reference

    simulation initialization and Decoders::decode_into -> SystemState::set

#### SystemState::get<T>

Borrows one concrete payload immutably.

##### Reference

    simulation inspection and analysis

#### SystemState::get_mut<T>

Borrows one concrete payload mutably; users mutate one field at a time.

##### Reference

    simulation evolution and StateSeries::field_mut

#### SystemState::take<T>

Moves one concrete payload out without cloning and restores it on type error.

##### Reference

    ownership handoff and allocation reuse

#### SystemState::clear

Drops one payload and reports whether it existed.

##### Reference

    explicit working-set cleanup

#### SystemState::clear_all

Drops every payload while retaining layout and slots.

##### Reference

    state reuse and cleanup

#### SystemState::serializable

Crate-private borrowed erased-Serde view of one populated payload.

##### Reference

    JsonEncoder::encode -> SystemState::serializable

#### SystemState::clone

Deep-clones each populated payload and shares only immutable layout metadata.

##### Reference

    explicit snapshots and StateSeries::clone

### StateError

Non-exhaustive state/template/time/access error vocabulary. It never owns a
scientific payload.

### SetError<T>

Ownership-preserving `SystemState::set` rejection containing `StateError` and
the unchanged incoming `T`.

#### SetError::new

Crate-private rejection construction.

##### Reference

    SystemState::set validation failure -> SetError::new

#### SetError::error

Borrows the rejection reason.

##### Reference

    application failure inspection

#### SetError::payload

Borrows the unchanged rejected payload.

##### Reference

    application recovery decision

#### SetError::into_parts

Returns `(StateError, T)` without cloning.

##### Reference

    decoder insertion handling and caller ownership recovery

### StateValue and ErasedValue

Private boxed type-erasure implementation. `StateValue::{new,type_id,type_name,
is,downcast_ref,downcast_mut,downcast,serializable}` delegate concrete type and
ownership operations to `ErasedValue::{clone_box,as_any,as_any_mut,into_any,
concrete_type_name,as_serialize}`.

#### StateValue and ErasedValue method references

##### Reference

    SystemState::set/get/get_mut/take/serializable/clone
        -> StateValue -> private ErasedValue blanket implementation

## Time series

### SeriesError

Non-exhaustive analysis error with only layout mismatch, non-increasing time,
position bounds, and contextualized field access variants. It contains no IO
or serialization concerns.

### StateSeries

Growable `Vec<SystemState>` analysis collection enforcing shared layout identity
and strictly increasing simulation indices.

#### StateSeries::new

Creates an empty series.

##### Reference

    analysis initialization

#### StateSeries::with_capacity

Creates an empty series with state-owner capacity.

##### Reference

    SeriesReader::read and known-size analysis

#### StateSeries::spec

Returns the canonical shared layout.

##### Reference

    SeriesReader state assembly and analysis

#### StateSeries::view

Returns lightweight copyable `SeriesRef`.

##### Reference

    borrowed analysis instead of deep clone

#### StateSeries::len / is_empty / capacity

Expose ordinary collection facts.

##### Reference

    analysis and tests

#### StateSeries::reserve

Reserves additional state-owner capacity.

##### Reference

    analysis allocation planning

#### StateSeries::get / first / last / states / iter

Provide immutable collection access without cloning.

##### Reference

    analysis traversal and reader verification

#### StateSeries::field_mut<T>

Mutates one typed payload without exposing mutable state time or structure.

##### Reference

    analysis mutation -> SystemState::get_mut

#### StateSeries::push

Moves a state into the collection after layout and time validation.

##### Reference

    analysis construction and SeriesReader::read_chunk

#### StateSeries::pop

Moves the last state out.

##### Reference

    analysis ownership recovery

#### StateSeries::clear

Drops states while retaining vector capacity and canonical spec.

##### Reference

    analysis working-set reuse

#### StateSeries::into_states

Consumes the series and returns its vector allocation.

##### Reference

    downstream ownership transfer

#### StateSeries::clone

Explicitly deep-clones every state payload.

##### Reference

    independent mutable analysis copies only

### SeriesRef

Copyable borrowed pair of canonical spec and immutable state slice.

#### SeriesRef::new

Private view construction.

##### Reference

    StateSeries::view -> SeriesRef::new

#### SeriesRef::spec / len / is_empty / get / first / last / states / iter

Expose borrowed collection facts and traversal.

##### Reference

    lightweight analysis paths

### PushError

Owns a `SeriesError` and the unchanged rejected `SystemState` in a failure-only
box so failed append never loses payload ownership.

#### PushError::new

Private rejection construction.

##### Reference

    StateSeries::push failure -> PushError::new

#### PushError::error / state

Borrow rejection reason and unchanged state.

##### Reference

    caller inspection after failed push

#### PushError::into_parts

Returns `(SeriesError, SystemState)` without cloning.

##### Reference

    SeriesReader invariant context and caller recovery

## Storage format

### On-disk layout

    run/
    ├── metadata.json
    ├── signal/
    │   ├── chunk-000000.jsonl
    │   └── chunk-000001.jsonl
    └── space/
        └── chunk-000000.jsonl

One compact record:

    {"index":12,"physical":0.25,"values":{"values":[1.0,2.0],"label":"sample"}}

`physical` is omitted when absent. Field keys remain for readability and exact
decoder dispatch. Metadata stores schemas, run facts, byte limits, lifecycle,
and chunk descriptors once; no sidecar metadata exists.

### RunMetadata

Complete versioned contents of the sole metadata document.

#### RunMetadata::running

Creates initial running metadata from time, run attributes, and streams.

##### Reference

    next-stage RunOutput construction -> RunMetadata::running

#### RunMetadata::validate

Validates structure without filesystem access.

##### Reference

    every metadata commit and SeriesReader::open

#### RunMetadata::stream / stream_mut

Look up immutable or mutable stream declarations by exact name.

##### Reference

    SeriesReader selection and RunOutput chunk bookkeeping

### RunStatus

Persisted `Running`, `Complete`, or non-empty-message `Failed` lifecycle.

#### RunStatus::validate

Validates lifecycle-specific content.

##### Reference

    RunMetadata::validate -> RunStatus::validate

### RecordFormat

Versioned JSON plus JSON Lines declaration.

#### RecordFormat::json_lines / validate

Construct and validate the only supported encoding pair.

##### Reference

    RunMetadata::running/validate -> RecordFormat

### TimeAxis

Metadata names and optional units for integer and physical coordinates.

#### TimeAxis::validate

Rejects empty labels and a physical unit without a physical name.

##### Reference

    RunMetadata::validate -> TimeAxis::validate

### StreamMetadata

One logical stream's directory, cadence, ordered fields, byte limits, and
committed chunk inventory.

#### StreamMetadata::validate

Validates exact names, safe paths, non-zero limits, unique fields, and ordered
non-overlapping chunks.

##### Reference

    RunMetadata::validate -> StreamMetadata::validate

### FieldMetadata

One exact payload key and optional natural-language description.

#### FieldMetadata::validate

Rejects empty names and empty present descriptions.

##### Reference

    StreamMetadata::validate -> FieldMetadata::validate

### ChunkMetadata

Immutable ordinal, filename, record/byte counts, checksum, and index range.

#### ChunkMetadata::validate

Validates deterministic naming, non-empty facts, range order, and checksum
syntax.

##### Reference

    StreamMetadata::validate and SeriesReader integrity verification

### EncodedRecord

Non-Clone owner of one complete compact JSON object plus its framing newline and
validated `TimePoint`.

#### EncodedRecord::new

Adds the single framing newline to encoded JSON bytes.

##### Reference

    JsonEncoder::encode -> EncodedRecord::new

#### EncodedRecord::time / len / bytes

Return temporal coordinate, exact framed length, and borrowed bytes.

##### Reference

    StateWriter admission, ordering, chunk rollover, and append

#### EncodedRecord::into_bytes

Moves out the complete framed allocation.

##### Reference

    ownership tests and future alternate sink integration

#### chunk_filename

Returns deterministic `chunk-NNNNNN.jsonl` naming.

##### Reference

    ActiveChunk::create and ChunkMetadata::validate

## Storage encoding and writing

### JsonEncoder

Crate-private immutable configuration for one stream. It borrows selected live
payloads and produces one owned `EncodedRecord` without cloning them.

#### JsonEncoder::new

Validates stream name and selected keys, rejects duplicates, and stores keys in
template order.

##### Reference

    next-stage RunOutput stream construction

#### JsonEncoder::stream / spec / fields

Expose immutable normalized configuration.

##### Reference

    metadata construction, diagnostics, and tests

#### JsonEncoder::encode

Preflights selected slots, serializes borrowed erased payloads into compact JSON,
and ends all borrows before returning the owned record.

##### Reference

    RunOutput::sample -> JsonEncoder::encode -> StateWriter::submit

### RecordRef, ValuesRef, and ErasedRef

Private borrowing-only Serde adapters used during encoding.

#### ValuesRef::serialize / ErasedRef::serialize

Serialize selected values in canonical order and delegate to each payload's
existing `Serialize` implementation.

##### Reference

    JsonEncoder::encode -> serde_json::to_vec -> private adapters

### WriterConfig

Immutable stream name, absent output directory, non-zero chunk target, and
strict queue-byte budget.

#### WriterConfig::new

Validates configuration without filesystem mutation.

##### Reference

    RunOutput construction -> WriterConfig::new

#### WriterConfig::stream / directory / max_chunk_bytes / queue_bytes

Expose validated writer configuration.

##### Reference

    StateWriter::start and metadata construction

### StateWriter

Non-Clone exclusive writer with one worker thread and bounded FIFO. It receives
only `EncodedRecord`, never a payload or serializer.

#### StateWriter::start

Creates a new stream directory and starts its worker; existing output is never
overwritten.

##### Reference

    RunOutput construction -> StateWriter::start

#### StateWriter::submit

Consumes a record and blocks until both record-count and byte capacity permit
FIFO admission. Impossible oversized records fail immediately.

##### Reference

    RunOutput::sample -> StateWriter::submit

#### StateWriter::finish

Closes admission, drains work, seals the final chunk, joins the worker, and
returns `WriterSummary`.

##### Reference

    RunOutput::finish -> StateWriter::finish

#### StateWriter::close_admission / join_worker / drop

Private terminal lifecycle that wakes waiters and prevents detached workers.

##### Reference

    StateWriter::finish and Drop cleanup

### WriterSummary

Final stream name, ordered chunk inventory, total records, and exact bytes.

#### WriterSummary::stream / chunks / records / bytes

Expose immutable completion facts.

##### Reference

    RunOutput metadata completion and diagnostics

### Shared and QueueState

Private mutex/condition-variable state for FIFO, capacity, terminal error, and
writer summary.

#### Shared::new

Creates an open empty queue state.

##### Reference

    StateWriter::start -> Shared::new

### ActiveChunk

Private temporary-file owner with incremental SHA-256 and exact counters.

#### ActiveChunk::create

Creates one deterministic temporary chunk.

##### Reference

    writer worker begins a chunk -> ActiveChunk::create

#### ActiveChunk::append

Appends one complete record and updates checksum and facts.

##### Reference

    writer worker FIFO loop -> ActiveChunk::append

#### ActiveChunk::seal

Synchronizes, atomically renames, and returns `ChunkMetadata`.

##### Reference

    chunk rollover or writer finish -> ActiveChunk::seal

## Payload decoding

### Responsibility split

1. `SeriesReader` validates a record and retrieves a raw value by schema key.
2. `Decoders` retrieves the decoder registered for the same exact key.
3. `PayloadDecoder<T>` receives only that raw JSON field and returns owned `T`.
4. The registry moves `T` into the matching empty state slot.

A configured decoder entry exists per payload key. A Rust decoder type may be
reused when several keys share the same representation. Decoders never perform
key lookup or see sibling fields.

### PayloadDecoder<T>

Thread-safe typed conversion contract from borrowed raw JSON `&str` to owned
`T`, with an owned thread-safe associated error. Compatible closures receive a
blanket implementation.

#### PayloadDecoder::decode

Converts exactly one complete raw JSON value.

##### Reference

    Decoders erased adapter -> PayloadDecoder::decode -> concrete T

### Decoders

Non-Clone heterogeneous exact-key registry. Additional entries are permitted so
one registry can cover several streams.

#### Decoders::new / with_capacity

Create an empty registry, optionally reserving key capacity.

##### Reference

    analysis setup -> Decoders construction

#### Decoders::add

Binds one exact key to one typed decoder, rejecting empty or duplicate keys.

##### Reference

    application reader configuration -> Decoders::add

#### Decoders::len / is_empty / contains / keys

Expose registry configuration without decoder internals.

##### Reference

    setup inspection, Debug, and tests

#### Decoders::require

Crate-private coverage check for every field in a selected stream.

##### Reference

    SeriesReader::read before chunk IO -> Decoders::require

#### Decoders::decode_into

Crate-private lookup, conversion, ownership transfer, and contextual error
wrapping for one field.

##### Reference

    SeriesReader canonical field loop -> Decoders::decode_into
        -> PayloadDecoder::decode -> SystemState::set

### TypedDecoder, ErasedPayloadDecoder, and DecoderInsertError

Private type-erasure adapter retaining each decoder's concrete output type. An
unexpected occupied destination is restored transactionally.

#### ErasedPayloadDecoder::decode_into

Performs typed conversion and insertion behind the heterogeneous registry.

##### Reference

    Decoders::decode_into -> ErasedPayloadDecoder::decode_into

### VecF64Decoder

Zero-sized default decoder for JSON numeric arrays to owned `Vec<f64>`. It adds
no length, finite-value, or domain validation.

#### VecF64Decoder::decode

Calls `serde_json::from_str::<Vec<f64>>` directly on the selected raw field.

##### Reference

    Decoders entry for key -> VecF64Decoder::decode -> Vec<f64>

### StringDecoder

Zero-sized default decoder for JSON strings to owned `String`. It preserves
content and performs no trimming or normalization.

#### StringDecoder::decode

Calls `serde_json::from_str::<String>` with standard escape and Unicode handling.

##### Reference

    Decoders entry for key -> StringDecoder::decode -> String

Only these two defaults are included during main development. Applications may
register closures or named decoder types for tensors and domain values.

## Storage reading

### SeriesReader

All-in-one eager reader owning output root, validated completed metadata, and a
caller-configured `Decoders` registry. It is intentionally non-Clone.

#### SeriesReader::open

Reads and validates `metadata.json`, requires `RunStatus::Complete`, and consumes
the registry.

##### Reference

    analysis startup -> SeriesReader::open

#### SeriesReader::root

Returns the supplied root without canonicalization.

##### Reference

    diagnostics and tests

#### SeriesReader::streams

Iterates stream names in metadata order.

##### Reference

    analysis stream discovery

#### SeriesReader::read

Checks stream existence and decoder coverage, verifies every chunk, decodes all
states transactionally, and returns one complete `StateSeries`.

##### Reference

    analysis request -> SeriesReader::read(stream)

#### SeriesReader::read_all

Returns ordered `(stream name, StateSeries)` pairs and drops prior results if a
later stream fails.

##### Reference

    whole-run eager analysis -> SeriesReader::read_all

#### SeriesReader::read_chunk

Private buffered JSONL traversal, size/checksum verification, strict ordering,
state assembly, and descriptor-fact validation.

##### Reference

    SeriesReader::read -> SeriesReader::read_chunk

### BorrowedRecord, BorrowedValues, and BorrowedValuesVisitor

Private record representation borrowing each `RawValue` from one line buffer.
Only small field keys are owned. Duplicate payload keys are rejected.

#### BorrowedValues::deserialize

Starts strict borrowed object parsing.

##### Reference

    serde_json::from_slice in SeriesReader::read_chunk

#### BorrowedValuesVisitor::expecting / visit_map

Describe and collect unique keys with borrowed raw value boundaries.

##### Reference

    BorrowedValues::deserialize -> BorrowedValuesVisitor

### StreamTemplateRef

Private borrowed adapter used to reconstruct a stream's shared `StateSpec` from
metadata field declarations.

## Errors

### StorageError

Non-exhaustive storage error vocabulary covering configuration, lifecycle,
metadata, chunk integrity, record structure, state access, encoding, decoder
registration/conversion, series invariants, IO, JSON, accounting, ordering,
queue termination, and worker panic. Decoder and lower-level state/series
errors preserve their source chains. No variant owns scientific payload data.

##### Reference

    every storage Result boundary -> StorageError

## Next stage: run-level storage facade

The next production module is `src/storage.rs`. It will expose a minimal public
facade around already verified primitives.

Planned `RunOutput` responsibilities:

- validate the run root and stream declarations;
- build one `JsonEncoder` and `StateWriter` per stream;
- write initial `Running` metadata before sampling;
- route `sample(stream, &state)` through encoder then writer;
- finish every writer, collect summaries, atomically commit `Complete` metadata;
- commit `Failed` metadata when a reportable terminal workflow error occurs;
- prevent repeated finish/sample lifecycle misuse;
- never own or clone a simulation payload.

The design decision required before implementation is the exact builder and
atomic metadata-commit API. Existing encoder, writer, decoder, and reader
contracts do not need restructuring.

## Verification gate

Before beginning the run-level facade:

1. `cargo fmt --all -- --check` passes.
2. `cargo test --all-targets --no-fail-fast --locked` passes.
3. `cargo clippy --all-targets --all-features --locked -- -D warnings` passes.
4. `cargo package --allow-dirty --no-verify` succeeds when dependency registry
   availability permits it.
5. `git diff --check` passes.

The unified storage target must prove both default decoder round trips and a
real PiP tensor workflow using application-provided per-key decoders. It prints
bounded logs under `cargo test --test storage_workflow -- --nocapture` and removes all
temporary output afterward.

## Integration-test architecture

The detailed and authoritative test architecture is maintained in `tests.md`.
This section records only its relationship to the crate architecture; when test
scope or file allocation changes, update `tests.md` first and keep this summary
consistent.

The former focused file-mirroring suites were useful during production-file
review and have now been replaced by four behavior-oriented Cargo integration
targets plus the real JSON fixture:

    tests/
    ├── fixtures/state.json
    ├── state_workflow.rs
    ├── analysis_workflow.rs
    ├── storage_workflow.rs
    └── storage_resilience.rs

Every target prints a short stable report under `--nocapture`. Logs contain
counts, indices, byte sizes, chunk facts, pointer/clone evidence, and expected
error classes; they never dump full scientific payloads or nondeterministic
thread timing.

### state_workflow.rs

One realistic simulation-state lifecycle using the checked-in template and PiP
tensors. It covers template semantic round trip, shared layouts, typed insertion
and sequential mutation, time advancement, zero-copy extraction with allocation
identity, explicit deep-clone accounting, rejected-set payload recovery, and
bounded diagnostics.

Key log output:

    [template] fields=3 round_trip=true
    [state] index=... loaded=... mutation=verified
    [ownership] set_take_pointer_preserved=true clone_calls=...
    [result] state_workflow=passed

### analysis_workflow.rs

Builds an ordered `StateSeries` from evolving states, verifies move-based push
and pop, shared-layout and increasing-time rejection with ownership recovery,
borrowed `SeriesRef` traversal, narrow field mutation, capacity reuse, and the
explicit cost boundary of deep cloning.

Key log output:

    [series] states=... indices=[...]
    [invariants] layout_rejection=true ordering_rejection=true
    [ownership] push_pop_preserved=true clone_calls=...
    [result] analysis_workflow=passed

### storage_workflow.rs

The principal success-path test. It evolves one live state, samples multiple
streams at different cadences, uses borrowed encoding and bounded writers,
commits one metadata file, verifies automatic byte chunking, then reconstructs
complete series. It exercises `StringDecoder`, `VecF64Decoder`, and an
application-provided PiP tensor decoder. It explicitly asserts semantic JSON
metadata round trip and typed payload equality.

Key log output:

    [sample] stream=... index=... encoded_bytes=...
    [writer] stream=... records=... chunks=... bytes=...
    [chunk] file=... records=... indices=.....=... checksum_verified=true
    [readback] stream=... states=... typed_round_trip=true
    [result] storage_workflow=passed

### storage_resilience.rs

One failure-oriented target retaining only cross-boundary risks: strict queue
byte rejection, non-increasing writer indices, existing-output refusal,
incomplete metadata, missing decoder coverage, wrong payload type with source
context, missing/size-changed/checksum-corrupt chunks, and worker termination.
Each case asserts the exact `StorageError` class and the most important owned
context without exhaustively snapshotting every display string.

Key log output:

    [expected-error] case=... variant=... context_verified=true
    [integrity] missing=true size=true checksum=true
    [backpressure] oversized_rejected=true ordering_rejected=true
    [result] storage_resilience=passed

Trivial getter, formatting, constructor, and one-variant tests are removed when
the same behavior is naturally exercised by these workflows. High-risk
properties remain explicit assertions rather than being considered covered
merely because a method was called. The four targets run independently and
clean up their own precisely owned temporary directories.

### Consolidated coverage rule

The four-file design must cover the complete implemented API surface, but it
does not recreate one test per method:

- every public structure is constructed or obtained in at least one workflow;
- every public method is invoked in its natural scenario;
- ownership, mutation, ordering, serialization, backpressure, integrity, and
  reconstruction methods receive explicit semantic assertions;
- trivial accessors may be checked together in one workflow section;
- `Debug`, `Display`, iterator, and `Error::source` implementations are invoked
  only where their bounded output or source preservation is part of a useful
  diagnostic;
- crate-private and private helpers are not tested directly merely to increase
  coverage. They are covered through public or staged boundary outcomes, such
  as checksum verification proving `ActiveChunk::append/seal` and reader
  corruption tests proving borrowed-record validation;
- not every `StorageError` variant needs an isolated constructor test. Every
  failure family and every externally reachable high-risk branch must be
  represented.

Required method allocation:

| Workflow | Structures and API families exercised |
|---|---|
| `state_workflow` | `FieldSpec`, `StateSpec`, `TimePoint`, `SystemState`, `StateError`, `SetError`; all public spec, time, state-access, ownership, clear, clone, and inspection methods |
| `analysis_workflow` | `StateSeries`, `SeriesRef`, `PushError`, `SeriesError`; all public construction, capacity, lookup, iteration, mutation, append/rejection, extraction, clear, and clone methods |
| `storage_workflow` | metadata/format structures, `EncodedRecord`, `JsonEncoder`, `WriterConfig`, `StateWriter`, `WriterSummary`, `PayloadDecoder`, `Decoders`, both default decoders, and `SeriesReader`; every success-path method including `read_all` |
| `storage_resilience` | `StorageError` source/context behavior and reachable configuration, lifecycle, queue, decoder, record, metadata, filesystem, and integrity failure families |

The finished source reads as four coherent workflows rather than an API census.
The old aggregators and focused subdirectories have been removed.

Current test architecture: four logged integration tests across four files plus
four doctests. Each workflow passes independently and the consolidated
all-target suite passes. Formatting and Clippy across all targets pass with
warnings denied. Archive preparation remains deferred only because the agreed local
`physics_in_parallel` 3.0.4 development dependency is not yet on crates.io,
whose latest matching candidate is 3.0.3; do not replace the local dependency
before the coordinated PiP publication.
