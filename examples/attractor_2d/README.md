# Two-dimensional attractor workflow

`attractor_2d` is a small scientific project demonstrating how to organize a
state-evolving parameter study with `scientific-workflow`.

The example uses a two-variable ordinary differential equation and explicit
Euler integration. Its purpose is to make configuration, state ownership,
sampling, bounded asynchronous writing, and typed round-trip validation visible
without hiding them behind a simulation framework.

> **Implementation status:** The full configuration-to-validation workflow is
> implemented, verified, and runnable.

## Suggested reading order

Read the example from scientific inputs toward orchestration. This makes each
library abstraction answer a question already raised by the preceding file:

1. [`config/fixed.json`](config/fixed.json),
   [`config/sweep.json`](config/sweep.json), and
   [`config/paths.json`](config/paths.json): see which values are shared, which
   create independent tasks, and where output belongs.
2. [`config/state.json`](config/state.json): see the stable names of the values
   that evolve. The template intentionally does not prescribe Rust types.
3. [`src/main.rs`](src/main.rs): follow the complete application flow:
   project setup, task orchestration, and delegation to focused modules.
4. [`src/task_execution.rs`](src/task_execution.rs): inspect per-task setup,
   progress tracking, and delegated recording/validation calls.
5. [`src/recording.rs`](src/recording.rs): inspect cadence, stream, and
   writer setup.
6. [`src/validation.rs`](src/validation.rs): inspect checkpoint round-trip checks.
7. [`src/cross_check.rs`](src/cross_check.rs): numerical correctness check
   correctness check.
8. [`src/hopf_model.rs`](src/hopf_model.rs): see the scientific core that
   owns the evolving `SystemState` directly.

Use [`steps.md`](steps.md) afterward when adapting this pattern to another
scientific project. It explains the general construction sequence; this README
explains this particular implementation.

## Scientific model

The system is the supercritical Hopf normal form:

```text
dx/dt = mu * x - omega * y - (x^2 + y^2) * x
dy/dt = omega * x + mu * y - (x^2 + y^2) * y
```

Writing `r = sqrt(x^2 + y^2)` gives:

```text
dr/dt     = mu * r - r^3
dtheta/dt = omega
```

The swept parameter `mu` controls the long-term attractor:

- `mu < 0`: the origin is stable and radius decays toward zero;
- `mu > 0`: the origin is unstable and the trajectory approaches a stable
  limit cycle of radius `sqrt(mu)`.

The example therefore has rich but easy-to-understand dynamics without needing
visualization in the minimum runtime demonstration.

## Project layout

```text
attractor_2d/
├── Cargo.toml             # Standalone application manifest
├── Cargo.lock             # Reproducible application resolution
├── README.md
├── design.md              # Example-specific architecture
├── steps.md               # General scientific-project tutorial
├── config/
│   ├── fixed.json          # Values shared by every task
│   ├── sweep.json          # Parameter-space definition
│   ├── paths.json          # Named project-relative paths
│   └── state.json          # State field names and descriptions
└── src/
    ├── main.rs                # orchestration: load project + run per-task work
    ├── task_execution.rs      # per-task model execution and task lifecycle
    ├── recording.rs           # writer config, sampling intervals, and output
    ├── validation.rs          # typed checkpoint replay and equality checks
    ├── cross_check.rs         # compulsory numerical correctness check
    └── hopf_model.rs          # scientific core and state ownership
```

The standalone crate root is also the scientific project root. The application
calls `ScientificProject::load` with the `attractor_2d` directory itself; no
nested project wrapper is needed.

`ScientificProject::task_configs()` lazily generates all three swept tasks as
cheap owned handles over shared fixed, sweep, and path data. The main loop can
therefore pass each complete configuration through model assembly and recording
without separately carrying `TaskParameters` and `ProjectPaths`.

The iterator is connected directly to Rayon's work-stealing pool with
`par_bridge()`. Tasks execute concurrently without first collecting the sweep,
while each worker owns an independent model and recording writer. The schema
and execution scope are shared immutably.

For a reusable explanation of this organization, see [steps.md](steps.md).

## Configuration

### Fixed values

[`config/fixed.json`](config/fixed.json) contains values shared by all tasks:

| Setting | Value | Meaning |
|---|---:|---|
| `model_name` | `supercritical_hopf_normal_form` | Stable model identifier |
| `initial_point` | `[0.25, 0.0]` | Initial `[x, y]` coordinates |
| `angular_frequency` | `1.0` | `omega` in the ODE |
| `physical_time_increment_per_step` | `0.01` | Physical-time increment applied by one Euler step |
| `step_count` | `5000` | Number of model-evolution actions per task |
| `trajectory_sampling_interval` | `{"iterations":10}` | Point sampling interval on the iteration coordinate |
| `radius_sampling_interval` | `{"iterations":5}` | Radius sampling interval on the iteration coordinate |
| `checkpoint_sampling_interval` | `{"iterations":1000}` | Complete-state sampling interval on the iteration coordinate |
| `maximum_chunk_bytes` | `8192` | Per-stream chunk rollover target |
| `writer_queue_bytes` | `65536` | Bounded asynchronous queue budget |

Each task reaches physical time `50.0`.

### Parameter sweep

[`config/sweep.json`](config/sweep.json) defines one Cartesian axis:

