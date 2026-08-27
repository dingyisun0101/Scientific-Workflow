# Runtime API

The `runtime` subsystem is the ultimate coordinator of active execution. It
accepts immutable intent from Study and owns output creation, replicate
admission, dependency scheduling, task concurrency, cooperative timeouts and
cancellation, model invocation, automatic persistence lifecycle, and
publication of inferred UI facts.

Runtime does not parse JSON, discover models, decode constants for planning,
or let application code construct phases/tasks.

## Basic API

`runtime::basic` intentionally exports no symbols. Ordinary applications call
the crate-level `scientific_workflow::run(&Path)` facade, which loads a Study
before handing it to Runtime. This keeps project discovery and compilation out
of the active-execution subsystem.

After successful preflight, Runtime creates a unique execution directory
beneath the inferred `<project-root>/output`, creates one isolated directory
per replicate, executes all phases and tasks, and completes their recordings.

For each task, Runtime receives Study's private effective persistence settings,
derives the destination, constructs the backend, submits initial/step/final
observations, applies backpressure, commits terminal status, and shuts the
backend down. Application/model code performs none of these operations.

Runtime also creates one automatic UI session from Study's private inferred UI
plan. It publishes execution, replicate, phase, task, iteration, target,
outcome, and recording-path facts. UI renders only to an interactive stderr
terminal and remains silent when output is redirected or captured. Rendering
is best-effort and cannot change the Runtime result.

An execution blocks until all admitted work has stopped and all successful
persistence sessions have durably completed. Task/phase timeout cancellation is cooperative: task
execution checks between model steps. Blocking application code inside one
step cannot be safely killed by Rust and may delay return.

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
Phases run in stable topological order. Within a phase, runtime respects
`max_concurrency`, `start_interval`, task timeout, phase timeout, and sibling
failure policy. Fail-fast stops further admission and requests cancellation of
active siblings; finish-all continues admitting declared siblings and returns
an error after they finish.

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

### `runtime::advanced::TaskRunSummary`

- `identity() -> &str`: Study-inferred task identity;
- `model() -> &str`: registered model key;
- `final_iteration() -> u64`: last successfully completed scientific iteration;
- `recording_directory() -> &Path`: completed durable recording path.

Summary paths are owned `PathBuf` internally and borrowed as `&Path`. Summary
values do not authorize append/resume and carry no live model or state.

### `runtime::advanced::RuntimeError`

This non-exhaustive enum reports failures after a valid Study is available:

- `OutputScope { path, source }`: unique execution or replicate directory could
  not be created;
- `Task { task, source }`: application/model, state, observation, config decode,
  or persistence operation failed during invocation;
- `TaskPanicked { task }`: task worker unwound unexpectedly;
- `TaskTimedOut { task, timeout }`: task stopped after its cooperative deadline;
- `TaskCancelled { task }`: runtime cancelled an active sibling;
- `PhaseTimedOut { phase, timeout }`: phase exceeded its cooperative deadline;
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
use scientific_workflow::runtime::advanced::execute;
use scientific_workflow::study::advanced::Study;

# fn run() -> Result<(), Box<dyn std::error::Error>> {
let study = Study::load(Path::new("."))?;
let summary = execute(study)?;
for replicate in summary.replicates() {
    println!("replicate {}: {}", replicate.index(), replicate.output_directory().display());
}
# Ok(())
# }
```

## Not API

Scheduler polling, worker thread names, active task handles, atomic cancellation
flags, metadata-map assembly, task output ordinals, `RuntimeTaskHost`,
`PersistenceSession`, UI events/session, backend ownership, and
topological-position calculation are private.

Recording metadata keeps complete resolved constants under `model_constants`
and Workflow identity/source facts under a separate `workflow` object. The
effective backend and byte settings are recorded under `workflow.persistence`.
These namespaces never overwrite one another even when scientific constants
use the same field names.

The private output-directory allocator and local persistence adapter are
implementation details. A future backend may change internal construction
and wire mechanics while preserving the documented inferred root, isolation,
summaries, failure atomicity, effective-plan provenance, and observation
lifecycle.
