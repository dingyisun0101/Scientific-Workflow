# Scientific Workflow NPY v2

## Scope

The standard `$npy` phase converts every field in each completed execution-unit
member recording into a manifest-directed collection of C-contiguous `.npy`
files. A converted member directory is one atomic dataset. Loose array files
without its `manifest.json` are not a supported interchange unit.

The format is independent of Rust payload types. Conversion operates on the
verified JSON value of each field and never dispatches on an application or
crate-specific type name.

## Directory Contract

Each member directory contains `manifest.json` and every array named by that
manifest. All component paths are relative, remain inside the member directory,
and are unique. Each descriptor records the array's role, stream, optional
field and logical path, dtype, shape, C-contiguity, and SHA-256 checksum.

The replicate processed directory contains a
`scientific-workflow-npy-batch.v2` manifest. Its ordered member entries carry a
contiguous ordinal, source recording, relative member-manifest path, and
member-manifest checksum.

Writers publish each member directory atomically only after reopening and
validating its manifest and all arrays. Readers require the manifest and reject
missing, escaping, duplicate, corrupt, incompatible, or undeclared components.

## Field Representations

Every field has exactly one representation.

### Numeric

A field that is Boolean or numeric in every record is stored directly. This
includes scalars, rectangular JSON arrays, and typed numeric envelopes with
`scalar`, `shape`, and flat C-order `data` members.

If every record has the same shape, `storage` is `fixed` and the data array has
shape `[record_count, *value_shape]`.

If rank and dtype remain stable while shape changes, `storage` is `ragged`:

- `data` is one flat C-order array containing records consecutively;
- `offsets` is `uint64[record_count + 1]`, starts at zero, is nondecreasing,
  and ends at `data.size`; and
- `shapes` is `uint64[record_count, rank]`.

For every record, its offset span equals the product of its shape. Empty values
are represented by equal adjacent offsets and one or more zero extents.

### Structured

Every other field receives a lossless fallback:

- `data` is a flat `uint8` array containing canonical UTF-8 JSON records; and
- `offsets` is `uint64[record_count + 1]` with the same span rules as ragged
  numeric data.

This fallback makes strings, nulls, heterogeneous objects, unsupported numeric
types, and changing structures reconstructable without NumPy object dtype or
pickle.

Workflow also recursively discovers stable numeric projections inside a
structured field. Object keys form escaped JSON-pointer paths. Homogeneous
sequences of objects use `*` as their sequence segment. Each projection uses
the same fixed or ragged numeric storage rules. A malformed recognized numeric
envelope is an error rather than a fallback.

Typed envelopes are structural. Workflow currently supports exact Boolean,
8/16/32/64-bit signed and unsigned integer, and 32/64-bit floating-point tags.
`isize` and `usize` are normalized to signed and unsigned 64-bit NPY values.
Unsupported tags remain available through the structured JSON fallback.

## Time Coordinates

Every stream has a `uint64` iteration array. Streams carrying physical time
also have a `float64` physical-time array. Their first dimension is the stream
record count and aligns exactly with every field representation.

## Reader Contract

`open_npy_conversion(directory)` fully verifies one member manifest and all of
its components. `NpyConversion.array(path)` opens only declared components with
`allow_pickle=False`. `field`, `reconstruct`, and `projection` resolve data
through required manifest metadata rather than file-name inference.

`open_npy_batch(directory)` verifies the batch manifest, every member-manifest
checksum, and every referenced member dataset.

Consumers may memory-map numeric arrays directly. Generic consumers use
`reconstruct` for a whole field record and `projection` for one numeric path.
No reader may infer component relationships by scanning the directory.

## Integrity And Versioning

NPY headers are not authoritative metadata. The manifest supplies component
roles and relationships, while headers independently supply dtype and shape;
the reader requires them to agree. Checksums cover complete `.npy` files,
including their headers.

The member format identifier is `scientific-workflow-npy.v2`; the batch format
identifier is `scientific-workflow-npy-batch.v2`. Readers fail closed on other
versions. Raw recording compatibility remains governed separately by the
recording protocol.
