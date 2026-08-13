# WorkflowRuntime Implementation Objectives

## Purpose

This file turns the agreed architecture in [`docs/design.md`](docs/design.md)
into executable, phase-gated work. Complete phases in order. A phase is complete
only when every objective and its exit gate pass; do not carry known failures
into the next phase.

Every completed phase must also increment the Rust crate patch version by
`0.0.1`, commit the complete phase deliberately, and push that commit to the
repository's default branch after its exit gate passes.

The final architecture has these fixed boundaries:

- `WorkflowRuntime` is the sole process-level owner of reporting, cancellation,
  cooperative resource control, and supported subprocess integration.
- Applications retain scientific task definitions, dependency graphs,
  scheduling decisions, and result handling.
- `ExecutionScope` remains a filesystem scope for recordings and artifacts.
- Storage writers remain per-recording owners; the runtime never merges writer
  queues or storage failure domains.
- Models receive task-local handles and never construct reporters.
- `SystemState` and `StateSeries` do not implement `Eq` or `PartialEq`.
- No deprecated compatibility layer is required after all repository consumers
  have migrated.

## Phase 0 — Baseline and contract lock

### Objectives

- [ ] Record the current Rust, Python, example, and documentation test results
      before structural changes.
- [ ] Update `docs/tests.md` so its planned final suite includes
      `runtime_workflow.rs` instead of `reporting_workflow.rs`.
- [ ] Convert the relevant unresolved statements in `audit.md` into links to
      this checklist or mark them resolved; retain the audit as history.
- [ ] Confirm the public vocabulary used by subsequent phases:
      `WorkflowRuntime`, `WorkflowRuntimeBuilder`, `RuntimeConfig`,
      `ResourcePolicy`, `TaskId`, `SectionId`, `RuntimeTask`, `TaskProgress`,
      `ActivityTask`, `RuntimeClientConfig`, and `RuntimeClient`.
- [ ] Confirm that explicit `RuntimeTask` registration is the core API and that
      project/configuration registration is a convenience adapter.
- [ ] Confirm that runtime configuration is separate from scientific
      `fixed.json` and `sweep.json`.

### Exit gate

```bash
cd workflow/rust
cargo test --all-targets --no-fail-fast --locked
cargo test --doc --locked
cargo clippy --all-targets --all-features --locked -- -D warnings

cd ../python
python -m pytest

cd ../examples/attractor_2d
cargo test --all-targets --locked
```

Save any pre-existing failure with its exact command and output before
continuing; new failures are not accepted as baseline.

## Phase 1 — Normalize the modern module tree without behavior changes

### Objectives

- [x] Retain the modern split-module layout (`foo.rs` plus `foo/`) throughout;
      do not introduce `mod.rs` files.
- [x] Rename `configuration/project.rs` to
      `configuration/project_config.rs` to distinguish `ProjectConfig` from
      top-level `ScientificProject`.
- [x] Remove the excluded development-only `rust/src/main.rs`.
- [x] Keep public paths, behavior, documentation examples, and error semantics
      unchanged during this mechanical phase.
- [x] Update package include rules if the normalized tree changes which files
      are published.
- [x] Compare the resulting tree against the intended tree in
      `docs/design.md`; document any deliberate deviation before proceeding.

### Exit gate

```bash
cd workflow/rust
cargo fmt --all --check
cargo test --all-targets --no-fail-fast --locked
cargo test --doc --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo package --allow-dirty --locked
```

The phase is behavior-preserving: the same public tests must pass before and
after the file moves.

## Phase 2 — Introduce the runtime task and presentation model

### Objectives

- [ ] Add opaque, stable `TaskId` and `SectionId` types; reject empty or
      duplicate identifiers before renderer startup.
- [ ] Add `RuntimeTask` declarations with a presentation label, optional
      section, stable display order, and an explicit progress/activity kind.
- [ ] Separate identity from labels throughout registration, lookup, errors,
      messages, and summaries.
- [ ] Add `ActivityTask` for lifecycle-only work with phase, message,
      cancellation, status, completion, failure, bounded `Debug`, and
      failure-on-drop behavior.
- [ ] Retain `TaskProgress` for iterative work and keep iteration updates
      allocation-free and lock-free on the hot path.
- [ ] Add section declarations and render one horizontal separator before the
      first task in each section.
- [ ] Provide a positional “split before task N” builder convenience that is
      normalized into section membership before startup.
- [ ] Divide interactive output into a stable task region and an append-only
      message region.
