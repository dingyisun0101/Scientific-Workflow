# Scientific Workflow TODO

This file records downstream work discovered while reviewing an earlier file.
Only the active production file may be edited; downstream items remain here
until their own one-file review unit. design.md remains the architectural source
of truth.

## Completed stage: system_state clean-slate refactor

### Confirmed scope: JSON only

- Implement and document JSON persistence only.
- Do not design alternate binary encodings or reserve public abstractions for
  hypothetical future formats.

### Confirmed module boundary

- Keep SystemState independent of directories, metadata, streams, chunks,
  readers, and reconstruction orchestration.
- Keep time_series limited to the in-memory StateSeries collection; it performs
  no JSON serialization, decoding, directory access, or disk IO.
- Add a separate `storage` module for JsonEncoder, StateWriter, StateDecoder,
  DecodedRun, metadata, readers, chunks, and directory lifecycle.
- Payload types own their Serialize implementations. JsonEncoder supplies only
  the JSON serializer and framing; StateWriter accepts only completed encoded
  records and writes bytes.

### Confirmed storage limits

- Configure each stream with max_chunk_bytes; calculate rollover from exact
  encoded JSONL bytes rather than a state-count limit.
- Never split an EncodedRecord. One record larger than max_chunk_bytes becomes
  one oversized chunk and its exact size is recorded.
- Bound each queue by encoded bytes and record slots. StateWriter::submit blocks
  until capacity is released, and writer termination wakes blocked submitters
  with the terminal error.
- Implement byte backpressure with an RAII QueuePermit released after append;
  use a bounded synchronous channel for the independent record-count limit.

### Confirmed design: sequential mutable field access

- SystemState intentionally exposes one mutable payload borrow at a time.
- Do not add get2_mut or related multi-field borrowing APIs.
- Tightly coupled values that require simultaneous mutation should be grouped
  into one application-defined aggregate payload.

### Refactor rule

- Edit exactly one file at a time and wait for user review before continuing.
- Never edit a downstream file early. Record each newly discovered dependency
  under its scheduled file below and in design.md.
- Production modules contain no tests. Focused tests live under
  `tests/system_state/` and mirror their production filenames.
- Do not edit time_series or storage source during this stage, even when an
  intermediate system_state change invalidates transitional code.

### 1. src/system_state/spec.rs — complete

- Remove `type_tag`, the JSON `type` property, Type-tag documentation, and
  `FieldSpec::type_tag`.
- Add `description: Option<Box<str>>` and
  `FieldSpec::description() -> Option<&str>`.
- Accept absent/null descriptions; trim present descriptions and normalize
  empty or whitespace-only descriptions to None.
- Keep required trimmed unique names, deterministic indices, strict unknown-
  property rejection, Arc-shared StateLayout, path-only public load, crate-
  private parse, empty template support, and normalized to_json.

### 2. tests/fixtures/state.json — complete

- Replace every type tag with a concise natural-language field description.
- Keep the existing population, space, and activity key order so tensor and
  ownership integration coverage remains comparable.

### 3. tests/system_state/spec.rs — complete

- Replace all type-tag assertions with description assertions.
- Cover absent, null, empty, whitespace-only, trimmed, and ordinary
  descriptions; normalized semantic round trip must be explicitly equal.
- Retain malformed JSON, unknown properties, empty/duplicate names, empty
  template, deterministic index/lookup, parse/load parity, source provenance,
  and Arc identity coverage.

### 4. src/system_state/error.rs — complete

- Remove `EmptyTypeTag` after spec.rs no longer references it.
- Remove unused `FieldCountMismatch`; restoration always allocates through
  StateSpec::empty and inserts through SystemState::set.
- Preserve ownership-returning SetError, IO/JSON sources, typed access errors,
  and transactional time errors unchanged.

### 5. tests/system_state/error.rs — complete

- Remove obsolete variant expectations and retain SetError ownership,
  formatting, source-chain, Send, and time-error coverage.

### 6. src/system_state/value.rs — complete

- Change the erased blanket bound to
  `T: Serialize + Clone + Send + 'static`.
- Add `ErasedValue::as_serialize` and `StateValue::serializable`, returning a
  borrowed `&dyn erased_serde::Serialize` without trait-object upcasting.
- Preserve exact TypeId checks, typed borrowed/mutable/owned downcasts,
  mismatch recovery, bounded Debug, and one T::clone call per explicit clone.
- Remove all stable-tag and codec wording.

### 7. tests/system_state/value.rs — complete

- Make focused payload fixtures Serialize without hiding Clone counters.
- Serialize borrowed erased views with serde_json and explicitly verify output,
  pointer identity, zero Clone calls, and continued typed access afterward.
- Retain ownership/downcast/type/debug/Send tests.

### 8. src/system_state/state.rs — complete

- Require `Serialize + Clone + Send + 'static` only where a new payload enters
  through `set`; typed inspection and extraction retain their minimal bounds.
- Add crate-private `serializable(key)` delegating to `value` and
  `StateValue::serializable`; it must allocate and clone nothing.
