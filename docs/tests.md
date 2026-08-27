# Test structure

Tests follow subsystem responsibility and supported boundaries. Observation
binding/session behavior, Config and Study compilation, and persistence write
mechanics use internal tests because their working types are deliberately not
public.

## Public integration tests

- `rust/tests/integration_surface.rs` verifies the crate-level `run(&Path)`
  facade, canonical error Basic/Advanced tiers, prelude aggregation, and the
  Study-only signature of Runtime's Advanced entry point, including
  `TaskRunKind` through `prelude::advanced`.
- `rust/tests/state_workflow.rs` exercises Path-based schema loading,
  heterogeneous payload ownership, tuple borrows, time advancement, schema
  inspection, maintenance, and public Basic/Advanced tier behavior.
- `rust/tests/analysis_workflow.rs` exercises schema identity and ordered
  in-memory `StateSeries` analysis.
- `rust/tests/observation_workflow.rs` exercises public plan/stream
  declarations, cadence, units, validation, and the Advanced-as-public-superset
  rule. Schema binding and encoding are private.
- `rust/tests/task_workflow.rs` exercises the downstream
  `ScientificModel`/model-attribute surface, including a model-owned stable
  state exposed by `state()` and coupled mutation through the public typed
  tuple-borrow API. Catalogs, type erasure, and host execution stay internal.

## Internal compiler and execution tests

- `rust/src/observation/tests/observation_workflow.rs` covers normalized
  declarations, schema-order binding, canonical encoding, schema identity,
  cadence, terminal deduplication, decreasing iterations, and failure-atomic
  session markers. The public declaration boundary remains covered separately
  by `rust/tests/observation_workflow.rs`.
- `rust/src/config/tests/config_workflow.rs` covers the required `wf_configs/`
  root and reserved files, optional `states/` grouping, rejection of schemas
  outside that root, duplicate keys, strict unknown-field rejection, canonical
  `wf_configs/parameters.json`, automatic model-key section selection, named
  state-path maps and explicit per-model selectors, unknown/missing state
  selectors, rejection of legacy task input paths, manifest/persistence
  defaults, positive limits, dependencies, generic model/program/Python task
  grammar, executable resolution, all supported Python environment lowering
  and executable preflight, contained/escaping JSON symlinks, authored
  snapshot keys, non-UTF-8 path rejection, RFC 6901 diagnostic pointers,
  deterministic `$sweep`/`$cases` expansion and malformed-marker rejection,
  private typed constants decoding, decimal-MB persistence-size conversion,
  overflow/legacy-byte-field rejection, central arbitrary-parameter capture,
  and contextual errors. Even an unreferenced JSON document is strict-parsed.
- `rust/src/study/tests/study_workflow.rs` covers linked model discovery,
  invalid/duplicate registrations, effect-free loading, unknown models, typed
  constants and one-time observation preflight, deterministic internal
  identities, per-model binding of multiple named state schemas, phase
  composition, replicate/persistence policy, runtime
  scheduling, automatic task recordings, and crate-level `run(&Path)`. Its
  recording checks require canonical `parameter_ordinal`/`parameter_source`
  provenance and reject legacy input-path fields. Its
  Unix program-task test verifies the frozen central Config after source files
  change, dependency-summary handoff, direct executable invocation, artifacts,
  logs, metadata, and generic runtime summaries. A separate direct Python task
  verifies that a non-executable `.py` script runs through its nested `system`
  environment without any Rust wrapper and records Python launcher provenance.
- `rust/src/ui/command.rs` verifies the former editor and exact lowercase
  `exit` contract. `ui/state.rs` verifies declaration-ordered event-reduced
  rows, per-phase task-panel replacement, progress, bounded message history,
  and cancelled/skipped outcomes. PTY validation
  confirms alternate-screen Ratatui rendering, keyboard exit, cooperative
  Runtime cancellation, and terminal restoration. Noninteractive tests receive
  stable plain lifecycle diagnostics rather than terminal control sequences.

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

The attractor example is an executable integration demonstration, not merely a
compile fixture. A manual or release validation run should confirm that its six
model tasks produce completed recordings and its dependent Python task writes
`attractor-sweep.svg` plus `plot-summary.json` beneath the configured
`output/plots` directory. The Python task uses the public verified reader and
the `plot` section of central `wf_configs/parameters.json`; it has no Rust caller
wrapper. A focused model test verifies that the demonstration-only configured
per-step delay is actually applied. A manifest regression test and Config
boundary coverage preserve the phase-owned two-second `start_interval_ms`
admission delay used by the example.
