# Scientific Workflow

Scientific Workflow turns registered Rust scientific models, arbitrary
executable programs, and declarative JSON into validated, recorded studies.

> **Breaking 0.11 update:** Version 0.11.0 replaces the 0.10
> `ScientificModel` task boundary with cardinality-aware `ExecutionUnit` and
> `ModelView` APIs. A unit may expose one standalone model or a coordinated
> ensemble, while every model owns its own state and recording. There is no
> `ScientificModel` compatibility trait; `#[model]` remains only as a spelling
> alias for the preferred `#[execution_unit]` registration attribute. The
> required `wf_configs/` project layout introduced in 0.10 is unchanged.

## Start here

- To use the library, follow the complete [Rust crate guide](rust/README.md).
  It contains installation, architecture and ownership diagrams, the full
  project procedure, model examples, JSON grammar, execution, and validation.
- To see a complete working project, open the
  [two-dimensional attractor example](examples/attractor_2d/README.md), its
  [study manifest](examples/attractor_2d/wf_configs/study.json), and its
  [registered model](examples/attractor_2d/src/hopf_model.rs).
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
+-- examples/attractor_2d/ complete Rust-model + Python-analysis project
+-- docs/                   architecture and test responsibility maps
`-- README.md               repository navigation
```

## Subsystem contracts

Each first-level Rust subsystem has an exhaustive API and replacement contract:

- [State](rust/src/state/api.md): schemas, typed payload ownership, time, and
  in-memory series.
- [Observation](rust/src/observation/api.md): stream declarations, binding,
  cadence, and encoding boundaries.
- [Task](rust/src/task/api.md): `ExecutionUnit`, per-model `ModelView`,
  ensemble contracts, registration, and generic program tasks.
- [Config](rust/src/config/api.md): project JSON, named state paths, parameter
  expansion, and program/Python resolution.
- [Study](rust/src/study/api.md): effect-free assembly, preflight, and immutable
  execution intent.
- [Persistence](rust/src/persistence/api.md): automatic recordings, lifecycle,
  format, and verified reconstruction.
- [Runtime](rust/src/runtime/api.md): execution, scheduling, cancellation,
  program environments, and summaries.
- [UI](rust/src/ui/api.md): automatic terminal presentation and exit handling.
- [Error](rust/src/error/api.md): complete-workflow error composition.
- [Prelude](rust/src/prelude/api.md): canonical Basic and Advanced API
  aggregation.

> This crate is pre-1.0 test software. Public API behavior may change through
> coordinated refactor releases.
