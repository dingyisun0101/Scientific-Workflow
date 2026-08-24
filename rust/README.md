# Scientific Workflow

Scientific Workflow provides configuration expansion, scientific state and
storage primitives, and a small task scheduler with a centralized terminal
display.

## Module boundaries and public API surfaces

The crate is intentionally split by ownership and responsibility:

- `study`: orchestration control plane for declared phases/tasks and execution policy.
  - Public entry points: `Study`, `StudyBuilder`, `Phase`, `Task`, `TaskContext`,
    `StudyError`, `StudySummary`, `PhaseSummary`.
  - Boundaries: does not resolve model config, load state schemas, or perform
    filesystem schema validation. It only executes workloads supplied by callers.

- `configuration`: deterministic experiment declarations from `fixed.json` and
  `sweep.json`.
  - Public entry points: `ConfigurationSpace`, `ResolvedConfiguration`.
  - Boundaries: does not schedule work, select concurrency, publish artifacts,
    or own recording I/O.

- `system_state`: typed heterogeneous state value model and schema.
  - Public entry points: `SystemStateSchema`, `SystemState`, `SimulationTime`,
    `StateFieldSchema`, `StateError`.
  - Boundaries: provides in-memory state shape, not persistence or scheduling.

- `time_series`: immutable-in-principle in-memory collections of states for analysis.
  - Public entry points: `StateSeries`, `StateSeriesView`, `StateSeriesError`.
  - Boundaries: no filesystem, no codecs, no queueing, no sampling logic.

- `storage`: durable recording and reconstruction boundary.
  - Public entry points: `SystemStateWriterBuilder`, `SystemStateWriter`,
    `StoredStateSeriesReader`, `CompletedRecording`, `StorageError`.
  - Boundaries: does not define model transitions or sampling semantics; it
    records whatever complete states the caller submits through writer APIs.

- `execution`: filesystem lifecycle for a run.
  - Public entry points: `ExecutionScope`.
  - Boundaries: creates and validates directories/paths; never defines model
    semantics.

- `artifact`: immutable input publishing.
  - Public entry points: `persist_artifact`, `load_verified_artifact`,
    `ArtifactDescriptor`, `PersistedArtifact`.
  - Boundaries: only content-addressed publication + verification, not execution.

- `rng_record`: reproducibility metadata for caller-owned RNG sources.
  - Public entry points: `RngRecord`, `RngRecordError`, `RNG_RECORDS_METADATA_KEY`.
  - Boundaries: persists metadata only.

The module boundaries are intentionally non-overlapping. Each concern is owned by
exactly one boundary first:

- Experiment definition and values: `configuration`.
- Scheduling and lifecycle orchestration: `study`.
- State schema, in-memory states, and analysis series:
  `system_state`, `time_series`.
- Durable persistence, checkpoints, and execution boundaries:
  `storage`, `execution`.
- Input publication and reproducibility metadata:
  `artifact`, `rng_record`.
- Import organization: `prelude`.

Use these seams to compose behavior without crossing boundaries twice.

## API ownership quick table

| Boundary | Public API surface | Boundary owner |
| - | - | - |
| Orchestration | `Study`, `StudyBuilder`, `StudyError`, `Phase`, `Task`, `TaskContext`, `StudySummary`, `StudyRecord` | `study` |
| Config space | `ConfigurationSpace`, `ResolvedConfiguration`, `ConfigurationIter`, `ConfigurationError` | `configuration` |
| State model | `SystemStateSchema`, `SystemState`, `StateFieldSchema`, `SimulationTime`, `StateError` | `system_state` |
| Analysis series | `StateSeries`, `StateSeriesView`, `StateSeriesError`, `StateSeriesPushError` | `time_series` |
| Persistence/reconstruction | `SystemStateWriterBuilder`, `SystemStateWriter`, `CompletedRecording`, `StoredStateSeriesReader`, `CompletedRecording`, `StorageError`, `JsonPayloadDecoder*` | `storage` |
| Execution scope | `ExecutionScope`, `ExecutionScopeError` | `execution` |
| Artifact IO | `persist_artifact`, `load_verified_artifact`, `ArtifactDescriptor`, `PersistedArtifact`, `ArtifactError`, `ArtifactLoadError` | `artifact` |
| RNG metadata | `RngRecord`, `RngRecordError`, `RNG_RECORDS_METADATA_KEY` | `rng_record` |

## Entry boundaries

| Entry surface | Intended use |
| - | - |
| `prelude::basics` | scientific primitives and data shapes |
| `prelude::study` | orchestration wiring and scheduling boundaries |

## Study vocabulary

The orchestration hierarchy is deliberately small:

```text
Study
└── Phase
    └── Task
        └── workload(&TaskContext) -> TaskResult
```

### Study

