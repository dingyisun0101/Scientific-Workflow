# Two-dimensional attractor workflow

This example exercises the current end-to-end migration boundary:

```text
project root
  → ProjectSpecification
  → resolved task inputs
  → typed AttractorConstants
  → transitional study phases
  → recordings and rendered trajectories
```

The model is the supercritical Hopf normal form:

```text
dx/dt = mu*x - omega*y - (x² + y²)*x
dy/dt = omega*x + mu*y - (x² + y²)*y
```

Its task input expands
`mu = [-0.25, 0.25, 1.0] × angular_frequency = [0.75, 1.25]` into six complete
typed constants values. Application code never calls a combination iterator or
decodes individual JSON keys.

## Source layout

```text
study.json              replicate, phase, task, and display declarations
config/
├── state.json          `point` and `radius` state fields
└── inputs/
    ├── run.json        model, writer, and sweep constants
    └── render.json     empty typed input for the render declaration
src/
├── main.rs             project loading and temporary study composition
├── attractor_run.rs    typed constants and producer descriptor
├── task_execution.rs   model initialization for one resolved input
├── hopf_model.rs       scientific state owner and Euler step
├── recording.rs        task-owned transitional recording loop
├── validation.rs       dependent checkpoint verification
└── rendering.rs        phase-three Python process invocation
scripts/
└── render_trajectories.py
                         plotting through the official Python reader
```

`ProjectSpecification` centrally reads the study manifest, state schema, and
task inputs. The example obtains its state schema through the parsed-value
state integration seam and passes the manifest's validated `ReplicatePolicy`
to the current `ReplicateExecutor`. The output root is inferred from the
project root as `target/output`; there is no named-path configuration file.

The example still aliases `study::Task` as `StudyTask`. That type and the manual
phase construction are transitional: the runtime pass will replace this
mapping with the single project-root run entry point and automatic task
identity, scheduling, display, provenance, and recording lifecycle.

## Study flow

The simulation phase produces one recording for every resolved run input.
Each recording contains three independent streams:

| Stream | Fields | Interval |
|---|---|---:|
| `trajectory` | `point` | 10 iterations |
| `radius` | `radius` | 5 iterations |
| `checkpoint` | `point`, `radius` | 1000 iterations |

The validation phase pairs each typed validation input with the producer having
the same `mu` and angular frequency. It reads only the latest completed
checkpoint and verifies its final iteration and
`radius == hypot(point)`. Durable recordings are the phase handoff.

The final phase invokes `scripts/render_trajectories.py` through
`mamba run -n DSES`, reads every verified trajectory using
`scientific_workflow_reader`, and writes one PNG per run.

The simulation and validation phases retain an explicit three-second admission
interval so their terminal headers remain readable. Each model step also
retains a required one-millisecond pause so progress is visible. These delays
are teaching requirements of the example, not scientific integration rules.

## Run

Ensure `mamba` is on `PATH`, then run from the Workflow repository root:

```bash
mamba run -n DSES cargo run --manifest-path examples/attractor_2d/Cargo.toml
```

Output is isolated beneath:

```text
examples/attractor_2d/target/output/replicate_0/
├── study-record.json
├── attractor-000000/
│   └── ... recording metadata and chunks
├── ...
└── plots/
    ├── trajectory-000000.png
    └── ...
```

Replicate directories are exclusive. Archive or remove `replicate_0` before a
fresh run; Workflow never overwrites an existing replicate implicitly.
