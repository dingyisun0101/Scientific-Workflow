# `attractor_2d` design

## Purpose

This example is the minimum complete evolution workflow for
`scientific-workflow`. It demonstrates conventional project configuration, a
parameter sweep, direct ownership and mutation of `SystemState`, independently
sampled output streams, bounded asynchronous writing, and exact typed readback.

It deliberately excludes visualization and scientific analysis. Those are
consumer uses of a recording and would obscure the core runtime pattern this
example is intended to teach.

## Scientific model

The model is the supercritical Hopf normal form:

```text
dx/dt = mu * x - omega * y - (x^2 + y^2) * x
dy/dt = omega * x + mu * y - (x^2 + y^2) * y
```

One fixed-step explicit-Euler call advances the model exactly once. Both
derivatives are evaluated from the old coordinates, both state payloads are
updated, and only then are iteration and physical time advanced. The selected
`dt = 0.01` is stable enough for this bounded demonstration.

The Cartesian sweep uses `mu = [-0.25, 0.25, 1.0]`, producing three independent
tasks. The richer dynamics justify recording a trajectory, but the application
does not analyze or render it during evolution.

`ScientificProject::task_configs()` emits those three complete task handles in
canonical order. Each handle shares the parsed fixed values, selected sweep
storage, and project paths, so the same type can later move directly into a
orchestration mission without a merged configuration allocation.

The example adapts this lazy iterator through
`progress_workloads_from_project`. The phase scheduler prepares workloads
through a bounded queue without requiring the application to maintain a second
task registry. Every concurrent closure owns its model and writer; only the
immutable configuration, schema, and execution scope are shared.

## Project configuration

The standalone crate root is the scientific project root. `ScientificProject`
loads the conventional files directly from `config/`:

```text
config/
├── fixed.json   shared model, evolution, sampling, and storage settings
├── sweep.json   Cartesian `mu` parameter axis
├── paths.json   project-relative recording root
└── state.json   stable state keys and natural-language descriptions
```

The schema is language-neutral. Rust payload types are established when the
application first inserts values into each state slot.

## State ownership

`HopfModel` is the sole owner of the live `SystemState`. No `FinalState`, copied
domain snapshot, or second authoritative structure exists.

The state contains:

- `point: Vec<f64>` for the mutable `[x, y]` coordinates;
- `radius: f64` for the synchronized radial diagnostic; and
- built-in `SimulationTime` for iteration and physical time.

`HopfModel` exposes only two operations:

- `state()` lends an immutable state view to recording and validation; and
- `step()` performs one scientific transition.

Coefficient and payload accessors are intentionally absent. Model coefficients
are implementation details of `step`, while consumer code can use the
ordinary typed `SystemState` API when it genuinely needs a payload.

The step uses one coordinated tuple borrow for `point` and `radius`. This gives
safe simultaneous mutable access to distinct slots without cloning or moving
either payload allocation.

## Recording

Each task owns one `SystemStateWriter` with three streams:

| Stream | Fields | Sampling interval | Expected records |
|---|---|---:|---:|
| `trajectory` | `point` | 10 iterations | 501 |
| `radius` | `radius` | 5 iterations | 1001 |
| `checkpoint` | `point`, `radius` | 1000 iterations | 6 |

The model offers its state initially and after every step. The writer owns the
sampling intervals, so the simulation contains no cadence branches. A non-due
observation returns before payload encoding; a due observation borrows selected
payloads only long enough to produce an owned record.

The configured byte-bounded queue applies backpressure if storage falls behind.
The target chunk size controls rollover, and a state record is never split
between chunks. Completion offers the final state once more, drains the queue,
seals the recording, and returns `CompletedRecording`.

Operational UTC timing and active duration are writer responsibilities. The
example manages only scientific iteration and physical time.

## Validation boundary

Validation is intentionally narrower than analysis. For each completed task it:

1. registers JSON payload decoders for `point: Vec<f64>` and `radius: f64`;
2. opens the completed recording;
3. reads only the latest complete checkpoint; and
4. asserts exact time and payload equality against the live final state.

`read_latest_state_from_stream` avoids reconstructing complete series merely to
inspect endpoints. Serde JSON's finite-float round-trip behavior preserves the
original `f64` bit patterns. Any mismatch becomes an application error before a
success result is printed.

