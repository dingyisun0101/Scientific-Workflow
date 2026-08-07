# Scientific Workflow TODO

Only incomplete, next-stage, or explicitly deferred work belongs here. The
implemented architecture and per-method references live in `design.md`.

## Performance validation

- Add a focused encoder benchmark before claiming that the cached borrowed-
  payload vector is faster than the former second hash lookup. Compare small
  and large selected-field counts; functional tests already verify ownership,
  encoded output, and error semantics.

## Deferred decoder catalog

The main-development defaults are intentionally limited to:

- `StringDecoder`;
- `VecF64Decoder`.

After core development, consider additional decoders only when their concrete
wire conversion or validation behavior is well defined. Application-specific
payloads, including PiP tensors, already work through registered closures or
named `PayloadDecoder<T>` implementations.

## Deferred PiP work

`physics_in_parallel` remains a local development dependency at `../pip` until
its coordinated version is published. Sparse tensor behavior and its remaining
publication work are tracked in the sibling PiP repository's `todo.md`, which
is intentionally ignored there.

Current publication gate: `cargo package --allow-dirty --no-verify --locked`
cannot resolve `physics_in_parallel = ^3.0.4` from crates.io because the registry
currently offers 3.0.3. `cargo package --list --allow-dirty` succeeds and the
package inventory is correct. After PiP 3.0.4 is published, rerun the archive
and publish dry-run checks without changing the local-development workflow
prematurely.

## Deferred project stages

- dispatcher accepting `fixed.json` and `sweep.json`;
- scoped execution, logging, and run organization;
- Python API and Rust/Python bridge;
- optional out-of-core reader method on `SeriesReader` if analysis workloads
  demonstrate that eager `StateSeries` reconstruction is insufficient;
- alternate encodings only after JSON workflow stability; protobuf remains out
  of current scope.

## Simulator integration gate

Simulator must own and mutate a `SystemState` directly. That state replaces both
its dedicated live-state field layout and its old IO snapshot struct; sampling
borrows this authoritative state and must never clone the PiP lattice.

The first identified gap, multi-payload live mutation, is resolved by the
assembly-retained type contract and `borrow[_mut]::<(A, B, ...)>(name_tuple)`.
The remaining crate-level gaps are:

1. **Interrupted-run recovery and append.** Persist sealed chunk descriptors
   incrementally, validate existing run/schema identity, clean abandoned temp
   files, resume ordinals/order, and expose verified latest-record recovery for
   the space checkpoint stream.
2. **Public manifest and terminal summary.** Expose read-only run status and
   user metadata, permit terminal values known only at finish, and return or
   expose aggregate per-stream record and byte statistics.
3. **Aggregate resources and failure lifecycle.** Benchmark concurrent systems;
   if per-stream writers exceed total memory/thread limits, introduce a shared
   bounded writer runtime or global byte budget without weakening per-stream
   ordering. Define explicit failure handling so simulator early returns do not
   leave an unrecoverable running output.

After these gates, coordinate the simulator migration: add local
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
