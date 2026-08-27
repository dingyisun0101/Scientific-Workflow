# Two-dimensional attractor study

This is a complete small scientific project rather than a collection of API
fragments. Rust owns the stateful Hopf model, JSON owns the study and all
configuration, and a final Python task turns the completed recordings into an
SVG figure.

```text
attractor_2d/
├── Cargo.toml
├── src/
│   ├── main.rs                 one scientific_workflow::run(&Path) call
│   └── hopf_model.rs           registered state-owning scientific model
├── study.json                  simulate phase followed by plot phase
├── config/
│   ├── state.json              canonical scientific state schema
│   ├── inputs/run.json         model constants and parameter sweeps
│   └── plot.json               arbitrary Python plotting configuration
└── scripts/
    └── plot.py                 directly declared Python task
```

## Scientific workload

`HopfModel` directly owns its canonical `SystemState`. It initializes the
two-dimensional `point` and derived `radius`, advances both values and physical
time in `step`, and reports completion through its configured iteration count.
Its custom `ObservationPlan` records:

- the phase-space trajectory every 10 iterations;
- radius every 5 iterations; and
- a combined checkpoint every 1,000 iterations.

`config/inputs/run.json` sweeps three growth parameters and two angular
frequencies. Config expands their Cartesian product into six model tasks;
Runtime executes up to three concurrently and automatically records every
observation stream.

## Python plot phase

The `plot` phase depends on `simulate` and declares `scripts/plot.py` directly:

```json
{
  "python": {
    "script": "scripts/plot.py",
    "environment": {"manager": "system"}
  }
}
```

No Rust closure or caller wraps Python. Workflow resolves `python3` during
Study loading, captures stdout/stderr, and supplies the central configuration,
completed simulation outputs, and isolated artifact directory through the
standard `WORKFLOW_*` contract.

The plotter retrieves its visual settings from `config/plot.json`, opens every
dependency recording with the official verified
`scientific_workflow_reader`, and produces:

```text
<plot-task-output>/artifacts/
├── attractor-sweep.svg
└── plot-summary.json
```

An installed `scientific-workflow-reader` is used normally. When this example
runs from the repository checkout, the script falls back to the adjacent
reader source tree so a clean checkout needs only Python 3.10 or newer.

## Run

From the repository root:

```bash
cargo run --manifest-path examples/attractor_2d/Cargo.toml
```

Workflow creates a unique execution beneath `examples/attractor_2d/output`.
The terminal UI appears automatically only on interactive standard error. The
model and plotter do not construct tasks, phases, output paths, persistence
sessions, progress counters, or message channels.

The omitted replicate, timeout, UI, and persistence settings use Workflow's
validated defaults. Only the scientifically meaningful observation cadence,
parameter sweep, plotting choices, and one justified concurrency bound are
authored.