`WorkflowRuntime` owns scheduling and display for the registered simulation
phase. All three parameter tasks receive rows before their task-owned model and
recording work starts. Each running row reports elapsed execution time and ETA.
Plain output ends with:

```text
[runtime] status=completed phases=1 tasks=3
```

Reaching this line proves that configuration loading, sweep expansion, state
evolution, sampling, persistence, decoding, and endpoint comparison succeeded
for every task. Generated data remains available beneath the reported ignored
`target/recordings` path.

## Source boundaries

```text
src/
├── main.rs                  end-to-end flow: prepare, run, record, validate
└── hopf_model.rs            sole state owner and Euler evolution kernel
```

`main.rs` performs only the reusable top-level sequence:

```text
load project
  -> create execution scope and configuration-generated runtime phase
  -> runtime schedules bounded TaskContext workloads
       -> task owns assemble -> evolve/record -> validate checkpoint
  -> runtime displays the phase summary
```

No module duplicates the live state. `main.rs` owns the complete task
orchestration flow and keeps `HopfModel` as the sole mutable state owner.

## Annotation policy

Comments concentrate on decisions that matter when adapting the example:

- who owns scientific data;
- why tuple borrowing is safe and allocation-free;
- when simulation time becomes authoritative;
- why observation is unconditional in the evolution loop;
- how queue and chunk byte limits differ;
- why the writer, rather than the model, owns sampling and operational timing;
- how payload types are rebound during decoding; and
- why latest-record reading is sufficient for validation.

Routine Rust syntax is left uncommented. This keeps annotation thorough at the
workflow boundary without turning the example into a line-by-line language
tutorial.

## Application API reference

### `HopfModel`

Owns immutable equation coefficients and the sole mutable `SystemState` for one
task.

#### Reference

```text
prepare_task -> HopfModel::new
record_model -> HopfModel::state, HopfModel::step
run validation closure -> HopfModel::state
```

### `HopfModel::new`

Creates simulation time zero, moves the point and derived radius into typed
state slots, and returns the sole state-owning model.

#### Reference

```text
prepare_task -> HopfModel::new
```

### `HopfModel::state`

Returns an immutable view without cloning or transferring payload ownership.

#### Reference

```text
record_model observation/completion -> HopfModel::state
run final validation -> HopfModel::state
```

### `HopfModel::step`

Mutates both payloads in place and advances simulation time exactly once.

#### Reference

```text
record_model evolution loop -> HopfModel::step
```

### `TaskSettings`

Holds the decoded evolution count, three sampling intervals, and two storage
byte limits for one task. It contains no duplicate task ordinal or model state.

#### Reference

```text
prepare_task -> TaskSettings construction
record_model/build_writer -> TaskSettings consumption by immutable borrow
```

### `prepare_task`

Decodes one complete shared `TaskConfig` and assembles its model and runtime
settings.

#### Reference

```text
runtime TaskContext workload -> prepare_task
```

### `record_model`

Builds one task writer, offers the initial state, performs and observes every
step, synchronizes `TaskProgress` from authoritative state time, and completes
storage with the final state.

#### Reference

```text
runtime TaskContext workload -> record_model(model, progress)
```

### `build_writer`

Maps application stream policy and byte limits onto `SystemStateWriterBuilder`.

#### Reference

```text
record_model -> build_writer
```

### `validate_recording`

Decodes the latest complete checkpoint and asserts exact equality with the live
completed state. Partial streams remain demonstrations of independent sampling;
rechecking their shared fields would add repetition rather than a new API.

#### Reference

```text
runtime TaskContext workload -> validate_recording
```

### `main`

Sequences project loading, scope creation, runtime startup, bounded task
execution, and runtime finalization after all task closures validate.

#### Reference

```text
process entry -> main
```

## Independent numerical reference

`src/cross_check.rs` contains the same Euler kernel as a single
standard-library program with hard-coded inputs and no workflow facilities.
Its final iteration, physical time, coordinates, and radius have been compared
bit-for-bit with the workflow implementation for all three `mu` values. This
validates that workflow ownership and recording do not alter the numerical
kernel; storage behavior is validated by the typed round trip above and by the
library integration suite. This cross-check is a required component of the
example workflow.
