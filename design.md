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
        -> JsonStateRecordEncoder borrows only that stream's selected fields
        -> one owned EncodedStateRecord is produced
        -> StateWriterWorker::submit_record applies bounded backpressure
        -> worker appends indivisible records to byte-targeted chunks
        -> writer facade commits chunk descriptors and completion metadata

    completed metadata.json and immutable chunks
        -> StoredStateSeriesReader validates metadata and selects a stream
        -> each JSONL record is read into borrowed raw field slices
        -> reader looks up the decoder registered for each exact key
        -> decoder converts only that raw field into its concrete payload
        -> reader assembles SystemState values
        -> reader returns one complete StateSeries

`StateSeries` is an analysis result, not a runtime writer buffer. The persisted
stream is the authoritative sampled history.

## Core invariants

1. `SystemStateSchema` fixes keys and order but never persists Rust type names.
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
        │   │   ├── state.rs      SimulationTime and SystemState
        │   │   └── value.rs      private boxed type erasure
        │   ├── time_series.rs    analysis-series facade and re-exports
        │   ├── time_series/
        │   │   ├── error.rs      collection/access errors
        │   │   └── series.rs     StateSeries, StateSeriesView, and StateSeriesPushError
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
            ├── fixtures/
            │   ├── state.json
            │   └── coupled_state.json
            ├── state_workflow.rs
            ├── analysis_workflow.rs
            ├── storage_workflow.rs
            └── storage_resilience.rs

Rust 2024 split-module layout is used. There are no `mod.rs` files. `lib.rs`
exports all three functional modules and the explicit prelude; storage internals
remain private behind `storage.rs`.

## System state

### StateFieldSchema

Immutable normalized field declaration: compact index, exact key, and optional
natural-language description. It contains no type or codec tag.

#### StateFieldSchema::new

Private normalized construction during template validation.

##### Reference

    SystemStateSchema::from_template -> StateFieldSchema::new

#### StateFieldSchema::position

Returns template-order slot index.

##### Reference

    SystemState compact slot lookup and metadata inspection

#### StateFieldSchema::name

Returns the exact normalized key.

##### Reference

    encoder field selection and dictionary-style access

#### StateFieldSchema::description

Returns optional documentation without affecting behavior.

##### Reference

    run metadata construction -> StateFieldMetadata

### SystemStateSchema

Cheap cloneable handle to one immutable `Arc`-shared layout containing ordered
fields, key lookup, and template provenance.

#### SystemStateSchema::load_json_template

Loads the first layout from the required JSON template path.

##### Reference

    program initialization -> SystemStateSchema::load_json_template

#### SystemStateSchema::parse

Crate-private reconstruction from metadata bytes using identical validation.

##### Reference

    StoredStateSeriesReader stream schema -> SystemStateSchema::parse

#### SystemStateSchema::create_empty_state

Creates a blank state sharing the layout.

##### Reference

    simulation initialization and StoredStateSeriesReader state assembly

#### SystemStateSchema::to_json_template

Returns normalized pretty JSON for semantic round trips.

##### Reference

    template inspection and tests -> SystemStateSchema::to_json_template

#### SystemStateSchema::template_path

Returns retained template or metadata provenance.

##### Reference

    diagnostics and tests -> SystemStateSchema::template_path

#### SystemStateSchema::field_schemas

Returns declarations in deterministic template order.

##### Reference

    encoder canonicalization and metadata construction

#### SystemStateSchema::len

Returns declared field count.

##### Reference

    SystemState allocation -> SystemStateSchema::len

#### SystemStateSchema::is_empty

Reports whether no fields are declared.

##### Reference

    time-only state inspection and tests

#### SystemStateSchema::field_schema

Looks up one declaration by exact key.

##### Reference

    JsonStateRecordEncoder configuration -> SystemStateSchema::field_schema

#### SystemStateSchema::contains_field

Reports whether a key is declared.

##### Reference

    configuration inspection and tests

#### SystemStateSchema::shares_schema_instance

Performs constant-time `Arc` identity comparison.

##### Reference

    StateSeries::push_state -> SystemStateSchema::shares_schema_instance

#### SystemStateSchema::index_of

Crate-private exact key-to-slot resolution.

##### Reference

    SystemState accessors -> SystemStateSchema::index_of

#### SystemStateSchema::from_template

Private semantic validation and layout construction.

##### Reference

    SystemStateSchema::load_json_template/parse -> SystemStateSchema::from_template

### SimulationTime

Small `Copy` coordinate with mandatory `u64` simulation index and optional
finite physical time.

#### SimulationTime::from_step

Creates an index-only coordinate.

##### Reference

    simulation or reader without physical time -> SimulationTime::from_step

#### SimulationTime::from_step_and_physical_time

Creates a coordinate only when physical time is finite.

##### Reference

    simulation initialization and StoredStateSeriesReader record reconstruction

#### SimulationTime::step

Returns the authoritative ordering index.

##### Reference

    StateSeries ordering, writer ordering, chunk descriptors

#### SimulationTime::physical_time

Returns optional physical time.

##### Reference

    JsonStateRecordEncoder record header and analysis

### SystemState

Simulation-owned fixed-layout dictionary of optional heterogeneous concrete
payloads plus one mutable `SimulationTime`.

#### SystemState::new

Crate-private allocation from a validated specification and time point. It is
the single structural invariant-establishing mechanism: it allocates exactly
one empty, initially type-unbound slot per declared field and is not part of the
downstream API.

##### Reference

    SystemStateSchema::create_empty_state -> SystemState::new

#### SystemState::clone_structure_without_payloads

Creates another blank state sharing the same specification and retaining the
source state's per-slot concrete type definitions without cloning payloads. Its
final allocation is produced from the same structural constructor, but its
contract is different: the caller supplies only a new time, and both field
layout and assembly-established type contracts are inherited. The name is
still potentially confusing because structural `is_empty` means “zero declared
fields,” whereas this method produces a payload-blank state (`is_blank ==
true`).

##### Reference

    assembled state -> SystemState::clone_structure_without_payloads -> typed simulation scratch state
    tests and state reuse

#### SystemState::simulation_time

Returns the complete `Copy` coordinate.

##### Reference

    encoder, writer, series, simulation

#### SystemState::replace_simulation_time

Replaces time and returns the previous coordinate.

##### Reference

    simulation-owned explicit time reset -> SystemState::replace_simulation_time

#### SystemState::advance_simulation_time

Increments index by one and optionally adds a physical-time delta
transactionally.

##### Reference

    simulation evolution loop -> SystemState::advance_simulation_time

#### SystemState::schema

Returns the shared immutable spec.

##### Reference

    StateSeries::push_state and JsonStateRecordEncoder compatibility checks

#### SystemState::declared_field_count

Returns structural slot count.

##### Reference

    state inspection and tests

#### SystemState::has_no_declared_fields

Reports whether the layout declares no slots.

##### Reference

    time-only state inspection

#### SystemState::populated_field_count

Counts populated slots.

##### Reference

    simulation diagnostics

#### SystemState::has_no_payloads

Reports whether all declared slots are empty.

##### Reference

    empty-state lifecycle checks

#### SystemState::field_schemas

Returns ordered field declarations.

##### Reference

    dictionary inspection

#### SystemState::contains_payload

Checks whether an exact declared key has a payload.

##### Reference

    simulation conditional access and decoder tests

#### SystemState::payload_has_type<T>

Checks the concrete type in one populated slot.

##### Reference

    runtime type inspection

#### SystemState::insert_payload<T>

Moves a payload into a slot. First insertion establishes the slot's concrete
type definition; same-type replacement returns the previous payload, while a
later different-type insertion is rejected even when the slot is temporarily
empty. Rejection returns the incoming payload through `PayloadInsertError<T>`.

##### Reference

    simulation initialization and JsonPayloadDecoderRegistry::decode_into -> SystemState::insert_payload

#### SystemState::payload<T>

Borrows one concrete payload immutably.

##### Reference

    simulation inspection and analysis

#### SystemState::payload_mut<T>

Borrows one concrete payload mutably through on-demand name resolution.

##### Reference

    simulation evolution and StateSeries::payload_mut_at

#### SystemState::borrow_payloads<Q>

Borrows a tuple of distinct populated payloads immutably. `Q` is written by the
caller as the tuple of expected concrete payload types; the method argument is
the equally sized tuple of field names. The sealed query implementation is
generated internally and is not an end-user concept.

##### Reference

    coupled scientific inspection and multi-input kernels

