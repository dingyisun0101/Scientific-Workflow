# Scientific Workflow

The Rust crate is located in `dev/`. Run commands from that directory:

```bash
cd dev
```

## Standalone scientific-project example

[`examples/attractor_2d`](examples/attractor_2d) is a complete downstream
application built against the local crate. It loads project-root
`config/{fixed,sweep,paths,state}.json` inputs, expands a parameter sweep,
iterates complete shared `TaskConfig` handles, evolves one directly owned
`SystemState` per task concurrently through Rayon, and
offers every evolved state to one writer that owns the independent trajectory,
radius, and checkpoint sampling intervals.

Run it from the repository root:

```bash
cargo run --manifest-path examples/attractor_2d/Cargo.toml
```

Recordings are written beneath the example's ignored `target/recordings`
directory. `ExecutionScope` selects a new timestamped, collision-resistant run directory, so rerunning the
example does not overwrite prior results. The executable covers configuration,
task expansion, state evolution, bounded recording, chunking, explicit
recording completion, latest-state reconstruction, and exact live-to-stored
verification. It prints only one minimal validation result.

The example is maintained as a repository-level project and is not included
in the library crate's crates.io archive. Its
[`steps.md`](examples/attractor_2d/steps.md) explains the reusable development
sequence for a general scientific project.

## Mandatory chunk integrity

Every sealed chunk carries a SHA-256 checksum in the recording's sole metadata
file. Checksum verification is a compulsory part of chunk validation: public
completed-recording read paths do not provide a flag, feature, or alternate API
that disables it. A missing chunk, byte-count mismatch, unsupported checksum,
or digest mismatch is a hard error, and a failed multi-chunk read exposes no
partial series as if it were valid scientific output.

Valid JSON is not sufficient evidence of an intact scientific record. A code
path that deliberately does not open an older sealed chunk has not validated
that chunk and must not describe it as verified. Any chunk selected for
scientific reconstruction must pass its checksum boundary.

Checksums detect accidental alteration and storage corruption. They do not by
themselves prove model correctness, authorship, or cryptographic authenticity;
those are separate provenance concerns.

## Official Python reader

The repository's [`python`](python) package provides the official eager Python
reader for completed format-v4 recordings. It validates the same lifecycle,
metadata, framing, schema, ordering, byte-count, and SHA-256 rules as Rust's
`StoredStateSeriesReader`. Both readers consume one checked-in conformance
fixture, preventing the Python implementation from becoming an undocumented
copy of incidental storage details.

The cross-language suite also performs a bidirectional round trip: the public
Rust writer emits a multi-chunk recording consumed by Python, a test-only
Python conformance producer re-encodes the reconstructed records, and the
public Rust reader verifies the Python output down to floating-point bits.

```python
from scientific_workflow_reader import open_completed_recording

reader = open_completed_recording("path/to/completed/recording")
signal = reader.read_stream("signal")
```

## RNG provenance

Workflow's `RngRecord` stores a resolved method, sequence-affecting version,
key encoding, key, and optional parameters beneath a caller-owned namespace.
It never generates random values or defines a competing RNG configuration.
Applications should pass one upstream configuration—such as PiP's
`RngConfig`—to the scientific component, then copy that component's resolved
seed and method identity into `RngRecord`. The complete mapping and example are
documented in [`dev/README.md`](dev/README.md#rng-records).

## Centralized progress reporting

`ProgressReporter` is the sole human-facing terminal owner while parallel work
is active. It derives task identities from any caller-selected unique parameter
combination, orders display rows by the automatically assigned task ordinal,
and polls per-task atomic iteration counters from one renderer thread. Models,
writers, and Rayon workers do not print or draw terminal elements directly.

Interactive sessions clear the terminal once after the reporter acquires its
exclusive lease, then receive one persistent row per configured task. Known
targets display elapsed execution time and estimated remaining time; pending and
open-ended tasks report that ETA is unknown. Redirected stderr receives stable
status lines and is never cleared. Task progress is observational: models
continue to own scientific iteration through `SystemState`, and workers
synchronize the reporter with `TaskProgress::set_iteration`.

## Integration tests

The permanent suite contains seven logged, behavior-oriented workflows rather
than source-file-level tests.

### Project configuration

```bash
cargo test --test configuration_workflow -- --nocapture
```

Loads real Cartesian and correlated-case projects, the conventional state
schema, and generated/named execution scopes; generates dict-like task
configurations, filters exact sweep values, rejects ambiguous unique selection,
resolves named paths, proves shared value ownership, performs an exact
three-file export/reload, and rejects ambiguous configuration.

### Simulation state

```bash
cargo test --test state_workflow -- --nocapture
```

Loads the real JSON template and verifies mutable live-state evolution,
zero-copy payload ownership, validation failures, transactional time, and
explicit deep-clone behavior.

### Analysis series

```bash
cargo test --test analysis_workflow -- --nocapture
```

Verifies ordered collection invariants, move-based ownership, borrowed views,
narrow field mutation, rejection recovery, capacity reuse, and deep cloning.

### Successful storage workflow

```bash
cargo test --test storage_workflow -- --nocapture
```

Runs default decoder round trips and a multi-stream PiP tensor workflow through
the public prelude and `SystemStateWriter`, including borrowed encoding, bounded
writing, automatic chunking, operational timing, terminal metadata, integrity
verification, and latest-state/full-series typed reconstruction. Successful output verifies that sealed chunks have final names
and no temporary files remain after durable publication.

### Storage resilience

```bash
cargo test --test storage_resilience -- --nocapture
```

Injects configuration, writer, decoder, record, and chunk-integrity failures
and verifies contextual errors without exposing partial results.

### Interrupted-run resume

```bash
cargo test --test resume_workflow -- --nocapture
```

Exercises both open-chunk crash windows, complete typed checkpoint
reconstruction, multiple-sealed-plus-open recovery without inspecting sealed
content, continued append ordering, continuation rejection boundaries,
explicit durability barriers, and artifact-free exclusive writer ownership.

### Parallel progress reporting

```bash
cargo test --test reporting_workflow -- --nocapture
```

Validates parameter-combination identity, automatic ordinal ordering,
parallel-safe atomic updates, exclusive terminal ownership, known and unknown
targets, lifecycle summaries, and failure-on-drop behavior.

## Complete verification

```bash
cargo test --all-targets --no-fail-fast --locked
cargo test --doc --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
```

The detailed coverage allocation and logging contract are documented in
[`tests.md`](tests.md).

Downstream crates can bring the complete supported API into scope with:

```rust
use scientific_workflow::prelude::*;
```
