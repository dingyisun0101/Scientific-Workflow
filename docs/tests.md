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
    │   ├── configuration/
    │   │   ├── cartesian_project/config/{fixed,sweep,paths}.json
    │   │   └── cases_project/config/{fixed,sweep,paths}.json
    │   ├── state.json
    │   └── coupled_state.json
    ├── analysis_workflow.rs
    ├── artifact_workflow.rs
    ├── configuration_workflow.rs
    ├── python_reader_conformance.rs
    ├── resume_workflow.rs
    ├── rng_record_workflow.rs
    ├── runtime_workflow.rs
    ├── state_workflow.rs
    ├── storage_resilience.rs
    └── storage_workflow.rs

The former `tests/system_state/`, `tests/time_series/`, and `tests/storage/`
subdirectories and their aggregator files have been removed. All ten
integration targets pass independently; the seven core workflow scenarios
retain the bounded logs specified below.

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
- Construct iteration-only and physical `SimulationTime` values and reject non-finite
  physical time.
- Create a blank state and inspect structural versus populated counts.
- Insert, borrow, type-check, mutate, replace, take, clear, and clear-all
  payloads.
- Borrow heterogeneous payload tuples immutably and mutably, including keys in
  reverse layout order.
- Compile and execute sealed tuple implementations for arities two through
  eight.
- Reject repeated tuple fields and prove complete preflight leaves all payloads
  unchanged.
- Retain concrete field types after `take_payload` and `clear_payload`, reject retyping empty
  slots, and inherit type contracts in a derived blank state.
- Verify `insert_payload` and `take_payload` preserve a large allocation pointer.
- Verify same-type replacement returns the previous payload.
- Verify a rejected insertion returns the unchanged incoming payload through
  `PayloadInsertError`.
- Verify failed typed extraction is rejected before moving the original
  payload.
- Advance simulation and physical time transactionally, including one failure
  that leaves time unchanged.
- Create a blank sibling state without cloning payloads.
- Explicitly deep-clone a state using a clone-counted payload and report the
  number of payload clone calls.
- Exercise bounded `Debug`, `Display`, and error-source behavior without
  printing payload contents.

### Structures and methods

`StateFieldSchema`:

- `index`, `name`, `description`.

`SystemStateSchema`:

- `load_json_template`, `to_json_template`, `template_path`, `field_schemas`,
  `len`, `is_empty`, `field_schema`, `contains_field`, and
  `create_empty_state`.
- Crate-private `parse`, `index_of`, and layout construction are covered
  indirectly through reader reconstruction and all typed state access.

`SimulationTime`:

- `from_iteration`, `from_iteration_and_physical_time`, `iteration`, and
  `physical_time`.

`SystemState`:

- `clone_structure_without_payloads`, `simulation_time`,
  `replace_simulation_time`, `advance_simulation_time`, `schema`,
  `declared_field_count`, `has_no_declared_fields`, `populated_field_count`,
  `has_no_payloads`, `field_schemas`, `contains_payload`, `payload_has_type`,
  `insert_payload`, `payload`, `payload_mut`, `borrow_payloads`,
  `borrow_payloads_mut`, `take_payload`, `clear_payload`,
  `clear_all_payloads`, and `clone`.
- Crate-private `new`, slot validation/separation, `value`, and `serializable`
  are covered by `SystemStateSchema::create_empty_state`, tuple access, and storage encoding.

Doc-hidden `PayloadTuple`:

- generated immutable and mutable mappings for every supported arity;
- duplicate, unknown, missing, and mismatch preflight through public tuple
  calls.

`PayloadInsertError<T>`:

- `error`, `payload`, `into_parts`, `Debug`, `Display`, and `Error::source`.

`StateValue` and `ErasedValue` remain private. Their construction, type checks,
downcasts, ownership recovery, erased serialization, type diagnostics, and
deep cloning are covered through the `SystemState` operations above.

### Log contract

    [template] fields=3 round_trip=true shared_layout=true
    [state] iteration=... physical_time=... loaded=... mutation_verified=true
    [ownership] pointer_preserved=true rejected_payload_recovered=true
    [tuple] immutable=true mutable=true duplicate_rejected=true unknown_rejected=true preflight_atomic=true
    [type-contract] take_retained=true clear_retained=true empty_inherited=true
    [tuple-arities] min=2 max=8 reverse_order_mutation=true
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
- Verify strictly increasing iteration and shared-layout invariants.
- Recover unchanged rejected states from both invariant failures.
- Distinguish zero-based collection position from simulation iteration.
- Exercise immutable lookup, first/last access, slices, iteration, and owned
  iteration.
