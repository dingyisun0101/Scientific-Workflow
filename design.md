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

The run-level storage facade is implemented and public. It connects private
encoders and writers, owns the sole `metadata.json` lifecycle, and is available
together with every supported crate API through `scientific_workflow::prelude`.

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
        │   ├── prelude.rs        explicit complete end-user API re-exports
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
        │   ├── storage.rs        public run, reader, and decoder facade
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

Rust 2024 split-module layout is used. There are no `mod.rs` files. `lib.rs`
exports all three functional modules and the explicit prelude; storage internals
remain private behind `storage.rs`.

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

Crate-private allocation from a validated specification and time point. It is
the single invariant-establishing mechanism: it allocates exactly one empty
slot per declared field and is not part of the downstream API.

##### Reference

    StateSpec::empty and SystemState::empty -> SystemState::new

#### SystemState::empty

Creates another blank state sharing the same layout without cloning payloads.
Its final allocation is produced by `SystemState::new`, but its contract is
different: the caller supplies only a new time and the specification is
inherited from the existing state. Thus these are not two equivalent public
constructors. The name is still potentially confusing because structural
`is_empty` means “zero declared fields,” whereas this method produces a
payload-blank state (`is_blank == true`).

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

    RunOutput::start -> RunMetadata::running

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

#### chunk_filename

Returns deterministic `chunk-NNNNNN.jsonl` naming.

##### Reference

    ActiveChunk::create and ChunkMetadata::validate

## Storage encoding and writing

### JsonEncoder

Crate-private immutable configuration for one stream. It retains only the
stream name and canonical selected keys, borrows selected live payloads, and
produces one owned `EncodedRecord` without cloning them. The construction
`StateSpec` is not retained in production.

#### JsonEncoder::new

Validates stream name and selected keys, rejects duplicates, and stores keys in
template order.

##### Reference

    RunOutput::start -> JsonEncoder::new

#### JsonEncoder::fields

Iterates selected names in canonical template order for metadata construction.

##### Reference

    RunOutput::start -> JsonEncoder::fields

#### JsonEncoder::encode

Preflights selected slots, serializes borrowed erased payloads into compact JSON,
and ends all borrows before returning the owned record.

The preflight retains the successfully resolved
`&dyn erased_serde::Serialize` values and `ValuesRef` serializes those
cached borrows. This preserves typed `StateAccess` errors and per-field
`EncodeField` context while reducing state lookup from twice to once per key.
The cache contains payload references only and continues using the
encoder's existing canonical field slice for keys, avoiding redundant `&str`
storage. This introduces one small `Vec` allocation per record, so its
performance effect remains a benchmark question rather than an assumed gain.

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

### StateWriter

Non-Clone exclusive writer with one worker thread and bounded FIFO. It receives
only `EncodedRecord`, never a payload or serializer.

#### StateWriter::start

Creates safe missing relative parent directories, exclusively creates the final
stream directory, and starts its worker; existing stream output is never
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
returns the committed chunk inventory in `WriterSummary`.

##### Reference

    RunOutput::finish -> StateWriter::finish

#### StateWriter::close_admission / join_worker / drop

Private terminal lifecycle that wakes waiters and prevents detached workers.

##### Reference

    StateWriter::finish and Drop cleanup

### WriterSummary

Final ordered chunk inventory transferred into run metadata. Aggregate counts
remain derivable from chunk descriptors and are not duplicated.

#### WriterSummary::chunks

Borrows committed chunks in ordinal order.

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

The temporary name is a commit marker, not a second payload copy. Records are
written once into one inode; sealing renames that same inode to its final name.
Under the current whole-run contract, chunk temporaries are not strictly
required for reader correctness because `metadata.json` does not advertise a
chunk until successful completion and readers reject running or failed runs.
They nevertheless make incomplete crash remnants distinguishable, prevent
directory observers from treating a growing file as committed, and establish a
per-chunk commit boundary useful for recovery or future incremental metadata.
Removing them would be a valid simplification only if final filenames are
explicitly documented as non-authoritative until metadata references them.

##### Reference

    writer worker begins a chunk -> ActiveChunk::create

#### ActiveChunk::append

Appends one complete record and updates checksum and facts.

##### Reference

    writer worker FIFO loop -> ActiveChunk::append

#### ActiveChunk::seal

Synchronizes, atomically renames, and returns `ChunkMetadata`.

After the rename, the writer synchronizes the stream
directory before returning the descriptor. File `sync_all` makes record bytes
durable, but on POSIX-like filesystems it does not by itself guarantee that the
new final filename survives a crash. Metadata must never become durable while
depending on a chunk directory entry that was not synchronized.

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

## Run-level storage facade

`src/storage.rs` is the only intended downstream entry point for persistence.
Its child modules remain private and it re-exports only reader, decoder, and
error types that form part of the supported public workflow. Low-level
encoding, framing, queue, writer, checksum, and raw metadata types remain
implementation details.

### `TimeAxis`

Public run-level documentation for integer simulation time and optional
physical time. It owns only small labels and units; it never stores a time
sample.

#### `TimeAxis::new`

