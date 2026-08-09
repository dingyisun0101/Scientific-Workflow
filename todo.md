# Scientific Workflow TODO

Only incomplete, next-stage, or explicitly deferred work belongs here. The
implemented architecture and per-method references live in `design.md`.

## Performance validation

- Add a focused encoder benchmark before claiming that the cached borrowed-
  payload vector is faster than the former second hash lookup. Compare small
  and large selected-field counts; functional tests already verify ownership,
  encoded output, and error semantics.

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

Public manifest/logging work is deliberately deferred. The remaining
integration policy is:

1. **Deferred public manifest and terminal summary.** Expose read-only run status and
   user metadata, permit terminal values known only at finish, and return or
   expose aggregate per-stream record and byte statistics.

Failure lifecycle is resolved: unexpected early returns deliberately leave a
recoverable `Running` recording; only intentional terminal decisions call
`mark_recording_failed`.

The crate is ready for the next stage: coordinate the simulator migration by adding local
`scientific-workflow` and PiP 3.0.4 dependencies, replace its snapshot and
specialized writers with named streams, supply custom decoders, update
dispatcher completion validation, and prove save/chunk/readback/crash-resume
behavior before deleting legacy IO.

## Project rules

- Keep production tests under `tests/`, never inside module files.
- Single-file tests mirror the source filename; cross-module tests use concise
  behavior names.
- Preserve user changes and ignore `legacy/`, targets, and generated run data.
- Update `design.md` and this TODO when architectural scope changes.
- During ordinary development, edit one production/test file per review unit;
  batch work only when explicitly authorized.
