# Scientific Workflow design

## 1. Replicate execution

`StudySettings` loads the exact process-level policy from `study.json`:
replicate count, sequential or parallel execution, failure policy, and the
study seed. Loading is side-effect free. `ReplicateExecutor` enacts that policy
only after the application supplies its resolved output root.

The original process is a controller. It starts the current executable once
per replicate, preserving its arguments and stdio. Each child re-enters the
dispatcher and receives one `ReplicateContext` rather than recursively
dispatching again. That context owns:

- a zero-based replicate index;
- the declared replicate count;
- the existing `output_root/replicate_<index>` scope;
- a lazy `ReplicateSeedDeriver` initialized from the study seed and index.

Sequential mode starts at most one child at a time. Parallel mode deliberately
starts one operating-system process per replicate without introducing another
pool or resource scheduler. `fail_fast` stops future sequential work or
terminates still-active parallel children after a failure is observed;
`finish_all` allows every replicate to finish before returning failure.

Existing replicate directories are rejected. This preserves output rather
than guessing whether a previous execution may be overwritten or resumed.
Applications use storage continuation APIs explicitly when continuation is
scientifically valid.

## 2. Study

`Study` is the largest orchestration scope. It owns:

- an ordered collection of phases;
- dependency validation and phase selection;
- study-wide cooperative cancellation;
- one scheduler and one renderer;
- a deterministic `StudyPlan`;
- one durable `StudyRecord`;
- a final `StudySummary`.

A study does not own scientific model behavior. It executes application-owned
task workloads and observes only their lifecycle reports.

Only one active study may own the process terminal. Plain and hidden modes use
the same scheduling and lifecycle rules as terminal mode.

## 3. Phase

`Phase` is a nonempty scheduling group owned by a study. It owns tasks and the
policy governing their admission:

- maximum active tasks;
- prepared-task queue capacity;
- delay between task starts;
- task timeout;
- phase deadline;
- phase dependencies;
- transition confirmation;
- fail-fast or finish-active behavior.

Phase boundaries are scheduling barriers. A dependent phase does not begin
until its dependencies are satisfied.

## 4. Task

`Task` is the only registerable workload type. A task has:

- a phase-local `TaskId`;
- a phase-qualified `TaskKey` after registration;
- a human-readable label;
- an application-defined category;
- immutable application metadata;
- a `TaskMode`;
- an optional workload when already completed.

`TaskMode::Progress` and `TaskMode::OneShot` are display/reporting modes of the
same type. One-shot work is not represented by a separate activity abstraction.

`Task::completed` registers application-verified work that is already
satisfied. It reaches the normal completed state without consuming a worker or
delay rank.

## 5. Workload and TaskContext

The workload contract is:

```text
FnOnce(&TaskContext) -> TaskResult
```

`TaskContext` is the sole task-to-study communication boundary. It exposes
read-only task identity and metadata plus:

- `set_target_iteration`;
- `set_iteration`;
- `should_continue`;
- `set_detail`;
- `report`;
- `is_cancelled`.

Iteration updates use atomics and do not allocate or lock. Detail text is
mutex-protected because it is updated infrequently. Messages use the bounded
renderer event channel. Cancellation is cooperative.

The underlying progress and one-shot handles are private. A workload cannot
access the scheduler, renderer, terminal, or phase state.

Task completion follows the workload result:

- `Ok(())` marks the task completed;
- `Err(error)` marks it failed and records the error;
- cancellation marks it cancelled when the workload cooperates.

## 6. Scheduler

The internal study scheduler consumes phase-owned tasks. It controls worker
count, prepared work, launch delay, timeouts, deadlines, and failure barriers.

The scheduler never parses configuration or performs model I/O. It starts a
task through the renderer, constructs its `TaskContext`, invokes the workload,
and records the terminal result.

## 7. Renderer

`StudyRenderer` is the exclusive human-facing output owner. Worker tasks update
shared task slots and send bounded `RenderEvent` messages. The renderer builds
an immutable `RenderSnapshot` for each refresh and performs every terminal
write.

The terminal display preserves the established progress-bar appearance. Its
layout separates study status, phase information, task progress, messages, and
the command line.

Plain mode emits append-only uncolored lifecycle lines. Hidden mode suppresses
display but retains lifecycle validation and recording.

## 8. Commands

`CommandInput` owns input editing and parsing. It produces `StudyCommand`
values for the study controller and has no scheduling or rendering authority.

The initial command contract contains one command:

```text
exit → StudyCommand::Exit → cooperative study cancellation
```

The separation allows future commands without coupling terminal input to task
registration or scheduler implementation.

## 9. Plans, records, and summaries

`StudyPlan` is a deterministic side-effect-free representation of every
registered phase and task. It includes phase policies, dependencies, task
identity, mode, metadata, registration order, and delay rank.

`StudyRecord` is the durable lifecycle record for one execution. It contains
`PhaseRecord` and `TaskRecord` entries with status and timing information.

`StudySummary` and `PhaseSummary` are in-memory completed-execution results.
They do not carry model-specific return values; scientific handoff occurs
through application-owned files, artifacts, databases, or other resources.

## 10. Configuration

Configuration is intentionally separate from the study hierarchy. The
library-defined inputs are independent:

```text
study.json      → StudySettings       → ReplicateExecutor
parameters.json → StudyConfiguration  → PhaseConfiguration
paths.json      → ProjectPaths        → application-selected paths
```

`StudySettings` validates replicate policy but does not create output or start
processes. `ProjectPaths` resolves named paths but does not inspect targets.
Scientific parameter expansion follows this separate flow:

```text
parameters.json
        ↓
StudyConfiguration
        ↓ phase(group, phase)
PhaseConfiguration
        ↓
ConfigurationIter
        ↓
ResolvedConfiguration
        ↓ application mapping
Task
```

`StudyConfiguration::load(study_root)` validates the complete study-wide
parameter registry. `phase(group, phase)` returns the only expandable space;
its `combinations()` method lazily enumerates global × group-shared ×
phase-local selections. `combination(ordinal)` provides deterministic indexed
access.

`ResolvedConfiguration` exposes its ordinal, lookup, typed decoding,
iteration, containment, and JSON serialization.

Scientific parameter configuration does not:

- load study settings or named paths;
- load state schemas;
- create execution directories;
- create or register tasks;
- select phases;
- schedule tasks or dispatch replicates;
- render output;
- perform model or storage operations.

Downstream applications own schemas and model-specific input bundles. They
select the named output root and explicitly connect validated study settings to
replicate dispatch.

## 11. Scientific ownership

Task workloads retain ownership of scientific effects. The orchestration layer
does not hide filesystem, storage, artifact, network, subprocess, or machine
resource operations behind `TaskContext`.

Supporting modules remain independent:

- `system_state` owns typed scientific state;
- `time_series` owns in-memory ordered analysis state;
- `storage` owns asynchronous persisted streams;
- `execution` owns replicate process isolation and filesystem execution scopes;
- `artifact` owns immutable content-addressed publication;
- `rng_record` owns persisted RNG provenance.

This boundary keeps configuration, scheduling, display, and scientific work
independently replaceable.
