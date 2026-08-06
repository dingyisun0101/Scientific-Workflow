# Scientific Workflow TODO

Only incomplete, next-stage, or explicitly deferred work belongs here. The
implemented architecture and per-method references live in `design.md`.

## Next stage: run-level storage facade

Create `dev/src/storage.rs` after discussing the final public builder and
metadata transaction API.

Required responsibilities:

- declare and curate public storage re-exports;
- configure a run root and one or more logical streams;
- validate each stream's exact selected keys and byte limits;
- own one `JsonEncoder` and `StateWriter` per stream;
- atomically write the sole initial `metadata.json` with `Running` status
  before accepting samples;
- route `sample(stream, &SystemState)` through the selected encoder and writer;
- preserve writer backpressure and surface terminal errors;
- finish all writers and atomically replace metadata with complete chunk
  inventories and `Complete` status;
- define failure metadata behavior without hiding the originating error;
- reject sampling after finish and repeated finish operations;
- never clone, retain, or take ownership of scientific payloads.

Before code, decide only these remaining public API details:

1. builder names and ownership flow;
2. stream configuration representation;
3. atomic metadata temporary-file and synchronization policy;
4. finish/failure transition behavior;
5. which currently crate-private storage types become public.

## Storage tests for the next stage

- Extend `tests/storage_workflow.rs` to use `RunOutput` instead of manually
  coordinating encoders, writers, and metadata.
- Add `RunOutput` lifecycle, metadata atomicity, stream routing, existing-path
  refusal, and failure behavior to `storage_workflow.rs` and
  `storage_resilience.rs`; retain the four-file test architecture.
- Export `storage` from `lib.rs` only after its complete public lifecycle passes.
- Update crate and repository READMEs with the final public storage example.

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

## Project rules

- Keep production tests under `tests/`, never inside module files.
- Single-file tests mirror the source filename; cross-module tests use concise
  behavior names.
- Preserve user changes and ignore `legacy/`, targets, and generated run data.
- Update `design.md` and this TODO when architectural scope changes.
- During ordinary development, edit one production/test file per review unit;
  batch work only when explicitly authorized.
