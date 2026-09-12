# Scientific Workflow

> **BREAKING COMPUTE UPDATE: Rust 0.14.2 / Python 0.4.5 unchanged.**
> Every study now requires `compute.mode`, and automatic allocation requires
> each linked execution unit to declare `THREAD_COUNT_INVARIANT = true`.
> This supersedes the single shared Rayon pool used by Rust 0.13.x. There is
> no compatibility default or alias: choose `auto` for equal dynamic shares
> among working tasks or `isolated` for fixed per-task `resources.threads`.
> Recording formats and the Python companion remain unchanged.

Rust 0.13.11 and Python 0.4.5 correct ensemble progress to the maximum member
iteration/target and support `"$npy":{"after":["evolve"],"exclude_streams":["checkpoint"]}`.
The UI preserves complete numeric counters at narrow widths. Stream exclusions
affect conversion only; raw recordings and scientific stepping are unchanged.

Scientific Workflow turns registered Rust scientific execution units, arbitrary
executable programs, and declarative JSON into validated, recorded studies.

## Nested sweep alternatives in 0.13.8

Workflow 0.13.8 accepts independent `$sweep` axes within an outer sweep
alternative. A null base plus a six-by-four parameter grid expands to 25 tasks
without application-side enumeration. Global/local scope, existing flat sweep
ordering, and runtime admission policy are unchanged. `$cases` remains terminal.
See the [Config contract](rust/src/config/api.md#nested-alternatives-and-independent-axes).

## Start here

- **Start with the [Linux and Python setup guide](https://github.com/dingyisun0101/Scientific-Workflow/blob/main/docs/setup.md).**
- Upgrading from Rust 0.13.x? Follow the [0.14.0 compute migration guide](docs/migration-0.14.0.md).
- For typed dependency handoff and whole-series analysis, run the
  [initialization → simulation → NPY → analysis example](examples/dependency_pipeline/README.md).

- If Workflow, Serde, or Rust traits are new to you, begin with the
  [getting-started guide](rust/getting-started.md). It defines studies, phases,
  tasks, execution units, and members, then builds one minimal runnable project.
- To use the library, follow the complete [Rust crate guide](rust/README.md).
  It contains installation, architecture and ownership diagrams, the full
  project procedure, execution unit examples, JSON grammar, execution, and validation.
- To see a complete working project, open the
  [two-dimensional attractor example](examples/attractor_2d/README.md), its
  [study manifest](examples/attractor_2d/wf_configs/study.json), and its
  [Hopf model implementing the execution-unit contract](examples/attractor_2d/src/hopf_model.rs).
- To understand subsystem boundaries and dependency direction, read the
  [architecture guide](docs/architecture.md).
- Coding agents should follow the [agent guide](docs/agent-guide.md) to prefer
  strict project JSON and documented APIs over changes to Workflow itself.
- To consume completed recordings from Python, use the verified
  [Scientific Workflow Reader and standard NumPy converter](python/README.md).
- To implement or audit a cross-language reader, use the normative
  [recording v7 protocol](protocol/recording-v7.md),
  [v8 boundary-sampling extension](protocol/recording-v8.md), and
  [compatibility matrix](protocol/compatibility.md).
- To find the tests for a behavior or run the required checks, use the
  [test map](docs/tests.md).
- For the Rust 0.13.9 and Python companion 0.4.4 release summary, see the
  [changelog](CHANGELOG.md).

## Repository map

```text
workflow/
+-- Cargo.toml / Cargo.lock  unified Rust workspace and dependency resolution
+-- rust/                   Rust crate, beginner/complete guides, sources, and tests
+-- macros/                 execution-unit registration procedural macros
+-- python/                 verified recording reader and Python tests
+-- protocol/               normative recording contract and compatibility data
+-- examples/attractor_2d/ complete Rust unit + Python-analysis project
+-- docs/                   architecture and test responsibility guides
+-- .github/workflows/      cross-language CI and contract checks
+-- CHANGELOG.md            coordinated package release notes
`-- README.md               repository navigation
```

## Subsystem contracts

Each first-level Rust subsystem has an exhaustive API and replacement contract:

- [State](rust/src/state/api.md): schemas, static upstream schema providers,
  typed payload ownership, time, and in-memory series.
- [Observation](rust/src/observation/api.md): stream declarations, binding,
  cadence, and encoding boundaries.
- [Task](rust/src/task/api.md): `ExecutionUnit`, immutable initialization/seed
  context, per-member `MemberView`, ensemble contracts, registration, and generic
  program tasks with optional centralized task-seed derivation.
- [Config](rust/src/config/api.md): required study-wide threads, project JSON,
  optional named state paths, inferred global/local parameter expansion,
  reserved `$npy` phase, and program/Python resolution.
- [Study](rust/src/study/api.md): effect-free assembly, preflight, and immutable
  execution intent.
- [Persistence](rust/src/persistence/api.md): automatic recordings, lifecycle,
  format, verified reconstruction, and the repository-level wire protocol.
- [NPY v2](protocol/npy-v2.md): manifest-directed fixed, ragged, structured,
  and fallback NumPy conversion for every recorded field.
- [Runtime](rust/src/runtime/api.md): execution, scheduling, cancellation,
  per-configuration dependency correlation, program environments, and summaries.
- [UI](rust/src/ui/api.md): automatic terminal presentation, live `log.txt`,
  CPU/RAM/disk usage, and exit handling.
- [Error](rust/src/error/api.md): complete-workflow error composition.
- [Prelude](rust/src/prelude/api.md): the ordinary execution-unit authoring
  imports.

> This crate is pre-1.0 test software. Public API behavior may change through
> coordinated refactor releases.

### Optional phase and execution-unit selection

Omitting `active_phases` selects every phase. To select a subset, set
`"active_phases": [0, 1, ...]`. Indices are zero-based in deterministic dependency
order: visit phases in JSON declaration order, recursively visit each `after`
list in its declared order, then assign each phase its index once. Selecting a
subset never renumbers phases, expanded tasks, output ordinals, or seed identities.
The order of indices in the selection does not change execution order. Duplicate,
negative, non-integer, and out-of-range indices are rejected. An empty list
explicitly selects no work.

Within a selected phase, an execution-unit task may set `"active": false`.
Inactive units remain compiled and visible in plan inspection with their stable
identities and output ordinals, but Runtime neither runs nor reuses them. The
field defaults to `true` and is invalid on program, Python, and `$npy` tasks.

For a six-phase preparation, reference, targets, lattice, export, and conversion
study, `"active_phases": [3, 4, 5]` starts at lattice. Supply
`"reuse_from": "output/execution-1234-0"` to reuse completed prerequisites.
The path names one execution directory and is resolved against the project root;
absolute paths are also accepted. Each replicate imports the matching source
replicate. Unselected phases that are not prerequisites are omitted entirely.

Workflow validates the complete graph and constants during Study loading.
Before creating new output, Runtime requires every imported task to have
successfully completed with matching captured inputs. `active_phases` and
`reuse_from` may differ between the captured and current study snapshots.
`$npy.exclude_streams` may also differ when the imported phase is neither
`$npy` nor a direct or transitive consumer of its output. NPY and its consumers
retain exact filter matching. Work-relevant inputs still must match: parameters,
schemas, seeds, programs/scripts, arguments, replicate count, and phase dependencies.
Compute/thread budgets, scheduling, timeouts, failure policy, persistence buffering,
disk policy, and Python environment-manager settings are operational provenance
and do not invalidate completed work. If a resource setting changes scientific
meaning, express that choice in scientific parameters.
A reused phase cannot depend on a phase selected to execute again. Missing,
failed, incompatible, or ambiguous legacy inputs fail without launching work.
Programs and `$npy` receive the original completed recording/artifact paths.
Recordings are never appended to or rewritten.

New executions commit private `workflow-result.json` receipts after each
successful phase, including references for reused tasks, so reuse can be chained.
Pre-0.13.9 program outputs may be imported from their successful `program.json`
and captured config. Legacy execution-unit imports additionally require an
authoritative matching summary in a dependent program's captured dependency
file; Workflow never guesses a final iteration from sampling cadence.


## Run cleanup and timestamps

The ordinary `run(&Path)` facade recognizes `--clean` before an optional `--`
argument delimiter, for example `cargo run -- --clean`. After complete study,
reuse, and Python preflight, it clears `<project-root>/output` before creating
the execution. Other arguments remain application-owned. `runtime::execute`
does not inspect process arguments and does not clean.

All runs hold a shared advisory lock on the project directory; a cleaning run
holds it exclusively until execution and presentation finish. Cleanup rejects
an output-root symlink/file, a conflicting active run, or a target containing
configuration, a resolved executable/script, a selected reuse source, or any
imported task/member/NPY directory. Child symlinks are removed without following
them. Deletion errors abort startup; already removed entries are not restored.
The output directory itself is preserved. Older Workflow versions do not
participate in this project-lock protocol.

Execution directories use `execution-YYYYMMDDTHHMMSS.nnnnnnnnnZ`, UTC, with an
additional numeric suffix only on collision. Atomic directory creation prevents
reuse even if the clock repeats. Names are invocation identities; scientific
task identity is unchanged. Every physical `log.txt` line receives an absolute
RFC 3339 UTC timestamp when appended, including buffered and multiline messages.
Headless builds continue to omit the UI-owned log.


## Disk guard and NPY resource policy

`study.json` accepts optional `"disk": {"pause_at_percent": 95}`. The default
is 95; a finite numeric threshold must be greater than 0 and at most 100. Use
`null` as the threshold to explicitly bypass the guard. Unknown fields and
invalid values are rejected during Config preflight.

Runtime samples the output filesystem before admitting work and every 250 ms
of wall time. At the threshold it requests a disk pause; it automatically clears
that reason when usage reaches `max(0, threshold - 2)` percent. A separate manual
pause remains in force, and manual resume cannot bypass the disk reason. The
shared active clock freezes while either reason holds. Cancellation wakes
paused work. These operations also run in headless builds. Sampling or monitor
startup failure produces `RuntimeError::DiskMonitor { path, source }`; an active
monitor failure cancels work and is returned after joining workers. The guard
is stopped before the terminal's final user-input wait.

Execution units pause between host calls; NPY/Python cooperative tasks acknowledge
through control IPC. For non-cooperative Unix programs, disk pause sends SIGSTOP
to the owned process group and recovery sends SIGCONT. Cleanup resumes a stopped
group before terminating it. Disk enforcement is sampled and cooperative calls
may take time to return, so the threshold is a pause trigger, not reserved free
space or a guarantee that in-flight writes cannot fill the filesystem.

The `$npy` phase accepts `"threads": 4` and `"mode": "auto"` (or `"fixed"`).
Threads default to `study.threads` and must be a positive integer no larger than
that global limit. Mode defaults to `fixed`; these fields are invalid on ordinary
phases. Runtime reserves `min(phase.threads, study.threads, recording_count)`
permits through the existing shared budget, including across replicates. Each
conversion worker limits native numerical pools to one thread. Fixed mode admits
workers up to that allowance immediately; auto begins at one and increases the
admission limit by one every 250 ms of active time until the allowance is reached.
Short batches may finish before reaching the limit. This does not measure CPU
or RAM utilization, and the full allowance remains reserved during ramp-up.

Python's `convert_workflow_dependencies` adds the optional keyword
`worker_mode="fixed"`; `"auto"` selects gradual admission. Other values raise
`NpyConversionError` before output creation. The Workflow CLI accepts
`--worker-mode=fixed|auto` with `--workflow-dependencies`; ordinary single-recording
conversion rejects it. Mode does not change result or reuse identity.


## Thread display and task pages

Runtime publishes a complete snapshot of active task allocations, serialized
across replicate schedulers and sampled at most every 50 ms, with immediate
publication after activation and at phase/execution completion. Internal pools
report their actual current Rayon pool sizes, including automatic rebalancing;
external programs and NPY report their reserved compute allowance. One private
working-task registry ties display entries to resource-lease lifetime.

The task table includes a `threads` column. Usage shows `THREADS allocated/budget`
across all running tasks, including tasks on other pages and in other replicates.
These are allocated compute threads, not measured CPU activity or every OS
thread. Paused tasks retain their allocation; pending and completed tasks count
as zero. NPY auto ramp-up reports the reserved allowance. The private
`ThreadAllocations { allocations, budget }` event carries the whole allocation
set so UI never combines partial rebalancing updates into an inflated total.

PageUp/PageDown move by the task table's visible data-row capacity, excluding
borders and its header. The footer no longer replaces a task row. The title
shows the current page and page count. Resizing recalculates capacity and clamps
to a valid page; an existing first-row anchor is retained where possible, and a
disappearing active-group anchor resets to the first page. All rows on a full
page are usable, and the final page may contain fewer rows.
