# Scientific Workflow

The Rust crate is located in `dev/`. Run commands from that directory:

```bash
cd dev
```

## Standalone scientific-project example

[`examples/attractor_2d`](examples/attractor_2d) is a complete downstream
application built against the local crate. It loads project-root
`config/{fixed,sweep,paths,state}.json` inputs, expands a parameter sweep,
evolves one directly owned `SystemState` per task with explicit Euler, and
records trajectory, radius, and checkpoint streams at independent cadences.

Run it from the repository root:

```bash
cargo run --manifest-path examples/attractor_2d/Cargo.toml
```

Recordings are written beneath the example's ignored `target/recordings`
directory. Each execution selects a new run directory, so rerunning the
example does not overwrite prior results. The executable covers configuration,
task expansion, state evolution, bounded recording, chunking, explicit
recording completion, typed stream reconstruction, numerical summaries, a
terminal phase portrait, and exact live-to-stored verification.

The example is maintained as a repository-level project and is not included
in the library crate's crates.io archive. Its
[`steps.md`](examples/attractor_2d/steps.md) explains the reusable development
sequence for a general scientific project.

## Integration tests

The permanent suite contains six logged, behavior-oriented workflows rather
than source-file-level tests.

### Project configuration

```bash
cargo test --test configuration_workflow -- --nocapture
```

Loads real Cartesian and correlated-case projects, generates dict-like task
parameters, resolves named paths, proves shared value ownership, performs an
exact three-file export/reload, and rejects ambiguous configuration.

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
writing, automatic chunking, metadata, integrity verification, and typed
reconstruction. Successful output verifies that sealed chunks have final names
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
