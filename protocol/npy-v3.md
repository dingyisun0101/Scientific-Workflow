# Scientific Workflow NPY v3

## Scope and compatibility

Workflow 0.16 and Python companion 0.6 produce and verify
`scientific-workflow-npy.v3` member datasets and
`scientific-workflow-npy-batch.v3` batches. Raw input remains a completed
`scientific-workflow-jsonl` recording in format 7 or 8.

NPY v3 preserves scalar rank, validates declared numeric types, and prevents
lossy conversion of ordinary JSON numbers. Readers also validate coordinate
layout and component ownership before exposing a dataset.

Workflow's current readers and reuse accept v3 only. Historical v2 datasets
are not rewritten, squeezed, or certified as v3. Downstream analysis owns
explicit historical v2/v3 interpretation. The [v2 protocol](npy-v2.md) remains
available for that purpose; there is no Workflow legacy migration mode.

## Directory and manifest contract

A member directory contains `manifest.json` and all declared `.npy` components.
The manifest requires exactly these structural keys:

| Key | Meaning |
| --- | --- |
| `format` | `scientific-workflow-npy.v3` |
| `source_format`, `source_version` | Raw Workflow format name and actual version 7 or 8 |
| `source_recording` | Absolute original member-recording path |
| `source_metadata_checksum` | SHA-256 of the exact raw `metadata.json` bytes |
| `exclude_streams` | Sorted, unique exact stream names excluded from conversion |
| `user_metadata`, `terminal_metadata` | Preserved raw-recording JSON objects |
| `streams` | Included streams in declaration order |
| `arrays` | Component descriptors |

Each stream has exactly `name`, positive u64 `records`, and a nonempty `fields`
array. Stream and field names are nonblank and unique within their scope.
Each field has `name`, `representation`, and either `dataset` for numeric
storage or `fallback` and `projections` for structured storage.

Every descriptor requires `role`, `stream`, `path`, `dtype`, `shape`,
`c_contiguous: true`, and `checksum`. Field components additionally require
`field`; numeric components also require `logical_path`. Coordinates carry
neither field nor logical path. Paths contain only normal relative components;
absolute paths, backslashes, empty or dot segments, escapes, and component
aliases are rejected. Each declared component has exactly one owner.

Checksums have exactly `sha256:` and 64 lowercase hexadecimal digits. They cover
complete `.npy` files including headers. A reader requires the header's dtype,
shape, and C-contiguity to agree with the descriptor. Components with object,
string, or complex dtype cannot serve as numeric datasets. Numeric floating
data must be finite.

The batch manifest has exactly `format`, `exclude_streams`, and a nonempty
`members` array. Each member entry has a contiguous u64 `ordinal`, unique
absolute `source_recording`, `manifest`, and `manifest_checksum`. The manifest
path is `member-<ordinal>/manifest.json`, with at least six zero-padded decimal
digits. Each member-manifest checksum, source identity, and stream exclusion
selection must agree with the referenced dataset.

Unknown structural keys, duplicate JSON object keys, nonstandard JSON constants,
missing or undeclared components, invalid roles, duplicate owners, and escaping
paths are errors. Application-owned objects retain their keys inside the
metadata and structured payload boundaries.

## Exact numeric representation

Ordinary Boolean or numeric JSON values use numeric storage only when one
supported dtype preserves every value and the dtype and rank remain stable
across records. Scalars retain rank zero. Rectangular arrays retain their
original rank and extents. Booleans are not coerced to integer or float values.

If NumPy's proposed dtype rounds an integer, or otherwise changes a value,
the converter uses the lossless JSON fallback. For example,
`[-1, 9223372036854775809]` must not be stored as a float64 array. It remains
reconstructable as the original JSON numbers; a whole-field numeric series
is unavailable. Any discovered numeric projections obey the same rule.

