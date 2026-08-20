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
python -m pip install scientific-workflow-reader
```

Python 3.10 or newer is required. The package has no runtime dependencies.

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

The initial release supports only
`scientific-workflow-jsonl` format version 7, positional JSON payload encoding, JSON Lines
framing, and `sha256:` chunk checksums. Unknown versions and algorithms fail
closed.

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
establish model correctness, authorship, or cryptographic authenticity.

The fixture under `tests/fixtures/complete` is also opened by Workflow's Rust
reader, making it a cross-language conformance contract.

Workflow's Rust integration suite additionally runs a bidirectional test. Its
public Rust writer produces a multi-chunk recording for this package to read;
the test-only Python bridge re-encodes those records; and the public Rust reader
validates the Python result, including exact sensitive-float bits and Unicode.
The bridge is test infrastructure, not a supported Python writer API.

## Development

```bash
cd python
python -m pip install -e .
python -m unittest discover -s tests -v
```

## License

Licensed under the MIT License.