`Study` is the largest scope. It owns the declared phases, phase ordering,
dependency checks, cooperative cancellation, the study renderer, and the
durable `StudyRecord`.

`StudyPlan` is the immutable serializable declaration. `StudySummary` is the
result of one execution.

```rust,no_run
use scientific_workflow::prelude::study::*;

# fn main() -> Result<(), StudyError> {
let phase = Phase::builder(1, "simulation")
    .task(Task::one_shot("prepare", "prepare inputs", |_| Ok(())))
    .build()?;

let summary = Study::builder("study-record.json")
    .phase(phase)
    .build()?
    .run()?;

assert!(summary.is_success());
# Ok(())
# }
```

Only one terminal-rendering study may execute in a process at a time.

### Phase

`Phase` owns many tasks and their scheduling policy. Its builder controls:

- `max_active_tasks`;
- prepared-task queue capacity;
- delay between task starts;
- per-task timeout;
- phase deadline;
- dependencies;
- confirmation before a transition;
- failure policy.

The scheduler operates only on declared phases and tasks. It has no knowledge
of model types, configuration formats, paths, recordings, or subprocesses.

### Task

`Task` is every registerable workload. There are two modes:

- `TaskMode::Progress` for iterative work;
- `TaskMode::OneShot` for lifecycle-only work.

Both modes use the same `Task` type and the same workload contract. An
application can also register an already satisfied task with `Task::completed`.

Task identity is explicit:

- `TaskId` is phase-local;
- `TaskKey` is phase-qualified;
- `category` groups application-defined task kinds;
- `metadata` carries immutable application-defined values.

### Workload communication

A workload communicates with the study only through `TaskContext`:

```rust,no_run
use scientific_workflow::prelude::study::*;

let task = Task::progress("simulation-0", "simulation 0", |context| {
    context.set_target_iteration(1_000)?;
    for iteration in 0..=1_000 {
        context.set_iteration(iteration)?;
        context.set_detail(format!("iteration {iteration}"));
        if context.is_cancelled() {
            break;
        }
    }
    context.report("simulation finished")?;
    Ok(())
});
```

Iteration updates are lock-free. Detail updates are intended for infrequent
human-readable state changes. Messages pass through the sole renderer, and
cancellation is cooperative.

Renderer-specific progress and one-shot handles are private. Tasks do not know
about Ratatui, terminal layout, renderer threads, or scheduler channels.

## Display and commands

The study renderer owns all terminal output. The interactive display keeps the
existing task progress bar and divides study, phase, task, message, and command
areas into stable sections.

`CommandInput` owns terminal editing and parsing. It produces `StudyCommand`
values without scheduling tasks or mutating renderer state directly. The
currently supported command is:

```text
exit
```

`exit` requests cooperative study cancellation.

Noninteractive output can be selected with `plain()`, and display can be
suppressed with `hidden()` without changing scheduling behavior.

## Configuration

Configuration is independent from the study hierarchy. Its complete flow is:

```text
directory containing fixed.json and sweep.json
→ ConfigurationSpace
→ ResolvedConfiguration combinations
→ application-defined Task values
```

`ConfigurationSpace` does exactly one job: validate fixed/sweep input and
enumerate every Cartesian combination or explicit case.

```rust,no_run
use scientific_workflow::configuration::ConfigurationSpace;
use scientific_workflow::prelude::study::{Phase, StudyError, Task};

# fn build_phase() -> Result<Phase, StudyError> {
let configurations = ConfigurationSpace::load("study/config")
    .map_err(|source| StudyError::TaskWorkload {
        task: "load-configuration".to_owned(),
        source: Box::new(source),
    })?;

let tasks = configurations.combinations().map(|configuration| {
    let ordinal = configuration.ordinal();
    let temperature = configuration
        .decode_value::<f64>("/temperature")
        .expect("validated application configuration");

    Task::one_shot(
        format!("simulation-{ordinal}"),
        format!("temperature {temperature}"),
        move |_| {
            run_simulation(configuration, temperature);
            Ok(())
        },
    )
    .category("simulation")
    .metadata("configuration_ordinal", ordinal)
});

Phase::builder(1, "simulations").tasks(tasks).build()
# }
# fn run_simulation(_: scientific_workflow::configuration::ResolvedConfiguration, _: f64) {}
```

Configuration does not load named paths or state schemas, create tasks,
register phases, select scheduling policies, or perform scientific work. Those
responsibilities belong to the downstream study application.

## Supporting scientific modules

- `system_state`: typed heterogeneous scientific state and schemas.
- `time_series`: ordered in-memory state collections.
- `storage`: bounded asynchronous state-stream persistence and reconstruction.
- `execution`: collision-resistant execution directories and task paths.
- `artifact`: immutable content-addressed artifact publication and verification.
- `rng_record`: validated RNG provenance records.

Use `prelude::basics` for scientific primitives and `prelude::study` at the
application orchestration boundary.
