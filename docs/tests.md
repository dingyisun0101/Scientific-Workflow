# Test structure

Tests are organized by subsystem responsibility and boundary, not by private
implementation file.

## New model-to-runtime workflow

`rust/tests/study_workflow.rs` verifies:

- `#[model]` discovery without a second registry list;
- effect-free `Study::load(&Path)`;
- explicit deterministic `ModelCatalog` injection and duplicate rejection;
- manifest model matching;
- typed constants and writer preflight before output;
- display-field validation before output;
- deterministic inferred task identities and labels;
- immutable phase/dependency composition;
- topological phase execution;
- automatic recording for every inferred task; and
- the crate-level single `run(&Path)` entry point.

`rust/tests/task_workflow.rs` verifies the reduced task boundary:

- the Basic surface centers on `ScientificModel` rather than Task construction;
- advanced type-erased task derivation from a model; and
- key sorting and registration-key validation.

## Config

`config_workflow.rs` verifies strict duplicate-key handling, unknown-field
rejection, path containment, unique source reads, exact source preservation,
manifest defaults, dependency validation, `$sweep`/`$cases` expansion,
`model()` identity, typed constants decoding, and contextual
`DecodeModelConstants` failures.

## State and writer

`state_workflow.rs` and `analysis_workflow.rs` cover schema loading, exact schema
identity, typed heterogeneous payload ownership, checked time, tuple borrowing,
and ordered in-memory series.

`writer_workflow.rs` covers inferred all-field observation, explicit streams,
cadence/units, schema binding, canonical field order, clone-free borrowed
observation, owned encoded handoff, and tier-superset behavior.

## Transitional durable record mechanics

`storage_workflow.rs`, `storage_resilience.rs`, and `resume_workflow.rs` retain
coverage for bounded queues, chunks, metadata atomicity, failure evidence,
checksums, checkpoint reconstruction, rewind, leases, and completed reads.

`python_reader_conformance.rs` checks Rust/Python format compatibility.
`artifact_workflow.rs` and `rng_record_workflow.rs` cover immutable artifact and
RNG provenance behavior pending their migration into `record`.

`replicate_workflow.rs` covers the still-direct legacy subprocess replicate
adapter. New end-to-end replicate behavior belongs in runtime tests as that
adapter is retired.

## Required validation commands

```bash
cargo fmt --manifest-path rust/Cargo.toml -- --check
cargo clippy --manifest-path rust/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path rust/Cargo.toml --all-targets
cargo test --manifest-path examples/attractor_2d/Cargo.toml
RUSTDOCFLAGS="-D warnings" cargo doc --manifest-path rust/Cargo.toml --no-deps
cargo test --manifest-path rust/Cargo.toml --doc
```

Package inspection must also confirm that every `api.md` and the proc-macro
support required by the published dependency arrangement are present.