- Exercise every `StateSeriesView` accessor and its copyable behavior.
- Mutate one typed field through `payload_mut_at` without exposing mutable state
  time.
- Verify bounds and typed field errors preserve `StateError` as a source.
- Pop, clear, and consume the series while checking allocation/ownership
  behavior.
- Explicitly clone a series with clone-counted payloads and demonstrate the
  expensive deep-clone boundary.
- Exercise bounded diagnostics without formatting scientific payloads.

### Structures and methods

`StateSeries`:

- `new`, `with_capacity`, `schema`, `as_view`, `len`, `is_empty`, `capacity`,
  `reserve`, `state_at`, `payload_mut_at`, `first_state`, `last_state`,
  `as_state_slice`, `iter`, `push_state`, `pop_state`, `clear_states`,
  `into_states`, `clone`, borrowed `IntoIterator`, owned
  `IntoIterator`, and `Debug`.

`StateSeriesView`:

- `schema`, `len`, `is_empty`, `state_at`, `first_state`, `last_state`,
  `as_state_slice`, `iter`,
  `IntoIterator`, and `Debug`.
- Private `new` is covered by `StateSeries::as_view`.

`StateSeriesPushError`:

- `error`, `state`, `into_parts`, `Debug`, `Display`, and `Error::source`.
- Private `new` is covered by both `StateSeries::push_state` rejection paths.

`StateSeriesError`:

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
multiple logical streams at different sampling intervals, encode borrowed payloads,
write byte-targeted chunks through bounded queues, commit one metadata file,
and reconstruct typed analysis series.

The target imports `scientific_workflow::prelude::basics::*` and never includes
private source files. This makes the test a compile-time audit of the supported
public surface.

### Required behavior

- Configure at least two streams with different field selections and sampling intervals.
- Prove encoding does not clone or retain the simulation payload borrow.
- Submit samples through `SystemStateWriter` and its bounded writer boundary.
- Produce multiple chunks through exact-byte rollover without splitting a
  record.
- Include one record larger than the chunk target but smaller than the queue
  budget and verify it occupies one oversized chunk.
- Verify deterministic filenames, counts, exact bytes, iteration ranges, and
  SHA-256 descriptors.
- Persist and semantically round-trip the sole `metadata.json`.
- Verify automatic UTC creation/finalization timestamps, monotonic active
  duration, continuation count, and separate terminal metadata.
- Assert no per-chunk metadata sidecars or temporary files remain.
- Assert every sealed chunk is visible only under its deterministic final name;
  this exercises the successful file-sync, rename, and directory-sync path.
- Register ordinary vector and string payloads through `with_json_field` under
  exact keys.
- Register a PiP tensor through `with_json_field` under its exact key.
- Open a completed run, enumerate streams, read one stream, and read all
  streams.
- Read only the latest state of one stream and inspect aggregate record/byte
  facts without loading earlier chunks.
- Verify complete `StateSeries` lengths, times, schemas, and typed payloads.
- Verify raw output remains readable without making a `serde_json::Value` tree
  part of the production reader API.
- Round-trip a heterogeneous PiP `PhysObj` through `SystemState`, queued JSONL
  storage, `with_json_field::<PhysObj>`, and typed reconstructed access; verify
  its mixed `f64`/`i64` columns.

### Structures and methods

Public run configuration and lifecycle:

- `TimeAxisMetadata::new`, `default`, `with_iteration_unit`,
  `with_physical_time_name`, `with_physical_time_unit`, and
  `with_physical_axis`;
- `SamplingInterval::Iterations` and `iterations`;
- `StateStreamConfig::new` with a typed sampling interval and
  `with_relative_directory`;
- `SystemStateWriterBuilder::new`, `with_time_axis_metadata`,
  `with_user_metadata`, `with_task_parameters`, `with_shared_stream_limits`,
  `add_state_stream`, `add_sampled_state_stream`, and
  `create_new_recording`;
- `SystemStateWriter::builder`, `recording_directory`, `stream_names`,
  `observe_state`, `flush_stream_to_storage`, `complete_recording`,
  `complete_recording_with_terminal_metadata`,
  `complete_recording_with_final_state`,
  `complete_recording_with_final_state_and_terminal_metadata`,
  `mark_recording_failed`, and `mark_recording_failed_with_terminal_metadata` across the
  successful, resume, and resilience workflows. Different stream sampling intervals,
  repeated-iteration no-op behavior, and non-aligned final-state insertion are
  observable in reconstructed record counts and times.

