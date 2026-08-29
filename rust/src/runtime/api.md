# Runtime API

The `runtime` subsystem is the ultimate coordinator of active execution. It
accepts immutable intent from Study and owns output creation, replicate
admission, dependency scheduling, task concurrency, cooperative timeouts and
cancellation, execution unit or external-program invocation (including Config-lowered
Python environments), automatic persistence lifecycle, and
publication of inferred UI facts.

Runtime does not parse JSON, discover execution units, decode constants for planning,
or let application code construct phases/tasks. It never reparses the central
Config retained by Study.

## Automatic runtime behavior

Ordinary applications call the crate-level `scientific_workflow::run(&Path)`
facade, which loads a Study before handing it to Runtime. This keeps project
discovery and compilation out of the active-execution subsystem.

After successful preflight, Runtime creates a unique execution directory
beneath the inferred `<project-root>/output`, creates one isolated directory
per replicate, executes all phases and tasks, and completes each member's
recording or each program workspace.

For each task, Runtime derives the destination and constructs private
persistence sessions. A standalone unit opens one member recording; an ensemble
opens one recording per member. ExecutionUnit views submit independent initial/step/final
observations with bounded backpressure. Program tasks receive frozen configuration snapshots, logs, and
an artifacts workspace. Runtime commits terminal status and shuts the session
down; application/execution unit code performs none of this coordination.

For each execution unit task, Runtime also constructs one immutable
`InitializationContext` from Study's optional master seed, replicate ordinal,
task identity, and registered execution-unit key. The unit may request
purpose-named shared or member-scoped seeds. Runtime validates member identities
after initialization and passes each member's applicable actual requests into
Persistence before that execution unit's recording begins. A program or Python
task may instead declare one purpose-named task seed in `study.json`. Runtime
derives it from the same master seed, replicate, inferred task identity,
program kind, and purpose; the child receives only that derived value, not an
`InitializationContext` or the master seed.

Runtime also creates one automatic UI session from Study's private inferred UI
plan. It publishes execution, replicate, phase, task, iteration, target,
outcome, and recording-path facts. Interactive stdin plus stderr selects the
Ratatui dashboard; otherwise UI emits stable plain lifecycle lines. Typing
`exit` while work is active or pressing Ctrl+C requests cooperative execution
cancellation. After any terminal outcome the dashboard remains visible until the
user explicitly types `exit`; Ctrl+C alone never closes it. Consequently an
interactive `execute` call returns only after execution has ended and that final
`exit` is submitted. Noninteractive execution never waits for input. UI is the sole presentation boundary: renderer startup,
terminal initialization/input/drawing, and plain-output failures panic rather
than becoming `RuntimeError` or silent fallback.

An execution blocks until all admitted work has stopped and all successful
persistence sessions have durably completed. ExecutionUnit cancellation is cooperative
between steps, so blocking application code inside one step may delay return.
External programs, including Python interpreters/environment managers, are
polled and are killed and reaped when cancellation or a timeout is observed.

Failure does not overwrite prior output. Execution that fails after output creation
retains its unique directory and any failed/running recording evidence for
diagnosis. The crate-level facade wraps an active `RuntimeError` in
`WorkflowError::Runtime`; the complete-workflow composition is documented by
`WorkflowError`.

## Public API

### `runtime::execute`

`execute(study: Study) -> Result<RunSummary, RuntimeError>` consumes one
clone-cheap immutable Study. It is Runtime's sole execution entry and cannot
accept a project root, manifest, execution unit catalog, or unresolved declarations.
It is the supported embedding seam for callers that loaded and optionally
inspected a Study first.

It performs the same output-producing work reached through the crate facade.
It never mutates the supplied
declaration; consuming the handle simply makes accidental double execution
less likely at the call site. A caller may explicitly clone Study before the
call when multiple independent runs are intended.

Replicates run sequentially or concurrently according to `ReplicatePolicy`.
Every replicate gets `replicate-XXXXXX` beneath the unique execution scope.
Parallel fail-fast observes replicate completion as it occurs: the first
reported failure requests cancellation of still-running sibling replicates,
and Runtime waits for their cleanup before returning that originating error.
Parallel finish-all never performs this policy cancellation and collects every
terminal outcome before returning the first observed failure. Successful
summary order remains independent of worker completion order.

Phases run in stable topological order. Within a phase, runtime respects
`max_concurrency`, the minimum `start_interval` between successive admissions,
task timeout, phase timeout, and sibling failure policy. The first eligible
task has no artificial pre-admission wait. Fail-fast stops further admission
and requests cancellation of active siblings; finish-all continues admitting
declared siblings and returns an error after they finish.

Task deadline classification uses the worker's completion timestamp, rather
than the later instant at which the polling scheduler joins it. A task that
completed before its deadline is therefore not retroactively timed out. Phase
deadlines likewise apply only while pending or unfinished work remains.

