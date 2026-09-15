# Test structure

This map is the release-qualification baseline for Rust 0.15.3 and Python
companion 0.5.0.

Explicit-phase coverage checks required numeric selection, stable dependency-order
indices and task identities, completed-program and legacy-unit reuse, chained
source references, and rejection of missing, failed, changed, or stale inputs
before creating new execution output.

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
  distinct sequential/parallel replicate admission, and execution-unit
  `active: false` remaining planned without creating task output. It also runs a
  two-member execution unit end to end and verifies independent recordings,
  member provenance, final iterations, and `MemberRunSummary` paths. Study execution tests
  retain topology and program/Python handoff coverage; successful program
  summaries verify the data-bearing program result variant.
- `rust/src/ui/command.rs` verifies the former editor and exact lowercase
  `exit` contract. `ui/session.rs` verifies that the interactive renderer closes
  only after both a terminal execution outcome and explicit `exit` submission.
  `ui/live_log.rs` verifies that messages queued before output creation and
  messages appended afterward are immediately readable from execution
  `log.txt`. `ui/usage.rs` verifies Linux CPU and RAM counter parsing and
  bounded percentages; terminal rendering verifies all CPU/RAM/DISK labels.
  `ui/state.rs` verifies declaration-ordered event-reduced
  rows, per-phase task-panel replacement, progress, bounded message history,
  source-neutral cancellation, and phase/replicate/execution closure of
  pending rows as skipped. `ui/session.rs` verifies that recorded renderer
  failure returns at the Runtime-facing health boundary, while Runtime tests
  verify conversion to `RuntimeError::Presentation`. Before a release,
  manual PTY validation should confirm alternate-screen Ratatui rendering,
  completion retention, explicit keyboard exit, Ctrl+C cancellation without
  closure, cooperative Runtime cancellation, and terminal restoration.
  Runtime unit tests use a test-only observer; the public facade is exercised
  in a real PTY by `rust/tests/dashboard.rs` and `terminal_probe.py`. Missing
  terminals are rejected before output creation or cleanup.

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
`python/tests/test_npy.py` verifies the split `scientific_workflow.npy`
format/planning/writing/workflow/CLI package through its stable public namespace,
including direct and nested numeric conversion,
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
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps --locked
cargo test --manifest-path rust/Cargo.toml --doc --all-features --locked
PYTHONPATH=python/src python -m unittest discover -s python/tests -v
```

The root virtual workspace owns the sole tracked `Cargo.lock`; member-local
lockfiles are not part of repository validation. `.github/workflows/ci.yml`
runs these checks, validates protocol JSON, inspects publishable crate contents,
and enforces the four required headings in every first-level subsystem
`api.md`.

Package inspection must verify that every first-level module's `api.md` is
included, proc-macro support resolves from its published dependency, and the
crate README links to the release's repository-owned `docs/architecture.md`.

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

The examples use the published Workflow dependency. Commands targeting the local
Workflow crate use `--manifest-path rust/Cargo.toml` to avoid ambiguity with that
registry package in the same dependency graph.

Nested sweep expansion is covered by Config's expansion unit tests and
`nested_local_axes_preserve_one_initialization_per_global_configuration` in
`rust/src/config/tests/config_workflow.rs`. They verify one literal base plus a
Cartesian product, deterministic ordering, global/local correlation, a single
initialization per global configuration, unchanged admission intervals, and
rejection of malformed nested or hidden literal-array markers.


## Release 0.13.7 qualification

The all-feature workspace suite passed 130 tests; the headless runtime and
integration suite passed 116 tests. Clippy and rustdoc passed with warnings
denied, all three doctests passed, and the Python 0.4.3 companion passed its
30 tests. The published-dependency initialization pipeline and six-case attractor
example completed, including NPY conversion and Python analysis. Package
verification built the crate against registry dependencies. Nested-selection
coverage additionally verifies Cartesian composition with sibling axes, literal
vector choices, and unchanged opaque arrays outside choices.

The two private example crates now explicitly require crates.io Workflow 0.13.7;
the workspace lockfile resolves that release separately from the local runtime
under test. The 130-test all-feature workspace suite passed with these updated
example dependencies. This consumer update does not change the runtime crate.
## 0.13.10 / 0.4.5 regressions

Runtime tests cover twelve-member clocks, early completion, and unknown targets.
UI rendering tests cover complete counters at widths 35, 80, 99, 100, and 173,
plus explicitly omitted counters when even the numbers cannot fit. Config tests
cover valid exclusions, unsafe argument interpretation, duplicate/invalid names,
and rejection on ordinary phases. Python tests cover mixed streams, skipped
corrupt excluded chunks, all-excluded datasets, unknown names, invalid lists,
CLI forwarding, serial/parallel parity, and filter-aware retries.

## 0.13.11 reuse compatibility regressions

Private Persistence tests check conversion-only exclusions, strict NPY/consumer
filter identity, phase selection, reuse paths, and unchanged scientific and
phase-graph validation. `rust/src/runtime/tests/reuse_npy_filters.rs` runs a miniature program
workflow, adds and changes downstream NPY filters, chains completed upstream
reuse, preserves original receipts, and rejects changed scientific inputs before
creating another execution. Dispatcher separately exercises the same transition
with completed GLV and target-generation outputs plus actual filtered NPY export.

## 0.14.0 compute-allocation regressions

Config tests require the global compute mode, reject fixed execution-unit
resources in automatic mode, require them in isolated mode, and retain fixed
allocations in the compiled task. Study tests reject automatic execution units
that do not declare the thread-count-invariant contract before output exists.
Private compute tests verify equal automatic shares across registered working
tasks, expansion after a sibling finishes, stable allocation provenance, and
unchanged isolated allocations. A deterministic stale-wait regression commits
a newcomer's requested allocation epoch, resumes an existing compute call before
the waiter checks coordinator state, and verifies that the completed registration
does not wait for another globally idle instant. Resource tests separately verify
automatic working-task admission and fixed-thread accounting. Runtime tests
execute units inside both automatic and isolated private pools and inspect
persisted compute metadata.


## Update pass: identity and JSON

Reuse tests distinguish operational resource/scheduling policy from scientific
inputs. Persistence tests reject changed program-input bytes (including JSON
whitespace) and repeated terminal status updates. Python retry tests require
both member and batch JSON bytes and modification times to remain unchanged.


Output tests force duplicate timestamps, verify collision suffixes, assert
shared/exclusive project-lock conflicts, and exercise protected reuse paths,
output-root symlinks, and child symlink cleanup. Live-log tests parse UTC stamps
on buffered, live, and multiline messages.


Resource-policy tests cover disk defaults/bypass/range checking, strict NPY
limits, exact threshold and recovery transitions, independent manual pause,
monitor failure/cancellation, and fixed/auto conversion equivalence.


Allocation tests follow automatic 4 → 2+2 → 4 rebalancing across replicates,
external thread reservations, and final release to zero, checking every total
against the shared budget. UI tests verify the thread column/Usage counts, full
page contents, last-page clamping, resizing, and unclipped progress counters.


## Rust 0.15.0 / Python 0.5.0 candidate qualification

The coordinated NPY test needs an installed Python 3.14+ companion with the NPY
extra, so the ordinary Rust suite marks it ignored. CI and release qualification
run it explicitly after installing the matching companion:

```bash
cargo test --manifest-path rust/Cargo.toml --lib coordinated_npy_handoff --locked -- --ignored
```

It exercises real Rust recordings through the synthesized Python invocation in
both fixed and auto mode, checking the phase thread allowance and completed
two-member batch. Runtime cleanup integration verifies replacement of arbitrary
old output after preflight without changing configuration. Lock tests also cover
an active run reaching the same output directory through a symlink alias.

The private examples now consume published Workflow 0.15.0 and Python 0.5.0.
Before publication both were also qualified against 0.14.2 / 0.4.5. CI installs
the examples' published companion from `examples/requirements.txt` in a separate
environment, independently of future candidate package versions.
The examples now declare the required isolated compute mode and per-unit thread
requests explicitly. This repairs a pre-existing runnable-example preflight gap.

The candidate's 35 Python tests passed against its installed wheel from outside
the source tree. Wheel and source-archive metadata checks passed. A Linux PTY
check exercised thread display, pause/resume, page keys, resizing, explicit exit,
and exact terminal-attribute restoration. Execution now requires a dashboard;
the no-default-features/headless validation lane has been removed.

### Final decision regressions

Disk policy tests require typed resume after safe recovery, reject early resume,
keep the execution clock frozen after recovery, recheck the gate when space
decreases again, and retain cancellation behavior. Config tests assert default
NPY auto mode and inheritance of the global thread budget. The public-facade
PTY test exercises typed pause/resume, disk reminders and rejected resume,
paging, resizing, cancellation, exit, and exact terminal restoration.
Noninteractive facade tests preserve existing output even with `--clean`.
No production headless observer or plain renderer remains.

The pause/timeout integration allows a 1-second active deadline and pauses for
1.2 seconds. The longer margin avoids the previously observed 80-ms deadline
flake on shared CI runners while still proving paused time is excluded.
Release examples run their real dashboard through
`scripts/run_dashboard_check.py`, which types exit after a terminal outcome
and checks terminal restoration. This harness is for qualification; users
launch inside screen/tmux.

## Documentation release 0.15.1

This patch changes documentation and release metadata only. The twelve numbered
chapters live in `docs/guide/`; their navigation, local Markdown targets and
anchors, and release-pinned README targets were checked. All 15 fenced JSON
examples and fragments parse. The complete `study.json` tables in the crate
README and chapter 4 were compared for synchronization.

The first-study source and semantic-loading snippet compile against the published
0.15.0 API (unchanged in 0.15.1). The first study ran in a real PTY and its Python
readback verified iterations 0–5, final population 15, and cumulative births 5.
The chapter 9 pipeline completed conversion and produced the expected SVG; the
chapter 11 selection reused its completed prerequisites and reran plotting.

Release validation also passed workspace formatting, warnings-denied Clippy,
Rust tests, rustdoc, doctests, coordinated NPY handoff, 35 Python tests, and Cargo
package verification. No source API or architectural boundary changed; subsystem
API contracts remain in place. Architecture documentation now maps the new user
documentation directories.


The release commit passed both GitHub CI jobs before tag `v0.15.1` was pushed
and Rust 0.15.1 was published. The published README's rendered HTML contains the
screenshot, parameter tables, and numbered-guide links; the release screenshot
matches the checked-in image byte for byte.

The first study was rerun against registry 0.15.1. Both bundled examples now
resolve registry 0.15.1 and completed through real dashboards; the consumer
workspace passes Clippy and Rust tests. Additional tutorial checks verified
chapter 6's sampled iterations `[0, 2, 4, 5]` and checkpoint `[0, 5]`, and chapter
12's 40-task study with five resulting figures. No local upstream overrides
were used for these checks.


## Agent-instruction documentation release 0.15.2

`docs/AGENTS.md` consolidates the integration patterns reviewed across all nine
Dispatcher project manifests and all eight OmniFluid instance manifests, with
supporting model adapters, initialization/checkpoint handoff, analysis consumers,
and validation tools. The published guidance uses generic patterns and current
Workflow contracts rather than project-specific scientific settings or older
runtime requirements. Neither source project was modified.

Both READMEs require all AI agents to read the new instructions. The documentation
index, previous agent-guide entry point, and AI-assisted-studies chapter lead to
the same canonical file. All 292 Markdown links were checked, including local
anchors and release-pinned targets. The complete `study.json` reference remains
synchronized between the crate README and chapter 4.

Local release checks passed: formatting, warnings-denied Clippy, 156 Rust unit
tests and 20 integration tests, three doctests, rustdoc, explicit coordinated
NPY handoff, 35 Python tests, and package verification. No scientific or runtime
source changed. Subsystem API contracts remain valid; the architecture guide
records the new documentation entry point.


Release commit `f90678b` passed both GitHub CI jobs. Tag `v0.15.2` was pushed
before publication. The live crates.io README contains the mandatory agent link,
and the release's `docs/AGENTS.md` matches the checked-in document byte for byte.
Both bundled examples resolve registry Workflow 0.15.2; Clippy passes and the
dependency pipeline completed with values `[7, 8, 9, 10, 11, 12]`.

The first post-publication workspace test run encountered `WouldBlock` in the
unchanged `timestamp_collisions_cleanup_scope_and_project_leases` test. Its
isolated rerun and a subsequent complete workspace run both passed without
source changes. This transient failure is retained here rather than being
reported as an uninterrupted pass.


## Project-migration documentation release 0.15.3

The agent instructions now contain an eight-step procedure for converting an
existing general-purpose scientific project to Workflow. Both READMEs advertise
that procedure, with direct links from the documentation index and AI guide.
The procedure distinguishes external-task orchestration from typed Rust member
recording, preserves scientific baselines and legacy data, and defines cutover
criteria. No runtime or scientific API changed; existing subsystem contracts and
architecture remain valid.

All 302 Markdown links and anchors pass, the two new JSON examples parse, and
the crate README's settings reference remains synchronized with chapter 4.
The migration example's exact manifest and parameters ran with two small Python
fixture tasks in a real PTY against published Workflow 0.15.2, whose runtime is
unchanged in this patch. It produced four tasks and two summaries: rate 0.1
mapped to 1.0 and rate 0.2 mapped to 2.0. Each analysis task verified that its
selected producer matched its resolved global configuration.

Local release validation passed formatting, warnings-denied Clippy, 156 Rust
unit tests, 20 integration tests, three doctests, rustdoc, the explicit coordinated
NPY handoff, 35 Python tests, and Cargo package verification.


Release commit `759eab6` passed both GitHub CI jobs before `v0.15.3` was pushed
and the crate published. The published agent document matches the checked-in
file, and the live crates.io README advertises migration instructions with the
correct section link. The required AI notice, screenshot, and parameter tables
remain present. The migration fixture reran against published 0.15.3 and retained
both correctly correlated results. Bundled examples now resolve registry 0.15.3
and the consumer workspace passes warnings-denied Clippy.
