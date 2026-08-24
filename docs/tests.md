# Test structure

Tests follow the public vocabulary from broad scope to specific behavior.

## Study tests

`study_workflow.rs` verifies:

- a study owns phases and produces `StudyPlan`, `StudyRecord`, and
  `StudySummary` values;
- phase dependencies and selected-phase execution;
- task registration and unique task selection;
- one-shot and progress modes on the same `Task` type;
- completed tasks that do not execute workloads;
- durable phase/task status and timing records;
- invalid studies, phases, task IDs, categories, and dependencies.

Internal renderer tests verify stable section sizing, command-line placement,
and preservation of progress-bar counts, elapsed time, and ETA fields.

Command tests verify editing, submission, strict `exit` parsing, and rejection
of unsupported commands.

## Phase and task behavior

Integration tests verify that phases own concurrency, prepared-queue capacity,
delay, timeouts, deadlines, dependencies, and failure policy.

Task workloads report only through `TaskContext`. Tests cover progress targets,
absolute iteration updates, detail text, messages, completion, failure, and
cooperative cancellation.

## Configuration tests

`configuration_workflow.rs` verifies the complete configuration contract:

```text
parameters.json → StudyConfiguration → WorkloadConfiguration → ResolvedConfiguration values
```

Coverage includes:

- strict Workflow-owned `study.json` fields, typed application settings, and
  rejection of unknown or invalid policy fields;
- nested fixed values;
- Cartesian sweep products and deterministic ordinal order;
- explicit cases;
- indexed combination access;
- typed value and tuple decoding;
- missing values and type errors retaining the combination ordinal;
- malformed JSON, duplicate keys, invalid documents, and fixed/sweep conflicts.

Configuration tests deliberately do not cover task creation, named paths,
schemas, scheduling, or display because those concerns are outside the module.

## Replicate execution tests

`replicate_workflow.rs` runs the integration-test executable through the public
dispatcher. It verifies that parallel mode re-enters exactly one worker per
declared replicate, creates isolated `replicate_<index>` scopes, preserves the
declared count, and binds every worker to the matching lazy seed deriver.

## Scientific modules

- `state_workflow.rs` verifies schemas, heterogeneous payload ownership, typed
  borrowing, mutation, extraction, cloning, and time progression.
- `analysis_workflow.rs` verifies ordered in-memory state series.
- `storage_workflow.rs`, `storage_resilience.rs`, and `resume_workflow.rs`
  verify bounded persistence, metadata, atomic publication, recovery,
  continuation, and reconstruction.
- `artifact_workflow.rs` verifies immutable content-addressed publication and
  digest validation.
- `rng_record_workflow.rs` verifies RNG provenance validation and persistence.
- `python_reader_conformance.rs` verifies the shared persisted format across
  Rust and Python readers. Both readers also consume one shared corpus of
  malformed metadata mutations and must reject every case.

## Downstream integration

The repository also checks:

- GLV-owned `GlvInputs` and `GlvConfiguration` mapping into tasks;
- Simulator-owned `SimulatorInputs` and `SimulatorConfiguration` grouping into
  ensemble tasks;
- Dispatcher-owned study directories, inputs, paths, phases, and plans;
- the attractor example's direct mapping from `WorkloadConfiguration`
  combinations into tasks.

These builds ensure downstream programs retain ownership of model-specific
inputs while depending only on the shared Task → Phase → Study contract.