An interactive `exit` submitted during execution stops further admission,
cancels active execution unit/program workers, waits for their cleanup, restores the
terminal, and returns `RuntimeError::ExecutionCancelled`. Ctrl+C performs the
cancellation but leaves the completed dashboard open until the required `exit`
command. If execution has already reached a successful or failed terminal outcome,
`exit` only closes the interface and does not change that outcome.

### `runtime::RunSummary`

Returned only after complete success. It is cloneable and owns paths/summaries.

- `output_directory() -> &Path` returns the unique generated execution path.
- `replicates() -> &[ReplicateRunSummary]` returns ascending replicate order.

The summary performs no IO and does not keep recording files open.

### `runtime::ReplicateRunSummary`

- `index() -> u64`: zero-based replicate index;
- `output_directory() -> &Path`: isolated replicate directory; and
- `phases() -> &[PhaseRunSummary]`: successful phases in dependency execution
  order.

Parallel worker completion order is never exposed; summaries are sorted by
index.

### `runtime::PhaseRunSummary`

- `name() -> &str`: stable manifest phase key;
- `tasks() -> &[TaskRunSummary]`: successful tasks restored to deterministic
  Study plan order, independent of concurrent completion order.

### `runtime::TaskRunKind`

`TaskRunKind` is a non-exhaustive data-bearing enum. `ExecutionUnit` contains
the registration key and stable `Box<[MemberRunSummary]>`; `Program` contains
the resolved launcher executable and optional canonical Python script. Callers
matching it must retain a fallback for future workload kinds.

### `runtime::TaskRunSummary`

- `identity() -> &str`: Study-inferred task identity;
- `kind() -> &TaskRunKind`: borrows the variant-specific result; and
- `output_directory() -> &Path`: task output root or program workspace. For a
  single-member unit this is also the recording directory; every recording
  path is available from the execution-unit variant's `members`.

Summary paths are owned. The enum prevents irrelevant or contradictory fields.
A task-level final iteration is intentionally absent because members can finish
at different iterations; callers can compute a maximum when meaningful.
Summary values do not authorize append/resume and carry no live task, process,
execution unit, or state.
Summaries and `RuntimeError` are `Send + Sync`.

### `runtime::MemberRunSummary`

One immutable per-member result owned by its task summary:

- `identity() -> &str` returns the stable identity supplied through
  `MemberView`;
- `final_iteration() -> u64` returns that member's terminal iteration; and
- `output_directory() -> &Path` returns that member's completed recording.

For a multi-member unit, directories are
`<task-output>/members/member-<index>` in stable member order. The identity is
metadata, not a filesystem fragment, so application identities cannot redirect
persistence. Summary borrows are tied to the owning summary and perform no IO.

### External program and Python contract

Runtime starts a resolved executable directly, without a shell, inside its
private `artifacts/` directory. Standard input is closed. Standard output and
error are captured as `stdout.log` and `stderr.log`. Runtime supplies absolute
paths through:

- `WORKFLOW_CONFIG_PATH`: immutable `workflow-config.json`, containing the
  captured `study` value and all `wf_configs/` JSON documents;
- `WORKFLOW_DEPENDENCIES_PATH`: immutable `workflow-dependencies.json`, with
  completed task summaries from declared dependency phases;
- `WORKFLOW_PROJECT_ROOT`: canonical project root;
- `WORKFLOW_EXECUTION_ROOT`: unique execution directory;
- `WORKFLOW_REPLICATE_ROOT`: current replicate directory; and
- `WORKFLOW_TASK_OUTPUT`: the task's `artifacts/` directory, also its working
  directory; and
- `WORKFLOW_TASK_SEED`: decimal unsigned 64-bit task seed, present only when
  the program/Python task declared `seed: {"purpose":"..."}`.

Programs may read any central configuration keys they understand. The supplied
workspace is their default location for temporary or task-scoped artifacts,
but an external program owns its domain-specific IO and may resolve a
project-relative destination from `wf_configs/parameters.json`. For example, the bundled
Python plotter writes directly to the configured `output/plots`. Rust
Persistence does not relocate or publish those Python-owned files. A Study uses
its captured snapshot: editing JSON after `Study::load` cannot alter these
files.
On completion Runtime writes terminal `program.json`; nonzero exit status is a
task failure. A seeded program's metadata records its purpose and actual
derived seed even when the child fails. Dependency JSON is deterministic. Each task contains `identity`,
`output_directory`, and one `workload` object. An execution-unit workload
contains its kind, registration key, and member identity/iteration/recording
summaries. A program workload contains `kind` (`program` or `python`), its
launcher executable, and the optional canonical Python script. Dependency JSON
remains a data handoff, not a shell command protocol.

