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
│   └── parameters.json         every model and plotting parameter
└── scripts/
    └── plot.py                 directly declared Python task
```

## Scientific workload

`HopfModel` directly owns its canonical `SystemState`. It initializes the
two-dimensional `point` and derived `radius`, advances both values and physical
time in `step`, and reports completion through its configured iteration count.
For presentation only, `artificial_step_delay_ms` sleeps after every successful
step so the automatic dashboard remains visible long enough to inspect. This
wall-clock delay is not scientific time and should be set to `0` for an
unpaced calculation.

Its custom `ObservationPlan` records:

- the phase-space trajectory every 10 iterations;
- radius every 5 iterations; and
- a combined checkpoint every 1,000 iterations.

The `attractor` section of `config/parameters.json` sweeps three growth
parameters and two angular frequencies. Config selects that section from the
registered model key and expands its Cartesian product into six model tasks;
Runtime executes up to three concurrently and automatically records every
observation stream. The phase's `start_interval_ms: 2000` setting waits two
seconds between successive task admissions (the first eligible task starts
immediately). This phase-owned scheduling delay is separate from the model's
one-millisecond per-step presentation delay. With the supplied workload, a
normal run takes roughly 16 seconds, subject to machine and IO overhead.

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

The plotter retrieves its visual settings from the `plot` section of
`config/parameters.json`, opens every dependency recording with the official
verified `scientific_workflow_reader`, and produces:

```text
output/plots/
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

Workflow creates a unique Rust execution beneath `examples/attractor_2d/output`.
Python owns its configured `output/plots` destination directly. When stdin and
stderr are interactive, the automatic Ratatui dashboard shows task rows,
progress, timing, messages, and an `exit` command. The task section refreshes
for each phase and shows that phase only; redirected runs use plain lifecycle
lines. The model and plotter do not construct tasks, phases,
persistence sessions, progress counters, or message channels.

The omitted replicate, timeout, UI, and persistence settings use Workflow's
validated defaults. The project authors scientifically meaningful observation
cadence, parameter sweep, plotting choices, one justified concurrency bound,
the two-second inter-task admission delay, and the explicitly non-scientific
per-step demonstration delay.
