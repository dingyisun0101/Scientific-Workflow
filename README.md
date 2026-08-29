# Scientific Workflow

Scientific Workflow turns registered Rust scientific execution units, arbitrary
executable programs, and declarative JSON into validated, recorded studies.

> **Breaking update — 0.11.8:** every `wf_configs/study.json` must now declare
> a positive top-level `threads` value. Workflow creates and enforces one
> shared study-wide compute pool with that exact worker count and propagates it
> to external tasks. Add, for example, `"threads": 16` beside `phases` before
> upgrading; there is no inferred or environment-controlled fallback.

> **Breaking update — 0.11.3:** this release supersedes the 0.11.0 public API
> generation. It replaces Basic/Advanced namespace tiers with one ordinary
> prelude and module-root specialized APIs, uses data-bearing runtime workload
> summaries, makes state inspection/maintenance inherent, and names the public
> execution-unit error boundary `UnitResult` instead of the scheduler-oriented
> `TaskResult`. No compatibility aliases are provided. Do not use 0.11.1: its
> published macro dependency can expand to the removed registration API;
> 0.11.3 requires the corrected macro.

## Start here

- To use the library, follow the complete [Rust crate guide](rust/README.md).
  It contains installation, architecture and ownership diagrams, the full
  project procedure, execution unit examples, JSON grammar, execution, and validation.
- To see a complete working project, open the
  [two-dimensional attractor example](examples/attractor_2d/README.md), its
  [study manifest](examples/attractor_2d/wf_configs/study.json), and its
  [Hopf model implementing the execution-unit contract](examples/attractor_2d/src/hopf_model.rs).
- To understand subsystem boundaries and dependency direction, read the
  [architecture guide](docs/architecture.md).
- To consume completed recordings from Python, use the verified
  [Scientific Workflow Reader](python/README.md).
- To find the tests for a behavior or run the required checks, use the
  [test map](docs/tests.md).

## Repository map

```text
workflow/
+-- rust/                   Rust crate, crate guide, sources, and Rust tests
+-- macros/                 execution-unit registration procedural macros
+-- python/                 verified recording reader and Python tests
+-- examples/attractor_2d/ complete Rust unit + Python-analysis project
+-- docs/                   architecture and test responsibility maps
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
  optional named state paths, parameter expansion, and program/Python resolution.
- [Study](rust/src/study/api.md): effect-free assembly, preflight, and immutable
  execution intent.
- [Persistence](rust/src/persistence/api.md): automatic recordings, lifecycle,
  format, and verified reconstruction.
- [Runtime](rust/src/runtime/api.md): execution, scheduling, cancellation,
  program environments, and summaries.
- [UI](rust/src/ui/api.md): automatic terminal presentation and exit handling.
- [Error](rust/src/error/api.md): complete-workflow error composition.
- [Prelude](rust/src/prelude/api.md): the ordinary execution-unit authoring
  imports.

> This crate is pre-1.0 test software. Public API behavior may change through
> coordinated refactor releases.