- [ ] Add terminal-only status colors for pending, running, completed, failed,
      and reused states.
- [ ] Keep plain output, redirected output, labels, and persisted values free
      of ANSI color codes.
- [ ] Preserve deterministic initial row materialization, elapsed timing, ETA,
      cancellation, and terminal restoration.
- [ ] Add project/configuration adapters that generate deterministic
      `RuntimeTask` declarations without making parameters the core identity
      mechanism.

### Required tests

- [ ] Duplicate task and section IDs fail before terminal acquisition.
- [ ] Duplicate labels are accepted when task IDs differ.
- [ ] Progress and activity tasks can run concurrently and summarize exactly.
- [ ] Activity tasks expose no iteration or target API.
- [ ] Section order and split placement remain stable with pending tasks.
- [ ] Interactive output contains status styling and no task/message overlap.
- [ ] Plain and redirected output contain no escape sequences.
- [ ] A task message remains visible without corrupting task rows.
- [ ] Dropped active handles fail only their reporting task.
- [ ] Hidden output retains complete lifecycle validation.

### Exit gate

```bash
cd workflow/rust
cargo test --test reporting_workflow -- --nocapture
cargo test --all-targets --no-fail-fast --locked
cargo test --doc --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
```

## Phase 3 — Make WorkflowRuntime the sole public owner

### Objectives

- [ ] Create `runtime.rs`, `runtime/error.rs`, `runtime/task.rs`,
      `runtime/reporting.rs`, and `runtime/renderer.rs`.
- [ ] Implement `WorkflowRuntimeBuilder` with task/section registration,
      project/configuration adapters, output policy, and startup validation.
- [ ] Implement non-clone `WorkflowRuntime` ownership of the process lease,
      renderer, registry, message channel, and cancellation source.
- [ ] Enforce one active runtime in every output mode, including hidden mode.
- [ ] Make task handles obtainable only from an active runtime.
- [ ] Ensure successful completion requires every task to be completed or
      explicitly marked reused.
- [ ] Ensure failure and drop restore terminal state, join owned threads, and
      release the process lease exactly once.
- [ ] Move reporting implementation under `runtime/` and remove the standalone
      public `reporting` module.
- [ ] Replace `ReportingError` with the runtime error vocabulary while
      retaining specific contextual variants and error sources.
- [ ] Update `lib.rs` and `prelude.rs` to expose the runtime API explicitly.
- [ ] Migrate `attractor_2d` so it constructs one runtime and passes only
      task-local handles into task execution and recording functions.
- [ ] Rename `reporting_workflow.rs` to `runtime_workflow.rs` and update
      `docs/tests.md` with its complete behavioral allocation.

### Required tests

- [ ] A second runtime fails while the first is active, including when both are
      hidden.
- [ ] Runtime startup failure never leaves the terminal lease held.
- [ ] Runtime completion, explicit failure, early return, and panic-safe drop
      each release all owned infrastructure.
- [ ] Project-derived, explicitly registered, progress, activity, and reused
      tasks coexist in one runtime.
- [ ] Reporting task failure does not mutate an independent recording
      lifecycle.

### Exit gate

```bash
cd workflow/rust
cargo fmt --all --check
cargo test --test runtime_workflow -- --nocapture
cargo test --all-targets --no-fail-fast --locked
cargo test --doc --locked
cargo clippy --all-targets --all-features --locked -- -D warnings

cd ../examples/attractor_2d
cargo test --all-targets --locked
```

No public `ProgressReporter` or `reporting` module reference may remain inside
the Workflow repository after this phase.

## Phase 4 — Add general cooperative resource control

### Objectives

- [ ] Add `runtime/resources.rs` containing `RuntimeConfig`, `ResourcePolicy`,
      validation errors, resource leases, and the private controller.
- [ ] Keep scheduling limits, compute parallelism, child-process concurrency,
      and operating-system isolation as distinct configuration concepts.
- [ ] Supply conservative host defaults without inventing unavailable OS
      isolation.
- [ ] Implement serde round-trip for runtime configuration independently of
      scientific parameter files.
- [ ] Enforce `max_active_tasks` for runtime-managed task execution without
      embedding a DAG or scientific scheduler.
- [ ] Enforce `compute_threads` through one runtime-owned compute boundary;
      document how cooperating callers execute work within it.
- [ ] Enforce `max_child_processes` with cancellation-aware leases in
      preparation for Phase 5.
- [ ] Define platform isolation as a backend-neutral public policy and keep
      systemd-specific translation private.