Private format, encoder, record, writer configuration, writer queue, summary,
and metadata transaction structures are covered only through observable public
outcomes: canonical field order, zero payload clones, exact JSONL framing,
FIFO ordering, byte rollover, checksums, deterministic filenames, absence of
temporary files, atomic lifecycle metadata, and typed reader reconstruction.

Decoder structures:

- `JsonPayloadDecoder::decode_json_payload` through both default and custom decoders;
- `JsonPayloadDecoderRegistry::new`, `with_capacity`, `register_for_field`,
  `with_json_field`, `len`, `is_empty`, `has_decoder_for_field`, and
  `registered_field_names`;
- crate-private coverage and insertion paths through complete reader dispatch;
- `JsonVecF64Decoder` and `JsonStringDecoder`, including empty values, escaped text,
  and Unicode.

`StoredStateSeriesReader`:

- `open_completed_recording`, `recording_directory`, `stream_names`,
  `format_version`, `user_metadata`, `terminal_metadata`, `recording_timing`,
  `stream_record_count`, `stream_encoded_bytes`,
  `read_latest_state_from_stream`, `read_stream_as_state_series`,
  `read_all_streams_as_state_series`, and
  bounded `Debug`.
- `CompletedRecording`, `RecordingTiming`, and `CompletedStreamSummary` through
  every public accessor.
- Private chunk traversal, borrowed raw-value parsing, schema reconstruction,
  canonical decoder dispatch, and checksumming are covered by exact readback
  and the resilience target.

### Log contract

    [sample] iteration=... physical_time=... signal=true space=...
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
- Exercise a writer terminal failure through `SystemStateWriter` and verify it propagates rather
  than silently succeeding.
- Reject unknown streams and missing decoder coverage before chunk decoding.
- Reject empty and duplicate decoder keys.
- Feed the wrong JSON kind to a registered decoder and preserve
  stream/index/key plus `serde_json::Error` source.
- Reject running and failed metadata when completed analysis is required.
- Detect missing chunks, exact-size mismatch, and same-size checksum corruption.
- Reject malformed/unterminated JSONL, too few or too many positional payload
  values, legacy object-valued records, invalid physical time, and
  non-increasing indices.
- Verify transactional reading returns no partial `StateSeries`.
- Verify the most important nested `StateError`, `StateSeriesError`, IO, JSON, and
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

## Test 5: resume_workflow.rs

### Scenario

Reproduce both crash windows using real encoded chunks, explicitly reopen the
running output, reconstruct a complete typed checkpoint, continue append
ordering, force a durability barrier, and finish into an ordinarily readable
analysis series. Separately exercise a multi-chunk open tail and meaningful
continuation rejection boundaries.

### Required behavior

- Convert one real sealed chunk into an unprepared open chunk and append a
  non-newline-terminated crash fragment.
- Verify recovery examines only that open payload, truncates only the fragment,
  retains all complete records, and continues the same chunk owner.
- Reconstruct every full-state field through
  `continue_recording_from_latest_checkpoint`, including a generic PiP tensor decoder,
  and verify time and typed values.
- Force `flush_stream_to_storage(stream)` below the automatic byte target and observe both the
  sealed filename and incrementally updated running metadata before finish.
- Reproduce the prepared-descriptor/before-rename crash window and verify the
  rename is completed without scanning sealed history.
- Build several one-record sealed chunks, corrupt an older sealed payload,
  convert only the highest chunk to an unprepared open tail, and verify resume
  still reconstructs that tail and continues at the next ordinal. Successful
  continuation is direct evidence that sealed history was not opened.
- Reject a competing writer through the artifact-free advisory root lease.
- Reject a partial stream as a full-state checkpoint while allowing the same
  output to continue through `continue_existing_recording`.
- Reject continuation of terminal metadata, mismatched builder configuration,
  and checkpoint reconstruction when no complete record exists.
- Append later indices after both recovery paths, finish, and reconstruct the
  complete final series through `StoredStateSeriesReader`.

### Structures and methods

- `SystemStateWriterBuilder::continue_existing_recording` and
  `continue_recording_from_latest_checkpoint`;
- `SystemStateWriter::flush_stream_to_storage`;
- interrupted descriptor preparation and sealing through `RecordingManifest`;
- `StateWriterWorker::recover_state_stream`,
  `continue_recovered_recording`, recovered ordering, and flush barriers;
