# Scientific Workflow

> This crate is test software. Its API and guarantees may change before 1.0.

Scientific Workflow provides typed state, configuration-driven scientific
tasks, observation definitions, durable recording infrastructure, replicate
isolation, and reproducibility support for Rust applications.

The target user workflow is deliberately small:

1. define scientific state and its writer;
2. define typed task behavior; and
3. write `study.json`, `config/state.json`, and task input documents beneath
   `config/inputs/`.

Workflow will infer identities, paths, task instances, scheduling, progress,
messages, provenance, and recording lifecycle from those declarations.

## Project declarations

There is one project-declaration subsystem: `config`.

```text
<project-root>/
├── study.json                  study manifest
└── config/
    ├── state.json              state schema document
    └── inputs/*.json           task input documents
```

`config` centrally parses all three document kinds, expands `$sweep` and
`$cases` internally, and supplies one complete typed constants value to each
task invocation. The former `configuration` module and its manual combination
API have been removed.

The implemented target boundaries are currently `state`, `writer`, `task`, and
`config`. Existing `study`, `execution`, and `storage` code remains
transitional while the next passes introduce `runtime`, `record`, and `ui`.

The Rust crate lives in [`rust/`](rust/). Read its
[public overview](rust/README.md), the complete
[target architecture](docs/architecture.md), and the
[test map](docs/tests.md).

## Attractor example

[`examples/attractor_2d`](examples/attractor_2d) demonstrates the current
migration boundary. It loads one project root through `ProjectSpecification`,
uses resolved task inputs and typed constants, reuses the centrally parsed
state schema, and passes the manifest's replicate policy to the current
execution adapter. It still maps those declarations into transitional study
phases until runtime owns that composition.

```bash
mamba run -n DSES cargo run --manifest-path examples/attractor_2d/Cargo.toml
```

The final phase uses Matplotlib from the maintained `DSES` Mamba environment.

## Validation

```bash
cargo test --all-targets --manifest-path rust/Cargo.toml
```

The suite covers centralized project parsing and expansion, typed task
execution, state ownership, writer inference, study scheduling, replicate
dispatch, storage recovery, artifact integrity, RNG provenance, and Rust/Python
format conformance.