- [ ] Validate memory high/max ordering, nonzero quotas and limits, timeout,
      platform availability, and incompatible combinations before work starts.
- [ ] Never silently fall back from requested isolation to unrestricted
      execution.
- [ ] Preserve per-recording writer queues and backpressure; do not add a
      global writer manager.

### Required tests

- [ ] Runtime configuration round-trips without loss and rejects unknown or
      invalid fields.
- [ ] Active-task permits never exceed the configured maximum under contention.
- [ ] Compute work uses no more runtime worker threads than configured.
- [ ] Waiting for a resource is cancellation-aware and does not leak permits.
- [ ] Panic, task failure, and runtime shutdown release every acquired lease.
- [ ] Unsupported requested isolation fails before scientific work starts.
- [ ] Multiple recordings remain independent while runtime task limits bound
      their aggregate concurrency.

### Exit gate

```bash
cd workflow/rust
cargo test --test runtime_workflow -- --nocapture
cargo test --test storage_workflow -- --nocapture
cargo test --test storage_resilience -- --nocapture
cargo test --all-targets --no-fail-fast --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
```

## Phase 5 — Add the supported subprocess boundary

### Objectives

- [ ] Add `runtime/subprocess.rs` with `RuntimeClientConfig`, `RuntimeClient`,
      task-scoped authorization, protocol versioning, launch configuration,
      and contextual errors.
- [ ] Choose and document the private local transport after testing it against
      cancellation, process loss, output capture, and supported platforms; do
      not expose the transport in the public API.
- [ ] Authenticate or otherwise bind each child client to exactly one declared
      task token.
- [ ] Permit child progress, phase, status, message, and cancellation exchange
      without allowing task registration, runtime finalization, policy changes,
      or terminal ownership.
- [ ] Add a runtime-owned child launcher that consumes child-process permits
      and applies the selected isolation backend.
- [ ] Capture child stdout and stderr and route bounded output to task messages
      or task logs without writing through the interactive display.
- [ ] Propagate parent cancellation to children and support a bounded graceful
      shutdown followed by an explicit termination policy.
- [ ] Treat protocol failure, unauthorized task use, disconnect, timeout, and
      abnormal exit as contextual task/runtime failures.
- [ ] Release task and process leases on every child terminal path.
- [ ] Implement Linux systemd-user-scope translation behind the generic
      isolation policy, including memory, swap, CPU, task-count, and timeout
      settings.

### Required tests

- [ ] A fixture child reports progress and messages into the parent runtime.
- [ ] Two child processes update separate tasks without owning reporters.
- [ ] A child cannot update another task or register a new task.
- [ ] Captured stdout/stderr cannot corrupt the terminal display.
- [ ] Parent cancellation reaches a cooperative child.
- [ ] Crash, disconnect, timeout, and malformed protocol fail the correct task
      and restore all leases.
- [ ] Child concurrency never exceeds `max_child_processes`.
- [ ] Requested systemd isolation produces exact validated properties; missing
      systemd support fails closed.

### Exit gate

```bash
cd workflow/rust
cargo test --test runtime_workflow -- --nocapture
cargo test --all-targets --no-fail-fast --locked
cargo test --doc --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
```

## Phase 6 — Runtime resilience and public documentation

### Objectives

- [ ] Exercise terminal startup, resize/input handling, Ctrl-C, message bursts,
      plain output, hidden output, and renderer shutdown through public APIs.
- [ ] Verify the runtime remains responsive with more registered rows than the
      terminal height and with pending tasks held by resource limits.
- [ ] Bound retained child output and message buffering; document truncation or
      spill-to-log behavior.
- [ ] Verify cancellation and error propagation do not overwrite recoverable
      `Running` storage recordings with `Failed`.
- [ ] Update crate-level docs, Rust README, repository README, examples, and
      doctests to use only `WorkflowRuntime`.
- [ ] Update `docs/design.md` from planned wording to implemented wording as
      each contract becomes true.
- [ ] Update `docs/tests.md` with every public runtime structure and method,
      including indirect coverage of private renderer/resource/process code.
- [ ] Confirm the published crate contains the complete intended module tree
      and no superseded reporting source files.

### Exit gate

```bash
cd workflow/rust
cargo fmt --all --check
cargo test --all-targets --no-fail-fast --locked
cargo test --doc --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo package --allow-dirty --locked

cd ../python
python -m pytest

cd ../examples/attractor_2d
cargo test --all-targets --locked
```

## Phase 7 — Migrate dependent scientific projects

