# Test structure

Tests follow subsystem responsibility and supported boundaries. Config, Study,
and persistence write mechanics use internal tests because their planning/write
types are deliberately not public.

## Public integration tests

- `rust/tests/integration_surface.rs` verifies the crate-level `run(&Path)`
  facade, canonical error Basic/Advanced tiers, prelude aggregation, and the
  Study-only signature of Runtime's Advanced entry point.
- `rust/tests/state_workflow.rs` exercises Path-based schema loading,
  heterogeneous payload ownership, tuple borrows, time advancement, schema
  inspection, maintenance, and public Basic/Advanced tier behavior.
- `rust/tests/analysis_workflow.rs` exercises schema identity and ordered
  in-memory `StateSeries` analysis.
- `rust/tests/observation_workflow.rs` exercises public plan/stream
  declarations, cadence, units, validation, and the Advanced-as-public-superset
  rule. Schema binding and encoding are private.
- `rust/tests/task_workflow.rs` exercises the downstream
  `ScientificModel`/model-attribute surface. Catalogs, type erasure, and host
  execution stay internal.

## Internal compiler and execution tests

- `rust/src/config/tests/config_workflow.rs` covers duplicate keys, strict
  unknown-field rejection, safe Path containment, manifest/persistence
  defaults, positive limits, dependencies, deterministic `$sweep`/`$cases`
  expansion, private typed constants decoding, and contextual errors.
- `rust/src/study/tests/study_workflow.rs` covers linked model discovery,
  invalid/duplicate registrations, effect-free loading, unknown models, typed
  constants and one-time observation preflight, deterministic internal
  identities, phase composition, replicate/persistence policy, runtime
  scheduling, automatic task recordings, and crate-level `run(&Path)`.
- `rust/src/ui/terminal.rs` contains pure formatting tests for Runtime-derived
  task progress and completion facts. Ordinary test capture keeps the automatic
  UI silent, so UI cannot disturb test output or execution results.

## Persistence tests

The write path is intentionally tested beneath
`rust/src/persistence/tests/`:

- `persistence_workflow.rs` covers automatic-plan-equivalent local writes,
  bounded chunking, metadata transitions, terminal state deduplication,
  clone-free encoding, typed verified readback, and generic payload types.
- `persistence_resilience.rs` injects configuration, encoding, writer,
  lifecycle, decoder, malformed-record, missing-file, size, and checksum
  failures and verifies contextual/no-partial-success behavior.
- `python_reader_conformance.rs` verifies Rust/Python format-v7 compatibility
  and exact floating-point/unicode round trips.

Recovery/resume, public writer builders, per-stream layout controls, legacy
execution scopes, artifacts, and RNG-record tests were removed with those
unsupported APIs.

## Required validation commands

```bash
cargo fmt --manifest-path rust/Cargo.toml -- --check
cargo clippy --manifest-path rust/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path rust/Cargo.toml --all-targets
cargo test --manifest-path examples/attractor_2d/Cargo.toml
RUSTDOCFLAGS="-D warnings" cargo doc --manifest-path rust/Cargo.toml --no-deps
cargo test --manifest-path rust/Cargo.toml --doc
python3 -m unittest discover -s python/tests
```

Package inspection must also verify that every first-level module's `api.md`,
`docs/architecture.md`, and proc-macro support are included where required.
