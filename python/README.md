# Scientific Workflow Reader

`scientific-workflow-reader` is the official Python reader for completed
Scientific Workflow recordings. It implements the same versioned metadata,
JSONL framing, lifecycle, schema, ordering, and mandatory chunk-integrity
contract as Workflow's Rust `StoredStateSeriesReader`.

The reader is intentionally eager. A stream is returned only after all selected
chunks pass byte-count, SHA-256, JSON record, field, descriptor, and iteration
validation. A failure never returns a partial scientific series.

## Installation

```bash
python -m pip install \
  "scientific-workflow-reader @ git+https://github.com/dingyisun0101/Scientific-Workflow.git@6c8e7d4#subdirectory=python"
```

Python 3.10 or newer is required. The core reader has no runtime dependencies.
Install the optional NumPy converter when a project uses Workflow's reserved
`$npy` phase or when converting a recording directly:

```bash
python -m pip install \
  "scientific-workflow-reader[npy] @ git+https://github.com/dingyisun0101/Scientific-Workflow.git@6c8e7d4#subdirectory=python"
```

This guide documents release 0.4.1.

## Reading a recording

```python
from scientific_workflow_reader import open_completed_recording

reader = open_completed_recording("results/study/recordings/task-000000")
print(reader.stream_names)
print(reader.user_metadata)
print(reader.terminal_metadata)

signal = reader.read_stream("signal")
for state in signal:
    print(state.iteration, state.physical_time, state.values["abundance"])

latest = reader.read_latest("checkpoint")
```

JSON payloads reconstruct into ordinary Python scalars, lists, and dictionaries
by default. Applications may supply explicit per-field decoders without making
the storage package depend on NumPy:

```python
import numpy as np
from scientific_workflow_reader import open_completed_recording

reader = open_completed_recording(
    "recording",
    decoders={
        "abundance": np.asarray,
        "space": np.asarray,
        "total": float,
    },
)
checkpoint = reader.read_latest("checkpoint")
```

When a decoder mapping is supplied, every selected stream field must have a
decoder. Decoder failures are reported as `DecoderError` with the original
exception chained as their cause.

## Public API

- `open_completed_recording(path, decoders=None)`
- `RecordingReader`
  - `stream_names`, `user_metadata`, `terminal_metadata`, and `timing`
  - `stream_record_count(name)` and `stream_encoded_bytes(name)`
  - `read_stream(name)`, `read_all_streams()`, and `read_latest(name)`
  - `iter_verified_records(name)` for bounded-memory incremental consumers
- structurally read-only `StateField`, `StateRecord`, and `StateSeries`
- typed exceptions rooted at `RecordingError`

Release 0.4.1 supports only
`scientific-workflow-jsonl` format version 7, positional JSON payload encoding, JSON Lines
framing, and `sha256:` chunk checksums. Unknown versions and algorithms fail
closed.

The normative language-neutral contract is the repository's
[recording v7 protocol](https://github.com/dingyisun0101/Scientific-Workflow/blob/main/protocol/recording-v7.md),
with a strict structural JSON Schema and a package
[compatibility matrix](https://github.com/dingyisun0101/Scientific-Workflow/blob/main/protocol/compatibility.md). This package is the
v7 reader listed there; it does not expose a supported writer.

The record containers cannot be reassigned and their value mappings are
read-only. Decoded payload objects retain the type and mutability chosen by
JSON decoding or by the caller's field decoder.

`read_stream()` is transactional: it returns a complete series or nothing.
`iter_verified_records()` verifies an entire bounded chunk before yielding its
first record, but a later chunk may fail after earlier records were consumed.
Incremental callers should therefore write only to private temporary storage
and publish it after iteration completes successfully.

## Integrity boundary

The reader validates:

- successful recording completion;
- strict metadata and record keys;
- safe relative stream and chunk paths;
- deterministic chunk names and contiguous ordinals;
- declared byte lengths and mandatory SHA-256 checksums;
- newline-terminated, nonempty JSONL records;
- exact stream field coverage;
- record counts and first/last iterations; and
- strictly increasing iterations across chunk boundaries.

Checksums detect storage corruption and accidental alteration. They do not
establish scientific correctness, authorship, or cryptographic authenticity.

The fixture under `tests/fixtures/complete` is also opened by Workflow's Rust
reader, making it the shared cross-language golden example. The normative
protocol and schema remain authoritative over any individual fixture.

Workflow's Rust integration suite additionally runs a bidirectional test. Its
internal automatic persistence path produces a multi-chunk recording for this
package to read; the test-only Python bridge re-encodes those records; and the
public Rust reader validates the Python result, including exact
sensitive-float bits and Unicode. The Rust write session and Python bridge are
test infrastructure, not supported writer APIs.

## NumPy conversion

The optional converter verifies one completed recording through the official
reader and writes fixed-shape numeric JSON fields as C-contiguous `.npy`
arrays. It also writes iteration arrays, physical-time arrays when present,
and a `scientific-workflow-npy.v1` manifest describing every array and every
field omitted because it was nonnumeric or changed shape or dtype.

```bash
scientific-workflow-to-npy path/to/member-recording
scientific-workflow-to-npy path/to/member-recording --output path/to/processed
```

The equivalent module entry point is
`python -m scientific_workflow_reader.npy`. With no `--output`, conversion uses
a sibling directory named `<recording>-npy`. It never writes inside or modifies
the raw recording. Conversion uses bounded-memory verified iteration, writes a
private temporary directory, atomically publishes the completed result, and
resumes an existing result only when its manifest and arrays still match.
From a source checkout, the dependency-free launcher script is
`python/scripts/recording_to_npy.py`; it adds the adjacent package source and
accepts the same recording and `--output` arguments.

Python callers may use
`scientific_workflow_reader.npy.convert_recording(recording, output=None)`.
Workflow itself invokes the same module in batch mode for the reserved `$npy`
phase and publishes one member directory plus a
`scientific-workflow-npy-batch.v1` manifest at the standard
`<execution>/processed/replicate-NNNNNN` path.

## Development

```bash
cd python
python -m pip install -e .
python -m unittest discover -s tests -v
```

## License

Licensed under the MIT License.
