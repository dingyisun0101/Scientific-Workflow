# Protocol compatibility

The machine-readable authority is [`compatibility.json`](compatibility.json).
Package versions, project configuration, recording formats, converted-data
formats, and program events are independently versioned. Unknown recording
versions fail closed.

| Implementation | Package version | Recording writes | Recording reads |
| --- | --- | --- | --- |
| Rust `scientific-workflow` | 0.16.1 | 7 or 8 | 7 and 8 |
| Python `scientific-workflow` | 0.6.1 | None | 7 and 8 |

Periodic-only recordings continue to use [format 7](recording-v7.md). A recording
with any `initial_and_final` stream uses [format 8](recording-v8.md), which adds an
explicit boundary-sampling policy. Older readers reject format 8; no v7 field has
been reinterpreted. Both versions retain positional JSON payloads, JSON Lines
framing, and mandatory `sha256:` checksums.

The Python package exposes no raw-recording writer. Its round-trip test bridge
is test infrastructure. Its optional converter writes and reads
[NPY member/batch v3](npy-v3.md).

Project manifests still use `workflow_schema: 1`, optional `active_phases`
indices, and required `compute.mode` (`auto` or `isolated`). Independent program diagnostics use
[program events v1](program-events-v1.md). Rust 0.16.1's `$npy` preflight and reuse accept stable Python companions
`>=0.6,<0.7`, Python 3.14+, and the `npy` extra. Python 0.6.1 is a documentation
patch with the same APIs and storage formats as 0.6.0. The initial Rust 0.16.0
release checked exactly Python 0.6.0; use Rust 0.16.1 for later compatible patches.

Task input snapshots use [input manifest v1 and receipt/program v2](task-inputs-v1.md).
Workflow reuse accepts current receipts only. Historical NPY v2/v3 interpretation
is downstream-owned; the archived [v2 protocol](npy-v2.md) remains a reference.

The previous pair, Rust 0.13.4 and Python `scientific-workflow-reader` 0.4.2,
reads recording v7 only. The Python distribution and import namespace changed;
there are no old-name aliases. See the [migration guide](../docs/migration-0.13.5.md).
