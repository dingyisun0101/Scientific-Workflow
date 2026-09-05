# Protocol compatibility

The machine-readable authority is [`compatibility.json`](compatibility.json).
Package versions, project configuration, recording formats, converted-data
formats, and program events are independently versioned. Unknown recording
versions fail closed.

| Implementation | Package version | Recording writes | Recording reads |
| --- | --- | --- | --- |
| Rust `scientific-workflow` | 0.13.5 | 7 or 8 | 7 and 8 |
| Python `scientific-workflow` | 0.4.3 | None | 7 and 8 |

Periodic-only recordings continue to use [format 7](recording-v7.md). A recording
with any `initial_and_final` stream uses [format 8](recording-v8.md), which adds an
explicit boundary-sampling policy. Older readers reject format 8; no v7 field has
been reinterpreted. Both versions retain positional JSON payloads, JSON Lines
framing, and mandatory `sha256:` checksums.

The Python package exposes no raw-recording writer. Its round-trip test bridge
is test infrastructure. Its optional converter writes and reads
[NPY member/batch v2](npy-v2.md).

Project manifests still use `workflow_schema: 1`. Independent program diagnostics
use [program events v1](program-events-v1.md). Rust 0.13.5's `$npy` preflight
requires Python companion 0.4.3, Python 3.14+, and the `npy` extra.

The previous pair, Rust 0.13.4 and Python `scientific-workflow-reader` 0.4.2,
reads recording v7 only. The Python distribution and import namespace changed;
there are no old-name aliases. See the [migration guide](../docs/migration-0.13.5.md).