#### SystemState::borrow_payloads_mut<Q>

Borrows a tuple of distinct populated payloads mutably without moving or
cloning them. It resolves and validates the complete request before returning
any reference and rejects repeated resolved slots. One borrow is intended to
surround an entire coupled kernel or simulation sweep.

##### Reference

    simulator EcoSystem sweep -> SystemState::borrow_payloads_mut<(SquareLattice, TaxonTable)>
    coupled scientific integrators and solvers

### PayloadTuple

Doc-hidden, sealed public trait required only as the generic mapping behind
`SystemState::borrow_payloads` and `SystemState::borrow_payloads_mut`. A private declarative macro
implements it for heterogeneous tuples of arity two through eight. It is not
re-exported by the prelude and cannot be implemented by downstream crates.

#### PayloadTuple::borrow

Resolves a tuple of field names, validates distinct indices, payload presence,
and retained concrete types, then returns the equally shaped immutable
reference tuple.

##### Reference

    SystemState::borrow_payloads -> PayloadTuple::borrow

#### PayloadTuple::borrow_mut

Performs the same complete preflight before safely separating the slot slice
and returning the equally shaped mutable reference tuple.

##### Reference

    SystemState::borrow_payloads_mut -> PayloadTuple::borrow_mut

#### SystemState::take_payload<T>

Moves one concrete payload out without cloning and restores it on type error.
The now-empty slot retains its assembly-established concrete type definition.

##### Reference

    ownership handoff and allocation reuse

#### SystemState::clear_payload

Drops one payload and reports whether it existed while retaining the field's
concrete type definition.

##### Reference

    explicit working-set cleanup

#### SystemState::clear_all_payloads

Drops every payload while retaining layout, slots, and concrete type
definitions.

##### Reference

    state reuse and cleanup

#### SystemState::serializable

Crate-private borrowed erased-Serde view of one populated payload.

##### Reference

    JsonStateRecordEncoder::encode -> SystemState::serializable

#### SystemState::clone

Deep-clones each populated payload and shares only immutable layout metadata.

##### Reference

    explicit snapshots and StateSeries::clone

### StateError

Non-exhaustive state/template/time/access error vocabulary. It includes
`RepeatedPayloadBorrow` for aliased tuple requests and never owns a scientific
payload.

### PayloadInsertError<T>

Ownership-preserving `SystemState::insert_payload` rejection containing `StateError` and
the unchanged incoming `T`.

#### PayloadInsertError::new

Crate-private rejection construction.

##### Reference

    SystemState::insert_payload validation failure -> PayloadInsertError::new

#### PayloadInsertError::error

Borrows the rejection reason.

##### Reference

    application failure inspection

#### PayloadInsertError::payload

Borrows the unchanged rejected payload.

##### Reference

    application recovery decision

#### PayloadInsertError::into_parts

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

    SystemState::insert_payload/payload/payload_mut/take_payload/serializable/clone
        -> StateValue -> private ErasedValue blanket implementation

### StateSlot

Private storage for one declared field. It pairs the optional owned
`StateValue` with an optional retained `ValueType`. A newly templated slot is
unbound; its first successful payload insertion establishes the type contract.
Removing the payload does not remove that contract.

#### StateSlot::unbound

Creates a payload-empty slot whose concrete type has not yet been established.

##### Reference

    SystemState::new -> StateSlot::unbound

#### StateSlot::empty_like

Creates a payload-empty slot while copying the existing retained type contract.

##### Reference

    SystemState::clone_structure_without_payloads -> StateSlot::empty_like

### ValueType

Private copyable runtime type identity. It retains both `TypeId` for exact
comparison and `type_name` for diagnostics after a slot's payload has been
taken or cleared.

#### ValueType::of<T>

Captures the concrete runtime identity and diagnostic name of `T` when the
first payload is installed.

##### Reference

    SystemState::insert_payload -> ValueType::of

#### ValueType::is<T>

Tests an expected concrete type against a slot's retained type contract.

##### Reference

    SystemState::payload_has_type / validate_slot / insert_payload
        -> ValueType::is

## Time series

### StateSeriesError

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

    StoredStateSeriesReader::read_stream_as_state_series and known-size analysis

#### StateSeries::schema

Returns the canonical shared layout.

##### Reference

    StoredStateSeriesReader state assembly and analysis

#### StateSeries::as_view

Returns lightweight copyable `StateSeriesView`.

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

#### StateSeries::state_at / first_state / last_state / as_state_slice / iter

Provide immutable collection access without cloning.

##### Reference

    analysis traversal and reader verification

#### StateSeries::payload_mut_at<T>

Mutates one typed payload without exposing mutable state time or structure.

##### Reference

    analysis mutation -> SystemState::payload_mut

#### StateSeries::push_state

Moves a state into the collection after layout and time validation.

##### Reference

    analysis construction and StoredStateSeriesReader::read_chunk

#### StateSeries::pop_state

Moves the last state out.

##### Reference

    analysis ownership recovery

#### StateSeries::clear_states

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

### StateSeriesView

Copyable borrowed pair of canonical spec and immutable state slice.

#### StateSeriesView::new

Private view construction.

##### Reference

    StateSeries::as_view -> StateSeriesView::new

#### StateSeriesView::schema / len / is_empty / state_at / first_state / last_state / as_state_slice / iter

Expose borrowed collection facts and traversal.

##### Reference

    lightweight analysis paths

### StateSeriesPushError

Owns a `StateSeriesError` and the unchanged rejected `SystemState` in a failure-only
box so failed append never loses payload ownership.

#### StateSeriesPushError::new

Private rejection construction.

##### Reference

    StateSeries::push_state failure -> StateSeriesPushError::new

#### StateSeriesPushError::error / state

Borrow rejection reason and unchanged state.

##### Reference

    caller inspection after failed push

#### StateSeriesPushError::into_parts

Returns `(StateSeriesError, SystemState)` without cloning.

##### Reference

    StoredStateSeriesReader invariant context and caller recovery

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

### RecordingMetadata

Complete versioned contents of the sole metadata document.

#### RecordingMetadata::running

Creates initial running metadata from time, run attributes, and streams.

##### Reference

    SystemStateWriter::create_new_recording -> RecordingMetadata::running

#### RecordingMetadata::validate

Validates structure without filesystem access.

##### Reference

    every metadata commit and StoredStateSeriesReader::open_completed_recording

#### RecordingMetadata::stream / stream_mut

Look up immutable or mutable stream declarations by exact name.

##### Reference

    StoredStateSeriesReader selection and SystemStateWriter chunk bookkeeping

### RecordingStatus

Persisted `Running`, `Complete`, or non-empty-message `Failed` lifecycle.

#### RecordingStatus::validate

Validates lifecycle-specific content.

##### Reference

    RecordingMetadata::validate -> RecordingStatus::validate

### RecordFormat

Versioned JSON plus JSON Lines declaration.

#### RecordFormat::json_lines / validate

Construct and validate the only supported encoding pair.

##### Reference

    RecordingMetadata::running/validate -> RecordFormat

### TimeAxisMetadata

Metadata names and optional units for integer and physical coordinates.

#### TimeAxisMetadata::validate

Rejects empty labels and a physical unit without a physical name.

##### Reference

    RecordingMetadata::validate -> TimeAxisMetadata::validate

### StateStreamMetadata

One logical stream's directory, cadence, ordered fields, byte limits, and
committed chunk inventory.

#### StateStreamMetadata::validate

Validates exact names, safe paths, non-zero limits, unique fields, and ordered
non-overlapping chunks.

##### Reference

    RecordingMetadata::validate -> StateStreamMetadata::validate

### StateFieldMetadata

One exact payload key and optional natural-language description.

#### StateFieldMetadata::validate

Rejects empty names and empty present descriptions.

##### Reference

    StateStreamMetadata::validate -> StateFieldMetadata::validate

### ChunkMetadata

Immutable ordinal, filename, record/byte counts, checksum, and index range.

#### ChunkMetadata::validate

Validates deterministic naming, non-empty facts, range order, and checksum
syntax.

##### Reference

    StateStreamMetadata::validate and StoredStateSeriesReader integrity verification

### EncodedStateRecord

Non-Clone owner of one complete compact JSON object plus its framing newline and
validated `SimulationTime`.

#### EncodedStateRecord::new

Adds the single framing newline to encoded JSON bytes.

##### Reference

    JsonStateRecordEncoder::encode -> EncodedStateRecord::new

