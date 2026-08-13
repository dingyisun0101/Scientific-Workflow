# Two-dimensional attractor workflow

This is the compact end-to-end example for `scientific-workflow`. It keeps the
scientific model small while demonstrating the intended public procedure:

```text
configuration -> generated tasks -> phases -> WorkflowRuntime
```

The project sweeps `mu = [-0.25, 0.25, 1.0]` for the supercritical Hopf model:

```text
dx/dt = mu*x - omega*y - (x² + y²)*x
dy/dt = omega*x + mu*y - (x² + y²)*y
```

## Source layout

```text
config/
├── fixed.json       model, sampling, and storage values
├── sweep.json       the `mu` sweep
├── paths.json       recording root
└── state.json       `point` and `radius` fields
src/
├── main.rs           phases and runtime
├── task_execution.rs model assembly for one generated task
├── hopf_model.rs     state owner and Euler step
├── recording.rs      task-owned writer and evolution loop
└── validation.rs     dependent checkpoint verification
```

No application-specific configuration struct or task registry exists.
`TaskConfig::decode_values` decodes heterogeneous parameter groups directly
from fixed and swept JSON values. The concise phase helpers apply one shared
callable to every generated task; advanced per-task `FnOnce` factories remain
available when a task must own a unique non-Clone resource.

## Runtime flow

Phase 1 generates one progress task per `mu`. Each task owns its `HopfModel`,
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

Both phases include an explicit three-second entrance pause so their refreshed
headers remain readable. Every model step also has a required one-millisecond
pause so task progress can be watched instead of completing instantaneously.
The per-step pause is a permanent teaching requirement of this example and must
not be removed or optimized away. Both delays belong to the example, not the
runtime API or the numerical method.

## Run

From the Workflow repository root:

```bash
cargo run --manifest-path examples/attractor_2d/Cargo.toml
```

Recordings remain under the ignored directory:

```text
examples/attractor_2d/target/recordings/
```

A successful run ends with:

```text
[runtime] status=completed phases=2 tasks=6
```
