# Test structure

This map is the release-qualification baseline for Rust 0.13.5 and Python
reader 0.4.3.

Tests follow subsystem responsibility and supported boundaries. Observation
binding/session behavior, Config and Study compilation, and persistence write
mechanics use internal tests because their working types are deliberately not
public.

## Public integration tests

- `rust/tests/integration_surface.rs` verifies the crate-level `run(&Path)`
  facade, root `WorkflowError`, the complete ordinary prelude inventory,
  specialized module-root imports, and the Study-only signature of Runtime's
  execution entry point. It also verifies `WorkflowError` stage conversions,
  transparent display/source behavior, and `Send + Sync`.
- `rust/tests/state_workflow.rs` exercises Path-based schema loading,
  heterogeneous payload ownership, tuple borrows, time advancement, schema
  inspection, inherent maintenance, and module-root/prelude type identity.
- `rust/tests/analysis_workflow.rs` exercises schema identity and ordered
  in-memory `StateSeries` analysis.
- `rust/tests/observation_workflow.rs` exercises public plan/stream
  declarations, cadence, units, and validation. Schema binding and encoding
  are private.
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
  root and reserved files, required supported configuration-schema generation,
  optional `states/` grouping, rejection of schemas
  outside that root, duplicate keys, strict unknown-field rejection, canonical
  `wf_configs/parameters.json`, automatic execution unit-key section selection, named
  state-path maps, explicit per-task selectors, omission for later provider
  resolution, unknown state selectors, rejection of legacy task input paths, manifest/persistence
  defaults, positive limits, dependencies, generic execution unit/program/Python task
  grammar, executable resolution, all supported Python environment lowering
  and executable preflight, contained/escaping JSON symlinks, authored
  snapshot keys, non-UTF-8 document and project-root rejection before JSON
  provenance, RFC 6901 diagnostic pointers,
  deterministic `$sweep`/`$cases` expansion, inferred global-versus-local
  scope, whole-graph task multiplication, reserved `$npy` synthesis and
  validation, one aggregate task across global configurations, and
  malformed-marker rejection,
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
  identities, per-task binding of multiple named state schemas, standard
  provider resolution and provenance, missing-provider rejection before output, phase
  composition, replicate/persistence policy, runtime
  scheduling, public read-only compiled-plan inspection, automatic task
  recordings, and crate-level `run(&Path)`. Its
  recording checks require canonical `parameter_ordinal`/`parameter_source`
  provenance, exact Persistence-owned backend/effective-setting metadata, and
  rejection of legacy input-path fields. Its
  Unix program-task test verifies the frozen central Config after source files
  change, dependency-summary handoff, direct executable invocation, artifacts,
  logs, metadata, and generic runtime summaries. A global-sweep execution test
  also verifies per-task resolved snapshots and same-configuration dependency
  filtering. A separate direct Python task
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
  summaries verify the data-bearing program result variant.
- `rust/src/ui/command.rs` verifies the former editor and exact lowercase
  `exit` contract. `ui/session.rs` verifies that the interactive renderer closes
  only after both a terminal execution outcome and explicit `exit` submission.
  `ui/state.rs` verifies declaration-ordered event-reduced
  rows, per-phase task-panel replacement, progress, bounded message history,
  source-neutral cancellation, and phase/replicate/execution closure of
  pending rows as skipped. `ui/session.rs` verifies that recorded renderer
  failure returns at the Runtime-facing health boundary, while Runtime tests
  verify conversion to `RuntimeError::Presentation`. Before a release,
  manual PTY validation should confirm alternate-screen Ratatui rendering,
  completion retention, explicit keyboard exit, Ctrl+C cancellation without
  closure, cooperative Runtime cancellation, and terminal restoration.
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
  and exact floating-point/unicode round trips. It also checks that
  `protocol/compatibility.json` names the active Rust package and recording
  version.

The normative protocol lives in `protocol/recording-v7.md`; its strict
structural companion is `protocol/recording-v7.schema.json`. Python tests open
the shared golden fixture and verify that the compatibility manifest matches
the Python package/version constants. Any wire-format change follows the bump
checklist in the protocol rather than editing version constants independently.
`python/tests/test_npy.py` verifies direct and nested numeric conversion,
structured JSON fallback, ragged and empty records, C-contiguity, component
checksums, mandatory manifests, reconstruction, immutable raw recordings,
integrity-failure atomicity, resume validation, and Workflow dependency-batch
conversion with duplicate member suppression.
Runtime unit coverage verifies that `$npy` receives every transitive global
configuration and that its standard `processed_directory` remains visible to
each downstream configuration.