#### EncodedStateRecord::simulation_time / len / bytes

Return temporal coordinate, exact framed length, and borrowed bytes.

##### Reference

    StateWriterWorker admission, ordering, chunk rollover, and append

#### chunk_filename

Returns deterministic `chunk-NNNNNN.jsonl` naming.

##### Reference

    ActiveChunk::create and ChunkMetadata::validate

#### chunk_temp_filename

Returns the corresponding `chunk-NNNNNN.jsonl.tmp` open lifecycle name.

##### Reference

    ActiveChunk creation/recovery and stream-directory inventory

## Storage encoding and writing

### JsonStateRecordEncoder

Crate-private immutable configuration for one stream. It retains only the
stream name and canonical selected keys, borrows selected live payloads, and
produces one owned `EncodedStateRecord` without cloning them. The construction
`SystemStateSchema` is not retained in production.

#### JsonStateRecordEncoder::new

Validates stream name and selected keys, rejects duplicates, and stores keys in
template order.

##### Reference

    SystemStateWriter::create_new_recording -> JsonStateRecordEncoder::new

#### JsonStateRecordEncoder::fields

Iterates selected names in canonical template order for metadata construction.

##### Reference

    SystemStateWriter::create_new_recording -> JsonStateRecordEncoder::fields

#### JsonStateRecordEncoder::encode

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

    SystemStateWriter::record_state_to_stream -> JsonStateRecordEncoder::encode -> StateWriterWorker::submit_record

### RecordRef, ValuesRef, and ErasedRef

Private borrowing-only Serde adapters used during encoding.

#### ValuesRef::serialize / ErasedRef::serialize

Serialize selected values in canonical order and delegate to each payload's
existing `Serialize` implementation.

##### Reference

    JsonStateRecordEncoder::encode -> serde_json::to_vec -> private adapters

### StateStreamStorageConfig

Immutable stream name, output directory, non-zero chunk target, and strict
queue-byte budget shared by start and resume.

#### StateStreamStorageConfig::new

Validates configuration without filesystem mutation.

##### Reference

    SystemStateWriter construction -> StateStreamStorageConfig::new

#### StateStreamStorageConfig::create_directory

Creates the configured stream directory without replacing an existing entry.

##### Reference

    SystemStateWriter::create_new_recording -> StateStreamStorageConfig::create_directory

### RecoveredStateStream

Owned append position for one stream: next ordinal, final accepted step,
optional active chunk, and optional locator for its latest complete open
record.

#### RecoveredStateStream::empty / latest_open_record

`empty` seeds a new stream. `latest_open_record` exposes only the recovered
locator needed for checkpoint reconstruction before the seed moves into the
worker.

##### Reference

    StateWriterWorker::start_new_recording -> RecoveredStateStream::empty
    SystemStateWriter::continue_recording -> RecoveredStateStream::latest_open_record

### RecoveredUnsealedRecord

Copy-free file range identifying the latest complete JSON object in a recovered
open chunk.

#### RecoveredUnsealedRecord::path / offset / bytes

Borrow the payload path and return the exact byte range used for one checkpoint
decode.

##### Reference

    decode_resume_state -> RecoveredUnsealedRecord::path / offset / bytes

### StateWriterWorker

Non-Clone exclusive recording worker with one bounded FIFO. It receives named
`EncodedStateRecord` values, never a payload or serializer. Private
`StateStreamSink` values preserve independent chunk state for every stream.

#### StateWriterWorker::start_new_recording

Starts one worker after recording startup has created every stream directory
and published initial running metadata.

##### Reference

    SystemStateWriter construction -> StateWriterWorker::start_new_recording

#### StateWriterWorker::recover_state_stream / resume

`recover` checks the sealed ordinal prefix by filename without opening sealed
payloads, then scans at most the highest open payload. It completes a prepared
rename or returns an owned `RecoveredStateStream` containing the valid unprepared file
handle, counters, checksum state, next ordinal, and a small file-range locator
for the latest complete record. The locator avoids copying its encoded bytes
out of the scan buffer.
`resume` transfers all recovered stream seeds directly into one append worker.

##### Reference

    SystemStateWriter::continue_recording -> StateWriterWorker::recover_state_stream
    successful recovery -> StateWriterWorker::continue_recovered_recording

#### StateWriterWorker::submit_record

Consumes a stream name and record, then blocks until the recording-wide record
count and that stream's byte capacity permit FIFO admission. Impossible
oversized records fail immediately.

##### Reference

    SystemStateWriter::record_state_to_stream -> StateWriterWorker::submit_record

#### StateWriterWorker::flush_state_stream

Queues a named ordered barrier and blocks until all earlier FIFO work and that
stream's current chunk are durably sealed and described by metadata.

##### Reference

    SystemStateWriter::flush_stream_to_storage -> StateWriterWorker::flush_state_stream

#### StateWriterWorker::finish_recording

Closes admission, drains work, seals every stream's final chunk through the
shared manifest, and joins the worker. Chunk descriptors were already installed
incrementally and are not returned as a duplicate summary.

##### Reference

    SystemStateWriter::complete_recording -> StateWriterWorker::finish_recording
    SystemStateWriter::mark_recording_failed -> StateWriterWorker::finish_recording

#### StateWriterWorker::close_admission / join_worker / drop

Private terminal lifecycle that wakes waiters and prevents detached workers.

##### Reference

    StateWriterWorker::finish_recording and Drop cleanup

#### StateWriterWorker::spawn

Builds per-stream admission state and worker-owned sinks, drops recovery-only
record locators, and transfers every file handle into the sole worker thread.

##### Reference

    StateWriterWorker::start_new_recording -> StateWriterWorker::spawn
    StateWriterWorker::continue_recovered_recording -> StateWriterWorker::spawn

### Work, Shared, QueueState, and StreamQueueState

`Work` is the named record/flush command enum. `Shared` owns the mutex and
condition variables. `QueueState` owns the recording-wide FIFO, count permit,
barrier acknowledgements, lifecycle, and per-stream admission map.
`StreamQueueState` owns one stream's byte limit, outstanding bytes, and final
accepted step.

#### Shared::new

Creates an open empty queue with one admission state seeded from each recovered
stream's final accepted step.

##### Reference

    StateWriterWorker::start_new_recording -> Shared::new

### StateStreamSink

Worker-owned persistence state for one named stream: directory, chunk target,
next ordinal, and optional active chunk.

#### StateStreamSink::new

Combines immutable stream configuration with a recovered append position.

##### Reference

    StateWriterWorker::spawn -> StateStreamSink::new

#### StateStreamSink::append

Rolls over only before or after complete records, creates an active chunk when
needed, and appends the encoded owner without splitting it.

##### Reference

    StateWriterWorker FIFO record command -> StateStreamSink::append

#### StateStreamSink::flush

Durably seals the stream's current non-empty chunk through the recording
manifest.

##### Reference

    StateWriterWorker flush command or shutdown -> StateStreamSink::flush

### ActiveChunk

Private open-payload owner with incremental SHA-256 and exact counters.

#### ActiveChunk::create

Creates one deterministic `chunk-NNNNNN.jsonl.tmp` payload without replacing
existing output. The open name is a lifecycle state, not a second payload copy:
records are written once into one inode, which sealing later renames.

##### Reference

    writer worker begins a chunk -> ActiveChunk::create

#### ActiveChunk::recovered

Reopens the already validated complete prefix of one unsealed chunk for direct
append without copying its payload.

##### Reference

    recover_open_chunk -> ActiveChunk::recovered

#### ActiveChunk::append

Appends one complete record and updates checksum and facts.

##### Reference

    writer worker FIFO loop -> ActiveChunk::append

#### ActiveChunk::descriptor

Builds the authoritative chunk metadata from incremental counters and a cloned
hasher state without consuming the open chunk.

##### Reference

    ActiveChunk::seal -> ActiveChunk::descriptor

#### ActiveChunk::seal

Synchronizes the open payload and its directory entry, constructs its
descriptor, asks `RecordingManifest::prepare_chunk` to commit that descriptor, then
atomically renames the same inode to `.jsonl` and synchronizes the stream
directory again. Metadata-before-rename ensures a sealed filename can never be
absent from the authoritative inventory. A crash after preparation leaves one
recognizable open payload that recovery can verify and finish.

##### Reference

    chunk rollover or writer finish -> ActiveChunk::seal
    storage_workflow final-name and temporary-file assertions -> successful seal path