- open-tail scanning/truncation and latest sealed-record fallback;
- `StorageError::RecordingDirectoryInUse`, `RecordingNotContinuable`,
  `RecordingConfigurationMismatch`, and `NoCheckpointState`, plus recovery
  conflict paths indirectly.

### Log contract

    [resume-state] iteration=... physical_time=... fields=... complete=true
    [recovery] incomplete_tail_truncated=true continued_open_chunk=true records=... durable_barrier=true
    [prepared] descriptor_verified=true rename_completed=true sealed_history_scanned=false lease_exclusive=true
    [multi-chunk] sealed_history_trusted=true open_tail_scanned=true resumed_index=... next_ordinal=...
    [resume-rejections] terminal=true configuration_mismatch=true no_checkpoint=true
    [schema] partial_checkpoint_rejected=true output_continued=true final_states=...
    [result] ..._resume=passed final_states=...

## Test 6: configuration_workflow.rs

### Scenario

Load real standard project directories through the public prelude, including
their conventional state schemas and execution scopes; generate
Cartesian and correlated explicit tasks, use their dict-like interfaces,
resolve project paths, export the three source documents byte for byte, reload
the copy, and reject meaningful ambiguous or invalid inputs.

### Required behavior

- Load `config/{fixed,sweep,paths,state}.json` through `ScientificProject` and
  retain lower-level `ProjectConfig` coverage.
- Create generated and named execution scopes, reopen one, reject an unsafe
  name, and derive an absent deterministic task recording path.
- Verify fixed, sweep, resolved-parameter, task, and path counts and ordered
  name iteration.
- Expand a two-axis Cartesian product with the final axis changing fastest.
- Expand explicit correlated cases whose later object declaration order differs
  from the first case, normalizing lookup/output order without changing values.
- Prove fixed values, repeated selected candidates, and cloned task handles
  refer to shared JSON values without allocating merged maps.
- Generate complete fixed/sweep/path `TaskConfig` handles for the full
  Cartesian product, filter one sweep value while retaining other-axis
  combinations, and reject missing or ambiguous unique selection.
- Exercise raw, required, single-value, and heterogeneous tuple decoding through
  arity twelve; resolved task iteration; deterministic task JSON; cheap
  cloning; owning iterator independence; and `Send + Sync` boundaries.
- Inspect unresolved paths and resolve relative paths against the project root
  without canonicalization or existence checks.
- Export all three exact source byte sequences to a new project, reload the
  exported configuration, and reject an overwrite attempt with its preserved
  IO source.
- Accept an empty Cartesian axis list as one empty fixed-only task.
- Reject out-of-range tasks, unknown parameters and paths, typed decode
  mismatch, recursively duplicated JSON keys, fixed/sweep overlap,
  inconsistent explicit cases, and non-string path values.

### Structures and methods

- `ProjectConfig::{load,project_root,configuration_directory,parameters,paths,
  task_count,task_config,task_configs,task_configs_matching,
  unique_task_config_matching,into_parts,write_source_config,clone}`;
- every public `ScientificProject` and `ExecutionScope` method;
- all public `ParameterSpace`, `TaskParameters`, `TaskParametersIter`, and
  `ProjectPaths` methods;
- all public `TaskConfig`, `TaskConfigIter`, and `MatchingTaskConfigIter`
  methods;
- reachable configuration error families with source preservation;
- strict parser, mixed-radix selection, explicit-case normalization, borrowed
  serialization, and exclusive exact export indirectly.

### Log contract

    [load] fixed=... swept=... parameters=... tasks=... paths=...
    [task-config] all=... selected=... shared_paths=true exact_match=true ambiguity_rejected=true
    [execution-scope] generated=... named=... task_path=... timestamp_managed=true
    [cartesian] tasks=... last_axis_fastest=true first=(...) last=(...)
    [ownership] fixed_shared=true selected_shared=true task_clone_shared=true merged_map_allocated=false
    [paths] declared=... relative_resolution=true canonicalization=false existence_check=false
    [round-trip] fixed_bytes=true sweep_bytes=true paths_bytes=true reload=true overwrite_rejected=true
    [lookup-errors] bounds=true missing=true type=true path=true
    [cases] tasks=... correlated=true key_order_normalized=true
    [validation] fixed_only=true nested_duplicate=true overlap=true legacy_axes_rejected=true object_candidates_rejected=true inconsistent_cases=true invalid_path=true
    [result] configuration_workflow=passed

