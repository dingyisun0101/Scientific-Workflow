# Scientific Workflow Test Architecture

## Purpose

The permanent test suite favors a few realistic, observable workflows over
many trivial source-file-level tests. Tests should demonstrate that the crate
works as a scientific workflow system, not merely call isolated getters.

The consolidated suite must:

- exercise every implemented public structure and method;
- cover storage exclusively through its supported public facade;
- explicitly verify high-risk ownership, mutation, ordering, serialization,
  backpressure, chunk integrity, and reconstruction contracts;
- test private helpers indirectly through observable behavior;
- print concise, stable results under `--nocapture`;
- never print complete large tensors, payload buffers, or unstable thread
  timing;
- clean up every precisely owned temporary output directory.

Test count is not a quality target. A meaningful integrated assertion may
replace many narrow tests.

## Final file layout

    tests/
    ├── fixtures/
    │   └── state.json
    ├── state_workflow.rs
    ├── analysis_workflow.rs
    ├── storage_workflow.rs
    └── storage_resilience.rs

The former `tests/system_state/`, `tests/time_series/`, and `tests/storage/`
subdirectories and their aggregator files have been removed. All four
replacement targets pass independently with the logs specified below.

Doctests remain in production documentation and are not replaced by this
consolidation.

## Test 1: state_workflow.rs

### Scenario

Load the checked-in JSON template, initialize a realistic simulation state with
PiP tensor and ordinary Rust payloads, mutate it through several time points,
and transfer payload ownership into and out of the state.

### Required behavior

- Load the actual `tests/fixtures/state.json` template.
- Assert semantic template JSON round trip explicitly.
- Verify deterministic field order, normalized descriptions, lookup, and
  shared-layout identity.
- Construct index-only and physical `TimePoint` values and reject non-finite
  physical time.
- Create a blank state and inspect structural versus populated counts.
- Insert, borrow, type-check, mutate, replace, take, clear, and clear-all
  payloads.
- Verify `set` and `take` preserve a large allocation pointer.
- Verify same-type replacement returns the previous payload.
- Verify a rejected set returns the unchanged incoming payload through
  `SetError`.
- Verify failed typed extraction restores the original payload.
- Advance simulation and physical time transactionally, including one failure
  that leaves time unchanged.
- Create a blank sibling state without cloning payloads.
- Explicitly deep-clone a state using a clone-counted payload and report the
  number of payload clone calls.
- Exercise bounded `Debug`, `Display`, and error-source behavior without
  printing payload contents.

### Structures and methods

`FieldSpec`:

- `index`, `name`, `description`.

`StateSpec`:

- `load`, `to_json`, `source`, `fields`, `len`, `is_empty`, `get`, `contains`,
  `shares_layout`, `empty`, and `clone`.
- Crate-private `parse`, `index_of`, and layout construction are covered
  indirectly through reader reconstruction and all typed state access.

`TimePoint`:

- `new`, `from_physical`, `index`, `physical`.

`SystemState`:

- `empty`, `time`, `set_time`, `advance`, `spec`, `len`, `is_empty`, `loaded`,
  `is_blank`, `fields`, `has`, `is`, `set`, `get`, `get_mut`, `take`, `clear`,
  `clear_all`, and `clone`.
- Crate-private `new`, `value`, `value_mut`, and `serializable` are covered by
  `StateSpec::empty`, typed access, and storage encoding.

`SetError<T>`:

- `error`, `payload`, `into_parts`, `Debug`, `Display`, and `Error::source`.

`StateValue` and `ErasedValue` remain private. Their construction, type checks,
downcasts, ownership recovery, erased serialization, type diagnostics, and
deep cloning are covered through the `SystemState` operations above.

### Log contract

    [template] fields=3 round_trip=true shared_layout=true
    [state] index=... physical=... loaded=... mutation_verified=true
    [ownership] pointer_preserved=true rejected_payload_recovered=true
    [clone] payload_clone_calls=... independent=true
    [result] state_workflow=passed

## Test 2: analysis_workflow.rs

### Scenario

Build an analysis series from multiple evolving states, traverse it through
owned and borrowed interfaces, mutate one stored field, reject invalid
appends, and recover ownership from failures.

### Required behavior

- Create empty series with and without reserved capacity.
- Reserve and reuse state-vector capacity.
- Move states into and out of the collection without cloning payloads.
- Verify strict increasing-index and shared-layout invariants.
- Recover unchanged rejected states from both invariant failures.
- Distinguish zero-based collection position from simulation index.
- Exercise immutable lookup, first/last access, slices, iteration, and owned
  iteration.