#### ActiveChunk::finish_prepared

Completes the crash-interrupted rename for an open chunk whose descriptor was
already committed.

##### Reference

    StateWriterWorker::recover_state_stream prepared tail -> ActiveChunk::finish_prepared

## Payload decoding

### Responsibility split

1. `StoredStateSeriesReader` validates a record and retrieves a raw value by schema key.
2. `JsonPayloadDecoderRegistry` retrieves the decoder registered for the same exact key.
3. `JsonPayloadDecoder<T>` receives only that raw JSON field and returns owned `T`.
4. The registry moves `T` into the matching empty state slot.

A configured decoder entry exists per payload key. A Rust decoder type may be
reused when several keys share the same representation. JsonPayloadDecoderRegistry never perform
key lookup or see sibling fields.

### JsonPayloadDecoder<T>

Thread-safe typed conversion contract from borrowed raw JSON `&str` to owned
`T`, with an owned thread-safe associated error. Compatible closures receive a
blanket implementation.

#### JsonPayloadDecoder::decode_json_payload

Converts exactly one complete raw JSON value.

##### Reference

    JsonPayloadDecoderRegistry erased adapter -> JsonPayloadDecoder::decode_json_payload -> concrete T

### JsonPayloadDecoderRegistry

Non-Clone heterogeneous exact-key registry. Additional entries are permitted so
one registry can cover several streams.

#### JsonPayloadDecoderRegistry::new / with_capacity

Create an empty registry, optionally reserving key capacity.

##### Reference

    analysis setup -> JsonPayloadDecoderRegistry construction

#### JsonPayloadDecoderRegistry::register_for_field

Binds one exact key to one typed decoder, rejecting empty or duplicate keys.

##### Reference

    application reader configuration -> JsonPayloadDecoderRegistry::register_for_field

#### JsonPayloadDecoderRegistry::len / is_empty / has_decoder_for_field / registered_field_names

Expose registry configuration without decoder internals.

##### Reference

    setup inspection, Debug, and tests

#### JsonPayloadDecoderRegistry::require

Crate-private coverage check for every field in a selected stream.

##### Reference

    StoredStateSeriesReader::read_stream_as_state_series before chunk IO -> JsonPayloadDecoderRegistry::require

#### JsonPayloadDecoderRegistry::decode_into

Crate-private lookup, conversion, ownership transfer, and contextual error
wrapping for one field.

##### Reference

    StoredStateSeriesReader canonical field loop -> JsonPayloadDecoderRegistry::decode_into
        -> JsonPayloadDecoder::decode_json_payload -> SystemState::insert_payload

### TypedDecoder, ErasedPayloadDecoder, and DecoderInsertError

Private type-erasure adapter retaining each decoder's concrete output type. An
unexpected occupied destination is restored transactionally.

#### ErasedPayloadDecoder::decode_into

Performs typed conversion and insertion behind the heterogeneous registry.

##### Reference

    JsonPayloadDecoderRegistry::decode_into -> ErasedPayloadDecoder::decode_into

### JsonVecF64Decoder

Zero-sized default decoder for JSON numeric arrays to owned `Vec<f64>`. It adds
no length, finite-value, or domain validation.

#### JsonVecF64Decoder::decode_json_payload

Calls `serde_json::from_str::<Vec<f64>>` directly on the selected raw field.

##### Reference

    JsonPayloadDecoderRegistry entry for key -> JsonVecF64Decoder::decode_json_payload -> Vec<f64>

### JsonStringDecoder

Zero-sized default decoder for JSON strings to owned `String`. It preserves
content and performs no trimming or normalization.

#### JsonStringDecoder::decode_json_payload

Calls `serde_json::from_str::<String>` with standard escape and Unicode handling.

##### Reference

    JsonPayloadDecoderRegistry entry for key -> JsonStringDecoder::decode_json_payload -> String

Only these two defaults are included during main development. Applications may
register closures or named decoder types for tensors and domain values.

## Storage reading

### StoredStateSeriesReader

All-in-one eager reader owning output root, validated completed metadata, and a
caller-configured `JsonPayloadDecoderRegistry` registry. It is intentionally non-Clone.

#### StoredStateSeriesReader::open_completed_recording

Reads and validates `metadata.json`, requires `RecordingStatus::Complete`, and consumes
the registry.

##### Reference

    analysis startup -> StoredStateSeriesReader::open_completed_recording

#### StoredStateSeriesReader::recording_directory

Returns the supplied root without canonicalization.

##### Reference

    diagnostics and tests

#### StoredStateSeriesReader::stream_names

Iterates stream names in metadata order.

##### Reference

    analysis stream discovery

#### StoredStateSeriesReader::read_stream_as_state_series

Checks stream existence and decoder coverage, verifies every chunk, decodes all
states transactionally, and returns one complete `StateSeries`.

##### Reference

    analysis request -> StoredStateSeriesReader::read_stream_as_state_series(stream)

#### StoredStateSeriesReader::read_all_streams_as_state_series

Returns ordered `(stream name, StateSeries)` pairs and drops prior results if a
later stream fails.

##### Reference

    whole-run eager analysis -> StoredStateSeriesReader::read_all_streams_as_state_series

#### StoredStateSeriesReader::read_chunk

Private buffered JSONL traversal, size/checksum verification, strict ordering,
state assembly, and descriptor-fact validation.

##### Reference

    StoredStateSeriesReader::read_stream_as_state_series -> StoredStateSeriesReader::read_chunk

### Resume-state reader helpers

`decode_resume_state` is the crate-private checkpoint reconstruction boundary
used by the writer facade. It validates that the selected stream exactly matches
the full `SystemStateSchema`, requires decoder coverage, prefers the last complete line
already recovered from the open chunk, and otherwise seeks directly to the
final line of the highest sealed chunk. It does not integrity-scan sealed
history.

##### Reference

    SystemStateWriter::continue_recording with checkpoint request -> decode_resume_state

`read_last_sealed_record` scans backward in fixed-size blocks to locate only the
last JSONL boundary, avoiding an eager pass over a potentially large chunk.

##### Reference

    decode_resume_state without complete open record -> read_last_sealed_record

### BorrowedRecord, BorrowedValues, and BorrowedValuesVisitor

Private record representation borrowing each `RawValue` from one line buffer.
Only small field keys are owned. Duplicate payload keys are rejected.

#### BorrowedValues::deserialize

Starts strict borrowed object parsing.

##### Reference

    serde_json::from_slice in StoredStateSeriesReader::read_chunk

#### BorrowedValuesVisitor::expecting / visit_map

Describe and collect unique keys with borrowed raw value boundaries.

##### Reference

    BorrowedValues::deserialize -> BorrowedValuesVisitor

### StreamTemplateRef

Private borrowed adapter used to reconstruct a stream's shared `SystemStateSchema` from
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

### `TimeAxisMetadata`

Public run-level documentation for integer simulation time and optional
physical time. It owns only small labels and units; it never stores a time
sample.

#### `TimeAxisMetadata::new`

Creates an index-only declaration. Complete semantic validation is deferred to
run startup so fluent configuration remains infallible.

##### Reference

    SystemStateWriterBuilder::default time -> TimeAxisMetadata::default -> TimeAxisMetadata::new
    downstream run configuration -> TimeAxisMetadata::new

#### `TimeAxisMetadata::with_step_unit`

Fluently declares the optional simulation-index unit.

##### Reference

    downstream run configuration -> TimeAxisMetadata::with_step_unit

#### `TimeAxisMetadata::with_physical_time_name`

Fluently declares the optional physical-coordinate name.

##### Reference

    downstream run configuration -> TimeAxisMetadata::with_physical_time_name

#### `TimeAxisMetadata::with_physical_time_unit`

Fluently declares the physical unit; startup requires a physical name.

##### Reference

    downstream run configuration -> TimeAxisMetadata::with_physical_time_unit

#### `TimeAxisMetadata::default`

Uses `index` with no unit or physical coordinate.

##### Reference

    SystemStateWriterBuilder::new -> TimeAxisMetadata::default

#### `TimeAxisMetadata::into_stored`

Moves public configuration into the private versioned metadata representation.

##### Reference

    SystemStateWriter::create_new_recording -> TimeAxisMetadata::into_stored

### `StateStreamConfig`

Owns one logical stream's exact selected keys, safe relative directory,
optional cadence description, soft chunk-byte target, and strict queue-byte
budget. Non-zero byte types reject zero limits at the public boundary.