Migrate in dependency order. Do not remove an old Workflow entry point until
all callers identified by `rg` have moved.

### GLV

- [ ] Change standalone GLV execution to construct one `WorkflowRuntime`.
- [ ] Keep the embedded runner accepting only `TaskProgress`; it must not
      construct or borrow the full runtime.
- [ ] Replace template/test references to `ProgressReporter` with runtime task
      declarations and handles.
- [ ] Verify GLV success, cancellation, storage validation, and error cleanup.

### Simulator

- [ ] Change standalone Simulator execution to construct one
      `WorkflowRuntime` for the complete ensemble.
- [ ] Keep the embedded ensemble runner accepting task-local handles and a
      runtime-compatible concurrency boundary, not a reporter.
- [ ] Reconcile ensemble concurrency with runtime active-task and compute
      limits so nested pools cannot silently oversubscribe the configured
      budget.
- [ ] Verify multi-task progress, cancellation, recording validation, and
      failure cleanup.

### Dispatcher

- [ ] Replace `StudyProgress`'s owned `ProgressReporter` with one
      `WorkflowRuntime` spanning every visible study task.
- [ ] Register typed stable task IDs separately from human-readable labels.
- [ ] Represent matrix generation, initial-state creation, conversion, and
      other step-less operations as activity tasks.
- [ ] Declare study phases as runtime sections rather than deriving separators
      from display positions.
- [ ] Pass task-local handles into GLV and Simulator exactly as before.
- [ ] Replace Dispatcher-owned Rayon pool/resource validation with runtime
      resource configuration and execution boundaries where applicable.
- [ ] Replace handwritten `systemd-run` assembly with the runtime child
      launcher and generic isolation policy.
- [ ] Connect internal workers to the parent through `RuntimeClient`; remove the
      direct-backend restriction on unified reporting.
- [ ] Route Python processor output and status through runtime-owned child
      handling without allowing it to corrupt the display.
- [ ] Replace batch-shaped ready-step execution with work-conserving dispatch:
      when one ready task completes, newly unblocked work may start without
      waiting for unrelated tasks in the previous batch.
- [ ] Preserve receipt verification, restart boundaries, scientific task
      identity, RNG coordinates, and output paths.

### Exit gate

```bash
cd glv
cargo test --all-targets --no-fail-fast --locked
cargo test --doc --locked
cargo clippy --all-targets --all-features --locked -- -D warnings

cd ../simulator
cargo test --all-targets --no-fail-fast --locked
cargo test --doc --locked
cargo clippy --all-targets --all-features --locked -- -D warnings

cd ../dispatcher
cargo test --all-targets --no-fail-fast --locked
cargo test --doc --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
```

Search gate:

```bash
rg -n "ProgressReporter|RegisteredProgressReporterBuilder|scientific_workflow::reporting" \
  workflow glv simulator dispatcher \
  --glob '!**/target/**'
```

The search must return no production, test, example, or documentation caller.

## Phase 8 — Final cleanup and release gate

### Objectives

- [ ] Remove superseded reporting types, exports, tests, documentation, and
      dependencies after the downstream search gate is clean.
- [ ] Remove temporary adapters and migration-only code.
- [ ] Confirm the filesystem matches the intended tree in `docs/design.md`.
- [ ] Audit public `Debug`, `Display`, and error sources for bounded output and
      useful runtime/task/process context.
- [ ] Audit every runtime thread, process, terminal lease, resource lease, and
      channel for deterministic shutdown.
- [ ] Audit platform-specific code so unsupported targets fail explicitly or
      compile without unavailable backends as documented.
- [ ] Run formatting and package-content checks for every changed Rust crate.
- [ ] Run the complete Rust, Python, example, and dependent-project suites.
- [ ] Review `audit.md`; close resolved items and move any genuinely deferred
      item into a newly justified phase rather than leaving an unowned note.
- [ ] Update version, changelog/release notes, and public migration guidance
      only after every gate is green.

### Final gate

```bash
cd workflow/rust
cargo fmt --all --check
cargo test --all-targets --no-fail-fast --locked
cargo test --doc --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo package --allow-dirty --locked

cd ../python
python -m pytest

cd ../examples/attractor_2d
cargo test --all-targets --locked

cd ../../../glv
cargo test --all-targets --no-fail-fast --locked

cd ../simulator
cargo test --all-targets --no-fail-fast --locked

cd ../dispatcher
cargo test --all-targets --no-fail-fast --locked
```

Completion means the architecture is implemented and used end to end—not only
that the new types exist inside Workflow.
