# Attractor example design

## Purpose

The example demonstrates the complete Workflow boundary without creating an
application framework around a two-variable model.

```text
ScientificProject
  -> configuration-generated simulation tasks
  -> phase 1: task-owned evolve and record
  -> durable recording directories
  -> phase 2: reconstruct and validate
  -> WorkflowRuntime summary
```

## Ownership

- `ScientificProject` owns validated fixed, sweep, path, and state-schema
  configuration.
- `TaskConfig` is the only resolved configuration object. Tuple decoding groups
  values at their point of use without an application settings struct.
- `HopfModel` owns the sole mutable `SystemState` for one simulation task.
- Each simulation task owns its writer and recording lifecycle.
- `WorkflowRuntime` owns only bounded phase scheduling, display, summaries, and
  cooperative cancellation.

## Phase handoff

The phases do not exchange a `HashMap`, result wrapper, or runtime-owned output.
Both receive the same configuration-generated task ordinal. Phase 1 writes to:

```text
ExecutionScope::task_recording_directory(task_ordinal)
```

After the dependency barrier, phase 2 derives that path again and verifies the
completed recording. This keeps scientific I/O application-owned and makes the
durable result the authoritative boundary.

## Display pause

Each phase workload waits three seconds before doing its work so its refreshed
header can be read. In addition, every model step has a one-millisecond pause so
the live per-task progress remains observable. The step pause is a permanent,
required teaching constraint for this example and must never be removed or
optimized away. These delays are example presentation policy: the runtime does
not impose them and they are not part of the numerical method.

## Files

- `main.rs`: load, declare two phases, and run.
- `task_execution.rs`: decode model inputs and assemble one model.
- `hopf_model.rs`: explicit-Euler scientific kernel.
- `recording.rs`: decode recording policy, evolve, sample, and complete.
- `validation.rs`: derive the durable path and validate the latest checkpoint.