## Test 7: runtime_workflow.rs

### Scenario

Build validated runtime plans exclusively from configuration-backed workloads,
then exercise dependency selection, bounded phase scheduling, task-local
progress, reuse, configurable failure barriers, and task-owned I/O. Rendering
is normally hidden so the test validates lifecycle without controlling the
test harness terminal; a bounded plain-output check covers renderer output.

### Required behavior

- Empty runtimes/phases, duplicate phase IDs, unknown dependencies, and
  dependency cycles fail during plan validation.
- Exact selection rejects an omitted unsatisfied dependency; inclusive
  selection adds dependencies in deterministic topological order.
- Application verification can satisfy an omitted completed dependency.
- Project helpers generate executable tasks retaining complete `TaskConfig`
  values and automatic labels.
- Concise helpers share one callable safely across generated tasks, while
  advanced factories retain distinct single-use workload ownership.
- Single-use workloads can move non-Clone resources directly into execution.
- Phase concurrency and prepared-work queue capacity remain bounded.
- Timing settings default to absent. Optional delayed admission preserves
  deterministic executable-task rank and pending status, while task timeout
  and phase deadline expiration request cooperative cancellation and return
  distinct structured errors.
- Progress, activity, and verified reused tasks share one phase lifecycle.
- Confirmation defaults off, accepts `yes` case-insensitively after rejecting
  other input, blocks the next phase until accepted, reports EOF context, and
  never prompts after the final selected phase.
- One active runtime excludes another even in hidden mode and releases its
  lease after success, failure, and panic-safe shutdown.
- Default fail-fast and optional finish-active policies stop admission after a
  failure, distinguish failed/cancelled/skipped outcomes, and prevent dependent
  phases from starting while retaining structured failure summaries.
- Exact partial selectors use structured parameters rather than display text.
- Task workloads own their files; the runtime neither creates nor removes them.

### Structures and methods

- `WorkflowRuntime`, `WorkflowRuntimeBuilder`, `RuntimeSummary`, and
  `PhaseSummary` construction, selection, execution, cancellation, and
  inspection;
- `Phase`, `PhaseBuilder`, `PhaseId`, `Task`, `TaskId`, `TaskKey`,
  `TaskDisplayKind`, `TaskSelector`, and configuration workload helpers;
- `TaskContext` direct progress forwarding, `TaskProgress`, `ActivityTask`, `TaskIdentity`,
  `ProgressSummary`, `TaskStatus`, and reachable `RuntimeError` families;
- scheduler barriers, bounded queues, renderer ownership, and terminal leases
  indirectly, including bounded message backpressure and plain/hidden output.

### Log contract

    test result: ok. 11 passed; 0 failed

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
5. `resume_workflow.rs` implemented for crash recovery and append.
6. `configuration_workflow.rs` implemented for project configuration and task
   expansion.
7. `runtime_workflow.rs` implements phase scheduling and runtime display.
8. Meaningful ownership, invariant, storage, decoder, integrity, recovery,
   configuration, and reporting assertions mapped into the core scenarios and
   focused artifact, RNG, and Python-conformance targets.
9. Old aggregators and test subdirectories removed after replacements passed.
10. README and architecture documentation updated to the consolidated layout.
11. Workflow-owned tests use only the relevant `prelude::basics` and
    `prelude::runtime` public boundaries.
12. Full formatting, all-target, doctest, and Clippy verification is the final
   closeout gate for every later change.

The migration preserved old tests until the replacement workflows compiled and
ran.

## Commands after consolidation

From `rust/`:

```bash
cargo test --test state_workflow -- --nocapture
cargo test --test analysis_workflow -- --nocapture
cargo test --test storage_workflow -- --nocapture
cargo test --test storage_resilience -- --nocapture
cargo test --test resume_workflow -- --nocapture
cargo test --test configuration_workflow -- --nocapture
cargo test --test runtime_workflow -- --nocapture
cargo test --all-targets --no-fail-fast --locked
cargo test --doc --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
```

## Completion criteria

The cleanup is complete only when:

- the final tree contains the fixtures and ten integration files;
- every implemented public structure and method is checked off above;
- every high-risk private subsystem has observable behavioral coverage;
- all core scenario targets emit their documented bounded logs;
- no test depends on execution order or retained generated data;
- the complete verification command set passes.

All criteria are satisfied for the current implemented crate when the release
gate below passes: every integration target and doctest succeeds, formatting
is clean, and Clippy passes across all targets with warnings denied.