#### `StateStreamConfig::new`

Creates a declaration whose directory initially equals its stream name.

##### Reference

    downstream run construction -> StateStreamConfig::new

#### `StateStreamConfig::with_relative_directory`

Overrides the relative output directory. Startup rejects unsafe paths and
directory collisions.

##### Reference

    downstream stream path customization -> StateStreamConfig::with_relative_directory

#### `StateStreamConfig::with_cadence_description`

Adds descriptive cadence metadata without scheduling samples.

##### Reference

    downstream cadence documentation -> StateStreamConfig::with_cadence_description

### `SystemStateWriterBuilder`

Owns unopened run configuration and a cheap shared `SystemStateSchema` handle.

#### `SystemStateWriterBuilder::new`

Creates a builder with default time documentation, empty run metadata, and no
streams.

##### Reference

    SystemStateWriter::builder -> SystemStateWriterBuilder::new
    direct downstream construction -> SystemStateWriterBuilder::new

#### `SystemStateWriterBuilder::with_time_axis_metadata`

Replaces temporal-coordinate documentation.

##### Reference

    downstream run configuration -> SystemStateWriterBuilder::with_time_axis_metadata

#### `SystemStateWriterBuilder::with_user_metadata`

Moves arbitrary JSON-compatible run metadata into the builder. It remains
separate from scientific payload records.

##### Reference

    dispatcher fixed/sweep provenance -> SystemStateWriterBuilder::with_user_metadata
    simulation run annotations -> SystemStateWriterBuilder::with_user_metadata

#### `SystemStateWriterBuilder::add_state_stream`

Appends one stream in deterministic metadata order. Cross-stream conflicts are
validated together at startup.

##### Reference

    downstream run configuration -> SystemStateWriterBuilder::add_state_stream

#### `SystemStateWriterBuilder::create_new_recording`

Delegates complete validation, exclusive filesystem creation, root leasing,
initial atomic metadata publication, and writer startup to `SystemStateWriter`.

##### Reference

    configured builder -> SystemStateWriterBuilder::create_new_recording -> SystemStateWriter::create_new_recording

#### `SystemStateWriterBuilder::continue_existing_recording`

Explicitly reopens a matching running recording, recovers at most one open chunk
per stream, seeds ordinal/index continuation, and starts append worker. It
never makes `start` silently reuse existing output.

##### Reference

    externally managed state restoration -> SystemStateWriterBuilder::continue_existing_recording

#### `SystemStateWriterBuilder::continue_recording_from_latest_checkpoint`

Coordinates recovery with typed checkpoint reconstruction and returns
`(SystemStateWriter, SystemState)`. The selected stream must exactly cover the complete
builder `SystemStateSchema`; decoder outputs populate every slot before writers start.

##### Reference

    simulator restart -> SystemStateWriterBuilder::continue_recording_from_latest_checkpoint

### `SystemStateWriter`

Non-clone exclusive owner of one active writer handle and the sole legal
terminal metadata transition. It never owns or retains a `SystemState`.

#### `SystemStateWriter::builder`

Provides the concise public construction entry point.

##### Reference

    downstream simulation setup -> SystemStateWriter::builder

#### `SystemStateWriter::recording_directory`

Borrows the configured output root.

##### Reference

    diagnostics and run logging -> SystemStateWriter::recording_directory

#### `SystemStateWriter::stream_names`

Iterates names in deterministic declaration order.

##### Reference

    diagnostics and run logging -> SystemStateWriter::stream_names

#### `SystemStateWriter::record_state_to_stream`

Looks up one stream, borrows selected live-state payloads for encoding, ends
those borrows, then submits the owned encoded record. Submission is the
blocking backpressure boundary.

##### Reference

    simulation cadence event -> SystemStateWriter::record_state_to_stream -> JsonStateRecordEncoder::encode
    SystemStateWriter::record_state_to_stream -> StateWriterWorker::submit_record

#### `SystemStateWriter::flush_stream_to_storage`

Looks up one stream and waits on its ordered durability barrier. The method
returns only after all earlier accepted records are sealed and their descriptor
is durable in `metadata.json`.

##### Reference

    resume-critical checkpoint cadence -> SystemStateWriter::flush_stream_to_storage

#### `SystemStateWriter::complete_recording`

Consumes the coordinator, drains all streams, and atomically publishes
`Complete`. Descriptors were committed incrementally at each seal. On writer
failure it attempts `Failed` metadata without hiding the first writer error.

##### Reference

    successful simulation termination -> SystemStateWriter::complete_recording

#### `SystemStateWriter::mark_recording_failed`

Consumes the coordinator, drains all streams, and publishes an explicit
non-empty failed reason. A concurrent writer failure takes precedence.

##### Reference

    simulation-level terminal error -> SystemStateWriter::mark_recording_failed

#### `SystemStateWriter::create_new_recording`

Privately validates all configuration before mutation, creates and leases the
root, creates stream directories, atomically commits `Running`, and then starts
one recording-wide writer worker.

##### Reference

    SystemStateWriterBuilder::create_new_recording -> SystemStateWriter::create_new_recording

#### `SystemStateWriter::continue_recording`

Privately loads running metadata under the exclusive root lease, rejects
terminal or mismatched output, removes only a known interrupted metadata
replacement temporary, recovers stream seeds, optionally reconstructs a full
checkpoint, and starts the append worker.

##### Reference

    SystemStateWriterBuilder::continue_existing_recording -> SystemStateWriter::continue_recording
    SystemStateWriterBuilder::continue_recording_from_latest_checkpoint -> SystemStateWriter::continue_recording

#### `SystemStateWriter::finish_writer`

Privately drains the sole writer. Its stream sinks commit successful
descriptors directly through `RecordingManifest`.

##### Reference

    SystemStateWriter::complete_recording -> SystemStateWriter::finish_writer
    SystemStateWriter::mark_recording_failed -> SystemStateWriter::finish_writer

### `RecordingManifest`

Private mutex-serialized authority over the sole metadata snapshot. Chunk
workers clone and mutate a candidate, atomically persist it, and replace the
in-memory snapshot only after commit succeeds.

#### `RecordingManifest::prepare_chunk`

Appends the next descriptor and durably publishes it before the writer performs
the open-to-sealed rename.

##### Reference

    ActiveChunk::seal -> RecordingManifest::prepare_chunk

#### `RecordingManifest::transition`

Publishes the terminal complete or failed lifecycle after all writers drain.

##### Reference

    SystemStateWriter::complete_recording and SystemStateWriter::mark_recording_failed -> RecordingManifest::transition

### `RecordingLease`

Private advisory exclusive lock held on the output directory handle. It creates
no lockfile, is released by the operating system after process death, and stays
owned until all writers are dropped.

#### `RecordingLease::acquire`

Opens and non-blockingly locks the existing output root.

##### Reference

    SystemStateWriter::create_new_recording and SystemStateWriter::continue_recording -> RecordingLease::acquire

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

    SystemStateWriter::create_new_recording -> ensure_absent -> create_root
    SystemStateWriter lifecycle transition -> commit_metadata -> write_and_replace_metadata

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

- `StateFieldSchema`, `SystemStateSchema`, `SystemState`, and `SimulationTime`;
- `PayloadInsertError` and `StateError`;
- `StateSeries`, `StateSeriesView`, `StateSeriesPushError`, and `StateSeriesError`.

The storage portion includes:

- `SystemStateWriter`, `SystemStateWriterBuilder`, `StateStreamConfig`, and `TimeAxisMetadata`;
- `StorageError`;
- `StoredStateSeriesReader`, `JsonPayloadDecoderRegistry`, and `JsonPayloadDecoder`;
- `JsonStringDecoder` and `JsonVecF64Decoder`.

Low-level encoding, queue, chunk-format, and metadata implementation types are
not prelude members. `JsonStateRecordEncoder`, `StateWriterWorker`, `EncodedStateRecord`, and raw
metadata structures remain private implementation details behind `SystemStateWriter`.
Both storage integration tests import only the public prelude, so an omitted or
accidentally private supported type is detected by compilation.

##### Reference

    downstream simulation and analysis modules -> use scientific_workflow::prelude::*
    public API integration tests -> use scientific_workflow::prelude::*
    crate root -> pub mod prelude

## Simulator integration audit

The crate-level work required for simulator state ownership, checkpoint
recovery, and the one-state/one-writer resource model is complete. Inspection
of simulator's hot loop, checkpoint path, and multi-system runner originally
identified four integration gates. Gates 1, 2, and 4 are resolved. Gate 3—the
public recording manifest and terminal summary—is deliberately deferred and is
not required to begin simulator migration.