A nested Python task follows this exact runtime contract after Config lowers
its environment to one invocation. Runtime has no Python-specific scheduler,
active-environment lookup, package installer, or import-path mutation. The
script reads the same `WORKFLOW_*` files/paths as any program. It may use
`WORKFLOW_TASK_OUTPUT` or a validated destination from its project parameters.
Its `.py` file need not be executable. The selected
manager/interpreter, manager arguments, canonical script, and script arguments
are fixed before output creation.

### `runtime::RuntimeError`

This non-exhaustive enum reports failures after a valid Study is available:

- `ExecutionCancelled`: the interactive `exit` command or Ctrl+C requested
  cooperative cancellation;
- `OutputScope { path, source }`: unique execution or replicate directory could
  not be created;
- `Task { task, source }`: execution unit, program, state, observation, config decode,
  or persistence operation failed during invocation;
- `TaskPanicked { task }`: task execution unwound; Runtime marks any active
  member recording failed with a bounded diagnostic before returning the stable
  existing error shape;
- `TaskTimedOut { task, timeout }`: an execution unit observed its cooperative deadline
  or an external process was terminated after its deadline;
- `TaskCancelled { task }`: runtime cancelled an active sibling;
- `PhaseTimedOut { phase, timeout }`: phase exceeded its deadline; active
  execution units stop cooperatively and active external programs are terminated;
- `StartWorker { scope, source }`: OS thread creation failed;
- `ReplicatePanicked { index }`: parallel replicate worker unwound; and
- `Replicate { index, source }`: contextual wrapper for a failed replicate.

Sources retain their original error chains. A runtime error never represents a
manifest grammar or execution unit-key preflight failure; those remain `StudyError`.

## Example

The complete ordinary workflow uses the crate facade rather than the embedding
Runtime API:

```rust,no_run
use std::path::Path;

fn main() -> Result<(), scientific_workflow::WorkflowError> {
    scientific_workflow::run(Path::new("."))
}
```

An embedding integration can retain completion paths:

```rust,no_run
use std::path::Path;
use scientific_workflow::runtime::{execute, TaskRunKind};
use scientific_workflow::study::Study;

# fn run() -> Result<(), Box<dyn std::error::Error>> {
let study = Study::load(Path::new("."))?;
let summary = execute(study)?;
for replicate in summary.replicates() {
    println!("replicate {}: {}", replicate.index(), replicate.output_directory().display());
    for phase in replicate.phases() {
        for task in phase.tasks() {
            match task.kind() {
                TaskRunKind::ExecutionUnit { execution_unit, members } => {
                    println!("unit {execution_unit}");
                    for member in members {
                        println!("  member {} at {}", member.identity(), member.final_iteration());
                    }
                }
                TaskRunKind::Program { executable, .. } => {
                    println!("program {}", executable.display())
                }
                _ => println!("another task kind"),
            }
        }
    }
}
# Ok(())
# }
```

## Not API

Scheduler polling, worker thread names, active task handles, atomic cancellation
flags, completion channels/timestamps, bounded panic-payload formatting,
task output ordinals, `RuntimeTaskHost`,
execution unit/program task environments, child-process polling, `PersistenceSession`,
UI events/session, backend ownership, and
topological-position calculation are private.

Runtime passes `MemberRecordingProvenance` as semantic facts—task identity,
registered unit key, selected state, parameter ordinal/source, and resolved constants—to
Persistence. Runtime does not construct durable JSON namespaces and does not
name the local backend or its format fields. Persistence authors recording
metadata, which keeps complete resolved constants under `constants`
and Workflow identity/source facts under a separate `workflow` object. The
workflow object names the selected registration, `parameter_ordinal`, and canonical
`parameter_source`, plus the explicitly selected `state` key; the effective
object also records `member_index` and `member_identity` for every execution unit;
backend and byte settings are recorded under
`workflow.persistence`. The user-authored `chunk_target_mb` and
`queue_capacity_mb` have already been converted, so provenance deliberately
records the exact effective values as `chunk_target_bytes` and
`queue_capacity_bytes`. These namespaces never overwrite one another even
when scientific constants use the same field names.

The private output-directory allocator and local persistence adapter are
implementation details. A future backend may change internal construction
and wire mechanics while preserving the documented inferred root, isolation,
summaries, failure atomicity, effective-plan provenance, and observation
lifecycle.

An active task retains Config's clone-cheap immutable byte snapshot, not the
Config lookup/parser object. Program launch receives those bytes and dependency
facts through Persistence. Runtime obtains task execution/provenance/summary
facts through Study's peer view rather than opening Task descriptors.
The program execution port likewise supplies Task's semantic
`ProgramTaskInvocation`; Runtime does not import Config's resolved-program
representation.
Paths written to program dependency snapshots are exact UTF-8 values validated
during Config preflight; Runtime performs no lossy path rendering.
