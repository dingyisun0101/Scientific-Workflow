# Runtime API

The `runtime` subsystem is the ultimate coordinator of active execution. It
accepts immutable intent from Study and owns output creation, replicate
admission, dependency scheduling, task concurrency, cooperative timeouts and
cancellation, model or external-program invocation (including Config-lowered
Python environments), automatic persistence lifecycle, and
publication of inferred UI facts.

Runtime does not parse JSON, discover models, decode constants for planning,
or let application code construct phases/tasks. It never reparses the central
Config retained by Study.

## Basic API

`runtime::basic` intentionally exports no symbols. Ordinary applications call
the crate-level `scientific_workflow::run(&Path)` facade, which loads a Study
before handing it to Runtime. This keeps project discovery and compilation out
of the active-execution subsystem.

After successful preflight, Runtime creates a unique execution directory
beneath the inferred `<project-root>/output`, creates one isolated directory
per replicate, executes all phases and tasks, and completes their model
recordings or program workspaces.

For each task, Runtime derives the destination and constructs its private
persistence session. Model tasks submit initial/step/final observations with
bounded backpressure. Program tasks receive frozen configuration snapshots, logs, and
an artifacts workspace. Runtime commits terminal status and shuts the session
down; application/model code performs none of this coordination.

Runtime also creates one automatic UI session from Study's private inferred UI
plan. It publishes execution, replicate, phase, task, iteration, target,
outcome, and recording-path facts. Interactive stdin plus stderr selects the
Ratatui dashboard; otherwise UI emits stable plain lifecycle lines. Typing
`exit` or pressing Ctrl+C in the dashboard requests cooperative execution
cancellation. UI is the sole presentation boundary: renderer startup,
terminal initialization/input/drawing, and plain-output failures panic rather
than becoming `RuntimeError` or silent fallback.

An execution blocks until all admitted work has stopped and all successful
persistence sessions have durably completed. Model cancellation is cooperative
between steps, so blocking application code inside one step may delay return.
External programs, including Python interpreters/environment managers, are
polled and are killed and reaped when cancellation or a timeout is observed.

Failure does not overwrite prior output. Execution that fails after output creation
retains its unique directory and any failed/running recording evidence for
diagnosis. The crate-level facade wraps an active `RuntimeError` in
`WorkflowError::Runtime`; the complete-workflow composition is documented by
`error::basic`.

## Advanced API

The Advanced scope adds the immutable execution input, summaries, and active
error vocabulary to the empty Basic scope.

### `runtime::advanced::execute`

`execute(study: Study) -> Result<RunSummary, RuntimeError>` consumes one
clone-cheap immutable Study. It is Runtime's sole execution entry and cannot
accept a project root, manifest, model catalog, or unresolved declarations.
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

An interactive exit request stops further admission across the execution,
cancels active model/program workers, waits for their cleanup, restores the
terminal, and returns `RuntimeError::ExecutionCancelled`.

### `runtime::advanced::RunSummary`

Returned only after complete success. It is cloneable and owns paths/summaries.

- `output_directory() -> &Path` returns the unique generated execution path.
- `replicates() -> &[ReplicateRunSummary]` returns ascending replicate order.

The summary performs no IO and does not keep recording files open.

### `runtime::advanced::ReplicateRunSummary`

- `index() -> u64`: zero-based replicate index;
- `output_directory() -> &Path`: isolated replicate directory; and
- `phases() -> &[PhaseRunSummary]`: successful phases in dependency execution
  order.

Parallel worker completion order is never exposed; summaries are sorted by
index.

### `runtime::advanced::PhaseRunSummary`

- `name() -> &str`: stable manifest phase key;
- `tasks() -> &[TaskRunSummary]`: successful tasks restored to deterministic
  Study plan order, independent of concurrent completion order.

### `runtime::advanced::TaskRunKind`

`TaskRunKind` is a non-exhaustive `Copy + Eq` enum distinguishing `Model` and
`Program`. Callers matching it must retain a fallback for future workload
kinds.

### `runtime::advanced::TaskRunSummary`

- `identity() -> &str`: Study-inferred task identity;
- `kind() -> TaskRunKind`: generic workload kind;
- `model() -> Option<&str>`: registered key for model tasks only;
- `program() -> Option<&Path>`: resolved launcher executable for program tasks
  only. For a Python declaration this is its interpreter or environment
  manager;
- `program_kind() -> Option<&str>`: `program` or `python` for program tasks,
  and `None` for model tasks;
- `python_script() -> Option<&Path>`: canonical script path for nested Python
  tasks only;
- `final_iteration() -> Option<u64>`: last scientific iteration for model
  tasks only; and
- `output_directory() -> &Path`: completed model recording or program
  workspace.

Summary paths are owned `PathBuf` internally and borrowed as `&Path`. The
model/program option pair and optional iteration agree with `kind`.
`program_kind` is populated exactly for program tasks, and `python_script` is
populated exactly when that program kind is `python`. Summary values do not
authorize append/resume and carry no live task, process, model, or state.
Summaries and `RuntimeError` are `Send + Sync`.