Typed numeric envelopes are structural objects containing `scalar`, `shape`,
and flat C-order `data`. Supported tags are `bool`, signed and unsigned
8/16/32/64-bit integers, `isize`, `usize`, `f32`, and `f64`. Machine-width tags
normalize to 64-bit values. Shape extents are nonnegative integers; data length
equals their product, including one value for scalar shape `[]`.

Boolean tags require JSON Boolean values. Integer tags require JSON integers
within the declared range; strings, Booleans, and floating tokens are not
coerced. Float tags accept integer and floating JSON numbers and permit normal
rounding to the declared precision, including underflow rounding. Overflow or
nonfinite results are errors. Unsupported scalar tags retain their envelope
through the structured fallback. Malformed recognized shapes, lengths, or
supported scalar values are errors and publish no successful member dataset.

## Numeric layout

A numeric dataset has `logical_path`, `storage`, `dtype`, `rank`, and `data`.
The whole-field logical path is `""`. Projections use escaped JSON pointers;
homogeneous sequences of objects use `*` as a sequence segment.

With fixed shapes, `storage: "fixed"` uses one data array of shape
`[record_count, *value_shape]`, including `[record_count]` for scalar values.
Its descriptor role is `field_data` or `projection_data`.

With a stable rank/dtype and changing shapes, `storage: "ragged"` additionally
requires `offsets` and `shapes`:

- `data` is flat C-order numeric data;
- `offsets` is uint64 `[record_count + 1]`, starts at zero, never decreases,
  and ends at `data.size`; and
- `shapes` is uint64 `[record_count, rank]`. Each adjacent offset span equals
  the product of that record's extents, including empty shapes.

Offset and shape descriptor roles append `_offsets` and `_shapes` to the
corresponding data role. Their stream, field, and logical path must match the
dataset owner. No array can satisfy two components or fields.

## Structured layout

Structured fields retain every record as canonical UTF-8 JSON, with a
`fallback` containing exactly `storage: "json_bytes"`,
`encoding: "utf-8-json"`, `data`, and `offsets`. Data is flat uint8; offsets
follow the same span rules as ragged data. Descriptor roles are `json_data`
and `json_offsets`, carrying the stream and field owner. Each byte span must
decode as strict JSON.

Numeric projections are optional and have unique escaped logical paths.
Changing structures, unsupported numeric types, and values without one exact
numeric representation remain available through `reconstruct` without object
arrays or pickle.

## Coordinates and readers

Every included stream requires exactly one `iterations` component: a uint64
vector of length `record_count` with strictly increasing values. If physical
time is present, exactly one `physical_times` component is a finite float64
vector of the same length. Physical time need not increase; the scientific
model owns its meaning. All coordinates align with the corresponding field
records.

`open_npy_conversion(directory)` verifies manifest structure, component hashes,
roles, ownership, dtypes, shapes, offsets, coordinates, and fallback JSON before
returning a member view. `open_npy_batch(directory)` additionally verifies
every member-manifest checksum and batch/member agreement. Read-only maps are
retained and reused by `array`, `series`, `reconstruct`, and `projection`.
Callers must keep source files and returned metadata unchanged after opening.

## Conversion, publication, and exclusions

Conversion verifies the completed recording metadata and each included chunk.
Excluded chunks are not read or verified; names are exact, case-sensitive,
nonempty strings without surrounding whitespace. Unknown names have no effect.
Excluding every stream publishes a valid metadata-only member with empty
`streams` and `arrays`.

Each member is prepared in a unique staging directory and published atomically
after complete v3 verification. Existing output is reused only when its current
format, source metadata, exclusions, and all components verify. Conflicting or
historical output requires a new destination; files are never overwritten to
manufacture compatible output. Batch publication refuses replacement, preserves
source ordering, and requires successful members. Verified members survive a
failed batch for a later retry with the same inputs.

Conversion workers follow Python's default or the embedding application's
selected multiprocessing context; Workflow does not change the global start
method. Queues come from that same context and workers explicitly receive the
current private pause/cancel configuration. Numeric native thread pools remain
limited to one thread per worker. Allocation and worker-start choices do not
alter scientific or format reuse identity.
