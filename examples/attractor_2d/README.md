# Two-dimensional attractor example

This example demonstrates the complete low-burden workflow:

```text
src/hopf_model.rs
    registered HopfModel + typed constants + custom Writer
study.json
    phase membership, concurrency, replicate policy, and model/input reference
config/state.json
    canonical state fields
config/inputs/run.json
    constants and parameter sweeps
src/main.rs
    one scientific_workflow::run(&Path) call
```

`HopfModel` directly owns its `SystemState`, initializes `point` and `radius`,
advances both fields and scientific time in `step`, declares completion through
its configured step count, and returns a custom three-stream writer. Workflow
creates one internal task for each `mu × angular_frequency` sweep combination.

The application does not parse JSON, construct tasks/phases, assign IDs or
paths, manage threads, report progress, or open recording writers. Completed
recordings are created beneath this project's inferred `output` directory.

Run it with:

```bash
cargo run --manifest-path examples/attractor_2d/Cargo.toml
```

The short sleep inside each model step is intentional teaching behavior so
concurrent evolution remains observable during development.