Recovery/resume, public writer builders, per-stream layout controls, legacy
execution scopes, artifacts, and RNG-record tests were removed with those
unsupported APIs.

## Required validation commands

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-targets --all-features --locked
cargo test -p scientific-workflow --all-targets --no-default-features --locked
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked
cargo test -p scientific-workflow --doc --all-features --locked
PYTHONPATH=python/src python -m unittest discover -s python/tests -v
```

The root virtual workspace owns the sole tracked `Cargo.lock`; member-local
lockfiles are not part of repository validation. `.github/workflows/ci.yml`
runs these checks, validates protocol JSON, inspects publishable crate contents,
and enforces the four required headings in every first-level subsystem
`api.md`.

Package inspection must also verify that every first-level module's `api.md`,
`docs/architecture.md`, and proc-macro support are included where required.

The attractor example is an executable integration demonstration, not merely a
compile fixture. A manual or release validation run should confirm that its six
execution unit tasks produce completed recordings, `$npy` publishes a batch of
C-contiguous arrays, and its dependent Python task writes
`attractor-sweep.svg` plus `plot-summary.json` beneath the configured
`output/plots` directory. The Python task reads only processed manifests and
arrays plus the `plot` section of central `wf_configs/parameters.json`; it has
no Rust caller wrapper. It verifies each recording's execution unit,
named-state selector, parameter ordinal, and canonical
parameter source before plotting. Project-file regression coverage preserves
the named state map/selector, central parameter sections, and phase-owned
two-second `start_interval_ms`; a public
`Study::load` test proves the complete example still passes current effect-free
preflight.

## Refactor qualification (Rust 0.13.5 / Python 0.4.3)

The runnable `examples/dependency_pipeline` covers the new public Rust imports,
typed checkpoint handoff, with_json_field, format-8 boundary output, format-7
periodic output, two-worker NPY conversion and whole-series Python analysis.
Its expected summary values are 7 through 12 at iterations 0 through 5.

Added Rust coverage: typed missing/ambiguous selection and preserved extensions;
nonzero/max-iteration boundary sampling; public module visibility; pause-aware
task and phase timeouts; start-before-progress ordering; live/raw program logs;
malformed framing; required-log failure via /dev/full; owned descendant cleanup;
interpreter symlink preservation; active-group visibility/history; narrow wrapped
message rendering. Existing execution/state/persistence behavior remains covered.

Added Python coverage: dependency/path/config accessors; fixed/ragged series and
map reuse; standard logging setup; serial/parallel equivalence, deterministic
ordering, worker failure/retry reuse; concurrent publication; cooperative pause,
resume, and cancellation; active spawn-worker acknowledgement before parent pause.

`python/benchmarks/conversion.py` is a reproducible Linux smoke benchmark, not a
universal performance claim. On this validation host, 20,000 records across four
uneven recordings (32-value vectors) took 1.499 seconds with one worker and 1.041
seconds with four. Sampled aggregate RSS was 63.5 MiB versus 281.8 MiB. RSS sums
processes and double-counts shared pages. These results support retaining the
explicit shared-budget rule; no undocumented memory/CPU cap is introduced.
Planning still scales with record/projection counts; large-workload optimization
and a shared persistence writer pool remain measurement-driven future work.


## Release 0.13.5 / 0.4.3 qualification

Linux qualification passed the all-feature workspace suite (126 tests), headless
crate suite, Clippy with warnings denied, rustdoc with warnings denied, and three
doctests. The built Python wheel passed all 30 tests from outside the source tree;
a separate dependency-free environment imported the core, dependencies, project,
and reporting modules without NumPy. The full dependency pipeline produced the
expected `[7, 8, 9, 10, 11, 12]` simulation series after v8 initialization.

OF's default `sw-version` was tested in an isolated copy with typed recording
selection, boundary sampling, renamed Python imports, and its generic series
implementation replaced by Workflow views: 22 Rust tests and its real PiP NPY
acceptance test passed, including deterministic fixed/ragged readback. GLV,
Simulator, and Eco Core pass Rust tests without source changes. GLV and Simulator
Python decoder tests pass after import migration. Dispatcher requires the typed
context migration and has an independently stale NPY v1 test expectation; its
migration guide records the exact validation outcome. After the JSON-access
bridge and correcting that v1 expectation to v2 in the isolated copy, all 16
Dispatcher tests passed. The admission-interval test measures Runtime start
events rather than child shell file timestamps, which include OS scheduling delay.

The manual Linux PTY check exercised pause/resume and both ordinary and forced
exit. Both restored terminal attributes; forced exit returned 130, ordinary
cancellation returned the application error exit status. These are local
qualification results, not a claim of CI or non-Linux coverage.