- Exercise every `SeriesRef` accessor and its copyable behavior.
- Mutate one typed field through `field_mut` without exposing mutable state
  time.
- Verify bounds and typed field errors preserve `StateError` as a source.
- Pop, clear, and consume the series while checking allocation/ownership
  behavior.
- Explicitly clone a series with clone-counted payloads and demonstrate the
  expensive deep-clone boundary.
- Exercise bounded diagnostics without formatting scientific payloads.

### Structures and methods

`StateSeries`:

- `new`, `with_capacity`, `spec`, `view`, `len`, `is_empty`, `capacity`,
  `reserve`, `get`, `field_mut`, `first`, `last`, `states`, `iter`, `push`,
  `pop`, `clear`, `into_states`, `clone`, borrowed `IntoIterator`, owned
  `IntoIterator`, and `Debug`.

`SeriesRef`:

- `spec`, `len`, `is_empty`, `get`, `first`, `last`, `states`, `iter`,
  `IntoIterator`, and `Debug`.
- Private `new` is covered by `StateSeries::view`.

`PushError`:

- `error`, `state`, `into_parts`, `Debug`, `Display`, and `Error::source`.
- Private `new` is covered by both `StateSeries::push` rejection paths.

`SeriesError`:

- Exercise layout mismatch, non-increasing time, out-of-bounds position, and
  contextualized field access in realistic series operations.

### Log contract

    [series] states=... capacity=... indices=[...]
    [invariants] layout_rejected=true ordering_rejected=true
    [ownership] push_pop_pointer_preserved=true rejected_state_recovered=true
    [clone] payload_clone_calls=... independent=true
    [result] analysis_workflow=passed

## Test 3: storage_workflow.rs

### Scenario

Run the complete successful persistence path. Evolve one live state, sample
multiple logical streams at different cadences, encode borrowed payloads,
write byte-targeted chunks through bounded queues, commit one metadata file,
and reconstruct typed analysis series.

The target imports `scientific_workflow::prelude::*` and never includes private
source files. This makes the test a compile-time audit of the supported public
surface.

### Required behavior

- Configure at least two streams with different field selections and cadences.
- Prove encoding does not clone or retain the simulation payload borrow.
- Submit samples through `RunOutput` and its bounded writer boundary.
- Produce multiple chunks through exact-byte rollover without splitting a
  record.
- Include one record larger than the chunk target but smaller than the queue
  budget and verify it occupies one oversized chunk.
- Verify deterministic filenames, counts, exact bytes, index ranges, and
  SHA-256 descriptors.
- Persist and semantically round-trip the sole `metadata.json`.
- Assert no per-chunk metadata sidecars or temporary files remain.
- Assert every sealed chunk is visible only under its deterministic final name;
  this exercises the successful file-sync, rename, and directory-sync path.
- Register `VecF64Decoder` and `StringDecoder` under exact keys.
- Register an application-provided PiP tensor decoder under its exact key.
- Open a completed run, enumerate streams, read one stream, and read all
  streams.
- Verify complete `StateSeries` lengths, times, schemas, and typed payloads.
- Verify raw output remains readable without making a `serde_json::Value` tree
  part of the production reader API.

### Structures and methods

Public run configuration and lifecycle:

- `TimeAxis::new`, `default`, `index_unit`, `physical_name`, and
  `physical_unit`;
- `StreamConfig::new`, `directory`, and `cadence`;
- `RunOutputBuilder::new`, `time_axis`, `run_metadata`, `stream`, and `start`;
- `RunOutput::builder`, `root`, `streams`, `sample`, `finish`, and `fail` across
  the successful and resilience workflows.

Private format, encoder, record, writer configuration, writer queue, summary,
and metadata transaction structures are covered only through observable public
outcomes: canonical field order, zero payload clones, exact JSONL framing,
FIFO ordering, byte rollover, checksums, deterministic filenames, absence of
temporary files, atomic lifecycle metadata, and typed reader reconstruction.

Decoder structures:

- `PayloadDecoder::decode` through both default and custom decoders;
- `Decoders::new`, `with_capacity`, `add`, `len`, `is_empty`, `contains`, and
  `keys`;
- crate-private coverage and insertion paths through complete reader dispatch;
- `VecF64Decoder` and `StringDecoder`, including empty values, escaped text,
  and Unicode.

`SeriesReader`:

- `open`, `root`, `streams`, `read`, `read_all`, and bounded `Debug`.
- Private chunk traversal, borrowed raw-value parsing, schema reconstruction,
  canonical decoder dispatch, and checksumming are covered by exact readback
  and the resilience target.

