# Scientific Workflow TODO

Only incomplete, next-stage, or explicitly deferred work belongs here. The
implemented architecture and per-method references live in `design.md`.

## Performance validation

- Add a focused encoder benchmark before claiming that the cached borrowed-
  payload vector is faster than the former second hash lookup. Compare small
  and large selected-field counts; functional tests already verify ownership,
  encoded output, and error semantics.

## Mandatory chunk integrity

- Make `continue_recording_from_latest_checkpoint` verify the exact byte count
  and SHA-256 checksum of the newest sealed checkpoint chunk before decoding
  its final record. Append-position recovery may continue to leave unrelated
  sealed history unopened, but any chunk used to reconstruct scientific state
  must satisfy the compulsory integrity contract documented in the READMEs and
  `design.md`.

## Deferred specialized decoders

Ordinary Serde JSON payloads now use
`JsonPayloadDecoderRegistry::with_json_field::<T>` and require no dedicated
decoder type. Add named decoders only when their concrete wire conversion,
configuration, or validation behavior differs from direct Serde decoding.
PiP tensors, matrices, vector lists, lattices, and `PhysObj` now implement
complete versioned Serde round trips and use `with_json_field::<T>` directly.
Other application-specific payloads already work through registered closures
or named `JsonPayloadDecoder<T>` implementations.

## PiP integration status

`physics_in_parallel` 3.0.4 is published and used as a crates.io development
dependency without a local path override. Its dense, sparse, heterogeneous
composite, float-fidelity, file-helper, and Python serialization refactor is
covered by Scientific Workflow's generic Serde integration tests. No active PiP
migration work remains.

## Deferred project stages

- dispatcher accepting `fixed.json` and `sweep.json`;
- scoped execution, logging, and run organization;
- Python API and Rust/Python bridge;
- optional out-of-core reader method on `StoredStateSeriesReader` if analysis workloads
  demonstrate that eager `StateSeries` reconstruction is insufficient;
- alternate encodings only after JSON workflow stability; protobuf remains out
  of current scope.

## Simulator integration gate

Simulator must own and mutate a `SystemState` directly. That state replaces both
its dedicated live-state field layout and its old IO snapshot struct; sampling
borrows this authoritative state and must never clone the PiP lattice.

The first identified gap, multi-payload live mutation, is resolved by the
assembly-retained type contract and
`borrow_payloads[_mut]::<(A, B, ...)>(name_tuple)`.
The interrupted-run recovery gate is implemented: descriptors are prepared
incrementally, `.jsonl.tmp`/`.jsonl` is the only chunk lifecycle marker,
sealed history is trusted during progress recovery, an advisory directory lease
prevents competing writers without a lockfile, `flush_stream_to_storage`
provides a durability barrier, and
`continue_recording_from_latest_checkpoint` reconstructs a complete typed
checkpoint.

The naming and one-state/one-writer refactor is complete. Each simulation owns
one evolving `SystemState` and one `SystemStateWriter`; its sole
`StateWriterWorker` coordinates all named streams through one bounded FIFO.
There is no centralized manager or process-global runtime. The public API,
prelude, tests, documentation, diagnostics, internal vocabulary, and filenames
use the approved explicit names without compatibility aliases.

Public manifest/logging work is partially deferred. Terminal metadata,
automatic timing, and the completed-recording handle are implemented. The remaining
integration policy is:

1. **Deferred public manifest and aggregate summary.** Expose read-only run status and
   initial user metadata, and return or expose aggregate per-stream record and
   byte statistics.

Failure lifecycle is resolved: unexpected early returns deliberately leave a
recoverable `Running` recording; only intentional terminal decisions call
`mark_recording_failed`.

The crate is ready for downstream migration. Refactor GLV first with a local
`scientific-workflow` dependency: replace its generic state, signal/space
writers, and task metadata with one authoritative state and one writer, then
prove numerical and output equivalence. Publish the migrated GLV before
refactoring dispatcher, because dispatcher directly imports GLV's current
solver and metadata APIs. Refactor simulator after the GLV pattern is proven,
using PiP 3.0.4 payloads, and then finish dispatcher completion validation
against the shared recording format.

GLV can now commit termination reason and completed step count through
`complete_recording_with_final_state_and_terminal_metadata`; it must not create
a second sidecar file.

## Reusable example patterns

- [x] `ProjectConfig` and `ScientificProject` lazily generate cheap owned
  `TaskConfig` handles for the full Cartesian product or explicit cases, with
  exact sweep filtering and ambiguity-safe unique selection.
- [x] Writer completion accepts structurally separate terminal metadata and
  returns an immutable completed-recording handle with directory and timing.
- [x] Efficient latest-state reconstruction avoids materializing a full series.
- [x] `ScientificProject` loads mandatory `project-root/config/state.json`.
- [x] `ExecutionScope` creates generated/named scopes, opens existing scopes,
  and derives absent deterministic task recording paths.
- Do not add a generic evolution trait or JSON-driven recording-plan format
  from the attractor example alone; reconsider after GLV and simulator provide
  independent evidence.

## Automatic operational timing

- [x] Add an automatic recording timing section with immutable RFC 3339 UTC
  `created_at_utc`, terminal `finalized_at_utc`, exact
  `active_duration_ns`, and `continuation_count`.
- [x] Measure active duration with a monotonic process-local clock; never derive it
  from subtracting wall-clock timestamps.
- [x] Preserve original creation time across continuation and accumulate only
  durations that can be committed truthfully.
- [x] Return timing through the completed-recording handle.
- [x] Commit terminal timing, terminal user metadata, and terminal status in one
  atomic metadata transition.
- [x] Give generated execution scopes an automatic UTC timestamp plus an
  opaque collision-resistant identifier; do not rely on timestamp text alone
  for uniqueness.
- [x] Keep automatic wall-clock timestamps out of state records and chunk payloads.

## Project rules

- Keep production tests under `tests/`, never inside module files.
- Single-file tests mirror the source filename; cross-module tests use concise
  behavior names.
- Preserve user changes and ignore `legacy/`, targets, and generated run data.
- Update `design.md` and this TODO when architectural scope changes.
- During ordinary development, edit one production/test file per review unit;
  batch work only when explicitly authorized.
