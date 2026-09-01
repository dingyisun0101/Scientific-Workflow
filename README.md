# Scientific Workflow

Scientific Workflow turns registered Rust scientific execution units, arbitrary
executable programs, and declarative JSON into validated, recorded studies.

> **Breaking update — 0.13.0:** This release supersedes Workflow 0.12.x
> parameter expansion. Top-level `$sweep` values outside execution-unit
> sections now expand the complete study graph; no compatibility alias retains
> their former local interpretation. Projects still declare
> `"workflow_schema": 1`.

## Start here

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
- To consume completed recordings from Python, use the verified
  [Scientific Workflow Reader and standard NumPy converter](python/README.md).
- To implement or audit a cross-language reader, use the normative
  [recording v7 protocol](protocol/recording-v7.md) and
  [compatibility matrix](protocol/compatibility.md).
- To find the tests for a behavior or run the required checks, use the
  [test map](docs/tests.md).
- For the Rust 0.13.2 and Python reader 0.4.1 release summary, see the
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
- [Runtime](rust/src/runtime/api.md): execution, scheduling, cancellation,
  per-configuration dependency correlation, program environments, and summaries.
- [UI](rust/src/ui/api.md): automatic terminal presentation and exit handling.
- [Error](rust/src/error/api.md): complete-workflow error composition.
- [Prelude](rust/src/prelude/api.md): the ordinary execution-unit authoring
  imports.

> This crate is pre-1.0 test software. Public API behavior may change through
> coordinated refactor releases.