### Log contract

    [sample] index=... physical=... signal=true space=...
    [writer] signal_records=... signal_bytes=... space_records=... space_bytes=...
    [chunk] stream=... file=... records=... bytes=... checksum_verified=true
    [durability] final_chunk_names=true temporary_files=false
    [metadata] files=... bytes=... semantic_round_trip=true status=complete
    [readback] signal_states=... space_states=... typed_round_trip=true clone_calls=0
    [result] storage_workflow=passed

## Test 4: storage_resilience.rs

### Scenario

Inject failures across configuration, queueing, metadata, decoding, and chunk
integrity. Assert error classes and essential context rather than maintaining
large exact-display snapshots.

### Required behavior

- Refuse an existing run output directory.
- Reject empty/invalid stream configuration.
- Reject one record larger than the strict queue byte budget immediately.
- Reject duplicate or decreasing writer indices.
- Exercise a writer terminal failure through `RunOutput` and verify it propagates rather
  than silently succeeding.
- Reject unknown streams and missing decoder coverage before chunk decoding.
- Reject empty and duplicate decoder keys.
- Feed the wrong JSON kind to a registered decoder and preserve
  stream/index/key plus `serde_json::Error` source.
- Reject running and failed metadata when completed analysis is required.
- Detect missing chunks, exact-size mismatch, and same-size checksum corruption.
- Reject malformed/unterminated JSONL, missing/additional/duplicate payload
  keys, invalid physical time, and non-increasing indices.
- Verify transactional reading returns no partial `StateSeries`.
- Verify the most important nested `StateError`, `SeriesError`, IO, JSON, and
  decoder sources remain traversable.

### Error-family coverage

`StorageError` families:

- configuration and lifecycle;
- metadata version/semantics/completion;
- missing, size-mismatched, and checksum-mismatched chunks;
- invalid record framing and schema;
- state access and field encoding;
- duplicate/missing decoder and contextual decode failure;
- series invariant context;
- IO and JSON source preservation;
- byte accounting and oversized records;
- record ordering;
- queue termination and worker failure.

Not every variant needs a standalone constructor test. Every externally
reachable high-risk branch and every source-bearing family must be observed
through the operation that produces it.

### Log contract

    [expected-error] case=... family=... context_verified=true
    [integrity] missing=true size=true checksum=true record=true
    [decoder] missing=true wrong_type=true source_preserved=true
    [backpressure] oversized_rejected=true ordering_rejected=true
    [result] storage_resilience=passed

## Logging rules

- Use stable category prefixes shown above.
- Print summaries only after assertions for that phase succeed.
- Include deterministic semantic facts: counts, indices, booleans, and byte
  lengths.
- Temporary absolute paths may appear only in a final cleanup message.
- Print checksum verification as a boolean or short prefix, never rely on the
  full digest as a golden snapshot.
- Never log entire large payloads, decoder internals, pointer addresses, or
  scheduler-dependent queue timing.
- Document `--nocapture` commands in both READMEs.

## Completed migration procedure

1. `state_workflow.rs` implemented and run independently.
2. `analysis_workflow.rs` implemented and run independently.
3. `storage_workflow.rs` implemented and run independently.
4. `storage_resilience.rs` implemented and run independently.
5. Meaningful ownership, invariant, storage, decoder, and integrity assertions
   mapped into the four scenarios.
6. Old aggregators and test subdirectories removed after replacements passed.
7. README and architecture documentation updated to the consolidated layout.
8. Storage tests migrated from source-path harnesses to the public prelude.
9. Full formatting, all-target, doctest, and Clippy verification is the final
   closeout gate for every later change.

The migration preserved old tests until all four replacements compiled and ran.

## Commands after consolidation

From `dev/`:

```bash
cargo test --test state_workflow -- --nocapture
cargo test --test analysis_workflow -- --nocapture
cargo test --test storage_workflow -- --nocapture
cargo test --test storage_resilience -- --nocapture
cargo test --all-targets --no-fail-fast --locked
cargo test --doc --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
```

## Completion criteria

The cleanup is complete only when:

- the final tree contains the fixture and four integration files;
- every implemented public structure and method is checked off above;
- every high-risk private subsystem has observable behavioral coverage;
- all four targets emit their documented bounded logs;
- no test depends on execution order or retained generated data;
- the complete verification command set passes.

All criteria are satisfied for the current implemented crate: the test tree is
the fixture plus four workflows, every target emits its bounded report, all
four integration tests and four doctests pass, formatting is clean, and Clippy
passes across all targets with warnings denied.