Creates an index-only declaration. Complete semantic validation is deferred to
run startup so fluent configuration remains infallible.

##### Reference

    RunOutputBuilder::default time -> TimeAxis::default -> TimeAxis::new
    downstream run configuration -> TimeAxis::new

#### `TimeAxis::index_unit`

Fluently declares the optional simulation-index unit.

##### Reference

    downstream run configuration -> TimeAxis::index_unit

#### `TimeAxis::physical_name`

Fluently declares the optional physical-coordinate name.

##### Reference

    downstream run configuration -> TimeAxis::physical_name

#### `TimeAxis::physical_unit`

Fluently declares the physical unit; startup requires a physical name.

##### Reference

    downstream run configuration -> TimeAxis::physical_unit

#### `TimeAxis::default`

Uses `index` with no unit or physical coordinate.

##### Reference

    RunOutputBuilder::new -> TimeAxis::default

#### `TimeAxis::into_stored`

Moves public configuration into the private versioned metadata representation.

##### Reference

    RunOutput::start -> TimeAxis::into_stored

### `StreamConfig`

Owns one logical stream's exact selected keys, safe relative directory,
optional cadence description, soft chunk-byte target, and strict queue-byte
budget. Non-zero byte types reject zero limits at the public boundary.

#### `StreamConfig::new`

Creates a declaration whose directory initially equals its stream name.

##### Reference

    downstream run construction -> StreamConfig::new

#### `StreamConfig::directory`

Overrides the relative output directory. Startup rejects unsafe paths and
directory collisions.

##### Reference

    downstream stream path customization -> StreamConfig::directory

#### `StreamConfig::cadence`

Adds descriptive cadence metadata without scheduling samples.

##### Reference

    downstream cadence documentation -> StreamConfig::cadence

### `RunOutputBuilder`

Owns unopened run configuration and a cheap shared `StateSpec` handle.

#### `RunOutputBuilder::new`

Creates a builder with default time documentation, empty run metadata, and no
streams.

##### Reference

    RunOutput::builder -> RunOutputBuilder::new
    direct downstream construction -> RunOutputBuilder::new

#### `RunOutputBuilder::time_axis`

Replaces temporal-coordinate documentation.

##### Reference

    downstream run configuration -> RunOutputBuilder::time_axis

#### `RunOutputBuilder::run_metadata`

Moves arbitrary JSON-compatible run metadata into the builder. It remains
separate from scientific payload records.

##### Reference

    dispatcher fixed/sweep provenance -> RunOutputBuilder::run_metadata
    simulation run annotations -> RunOutputBuilder::run_metadata

#### `RunOutputBuilder::stream`

Appends one stream in deterministic metadata order. Cross-stream conflicts are
validated together at startup.

##### Reference

    downstream run configuration -> RunOutputBuilder::stream

#### `RunOutputBuilder::start`

Delegates complete validation, exclusive filesystem creation, writer startup,
and initial atomic metadata publication to `RunOutput`.

##### Reference

    configured builder -> RunOutputBuilder::start -> RunOutput::start

### `RunOutput`

Non-clone exclusive owner of all active writer handles and the sole legal
terminal metadata transition. It never owns or retains a `SystemState`.

#### `RunOutput::builder`

Provides the concise public construction entry point.

##### Reference

    downstream simulation setup -> RunOutput::builder

#### `RunOutput::root`

Borrows the configured output root.

##### Reference

    diagnostics and run logging -> RunOutput::root

#### `RunOutput::streams`

Iterates names in deterministic declaration order.

##### Reference

    diagnostics and run logging -> RunOutput::streams

#### `RunOutput::sample`

Looks up one stream, borrows selected live-state payloads for encoding, ends
those borrows, then submits the owned encoded record. Submission is the
blocking backpressure boundary.

##### Reference

    simulation cadence event -> RunOutput::sample -> JsonEncoder::encode
    RunOutput::sample -> StateWriter::submit

#### `RunOutput::finish`

Consumes the coordinator, drains all streams, installs their chunk inventories,
and atomically publishes `Complete`. On writer failure it attempts `Failed`
metadata without hiding the first writer error.

##### Reference

    successful simulation termination -> RunOutput::finish

#### `RunOutput::fail`

Consumes the coordinator, drains all streams, and publishes an explicit
non-empty failed reason. A concurrent writer failure takes precedence.

##### Reference

    simulation-level terminal error -> RunOutput::fail

#### `RunOutput::start`

Privately validates all configuration before mutation, creates the root
exclusively, starts one writer per stream, and atomically commits `Running`
before returning the coordinator.

##### Reference

    RunOutputBuilder::start -> RunOutput::start

#### `RunOutput::finish_writers`

Privately drains every writer even after an earlier failure, transfers
successful chunk descriptors into metadata, and retains the first error.

##### Reference

    RunOutput::finish -> RunOutput::finish_writers
    RunOutput::fail -> RunOutput::finish_writers

### `ActiveStream`

Private pairing of one immutable borrowed-state encoder and one exclusive
bounded writer. There is exactly one entry for each running metadata stream.

### Metadata transaction helpers

