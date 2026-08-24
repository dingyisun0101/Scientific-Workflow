# Attractor example design

## Purpose

The example demonstrates the complete Workflow boundary without creating an
application framework around a two-variable model.

```text
ConfigurationSpace
  -> ResolvedConfiguration values
  -> application-mapped simulation tasks
  -> phase 1: task-owned evolve and record
  -> durable recording directories
  -> phase 2: reconstruct and validate
  -> phase 3: render trajectories with Python
  -> Study summary
```

## Ownership

- `ConfigurationSpace` owns only validated fixed-and-sweep configuration.
- The example application independently loads `paths.json` and `state.json`.
- `ResolvedConfiguration` is the only resolved configuration object. Tuple decoding groups
  values at their point of use without an application settings struct.
- `HopfModel` owns the sole mutable `SystemState` for one simulation task.
- Each simulation task owns its writer and recording lifecycle.
- The rendering task owns its Python subprocess and derived PNG files.
- `Study` owns only bounded phase scheduling, display, summaries, and
  cooperative cancellation.

## Phase handoff

The phases do not exchange a `HashMap`, result wrapper, or study-owned output.
Both receive the same configuration-generated task ordinal. Phase 1 writes to:

```text
ExecutionScope::task_recording_directory(ordinal)
```

After the dependency barrier, phase 2 derives that path again and verifies the
completed recording. This keeps scientific I/O application-owned and makes the
durable result the authoritative boundary.

Phase 3 receives only the execution and output paths. Its one-shot task calls
`mamba run -n DSES python scripts/render_trajectories.py`; the script verifies
recordings through the official Python reader before plotting. Neither the
phase nor the study carries trajectory values in memory between tasks.

## Display pause

Each phase workload waits three seconds before doing its work so its refreshed
header can be read. In addition, every model step has a one-millisecond pause so
the live per-task progress remains observable. The step pause is a permanent,
required teaching constraint for this example and must never be removed or
optimized away. These delays are example presentation policy: the study does
not impose them and they are not part of the numerical method.

## Files

- `main.rs`: load, declare three phases, and run.
- `task_execution.rs`: decode model inputs and assemble one model.
- `hopf_model.rs`: explicit-Euler scientific kernel.
- `recording.rs`: decode recording policy, evolve, sample, and complete.
- `validation.rs`: derive the durable path and validate the latest checkpoint.
- `rendering.rs`: invoke the renderer in the `DSES` Mamba environment.
- `scripts/render_trajectories.py`: render verified trajectories as PNG files.

## Environment

Run the example with `mamba run -n DSES cargo run ...`. The Rust task also
selects that environment explicitly for Python so Matplotlib and the intended
scientific environment are not inherited accidentally from another shell.