Already compatible:

- named `signal` and `space` streams naturally preserve independent sampling
  cadences and output identities;
- PiP's local `SquareLattice` serializer borrows its dense storage, so
  `SystemStateWriter::record_state_to_stream` can encode a lattice without first cloning it;
- exact encoded-byte chunking is a stricter implementation of simulator's
  desired maximum-file-size policy than its current estimated record sizing;
- bounded stream queues provide deterministic per-stream backpressure;
- application decoders can reconstruct `Vec<usize>`, PiP lattices,
  `ActivityStatus`, and scalar payloads without built-in crate support;
- immutable JSONL chunks, checksums, and complete-run reconstruction satisfy
  analysis once a run has terminated successfully.

### Gate 1: live-state mutation boundary

`EcoSystem` evolves `SquareLattice` and `TaxonTable` through simultaneous
mutable borrows on every event. This boundary is now supported by
`SystemState::borrow_payloads_mut`, which returns distinct heterogeneous payload
references after complete validation. Exporting simulator's old snapshot is no
longer needed and would remain unacceptable because it clones the full lattice
for every space sample.

The integration contract is now fixed: simulator must own and mutate one
`SystemState` directly, replacing its dedicated runtime state fields and old IO
snapshot struct. Storage will continue sampling that same state by borrow; an
external keyed-field sampling abstraction is not the chosen architecture.

Gate 1 completion criteria are satisfied: payload types are established during
state assembly and retained independently of payload presence; blank derived
states inherit those contracts; immutable and mutable tuple borrowing supports
arities two through eight; duplicate, unknown, missing, and mismatched requests
fail before references are returned; and safe stack-only slot separation adds
no payload copy, ownership transfer, heap allocation, lock, or unsafe code.
Integrated PiP tensor tests cover the complete public boundary.

#### Simulator mutation dependency

Simulator does not literally write the lattice and taxon table simultaneously.
For one event, `Decider` reads source and target taxa from the lattice, may read
the current table to sample an effective target, decides whether replacement
occurs, writes the lattice target, and then adjusts two table counts. Those
instructions can be expressed as a read/decide phase followed by separate
lattice and table writes.

They cannot generally be decoupled at sweep or batch granularity without
changing behavior. In randomized-target mode, event `n + 1` samples from the
table produced by event `n`; delaying count updates would sample from stale
abundances. Every accepted replacement must therefore update both
representations before the next event. The table is a derived cache of lattice
population counts, but it is also live model input and must remain consistent.

A strictly single-field state API could preserve semantics by repeatedly
borrowing the lattice and table in separate scopes for every event. That would
introduce several key lookups and dynamic type checks per lattice event and
create a visible two-step commit in application code. It is viable for
correctness, but it is an inferior hot-loop boundary. Simultaneous mutable
borrowing is not required for mathematical expressiveness; it is required to
retain the current efficient sweep shape, in which both payload references are
resolved once and reused across all events.

Accordingly, coordinated tuple borrowing is required to retain simulator's
efficient sweep shape. The intended use is one borrow before the inner event
loop, not one lookup per event:

```text
borrow_mut::<(SquareLattice, TaxonTable)>(("space", "population"))
    -> (space, population)
    -> for each event: decide(space, population, source, target)
    -> release both borrows
    -> advance SystemState time and evaluate other fields
```

This keeps `SystemState` authoritative, preserves exact randomized-target
semantics, and adds only one pair of key/type validations per sweep.

#### General scientific workload

Coordinated mutation of multiple state components is a common scientific
computing requirement. Representative cases include position/velocity/force
arrays in particle models, density/momentum/energy fields in fluid solvers,
coupled species in reaction systems, primal/dual or parameter/momentum arrays
in optimization, field values and auxiliary caches, and simulator's lattice
plus population table. Even when an algorithm stages its numerical writes, it
often needs several mutable component references for the duration of one
kernel, integrator step, or sweep.

“At the same time” means simultaneous exclusive Rust borrows, not simultaneous
machine instructions. Individual writes remain ordered. The capability matters
because resolving typed state components once around a hot kernel avoids
repeated name lookup, dynamic type validation, and artificial take/reinsert
ownership cycles.

The typical borrow arity is small and statically known. A state may contain many
fields, while a particular kernel usually couples two or three. The sealed tuple
contract supports arities two through eight without exposing erased wrappers,
query objects, generated numbered methods, or public macros.

Users may still group fields that form one inseparable domain object into one
payload. Grouping is a modeling choice, not a workaround imposed by the borrow
API: independently sampled, encoded, or analyzed fields should remain separate
state keys.

#### Simplified assembly-bound type model

Users should not define or retain a parallel field-selector structure. Payload
definition occurs while the initial state is assembled:

```text
state = spec.create_empty_state(time)
state.insert_payload("space", space_payload)
state.insert_payload("population", population_payload)
state.insert_payload("activity", activity_payload)
```

The first successful insertion into a slot establishes that field's runtime
concrete type for this state layout. Clearing or taking the payload empties the
slot but retains its type definition; subsequent insertion must use the same
type. An empty state derived from an assembled state retains all field type
definitions while omitting payloads. This is distinct from a fresh unassembled
state created directly from the type-free JSON `SystemStateSchema`.

Coordinated access then names and types fields in one expression:

```text
(space, population) =
    state.borrow_payloads_mut::<(SquareLattice<usize>, TaxonTable)>(
        ("space", "population"),
    )
```

The public vocabulary is only `borrow` and `borrow_mut`. A sealed tuple trait
implemented internally for arities two through eight associates each type
tuple with its equally sized name tuple and returned reference tuple. Users do
not name the trait, create query objects, keep typed handles, or invoke macros.
Existing `payload<T>` and `payload_mut<T>` remain the explicit single-field
operations.

Each multi-borrow resolves the names through the specification's existing hash
map and validates retained field types before producing references. Distinct
slot separation uses fixed const-generic stack arrays and safe `split_at_mut`;
it performs no heap allocation, unsafe pointer construction, payload movement,
or cloning. The intended call surrounds a full kernel or sweep, so lookup cost
occurs once rather than per event. A future cached-handle layer is warranted
only if measurement shows repeated borrow setup itself is material.

#### Idiomaticity assessment

The simplified API is idiomatic Rust despite relying on private generated tuple
implementations. Rust has no variadic generics, so sealed traits implemented for
a documented range of tuple arities are the conventional way to express a
heterogeneous operation whose arity is known at compile time. The public call
uses normal method syntax, turbofish type selection, tuple construction and
destructuring, `Result`-based validation, and borrow lifetimes enforced by the
compiler.

It also follows core ownership conventions:

- the state owns every concrete payload;
- assembly moves payloads into the state;
- access returns ordinary `&T` or `&mut T`, not guards or smart wrappers;
- one `&mut SystemState` is the exclusive authority from which all disjoint
  mutable references originate;
- an error is reported before any reference is returned and cannot partially
  mutate or empty the state; and
- all references expire before time advancement, sampling, or another state
  operation can borrow the state again.

`borrow_mut` is acceptable as an inherent method name: it describes granting
temporary references and does not imply interior mutability because it requires
`&mut self`. Rust also has `BorrowMut`, but an inherent method with tuple and
type arguments is unambiguous. `get_many_mut` or `get_disjoint_mut` would more
closely resemble collection APIs, but expose implementation mechanics and are
less symmetrical with immutable `borrow`. The concise `borrow`/`borrow_mut`
pair is retained unless implementation experience reveals confusing compiler
diagnostics.

The deliberately non-standard part is dynamic, assembly-established field
typing, which follows directly from the type-free JSON template. Retaining a
slot's `TypeId` after `take` or `clear` makes that dynamic boundary predictable
and gives derived empty states a stable program-level schema. This invariant
must be documented prominently because users may otherwise expect an empty
dictionary slot to accept a different type.

#### Concrete payload type and erasure boundaries

Every payload remains its original concrete Rust value `T` for its entire time
inside `SystemState`. Insertion moves that `T` into owned storage; typed access
returns `&T` or `&mut T`; extraction returns the same owned `T`. No conversion
to JSON, `serde_json::Value`, bytes, or a common scientific container occurs.
Runtime `TypeId` and the concrete Rust type name remain available for checked
downcasting and diagnostics.

There are two distinct meanings of type erasure:

1. **Heterogeneous storage erasure is unavoidable internally.** A Rust `Vec`
   cannot directly contain unrelated concrete types. Each slot therefore holds
   a private trait-object owner whose concrete allocation is still `T`. This
   erases the static type only from the vector's element type; it does not erase
   runtime type identity, transform data, clone payloads, or expose erasure to
   the user. A type map based on `Any` would make the same tradeoff under a
   different name.
2. **Serialization erasure is demand-driven.** When storage samples a field,
   the private wrapper temporarily borrows its concrete `T` as
   `&dyn erased_serde::Serialize`. That borrowed serialization view exists only
   for encoding. It neither replaces the stored payload nor persists after the
   call. Capturing the serialization function in the private value vtable at
   insertion is necessary because, after heterogeneous storage erasure, plain
   `Any` alone cannot rediscover an arbitrary type's `Serialize`
   implementation.

The type tuple supplied to `borrow_payloads` or `borrow_payloads_mut` states what the program
expects from each named slot. The method compares those expectations with the
concrete `TypeId` definitions retained during state assembly before returning
typed references. The tuple does not own, wrap, convert, or serialize any
payload.

Avoiding even internal storage erasure would require abandoning at least one
core requirement: use a compile-time fixed generic state struct, generate a
typed struct from a typed schema, or restrict payloads to a closed enum. All
three conflict with the runtime JSON key template and open-ended payload types.
The private erased owner plus demand-driven serialization view is therefore the
appropriate implementation for this general state.

### Gate 2: interrupted-run resume and append (implemented)

Simulator can continue an incomplete task in its existing directory, load only
the newest full-state checkpoint, and append later chunks.
`create_new_recording` remains strictly new-directory-only; explicit
`continue_existing_recording` and
`continue_recording_from_latest_checkpoint` validate recording identity and
recover append position. Descriptors are committed incrementally, while
ordinary `StoredStateSeriesReader` analysis remains completed-recording-only.

#### Filename-based seal contract

No per-chunk status, journal, checksum sidecar, or recovery marker is added.
Each chunk payload has exactly two possible names:

    chunk-000012.jsonl.tmp    open and recoverable
    chunk-000012.jsonl        sealed and immutable

These are two names for the same payload inode at different lifecycle stages,
not two payload copies. A final `.jsonl` name is the authoritative seal marker.
Recovery assumes every sealed chunk is valid and never opens, hashes, parses,
or decodes it. It may inspect directory names to enforce consecutive ordinals,
but content validation is intentionally outside the resume hot path.

To make that rule compatible with the single `metadata.json` inventory, sealing
uses a prepare-then-rename transaction:

1. finish the open chunk, synchronize its bytes, and build its descriptor;
2. atomically commit that descriptor to `metadata.json` while the recording remains
   `running`;
3. rename `.jsonl.tmp` to `.jsonl`; and
4. synchronize the stream directory.

This ordering prevents a crash from producing a sealed chunk whose descriptor
is absent from metadata. A crash between steps 2 and 3 instead leaves the one
open chunk plus its prepared descriptor, which is unambiguous and recoverable.
Readers continue to reject ordinary analysis of a running recording, so the brief
prepared state is never presented as completed output.

At resume, each stream can contain at most one open chunk: its highest ordinal.
The recovery pass opens and examines only that chunk. If metadata already has
its prepared descriptor, recovery verifies the open bytes against that
descriptor and completes the rename. Otherwise it parses complete JSONL
records, truncates an incomplete trailing record if present, rebuilds the
incremental checksum/counters, and continues appending to the same chunk (or
seals it immediately when it already meets the byte target). Older sealed
chunks are trusted without revalidation.

`SystemStateWriterBuilder::continue_existing_recording` is explicit rather
than making `create_new_recording` silently reuse a directory. It accepts only
`running` metadata, compares the caller's expected recording/schema/stream
configuration, acquires exclusive writer ownership, performs the
one-open-chunk recovery above, and seeds every stream sink in the sole worker.
Complete and failed recordings remain terminal.

#### Resume-state reconstruction

The writer facade provides the convenience path needed by simulations:

    let (writer, state) = builder.continue_recording_from_latest_checkpoint("space", decoders)?;

`continue_recording_from_latest_checkpoint` is deliberately not a `SystemState` constructor.
Directory layout, JSONL recovery, stream selection, and decoder dispatch belong to
storage; keeping them out of `system_state` preserves the core state's format-
and IO-independence. The operation performs one coordinated transaction:

1. acquire exclusive ownership of the existing running recording;
2. validate expected recording, schema, and stream declarations;
3. recover each stream's open chunk;
4. select and decode the newest complete checkpoint record from the requested
   stream;
5. seed and start the append worker; and
6. return both the live writer and reconstructed owned state.

For the selected checkpoint stream, recovery first examines its highest open
chunk. A final non-newline-terminated fragment is ignored and truncated as a
crash remnant. Every earlier newline-terminated record must be structurally
valid; recovery never skips corruption in the middle and calls a later record
"valid." If the open chunk contains no complete record, lookup falls back to
the last record of the highest sealed chunk. That sealed chunk is not integrity
scanned: the reader seeks to its final complete line and trusts the filename
seal contract.

The returned `SystemState` is complete, not a partially populated full-state
shell. Therefore the selected checkpoint stream must declare every field in
the builder's full `SystemStateSchema`, and the supplied registry must provide one
decoder for every field. A partial diagnostic stream such as `signal` is valid
for analysis but is rejected as a resume source. Successful reconstruction
creates an owned state at the record's stored `SimulationTime`, with every slot
populated by its concrete decoder output.

The lower-level `continue_existing_recording()` remains useful when an application restores
its state elsewhere. `continue_recording_from_latest_checkpoint` is its checkpoint-aware
convenience counterpart, not a second recovery implementation.

#### Resume API naming

The explicit public vocabulary is:

    builder.continue_existing_recording()?;
    builder.continue_recording_from_latest_checkpoint("space", decoders)?;

`continue_existing_recording` means storage-only continuation: validate and recover an
existing running recording, then return `SystemStateWriter` ready for later records.
`continue_recording_from_latest_checkpoint` means workflow continuation: perform the same storage
recovery, reconstruct a full checkpoint `SystemState`, and return
`(SystemStateWriter, SystemState)`. Both names state their result rather than exposing
an internal file-open mode.

The writer provides a per-stream durability barrier. `flush_stream_to_storage(stream)` blocks
until all records accepted before the call are written, prepares and seals a
non-empty open chunk even below its byte target, and commits its descriptor.
Simulator can call this after a resume-critical space checkpoint; ordinary
signal samples can retain automatic byte-target chunking.

### Gate 3: recording manifest and terminal summary (deferred)

Simulator and dispatcher inspect configuration, end time, activity, sample
counts, and writer statistics without decoding payload chunks. Current run
metadata accepts annotations only at startup, remains private behind the
reader, and `SystemStateWriter::complete_recording` returns no summary. The public boundary needs:

- a read-only recording manifest/status view, including per-stream aggregate records
  and bytes;
- access to user recording metadata from the reader;
- a terminal metadata update for values known only after simulation; and
- a `RecordingSummary` returned by successful completion, or an equivalent cheap
  inspection API after finish.

This allows dispatcher validation to depend on the scientific-workflow format
instead of simulator-private metadata.

Gate 2 is a sufficient foundation for this work: chunk descriptors now enter
the shared manifest incrementally, lifecycle transitions are serialized, and
resume consumes the same authoritative metadata. Gate 3 can therefore expose
owned read-only snapshots and derived aggregates without changing chunk files,
payload encoding, recovery rules, or writer ownership.

#### Proposed public responsibility

Use one public owned `RecordingSummary`. It is a read-only snapshot of the sole `metadata.json` at a
particular lifecycle point. It contains no payload bytes, decoder registry,
file handles, writer handles, or mutable access to internal metadata.

The snapshot exposes:

- format version and `Running` / `Complete` / `Failed` status;
- time-axis documentation;
- immutable run metadata supplied at startup;
- terminal result metadata supplied when finishing or failing;
- stream names, field declarations, cadence, and configured byte limits; and
- derived per-stream chunk count, record count, encoded bytes, and first/last
  recorded index.

Aggregate facts are derived from already committed chunk descriptors. Creating
or inspecting a manifest never opens, hashes, parses, or decodes a chunk.

