# Scientific Workflow

Scientific Workflow turns registered Rust scientific models, arbitrary
executable programs, and declarative JSON into validated, recorded studies.

> **Breaking 0.10 update:** Version 0.10.2 is the current patch release of the
> 0.10 API generation that intentionally replaced the pre-0.10 orchestration,
> configuration, storage/writer, and study APIs. Projects using 0.9.x or
> earlier must migrate to the model registration plus `wf_configs/study.json`
> named-state map and `wf_configs/parameters.json` workflow. A root-level
> `study.json` and the former `config/` directory are not compatibility aliases.
> Legacy Rust entry points and legacy JSON fields are likewise not accepted.

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
- To find the tests for a behavior or run the required checks, use the
  [test map](docs/tests.md).
- Before contributing changes, read the repository [agent and edit
  instructions](AGENTS.md).

## Repository map

```text
workflow/
+-- rust/                   Rust crate, crate guide, sources, and Rust tests
+-- python/                 verified recording reader and Python tests
+-- examples/attractor_2d/ complete Rust-model + Python-analysis project
+-- docs/                   architecture and test responsibility maps
+-- AGENTS.md               repository editing and documentation rules
`-- README.md               repository navigation
```

## Subsystem contracts

Each first-level Rust subsystem has an exhaustive API and replacement contract:

- [State](rust/src/state/api.md): schemas, typed payload ownership, time, and
  in-memory series.
- [Observation](rust/src/observation/api.md): stream declarations, binding,
  cadence, and encoding boundaries.
- [Task](rust/src/task/api.md): `ScientificModel`, registration, model
  contracts, and generic program tasks.
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