### External program and Python contract

Runtime starts a resolved executable directly, without a shell, inside its
private `artifacts/` directory. Standard input is closed. Standard output and
error are captured as `stdout.log` and `stderr.log`. Runtime supplies absolute
paths through:

- `WORKFLOW_CONFIG_PATH`: immutable `workflow-config.json`, containing the
  captured `study` value and all `config/` JSON documents;
- `WORKFLOW_DEPENDENCIES_PATH`: immutable `workflow-dependencies.json`, with
  completed task summaries from declared dependency phases;
- `WORKFLOW_PROJECT_ROOT`: canonical project root;
- `WORKFLOW_EXECUTION_ROOT`: unique execution directory;
- `WORKFLOW_REPLICATE_ROOT`: current replicate directory; and
- `WORKFLOW_TASK_OUTPUT`: the task's `artifacts/` directory, also its working
  directory.

Programs may read any central configuration keys they understand. The supplied
workspace is their default location for temporary or task-scoped artifacts,
but an external program owns its domain-specific IO and may resolve a
project-relative destination from `wf_configs/parameters.json`. For example, the bundled
Python plotter writes directly to the configured `output/plots`. Rust
Persistence does not relocate or publish those Python-owned files. A Study uses
its captured snapshot: editing JSON after `Study::load` cannot alter these
files.
On completion Runtime writes terminal `program.json`; nonzero exit status is a
task failure. Dependency JSON is deterministic and contains each dependency
phase, task identity/kind, optional model/program/final iteration, and output
directory. Program entries additionally carry `program_kind` (`program` or
`python`) and the optional canonical `python_script`; the same facts are
available through the successful Rust task summary. Dependency JSON remains a
data handoff, not a shell command protocol.

A nested Python task follows this exact runtime contract after Config lowers
its environment to one invocation. Runtime has no Python-specific scheduler,
active-environment lookup, package installer, or import-path mutation. The
script reads the same `WORKFLOW_*` files/paths as any program. It may use
`WORKFLOW_TASK_OUTPUT` or a validated destination from its project parameters.
Its `.py` file need not be executable. The selected
manager/interpreter, manager arguments, canonical script, and script arguments
are fixed before output creation.

### `runtime::advanced::RuntimeError`

This non-exhaustive enum reports failures after a valid Study is available:

- `ExecutionCancelled`: the interactive `exit` command or Ctrl+C requested
  cooperative cancellation;
- `OutputScope { path, source }`: unique execution or replicate directory could
  not be created;
- `Task { task, source }`: model, program, state, observation, config decode,
  or persistence operation failed during invocation;
- `TaskPanicked { task }`: task execution unwound; Runtime marks any active
  model recording failed with a bounded diagnostic before returning the stable
  existing error shape;
- `TaskTimedOut { task, timeout }`: a model observed its cooperative deadline
  or an external process was terminated after its deadline;
- `TaskCancelled { task }`: runtime cancelled an active sibling;
- `PhaseTimedOut { phase, timeout }`: phase exceeded its deadline; active
  models stop cooperatively and active external programs are terminated;
- `StartWorker { scope, source }`: OS thread creation failed;
- `ReplicatePanicked { index }`: parallel replicate worker unwound; and
- `Replicate { index, source }`: contextual wrapper for a failed replicate.

Sources retain their original error chains. A runtime error never represents a
manifest grammar or model-key preflight failure; those remain `StudyError`.

## Example

The complete ordinary workflow uses the crate facade rather than a Runtime
Basic API:

```rust,no_run
use std::path::Path;

fn main() -> Result<(), scientific_workflow::WorkflowError> {
    scientific_workflow::run(Path::new("."))
}
```

An advanced integration can retain completion paths:

```rust,no_run
use std::path::Path;
use scientific_workflow::runtime::advanced::{execute, TaskRunKind};
use scientific_workflow::study::advanced::Study;

# fn run() -> Result<(), Box<dyn std::error::Error>> {
let study = Study::load(Path::new("."))?;
let summary = execute(study)?;
for replicate in summary.replicates() {
    println!("replicate {}: {}", replicate.index(), replicate.output_directory().display());
    for phase in replicate.phases() {
        for task in phase.tasks() {
            match task.kind() {
                TaskRunKind::Model => println!("model {}", task.model().unwrap()),
                TaskRunKind::Program => println!("program {}", task.program().unwrap().display()),
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
model/program task environments, child-process polling, `PersistenceSession`,
UI events/session, backend ownership, and
topological-position calculation are private.

Runtime passes `ModelRecordingProvenance` as semantic facts—task identity,
model, selected state, parameter ordinal/source, and resolved constants—to
Persistence. Runtime does not construct durable JSON namespaces and does not
name the local backend or its format fields. Persistence authors recording
metadata, which keeps complete resolved constants under `model_constants`
and Workflow identity/source facts under a separate `workflow` object. The
workflow object names the selected model, `parameter_ordinal`, and canonical
`parameter_source`, plus the explicitly selected `state` key; the effective
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
