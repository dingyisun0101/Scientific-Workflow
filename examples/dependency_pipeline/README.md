# Initialization, simulation, and Python analysis

**Linux only. Python 3.14+ and an activated environment containing the compatible
`scientific-workflow[npy]` package are REQUIRED. Cargo does not install Python.**

The coordinated patch release is Rust 0.16.1 / Python 0.6.1. Rust 0.16.1
accepts stable companions `>=0.6,<0.7`; this example's exact online pins are maintained
in its Cargo manifest and `examples/requirements.txt` after publication.

From the repository root, install the published companion with
`python -m pip install -r examples/requirements.txt`, then run inside
`tmux new -s workflow` or `screen -S workflow`:

```sh
cargo run -p workflow-dependency-pipeline
```

The project uses the required layout:

```text
wf_configs/
  study.json
  parameters.json
  states/value.json
scripts/analyze.py
src/main.rs
```

The `initialize` unit records its already-complete value once using
`initial_and_final()`. That recording uses format 8. The simulation selects this
recording through typed dependencies, decodes `u64` with `with_json_field`, moves
its payload into a new state, and advances five steps. Its periodic recording
uses format 7. The standard `$npy` task receives both transitive recordings and
converts them with gradual admission of up to two shared-budget workers. NPY
uses format 3.

Simulation initialization rejects a checkpoint value whose sum with the
requested step count exceeds `u64`. Each step checks both the next count and
iteration before assigning either, so overflow returns an error with the
previous state intact in debug and release builds.

Python locates the aggregate batch using `Dependencies.from_env()`, selects
simulation members using `execution_unit`, and reads a cached whole-series view.
The summary written into the analysis task's `artifacts/summary.json` contains
values `[7, 8, 9, 10, 11, 12]` at iterations `[0, 1, 2, 3, 4, 5]`.

If several phases produce the same unit/member, `.one()` reports all matching
sources. Add `.in_phase("initialize")` or `.task(identity)` to select the intended
source. Use `.iter()` when all matches are intentional; `.optional()` still
rejects ambiguity.

Do not rename or relocate required files. External programs use the resolved
runtime snapshots supplied by Workflow; reading raw source parameters bypasses
sweep/override resolution. Domain adapters may wrap numeric series, but should
retain the verified conversion object rather than repeatedly reopening it.


The checked-in study explicitly selects `compute.mode = "isolated"` and assigns
one thread to each execution-unit task within the global two-thread budget.
The example consumes published Workflow 0.16.0 and the Python 0.6.0 companion
pinned in `examples/requirements.txt`. Execution requires the dashboard; run
inside screen/tmux. Disk pauses require freeing space and typing `resume`.
Leave NPY worker settings unset for the default gradual auto allocation unless
a lower limit is needed to reserve resources for other work.