The intended entry points are:

    writer.recording_summary()?                  // owned running snapshot
    writer.complete_recording()?                    // complete RecordingSummary
    writer.complete_recording_with(result)?         // complete RecordingSummary plus result data
    writer.mark_recording_failed(message)?               // failed RecordingSummary
    writer.mark_recording_failed_with(message, result)?  // failed RecordingSummary plus result data
    RecordingSummary::open(path)?            // standalone metadata inspection
    reader.recording_summary()                   // borrow completed reader snapshot

`run` metadata and terminal `result` metadata remain separate namespaces.
Startup provenance must not be overwritten by terminal measurements. Empty
result metadata is omitted from JSON. `SystemStateWriter::complete_recording` and `SystemStateWriter::mark_recording_failed` are convenience
forms of their `_with` counterparts using an empty result map.

This API lets a dispatcher determine whether a task completed, validate run
identity, list its output streams, report sample/byte counts, and inspect final
measurements without knowing payload types. A simulator can inspect current
committed progress while running and receives the authoritative terminal
manifest directly from its consuming lifecycle call.

The manifest does not reconstruct states, schedule sampling, control chunking,
or expose live queue internals. Checkpoint reconstruction remains
`continue_recording_from_latest_checkpoint`; payload analysis remains `StoredStateSeriesReader`.

### Gate 4: aggregate resource control and failure lifecycle

Backpressure is per stream, while each recording owns exactly one bounded FIFO
and worker thread. A full encoded record is allocated before admission and must
individually fit its configured stream budget. Independent simulations remain
independent resource and failure domains.

Simulator error propagation currently uses early `?` returns. Dropping
`SystemStateWriter` drains its writer but cannot infer a simulation failure,
leaving a running manifest. Integration must either explicitly call `SystemStateWriter::mark_recording_failed` on
all terminal paths or introduce a run guard/coordinator whose failure policy is
compatible with recovery.

#### Failure-lifecycle decision

An unexpected early return, panic, process loss, or transient IO error should
leave `Running`, not automatically publish `Failed`. `Running` now means
interrupted but recoverable: `SystemStateWriter` drop drains its in-process
queue and seals what it can, while `continue_existing_recording` or
`continue_recording_from_latest_checkpoint` can restart the same recording. An
RAII guard that converts every drop into `Failed` would destroy
this recovery path and is therefore the wrong default.

`Failed` is reserved for an explicit scientifically terminal decision for which
the caller does not intend continuation, such as rejected model conditions or
an intentional abort. Those paths call `SystemStateWriter::mark_recording_failed`. Successful paths call
`SystemStateWriter::complete_recording`. Ordinary `?` propagation needs no special guard and leaves a
recoverable recording. This resolves the lifecycle half of Gate 4 without another
type or automatic policy.

#### Aggregate-resource decision

Each simulation directly owns and evolves one `SystemState` and owns one
corresponding queued state-output writer. That writer coordinates every named
partial-state stream belonging to the simulation. Writers are not shared
between simulations and there is no process-global writer manager, registry,
global queue, worker pool, or aggregate backpressure policy.

    simulation thread
        -> owned evolving SystemState
        -> owned SystemStateWriter
        -> writer-owned bounded queue
        -> writer-owned stream/chunk state

The one-state/one-writer boundary keeps failures, queue pressure, output paths,
and lifecycle transitions local to their scientific recording. Per-stream byte
limits bound only that recording. Applications running many simulations choose their
own concurrency and therefore naturally control aggregate memory and writer
count. Storage does not attempt to infer relationships among independent runs.

This is the final architecture for the current stage; no central-writer
benchmark or shared-runtime implementation is planned. A future explicitly
requested scalability stage may reconsider implementation internals without
changing the on-disk format.

#### Naming refactor completed

The approved vocabulary is implemented throughout production code, tests, the
prelude, examples, diagnostics, and internal modules. Public concepts are
`SystemStateSchema`, `SystemState`, `StateSeries`, `SystemStateWriter`,
`StateStreamConfig`, `JsonPayloadDecoderRegistry`, and
`StoredStateSeriesReader`. “Recording” means the complete on-disk directory and
its lifecycle; “stream” means one named cadence and field selection within that
recording. The former ambiguous coordinator name and method vocabulary no
longer exist.

Method names state their effects explicitly: state methods refer to payloads,
writer methods distinguish recording, queue admission, and durable stream
flushes, and reader methods say whether they open a completed recording or
reconstruct a state series. The refactor intentionally provides no deprecated
aliases or legacy compatibility layer.

The sole `StateWriterWorker` owns one recording-wide FIFO queue. Private
`StateStreamSink` values retain independent chunk rollover and recovered append
positions. Per-stream byte budgets and the recording-wide hard-coded record
limit apply backpressure before queue admission.

### Simulator migration

Migration can now begin. It consists of adding local scientific-workflow and
PiP dependencies, declaring simulator keys and streams, using checked
`usize`/`u64` time conversion, registering application decoders, replacing
cadence writes, and updating dispatcher completion validation. Those are
downstream adaptation tasks rather than missing state or storage features.

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
targets plus their real JSON fixtures:

    tests/
    ├── fixtures/
    │   ├── state.json
    │   └── coupled_state.json
    ├── state_workflow.rs
    ├── analysis_workflow.rs
    ├── storage_workflow.rs
    └── storage_resilience.rs

Every target prints a short stable report under `--nocapture`. Logs contain
counts, indices, byte sizes, chunk facts, pointer/clone evidence, and expected
error classes; they never dump full scientific payloads or nondeterministic
thread timing.

### state_workflow.rs

One realistic simulation-state lifecycle using checked-in templates and PiP
tensors. It covers template semantic round trip, shared layouts, assembly-bound
type retention, heterogeneous immutable/mutable tuple access for arities two
through eight, duplicate/preflight errors, time advancement, zero-copy
extraction with allocation identity, explicit deep-clone accounting,
rejected-set payload recovery, and bounded diagnostics.

Key log output:

    [template] fields=3 round_trip=true
    [state] index=... loaded=... mutation=verified
    [ownership] set_take_pointer_preserved=true clone_calls=...
    [tuple] immutable=true mutable=true duplicate_rejected=true unknown_rejected=true preflight_atomic=true
    [type-contract] take_retained=true clear_retained=true empty_inherited=true
    [tuple-arities] min=2 max=8 reverse_order_mutation=true
    [result] state_workflow=passed

### analysis_workflow.rs

Builds an ordered `StateSeries` from evolving states, verifies move-based push
and pop, shared-layout and increasing-time rejection with ownership recovery,
borrowed `StateSeriesView` traversal, narrow field mutation, capacity reuse, and the
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
complete series. It exercises `JsonStringDecoder`, `JsonVecF64Decoder`, and an
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
| `state_workflow` | `StateFieldSchema`, `SystemStateSchema`, `SimulationTime`, `SystemState`, doc-hidden `PayloadTuple`, `StateError`, `PayloadInsertError`; all public spec, time, single/tuple state access, ownership, retained-type, clear, clone, and inspection methods |
| `analysis_workflow` | `StateSeries`, `StateSeriesView`, `StateSeriesPushError`, `StateSeriesError`; all public construction, capacity, lookup, iteration, mutation, append/rejection, extraction, clear, and clone methods |
| `storage_workflow` | `TimeAxisMetadata`, `StateStreamConfig`, `SystemStateWriterBuilder`, `SystemStateWriter`, `JsonPayloadDecoder`, `JsonPayloadDecoderRegistry`, both default decoders, and `StoredStateSeriesReader`; every public success-path method including `read_all`, with private encoding/writing/format behavior verified through files and readback |
| `storage_resilience` | `StorageError` source/context behavior and reachable configuration, lifecycle, queue, decoder, record, metadata, filesystem, and integrity failure families |
| `resume_workflow` | explicit `continue_existing_recording`/`continue_recording_from_latest_checkpoint`, full-state schema enforcement, typed checkpoint reconstruction, prepared and unprepared crash windows, append seeding, `flush`, and exclusive root leasing |

The finished source reads as five coherent workflows rather than an API census.
The old aggregators and focused subdirectories have been removed.

Current test architecture: five logged integration files plus production
doctests. Each workflow passes independently and the consolidated all-target
suite passes. Formatting and Clippy across all targets pass with warnings
denied. Archive preparation also succeeds. The manifest declares the published
compatible floor `physics_in_parallel = "3.0.3"` while its development path
resolves the local 3.0.4 source; this keeps coordinated local development and a
packageable crates.io dependency declaration compatible.
