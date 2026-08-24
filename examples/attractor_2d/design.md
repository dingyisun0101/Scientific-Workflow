# Attractor example design

## Purpose

The example demonstrates the complete Workflow boundary without creating an
application framework around a two-variable model.

```text
StudySettings
  -> ReplicateExecutor
  -> output/replicate_0 worker context
  -> StudyConfiguration
  -> simulation and validation WorkloadConfiguration values
  -> ResolvedConfiguration values
  -> application-mapped simulation tasks
  -> phase 1: task-owned evolve and record
  -> durable recording directories
  -> phase 2: reconstruct and validate
  -> phase 3: render trajectories with Python
  -> Study summary
```

## Ownership

- `StudySettings` owns the validated replicate count, scheduling mode, failure
  policy, and base seed.
- `ReplicateExecutor` owns one subprocess per replicate and the exclusive
  `replicate_<index>` output scope. This example declares exactly one.
- `StudyConfiguration` owns the validated study-wide parameter registry and
  exposes only workload-scoped combination spaces.
- The example uses `ProjectPaths` to select the replicate output root and
  independently loads `state.json` through the state-schema API.
- `ResolvedConfiguration` is the only resolved configuration object. Tuple decoding groups
  values at their point of use without an application settings struct.
- `HopfModel` owns the sole mutable `SystemState` for one simulation task.
- Each simulation task owns its writer and recording lifecycle.
- The rendering task owns its Python subprocess and derived PNG files.
- `Study` owns only bounded phase scheduling inside one replicate, display,
  summaries, and cooperative cancellation.

## Phase handoff

The phases do not exchange a `HashMap`, result wrapper, or study-owned output.
Planning decodes every simulation configuration into an application-owned
`AttractorRun` descriptor. The descriptor carries a scoped producer identity,
the exact simulation configuration, and an explicit path derived through:

```text
ExecutionScope::named_task_recording_directory(producer_identity)
```

Validation configurations are paired with producers from the same global and
component-shared selections. After the dependency barrier, each validation task
receives the producer descriptor directly and verifies its completed recording.
Independent workload-local sweeps may therefore differ in count and ordering
without redirecting validation to another producer. Scientific I/O remains
application-owned and the durable result remains the authoritative boundary.

Phase 3 receives the explicit producer recording paths and the output path. Its one-shot task calls
`mamba run -n DSES python scripts/render_trajectories.py`; the script verifies
recordings through the official Python reader before plotting. Neither the
phase nor the study carries trajectory values in memory between tasks.

Every recording, the study record, and every rendered plot is below the
worker's `ReplicateContext::output_directory()`. The application does not
reconstruct replicate paths or duplicate the executor's naming rule.

## Display pause

Each phase workload waits three seconds before doing its work so its refreshed
header can be read. In addition, every model step has a one-millisecond pause so
the live per-task progress remains observable. The step pause is a permanent,
required teaching constraint for this example and must never be removed or
optimized away. These delays are example presentation policy: the study does
not impose them and they are not part of the numerical method.

## Files

- `study.json`: declare one sequential, fail-fast replicate and the base seed.
- `main.rs`: dispatch replicates, then load, declare three phases, and run one
  worker study.
- `attractor_run.rs`: decode producer identity and retain its explicit output path.
- `task_execution.rs`: decode model inputs and assemble one model.
- `hopf_model.rs`: explicit-Euler scientific kernel.
- `recording.rs`: decode recording policy, evolve, sample, and complete.
- `validation.rs`: validate the latest checkpoint at an explicit producer path.
- `rendering.rs`: invoke the renderer in the `DSES` Mamba environment.
- `scripts/render_trajectories.py`: render verified trajectories as PNG files.

## Environment

Run the example with `mamba run -n DSES cargo run ...`. The Rust task also
selects that environment explicitly for Python so Matplotlib and the intended
scientific environment are not inherited accidentally from another shell.
