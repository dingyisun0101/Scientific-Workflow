# Scientific Workflow Recording Protocol v7

Rust 0.13.3 and Python reader 0.4.2 completed a coordinated compatibility
review against this unchanged protocol version. Package support remains
authoritative in [`compatibility.json`](compatibility.json).

This document is the normative cross-language contract for
`scientific-workflow-jsonl` recording format version `7`. The words MUST,
MUST NOT, SHOULD, and MAY are requirements terms.

The protocol covers one execution-unit member recording: its `metadata.json`
and immutable state-stream chunks. It does not cover an external program or
Python task workspace, `program.json`, captured logs, dependency snapshots, or
program-owned artifacts.

## Compatibility model

A v7 reader MUST require `format == "scientific-workflow-jsonl"` and
`version == 7`. It MUST reject unknown structural keys. An additive structural
field is therefore incompatible and requires a new recording-format version.
Application-owned keys are allowed only inside `user_metadata` and
`terminal_metadata`.

The machine-readable structural companion is
[`recording-v7.schema.json`](recording-v7.schema.json). This document defines
semantic and filesystem requirements that JSON Schema cannot fully express.
The current package support matrix is in
[`compatibility.md`](compatibility.md), backed by
[`compatibility.json`](compatibility.json).

## Directory layout

```text
<recording>/
├── metadata.json
└── <stream.directory>/
    ├── chunk-000000.jsonl
    ├── chunk-000001.jsonl
    └── ...
```

`metadata.json` is the sole structural authority. Every stream directory and
chunk file is relative to the recording root and MUST contain only normal path
components: no absolute path, empty component, `.` component, `..` component,
root, or platform prefix is valid. Stream directories MUST be unique.

Committed chunk names are exactly `chunk-<ordinal>.jsonl`, with the ordinal
rendered as at least six zero-padded decimal digits. The chunk at array
position `n` MUST declare ordinal `n` and its corresponding deterministic
filename. Temporary files and atomic-publication mechanics are writer-private
and are not part of a completed recording.

## JSON rules

Metadata and records MUST be UTF-8 JSON and producers MUST NOT emit duplicate
object keys or nonstandard numeric constants. Readers MUST reject duplicate
protocol-structural keys; interpretation of nested application-owned payload
objects remains the selected payload decoder's responsibility. Structural
objects accept exactly the keys specified here and in the schema. Unsigned
integers are in the inclusive range `0..=2^64-1`; fields marked positive
exclude zero. Physical time, when present, MUST be finite.

`user_metadata` and `terminal_metadata` are arbitrary JSON objects. Readers
MUST preserve their JSON meaning but need not map them to mutable containers.

## Metadata

The top-level object contains:

- `format`: exactly `"scientific-workflow-jsonl"`;
- `version`: exactly `7`;
- `status`: one lifecycle object described below;
- `timing`: writer lifecycle timing;
- `records`: exactly `{"encoding":"json","framing":"json_lines"}`;
- `time`: temporal-coordinate names and optional units;
- optional `user_metadata` and `terminal_metadata` objects, each defaulting to
  an empty object for reading; and
- `streams`: a nonempty declaration-order array of stream objects.

Stream and field names MUST be nonblank. Stream names MUST be unique within a
recording. Field names MUST be unique within their stream. Descriptions and
time names/units, when non-null, MUST be nonblank. A
`physical_time_unit` requires `physical_time_name`.

Each stream declares:

- its unique `name` and safe relative `directory`;
- `sampling_interval: {"iterations": N}` with positive `N`;
- declaration-order `fields`, where each item has `name` and optional
  `description`;
- `storage`, containing a positive `storage_queue_bytes` value and either
  `{"kind":"chunked","target_bytes":N}` with positive `N` or
  `{"kind":"individual_files"}`; and
- optional `chunks`, defaulting to an empty array.

Storage settings are provenance. Readers MUST validate them but do not use
them to weaken integrity checks.

## Lifecycle and timing

`status` is exactly one of:

```json
{"state":"running"}
{"state":"complete"}
{"state":"failed","message":"nonblank explanation"}
```

`timing.created_at_utc` and a terminal `timing.finalized_at_utc` MUST be UTC
RFC 3339 timestamps ending in `Z`. A running recording MUST omit or set
`finalized_at_utc` to null and MUST have empty terminal metadata. A complete or
failed recording MUST have a non-null finalized timestamp.
`active_duration_ns` and `continuation_count` are unsigned integers. The v7
Workflow writer does not support resume and writes continuation count zero;
readers validate the unsigned value but MUST NOT infer resume support from it.

The supported public readers open only `complete` recordings. They MUST reject
`running` and `failed` recordings before returning a reader authority.

## Chunk descriptors and integrity

Every committed chunk descriptor contains:

- contiguous `ordinal` and deterministic `file`;
- positive `records` and exact positive `bytes`, including every newline;
- `checksum` exactly `sha256:` followed by 64 lowercase hexadecimal digits;
- `first_iteration`; and
- `last_iteration` not less than `first_iteration`.

Chunk iteration ranges MUST be strictly ordered: a later first iteration is
greater than the preceding last iteration. An `individual_files` stream MUST
contain exactly one record per chunk.

Before decoding a chunk, a reader MUST verify file existence, exact byte
length, SHA-256 over the complete file bytes, newline framing, nonempty lines,
record count, descriptor endpoints, and strict iteration order within and
across chunks. An eager read MUST return either one fully verified series or
no series. An incremental reader MAY yield earlier fully verified chunks
before a later chunk fails, but MUST verify a complete chunk before yielding
its first record.

## JSON Lines records

Each nonempty line is exactly one strict JSON object followed by `\n`:

```json
{"iteration":12,"physical_time":0.25,"values":[[1,2,3],"label"]}
```

The required keys are `iteration` and `values`; `physical_time` is optional
and MAY be null. No other keys are valid. `iteration` is an unsigned 64-bit
integer and MUST increase strictly throughout one stream. `values` is an array
whose length exactly equals the stream field count. Position `i` contains the
JSON payload for field declaration `i`; records do not repeat field names or
type tags.

Payload interpretation belongs to the reader application. A typed reader MUST
require a decoder for every selected field when explicit decoders are used and
MUST report conversion failure with stream, field, and iteration context.

## Golden and invalid fixtures

[`../python/tests/fixtures/complete`](../python/tests/fixtures/complete) is the
shared valid v7 golden recording. Both official readers open it. The shared
[`invalid_metadata_cases.json`](../python/tests/fixtures/invalid_metadata_cases.json)
mutation corpus MUST be rejected by both readers. Bidirectional Rust/Python
round-trip tests additionally preserve sensitive floating-point bits, Unicode,
multi-chunk ordering, and integrity checks.

Fixtures illustrate the contract; this specification and its structural
schema are normative when an example and a stated rule differ.

## Version change checklist

Any incompatible structural, semantic, framing, lifecycle, integrity, or
path-safety change requires all of the following in one coordinated change:

1. Allocate a new recording-format integer; never reinterpret version 7.
2. Add a new normative protocol document and structural schema. Keep the v7
   documents available for historical interpretation.
3. Update writer and reader constants deliberately; do not derive the wire
   version from a package version.
4. Add new valid and invalid fixtures and cross-language conformance coverage.
5. Update `compatibility.json`, `compatibility.md`, both package guides,
   Persistence API documentation, architecture, and the test map.
6. Decide explicitly whether each reader supports one version or multiple
   versions. Unknown versions continue to fail closed.
7. Bump affected Rust/Python package versions according to their own release
   policies and validate package contents.
8. Publish only after the coordinated compatibility review is accepted.