```text
mu = [-0.25, 0.25, 1.0]
```

This produces three deterministic tasks:

| Task | `mu` | Expected attractor |
|---:|---:|---|
| 0 | `-0.25` | Stable origin |
| 1 | `0.25` | Limit cycle with radius near `0.5` |
| 2 | `1.0` | Limit cycle with radius near `1.0` |

### Named paths

[`config/paths.json`](config/paths.json) declares:

```text
recording_root -> target/recordings
```

`config/state.json` is loaded automatically by `ScientificProject`; it is
structural configuration rather than a named runtime path. The recording path
is resolved relative to this project root. Generated recordings
remain beneath the ignored Cargo `target` directory and do not dirty tracked
source files.

## Runtime state

[`config/state.json`](config/state.json) declares two payload keys:

| Key | Rust payload selected by the application | Responsibility |
|---|---|---|
| `point` | `Vec<f64>` | Evolving coordinates ordered as `[x, y]` |
| `radius` | `f64` | Synchronized diagnostic `sqrt(x^2 + y^2)` |

The JSON template intentionally does not contain Rust type names. Concrete
types become retained slot contracts when application code first inserts each
payload.

Every task directly owns one `SystemState`. It also carries built-in
`SimulationTime`, containing the authoritative iteration and physical time.
Parameters, paths, sampling intervals, and writer limits remain outside the evolving
state.

`Vec<f64>` is used instead of `ndarray` or a PiP tensor because this model has
only two coordinates. It keeps each JSON payload lean and matches the crate's
built-in vector decoder. PiP tensors remain the preferred choice when a real
scientific payload has meaningful rank, shape, or tensor operations.

## Evolution step

Each explicit-Euler step will:

1. borrow `point` and `radius` mutably in one coordinated tuple query;
2. copy the old `x` and `y` values into local scalars;
3. calculate both derivatives from the same old point;
4. assign `x + dt * dx` and `y + dt * dy` to `point`;
5. recompute `radius` from the updated point; and
6. advance integer and physical simulation time.

No payload is cloned, extracted, or reallocated during this loop.

## Recording streams

One task owns one `SystemStateWriter` with three independently sampled streams:

| Stream | Selected fields | Cadence | Expected records |
|---|---|---:|---:|
| `trajectory` | `point` | 10 iterations | 501 |
| `radius` | `radius` | 5 iterations | 1001 |
| `checkpoint` | `point`, `radius` | 1000 iterations | 6 |

Iterations 0 and 5000 are included. `trajectory` and `radius` demonstrate partial
state recording, while `checkpoint` contains every payload required to restore
a complete state.

The writer owns each sampling interval decoded from `fixed.json`. The evolution loop
offers the current state after every step; non-due streams return before field
lookup or serialization. Due streams borrow selected payloads only for
encoding, then queue owned JSON records under byte-bounded backpressure. Final
completion offers the endpoint once more so a sampling-interval-misaligned final state is
recorded exactly once. Records remain whole across chunk rollover.

## Run the example

Run it from the repository root:

```bash
cargo run --manifest-path examples/attractor_2d/Cargo.toml
```

Recordings will be written beneath:

```text
examples/attractor_2d/target/recordings/
```

`ExecutionScope` gives each execution a UTC-readable, collision-resistant
directory and exposes its creation timestamp. Existing recordings are not
deleted or deliberately reused. Each task recording automatically persists its
UTC creation/finalization timestamps and monotonic active duration.

Rayon bounds simultaneous model execution by its worker-pool size. Successful
return from the parallel operation means every task also completed typed
checkpoint round-trip validation; an error prevents the final success line.

The centralized reporter is the only human-facing terminal writer. It clears
an interactive terminal once when reporting starts and immediately creates all
three `mu` rows, even if Rayon has fewer than three workers. Running rows show
elapsed execution time and ETA. In a redirected or CI run, no clearing occurs
and the final line has this form:

```text
[workflow] status=completed tasks=3 completed=3 failed=0 pending=0 message=round_trip=true output=...
```

## Readback validation

After each writer reaches completed status, the application:

- registers direct Serde JSON decoding for `Vec<f64>` under `point` and `f64`
  under `radius`;
- reads only the latest complete checkpoint; and
- requires exact final time and payload equality with the live state.

Plotting and numerical analysis are deliberately absent. They are consumer
consumers of valid recordings, not part of the minimum evolution workflow.

The library enables Serde JSON's `float_roundtrip` behavior so finite `f64`
payloads recover their original binary values when decoded from the emitted
decimal representation. A mismatch terminates the example instead of merely
printing a failed informational flag.

## Cross-check

The cross-check in [`src/cross_check.rs`](src/cross_check.rs) is a compulsory
numerical correctness verification in this example.

Run it with the normal command:

```bash
cargo run --manifest-path examples/attractor_2d/Cargo.toml
```

[`src/cross_check.rs`](src/cross_check.rs) contains the same Euler calculation
in one file using only hard-coded constants, `[f64; 2]`, and the standard
library. It deliberately contains no `scientific-workflow`, JSON, state
container, storage, or recording APIs.

The workflow and naive implementation have been compared for every swept
`mu`. Their final iteration, accumulated physical time, both point coordinates, and
radius have identical `f64` bit patterns for all three tasks.