- Keep all public names and ownership behavior: empty, time, set_time, advance,
  spec, structural inspection, has/is, set/get/get_mut/take, clear/clear_all,
  Clone, and bounded Debug.
- Document sequential mutable access and exact T-defined Clone semantics.

### 9. tests/system_state/state.rs — complete

- Make every inserted test payload Serialize.
- Add coverage for the crate-private serializable accessor, including unknown
  and missing fields, exact JSON, zero Clone calls, and post-serialization
  mutability.
- Retain pointer/capacity preservation, replacement return, rejected payload
  recovery, mismatch restoration, empty derivation, clone counts, time
  transactionality, clearing, and bounded Debug.

### 10. src/system_state.rs — complete

- Rewrite module workflow and examples for key-only templates and
  Serialize-compatible payloads.
- Remove stable-tag, codec, and automatic reconstruction language.
- Keep only FieldSpec, StateSpec, SystemState, TimePoint, StateError, and
  SetError public re-exports; value erasure stays private.

### 11. tests/system_state.rs — complete

- Include the four focused files under `tests/system_state/` so
  `cargo test --test system_state` runs them with Cargo-managed dependencies.
- Update the real fixture and FieldSpec assertions to descriptions.
- Preserve the downstream tensor lifecycle and explicit template round-trip
  equality; compilation must prove the tensor satisfies Serialize.

### 12. src/lib.rs — complete

- Update crate-level examples, payload bounds, and module-responsibility text.
- Export only system_state during this stage. Do not expose staged time_series
  or not-yet-created storage modules.

### Verification after item 12

- Run `cargo fmt --check`.
- Run `cargo check --lib`.
- Run `cargo test --test system_state` and `cargo test --doc`.
- Run `cargo clippy --lib --test system_state -- -D warnings`.
- Do not use full `cargo test` as the stage gate until the obsolete direct
  time_series codec tests are removed in their own stage.

Verified results:

- focused suites: spec 6, error 4, value 7, state 15;
- joint system_state target: 33 passed;
- doctests: 3 passed;
- formatting, library check, and Clippy with warnings denied: passed.
- root and crate READMEs were aligned with the completed contract.

## Next stage: time_series reconciliation

Implement one reviewed file at a time:

1. Refactor `src/time_series/error.rs` to collection-only errors. — complete
2. Rewrite `tests/time_series/error.rs`. — complete
3. Refactor `src/time_series/series.rs`, remove StateChunk, add narrow
   `field_mut`, and make PushError small without losing ownership. — complete
4. Rewrite `tests/time_series/series.rs`. — complete
5. Delete obsolete `src/time_series/codec.rs`. — complete
6. Delete obsolete `tests/time_series/codec.rs`.
7. Create the public `src/time_series.rs` facade.
8. Rewrite the unified `tests/time_series.rs` target.
9. Export the module from `src/lib.rs`, update README instructions, and run all
   stage verification commands.

Do not add serialization, decoding, chunks, queues, metadata, or filesystem IO
to this module. Those belong to the later `storage` stage.

## Deferred stage: storage separation

### External integration: physics_in_parallel serialization

- Completed in local `physics_in_parallel` 3.0.4: dense tensor storage, dense
  tensor facades, SquareLattice, VectorList, and contiguous dense Matrix
  serialization now borrow their buffers and preserve the existing JSON schema.
- Completed: Scientific Workflow resolves the local 3.0.4 source and its 33-test
  SystemState integration target passes with a real tensor payload.
- Choose the sparse persisted representation deliberately: streaming dense JSON
  preserves format but remains O(logical size), while true sparse JSON requires
  a matching versioned Deserialize implementation and deterministic ordering.
- Remaining: add allocation benchmarks, publish `physics_in_parallel` 3.0.4,
  then remove the temporary local path from the versioned dev dependency.
- Until publication, keep `../../pip` as the authoritative dependency and test
  coordinated contract changes in both crates before considering them complete.

### src/time_series/codec.rs

- Remove this transitional registry module after its obsolete callers and tests
  are removed. It has no replacement in time_series.

### src/storage/reader.rs

- Reconstruct each embedded stream schema through crate-private
  StateSpec::parse using metadata.json as the provenance path.

### src/storage/encoder.rs

- Accept a borrowed simulation-owned SystemState at a sampling boundary.
- Remove the temporary justified `dead_code` allowances from
  `SystemState::serializable`, `StateValue::serializable`, and
  `ErasedValue::as_serialize` when JsonEncoder makes the boundary live.
- Encode only the fields declared by that logical stream without cloning or
  taking payload ownership.
- End every payload borrow before sample returns so the simulation can resume
  in-place evolution immediately after encoded-record queue acceptance.

### src/storage/writer.rs

- Accept only complete EncodedRecord values from JsonEncoder.
- Perform blocking bounded-queue management, byte-targeted chunking, and disk IO without accessing
  payload types or invoking Serialize.

### src/time_series/series.rs

- Do not expose &mut SystemState because set_time and advance can invalidate
  ordering.
- Design a narrow field-level mutable analysis accessor during the time_series
  stage.
