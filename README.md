# Scientific Workflow

> **BREAKING COMPUTE UPDATE: Rust 0.14.1 / Python 0.4.5 unchanged.**
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
retain exact filter matching; all other captured inputs still must match.
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
