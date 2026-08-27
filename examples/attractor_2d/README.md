# Two-dimensional attractor example

This example demonstrates the complete low-burden workflow:

```text
src/hopf_model.rs
    registered HopfModel + typed constants + custom ObservationPlan
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
its configured step count, and returns a custom three-stream observation plan.
Workflow creates one internal task for each `mu × angular_frequency` sweep
combination. The omitted `persistence` object uses inferred local defaults;
Runtime privately constructs and finalizes each task backend.

The application does not parse JSON, construct tasks/phases, assign IDs or
paths, manage threads, report progress, or open persistence writers. Completed
recordings are created beneath this project's inferred `output` directory.
When run interactively, Workflow automatically displays inferred task and
iteration progress on standard error; no example-specific UI code or settings
exist.

Run it with:

```bash
cargo run --manifest-path examples/attractor_2d/Cargo.toml
```

The model performs no display, progress reporting, or artificial delay;
Runtime infers execution progress from its normal state boundaries.
