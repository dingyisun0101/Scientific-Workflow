# Two-dimensional attractor workflow

This is the compact end-to-end example for `scientific-workflow`. It keeps the
scientific model small while demonstrating the intended public procedure:

```text
configuration -> generated tasks -> phases -> Study
```

The study evaluates the Cartesian sweep
`mu = [-0.25, 0.25, 1.0] × angular_frequency = [0.75, 1.25]` for the
supercritical Hopf model:

```text
dx/dt = mu*x - omega*y - (x² + y²)*x
dy/dt = omega*x + mu*y - (x² + y²)*y
```

## Source layout

```text
config/
├── fixed.json       model, sampling, and storage values
├── sweep.json       the `mu × angular_frequency` sweep
├── paths.json       recording root
└── state.json       `point` and `radius` fields
src/
├── main.rs           phases and study
├── task_execution.rs model assembly for one generated task
├── hopf_model.rs     state owner and Euler step
├── recording.rs      task-owned writer and evolution loop
├── validation.rs     dependent checkpoint verification
└── rendering.rs      phase-3 Python process invocation
scripts/
└── render_trajectories.py
                      trajectory plotting with the official reader
```

No application-specific configuration struct or task registry exists.
`ResolvedConfiguration::decode_values` decodes heterogeneous parameter groups directly
from fixed and swept JSON values. The concise phase helpers apply one shared
callable to every generated task; advanced per-task `FnOnce` factories remain
available when a task must own a unique non-Clone resource.

## Study flow

Phase 1 generates one progress task per resolved `(mu, angular_frequency)`
combination. Each task owns its `HopfModel`,
`SystemStateWriter`, and recording directory. Three streams demonstrate
independent sampling:

| Stream | Fields | Interval |
|---|---|---:|
| `trajectory` | `point` | 10 iterations |
| `radius` | `radius` | 5 iterations |
| `checkpoint` | `point`, `radius` | 1000 iterations |

Phase 2 depends on phase 1. Its tasks derive the corresponding recording path
from the same deterministic task ordinal, read the latest checkpoint, and
verify its final iteration and `radius == hypot(point)`. Durable recordings are
the phase handoff; Workflow does not transport application data between phases.

Phase 3 depends on phase 2 and contains one ordinary one-shot task. It invokes
`scripts/render_trajectories.py` through `mamba run -n DSES`, reads every
verified trajectory using `scientific_workflow_reader`, and writes one PNG per
configuration to the execution's `outputs/` directory. The study scheduler
does not know that this task launches Python or creates images.

Both phases include an explicit three-second entrance pause so their refreshed
headers remain readable. Every model step also has a required one-millisecond
pause so task progress can be watched instead of completing instantaneously.
The per-step pause is a permanent teaching requirement of this example and must
not be removed or optimized away. Both delays belong to the example, not the
study API or the numerical method.

## Run

This example requires the maintained `DSES` Mamba environment because the
final phase uses Matplotlib. Ensure that `mamba` is on `PATH`, then run from the
Workflow repository root with the appropriate environment:

```bash
mamba run -n DSES cargo run --manifest-path examples/attractor_2d/Cargo.toml
```

The rendering task also invokes Python explicitly through
`mamba run -n DSES`; this keeps the subprocess environment unambiguous even
when the Rust binary is launched another way.

Recordings remain under the ignored directory:

```text
examples/attractor_2d/target/recordings/
```

Each generated execution contains the rendered images alongside its study
record and task recordings:

```text
execution-.../
├── task-000000/
├── ...
├── study-record.json
└── outputs/
    ├── trajectory-000000.png
    └── ...
```

A successful run ends with:

```text
[study] status=completed phases=3 tasks=13
```