`ensure_absent` performs the read-only preflight; `create_root` closes its race
with exclusive creation. `commit_metadata` validates and serializes a snapshot,
while `write_and_replace_metadata` exclusively creates a temporary sibling,
syncs it, renames it over the authoritative file, and syncs the root directory.

Unlike a chunk temporary, the metadata temporary is required for the current
lifecycle guarantee. `Running` metadata already exists when `Complete` or
`Failed` is published. Rewriting that file in place can expose truncation,
mixed old/new bytes, or invalid JSON after a crash. Writing and synchronizing a
complete sibling before atomic rename guarantees that observers see either the
previous complete metadata document or the next complete document.

##### Reference

    RunOutput::start -> ensure_absent -> create_root
    RunOutput lifecycle transition -> commit_metadata -> write_and_replace_metadata

## Public API and prelude

The crate provides `scientific_workflow::prelude`, allowing downstream code to
import the complete intended end-user API with:

```rust
use scientific_workflow::prelude::*;
```

The prelude is an explicit, curated list of crate-owned public types and
traits. It must not use wildcard re-exports from internal modules and must not
re-export general external traits such as `serde::Serialize`. This keeps
compiler errors, generated documentation, and future API reviews precise.

The state and analysis portion includes:

- `FieldSpec`, `StateSpec`, `SystemState`, and `TimePoint`;
- `SetError` and `StateError`;
- `StateSeries`, `SeriesRef`, `PushError`, and `SeriesError`.

The storage portion includes:

- `RunOutput`, `RunOutputBuilder`, `StreamConfig`, and `TimeAxis`;
- `StorageError`;
- `SeriesReader`, `Decoders`, and `PayloadDecoder`;
- `StringDecoder` and `VecF64Decoder`.

Low-level encoding, queue, chunk-format, and metadata implementation types are
not prelude members. `JsonEncoder`, `StateWriter`, `EncodedRecord`, and raw
metadata structures remain private implementation details behind `RunOutput`.
Both storage integration tests import only the public prelude, so an omitted or
accidentally private supported type is detected by compilation.

##### Reference

    downstream simulation and analysis modules -> use scientific_workflow::prelude::*
    public API integration tests -> use scientific_workflow::prelude::*
    crate root -> pub mod prelude

## Simulator integration readiness

The crate is architecture-compatible with simulator, but full integration is
not ready yet.

Ready now:

- public `StateSpec`, `SystemState`, and `TimePoint` can represent and mutate a
  simulator-owned live state;
- arbitrary simulator payloads satisfying `Serialize + Clone + Send + 'static`
  can occupy exact template keys;
- borrowed encoding, bounded writer behavior, chunk integrity, custom decoder
  registration, and typed series reconstruction are implemented and verified;
- application-specific decoder closures can reconstruct `Vec<usize>`, PiP
  lattices, activity status, or simulator aggregate payloads without adding
  built-in decoders.

Remaining simulator-side migration work:

- simulator still owns a separate fixed `io::SystemState` and separate
  `SignalWriter`/`SpaceWriter` formats, so it is not using the agreed live-state
  ownership model;
- simulator depends on registry PiP 3.0.3, while coordinated local development
  uses PiP 3.0.4.

The crate-side migration boundary is now ready. Simulator can replace its fixed
snapshot struct and two specialized writers as one coherent migration without
depending on crate-private storage internals or maintaining two new formats.

Simulator migration will require:

1. add path dependencies on local `scientific-workflow` and PiP 3.0.4 during
   coordinated development;
2. define the simulator state template and exact stream field selections;
3. make `EcoSystem` own or directly expose the scientific-workflow live state
   instead of exporting a lattice-cloning snapshot;
4. route signal and space cadence decisions through `RunOutput::sample`;
5. register simulator-specific decoders for resume and analysis;
6. replace legacy signal/space loaders only after new-format round-trip and
   resume integration tests pass.

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

    [sample] index=... physical=... signal=true space=...
    [writer] signal_records=... signal_bytes=... space_records=... space_bytes=...
    [chunk] stream=... file=... records=... bytes=... checksum_verified=true
    [readback] signal_states=... space_states=... typed_round_trip=true clone_calls=0
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
  coverage. They are covered through public boundary outcomes, such
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
| `storage_workflow` | `TimeAxis`, `StreamConfig`, `RunOutputBuilder`, `RunOutput`, `PayloadDecoder`, `Decoders`, both default decoders, and `SeriesReader`; every public success-path method including `read_all`, with private encoding/writing/format behavior verified through files and readback |
| `storage_resilience` | `StorageError` source/context behavior and reachable configuration, lifecycle, queue, decoder, record, metadata, filesystem, and integrity failure families |

The finished source reads as four coherent workflows rather than an API census.
The old aggregators and focused subdirectories have been removed.

Current test architecture: four logged integration tests across four files plus
production doctests. Each workflow passes independently and the consolidated
all-target suite passes. Formatting and Clippy across all targets pass with
warnings denied. Archive preparation remains deferred only because the agreed
local `physics_in_parallel` 3.0.4 development dependency is not yet on
crates.io, whose latest matching candidate is 3.0.3; do not replace the local
dependency before the coordinated PiP publication.
