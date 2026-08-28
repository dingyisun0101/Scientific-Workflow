# Test structure

Tests follow subsystem responsibility and supported boundaries. Observation
binding/session behavior, Config and Study compilation, and persistence write
mechanics use internal tests because their working types are deliberately not
public.

## Public integration tests

- `rust/tests/integration_surface.rs` verifies the crate-level `run(&Path)`
  facade, canonical error Basic/Advanced tiers, the complete supported Basic
  and Advanced prelude inventories, and the Study-only signature of Runtime's
  Advanced entry point. It also verifies `WorkflowError` stage conversions,
  transparent display/source behavior, `Send + Sync`, and `TaskRunKind`
  through `prelude::advanced`.
- `rust/tests/state_workflow.rs` exercises Path-based schema loading,
  heterogeneous payload ownership, tuple borrows, time advancement, schema
  inspection, maintenance, and public Basic/Advanced tier behavior.
- `rust/tests/analysis_workflow.rs` exercises schema identity and ordered
  in-memory `StateSeries` analysis.
- `rust/tests/observation_workflow.rs` exercises public plan/stream
  declarations, cadence, units, validation, and the Advanced-as-public-superset
  rule. Schema binding and encoding are private.
- `rust/tests/task_workflow.rs` exercises the downstream
  `ExecutionUnit`/registration-attribute surface, including stable per-member
  states exposed by `MemberView` and coupled mutation through the public typed
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
  `wf_configs/parameters.json`, automatic execution unit-key section selection, named
  state-path maps and explicit per-task selectors, unknown/missing state
  selectors, rejection of legacy task input paths, manifest/persistence
  defaults, positive limits, dependencies, generic execution unit/program/Python task
  grammar, executable resolution, all supported Python environment lowering
  and executable preflight, contained/escaping JSON symlinks, authored
  snapshot keys, non-UTF-8 document and project-root rejection before JSON
  provenance, RFC 6901 diagnostic pointers,
  deterministic `$sweep`/`$cases` expansion and malformed-marker rejection,
  private typed constants decoding, decimal-MB persistence-size conversion,
  overflow/legacy-byte-field rejection, central arbitrary-parameter capture,
  clone-cheap frozen snapshot bytes, and contextual errors. Even an
  unreferenced JSON document is strict-parsed.
- `rust/src/task/tests/task_workflow.rs` covers invalid/duplicate registration
  keys, `!Send + !Sync` constants, cancellation before initialization and
  between steps, initial/step/final observation ordering, failure atomicity,
  state-owner/schema stability, stable positive member count/order/identity,
  independent ensemble completion, strict unit advancement, and target
  progression invariants through a private fake host. A direct program-port
  test verifies Task's semantic invocation view, including Python provenance,
  without exposing Config's resolved-program representation to Runtime.
- `rust/src/study/tests/study_workflow.rs` covers linked execution unit discovery,
  effect-free loading, unknown execution units, typed
  constants and one-time observation preflight, contextual named-schema
  validation errors, Study/error `Send + Sync`, deterministic internal
  identities, per-task binding of multiple named state schemas, phase
  composition, replicate/persistence policy, runtime
  scheduling, automatic task recordings, and crate-level `run(&Path)`. Its
  recording checks require canonical `parameter_ordinal`/`parameter_source`
  provenance, exact Persistence-owned backend/effective-setting metadata, and
  rejection of legacy input-path fields. Its
  Unix program-task test verifies the frozen central Config after source files
  change, dependency-summary handoff, direct executable invocation, artifacts,
  logs, metadata, and generic runtime summaries. A separate direct Python task
  verifies that a non-executable `.py` script runs through its nested `system`
  environment without any Rust wrapper and records Python launcher provenance.
- `rust/src/runtime/tests/runtime_workflow.rs` covers summary/error
  thread-safety, completion-time deadline classification, task and phase
  timeout lifecycle, panic-to-failed-recording cleanup, parallel replicate
  fail-fast cancellation, parallel finish-all completion, phase-level failure
  policies, start-interval/concurrency admission, deterministic task order,
  and distinct sequential/parallel replicate admission. It also runs a
  two-member execution unit end to end and verifies independent recordings,
  member provenance, final iterations, and `MemberRunSummary` paths. Study execution tests
  retain topology and program/Python handoff coverage; successful program
  summaries verify the public program-kind and Python-script accessors.
- `rust/src/ui/command.rs` verifies the former editor and exact lowercase
  `exit` contract. `ui/state.rs` verifies declaration-ordered event-reduced
  rows, per-phase task-panel replacement, progress, bounded message history,
  source-neutral cancellation, and phase/replicate/execution closure of
  pending rows as skipped. `ui/session.rs` verifies that recorded renderer
  failure panics at the Runtime-facing health boundary. Before a release,
  manual PTY validation should confirm alternate-screen Ratatui rendering,
  keyboard exit, cooperative Runtime cancellation, and terminal restoration.
  Noninteractive tests receive stable plain lifecycle diagnostics rather than
  terminal control sequences.

## Persistence tests

The write path is intentionally tested beneath
`rust/src/persistence/tests/`:

- `persistence_workflow.rs` covers automatic-plan-equivalent local writes,
  bounded chunking, metadata transitions, terminal state deduplication,
  clone-free encoding, typed verified readback, and generic payload types.
- `persistence_resilience.rs` injects configuration, initial/final session
  observation, encoding, writer, lifecycle, decoder, malformed-record,
  missing-file, size, checksum, latest-chunk descriptor/order, and program
  status transitions and verifies failed-metadata and
  no-partial-success behavior. Reader construction exercises State's direct
  ordered-field reconstruction boundary rather than a JSON serialization and
  reparse round trip.
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
execution unit tasks produce completed recordings and its dependent Python task writes
`attractor-sweep.svg` plus `plot-summary.json` beneath the configured
`output/plots` directory. The Python task uses the public verified reader and
the `plot` section of central `wf_configs/parameters.json`; it has no Rust caller
wrapper. It consumes the reader's immutable metadata mapping and verifies each
recording's execution unit, named-state selector, parameter ordinal, and canonical
parameter source before plotting. Project-file regression coverage preserves
the named state map/selector, central parameter sections, and phase-owned
two-second `start_interval_ms`; a public
`Study::load` test proves the complete example still passes current effect-free
preflight.
