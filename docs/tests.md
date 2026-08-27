# Test structure

Tests follow the public vocabulary from broad scope to specific behavior.

## Study tests

`study_workflow.rs` verifies:

- a study owns phases and produces `StudyPlan`, `StudyRecord`, and
  `StudySummary` values;
- phase dependencies and selected-phase execution;
- whole-phase completion reuse for selected phases and omitted dependencies;
- dependency closure that omits complete phases and invokes incomplete phases;
- launch-level completion-examination disablement;
- invalid and panicking examiners failing before workloads or record writes;
- reused phase disposition in durable records;
- task registration and unique task selection;
- one-shot and progress modes on the same `Task` type;
- completed tasks that do not execute workloads;
- durable phase/task status and timing records;
- invalid studies, phases, task IDs, categories, and dependencies.

Completion tests deliberately stop at the phase boundary. They verify whether
Workflow reuses or invokes a phase, but do not prescribe task-level artifact
validation or continuation. Those policies belong to application and model
workload tests.

Internal renderer tests verify stable section sizing, command-line placement,
and preservation of progress-bar counts, elapsed time, and ETA fields.

Command tests verify editing, submission, strict `exit` parsing, and rejection
of unsupported commands.

## Phase and task behavior

Integration tests verify that phases own concurrency, prepared-queue capacity,
delay, timeouts, deadlines, dependencies, and failure policy.

Legacy `study::Task` workloads report only through `study::TaskContext`. Those
tests cover progress targets, absolute iteration updates, detail text,
messages, completion, failure, and cooperative cancellation. The separate
new-task tests below cover context-free `ScientificModel` execution.

## Configuration tests

`config_workflow.rs` verifies the target project-specification contract:

```text
project root → centrally parsed documents → resolved task inputs → typed constants
```

Coverage includes strict study-manifest parsing, exact source preservation,
canonical containment beneath `config/inputs`, state-schema handoff without a
second file read, effective policies, dependency validation, Cartesian sweeps,
correlated cases, literal arrays, deterministic ordinals, config-owned typed
decoding, contextual errors, and advanced-prelude exports.

`configuration_workflow.rs` continues to verify the transitional legacy
configuration contract:

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

Target config tests deliberately stop before compiled-task matching,
state-schema semantics, scheduling, display-field/schema validation, output,
and execution. Those compositions belong to runtime and the participating
module boundaries.

## Replicate execution tests

`replicate_workflow.rs` runs the integration-test executable through the public
dispatcher. It verifies that parallel mode re-enters exactly one worker per
declared replicate, creates isolated `replicate_<index>` scopes, preserves the
declared count, and binds every worker to the matching lazy seed deriver.

## Scientific modules

- `state_workflow.rs` verifies typed-path schema loading, basic/advanced API
  tiers, one-time payload initialization, heterogeneous ownership, typed
  borrowing, mutation, extraction, cloning, and `StateTime` progression.
- `writer_workflow.rs` verifies the minimal `Writer`/`Stream` definition
  surface, inferred all-field defaults, schema-bound canonical field order,
  validation failures, strict basic/advanced tiering, checked borrowed
  observations, canonical owned encoding, and the replaceable sink port.
- `task_workflow.rs` verifies config-supplied single-decode stateful and one-shot
  definitions, automatic initial/step/final host boundaries, writer validation
  before model initialization, cooperative cancellation, task descriptors,
  strict basic/advanced tiering, direct-state observable enforcement, schema
  allocation identity, advancing iterations, and target validity.
- `analysis_workflow.rs` verifies ordered in-memory `StateSeries` behavior and
  ordinary borrowed access without a separate view type.
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
  combinations into explicitly named transitional `StudyTask` values.

These builds ensure downstream programs retain ownership of model-specific
inputs while the runtime migration replaces the transitional
study-task → phase → study contract with the new context-free task boundary.
