# Scientific Workflow

The Rust crate is located in `dev/`. Run commands from that directory:

```bash
cd dev
```

## Integration tests

The permanent suite contains four logged, behavior-oriented workflows rather
than source-file-level tests.

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
the public prelude and `RunOutput`, including borrowed encoding, bounded
writing, automatic chunking, metadata, integrity verification, and typed
reconstruction.

### Storage resilience

```bash
cargo test --test storage_resilience -- --nocapture
```

Injects configuration, writer, decoder, record, and chunk-integrity failures
and verifies contextual errors without exposing partial results.

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
