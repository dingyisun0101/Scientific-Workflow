1. scan for possible: architectural, efficiency, and ergo upgrades.
2. add multiprocessing to npy processing. discuss if we can and should also add progress reporting? at least we need more logging messages. To do that, how to allow rust to accept logs from python?
3. make sure all python scripts are wrapped so user can directly use them by adding to cargo.toml the online version
4. UI: assign colors to messages of different importance in message panal. Better layout for the first panel: texts are too dense and close together right now. add a new command: pause and resume.

## Scan conclusions — 2026-09-04

The four requests above are worthwhile. Prioritize reliable Python execution
and live diagnostics before adding parallel conversion and pause/resume. Keep
the existing Config → Study → Runtime ownership model; this scan does not
justify a broad subsystem rewrite. The recommendations below are proposals,
not implemented changes.

Reviewed the architecture, configuration/program resolution, execution and
resource scheduling, persistence, Python reader/converter, UI, packaging,
release documentation, and related tests. Only this planning file was edited.

### 1. NPY multiprocessing: yes, at the recording boundary

**Current evidence:** `convert_workflow_dependencies` in
[npy.py](python/src/scientific_workflow_reader/npy.py) converts member recordings
in a serial loop. Each member already has a distinct output directory and an
atomic publication boundary. `ResolvedProgramTask::for_npy` in
[config/program.rs](rust/src/config/program.rs) hard-codes `threads: 1`.
[runtime/host.rs](rust/src/runtime/host.rs) passes the assigned count through
`WORKFLOW_THREADS`; [runtime/resource.rs](rust/src/runtime/resource.rs) charges
external tasks against the shared study budget.

**Recommendation:** use a bounded process pool with one recording per job.
Pass paths and stable ordinals to workers; return small manifest summaries.
Keep verified readers, parsed records, and NumPy arrays inside their worker.
Let the parent collect completion events and publish the final batch manifest
in original ordinal order, regardless of completion order. Retain serial
execution for one worker or one recording.

**Accepted decision 4:** automatically allocate
`min(study_thread_budget, recording_count)` conversion workers, using the
deduplicated member recordings in the batch. Reserve that same allowance in
Rust's shared `ResourceBudget` before launch; concurrent replicates share the
budget. A converter reserving the full budget makes other compute work wait.
Keep serial execution for one worker and introduce no separate NPY worker
setting initially. Do not independently size each replicate's pool from the
machine CPU count. Benchmark memory consumption and storage throughput before
finalizing implementation; any additional automatic cap needs an explicit
policy rather than silently changing the accepted allocation rule.
Control nested numerical-library thread pools so worker processes do not each
consume the entire compute allowance.

Use an explicit, tested multiprocessing start context and importable worker
functions. Decision 1 below supersedes the original compatibility proposal:
Workflow's Python tools will require Python 3.14 or newer, allowing the newer
pool termination and `map(buffersize=...)` APIs without compatibility branches.
Running futures are not stopped by `cancel_futures=True`, so cancellation
requires a separate design. These constraints follow the
[Python executor contract](https://docs.python.org/3/library/concurrent.futures.html).

On failure, stop admission, terminate/join workers as appropriate, and leave
only individually verified completed members reusable. Publish no new success
batch manifest until all requested members succeed. Distinguish reused members
from newly converted members in diagnostics. Give concurrent attempts unique
temporary directories and enforce one publisher per final destination; the
current PID-based temporary names are not sufficient for every same-process
concurrent caller.

Member parallelism will not speed up a single large recording. Defer stream
parallelism until benchmarks demonstrate that it is needed; it complicates
memory limits and member-level atomic publication.

**Acceptance:** serial/parallel output equivalence, deterministic manifest
ordering, deduplication, uneven member sizes, worker failure, cancellation,
retry reuse, and simultaneous replicates respecting the combined budget.

### 2. Python logging and progress: add both

**Current evidence:** the converter prints one flushed line after each member.
Runtime directs child stdout/stderr straight into persistence-owned files.
[runtime/event.rs](rust/src/runtime/event.rs) has lifecycle events and
iteration-based `TaskProgress`, but no program log or program progress event.
The UI deliberately renders every running non-unit task as a spinner.

**Recommendation:** introduce a small, versioned program-event protocol owned
by Runtime, with a Python helper that integrates with `logging`. A practical
first transport is opt-in, prefixed JSON lines on stderr. Runtime should drain
stdout and stderr concurrently, preserve the original bytes in the existing
log files, parse only recognized event frames, and publish task-scoped events
to presentation. Attach replicate/task identity in Rust rather than trusting
child-provided routing identifiers. No Python-to-Rust FFI is needed.

Keep arbitrary program output valid. Malformed, unknown-version, or oversized
event frames should remain diagnosable without crashing the scientific task.
Limit frame sizes, sanitize terminal control characters for display, and bound
the UI queue. Coalesce frequent progress updates while preserving lifecycle
events and complete durable logs. Pipe draining must continue during pause and shutdown; otherwise
children can block on full pipes. The
[subprocess documentation](https://docs.python.org/3/library/subprocess.html)
describes this pipe deadlock risk.

**Accepted decision 5:** failure to write required stdout/stderr logs or an
unexpected output-pipe read failure stops the affected task, marks it failed,
and applies the configured failure policy. Malformed or unsupported progress
frames remain in the raw log, produce bounded warnings, and do not fail the
scientific task. Coalesce excess display updates while retaining the latest
progress and complete raw logs. Preserve the existing run-failure and cleanup
behavior for dashboard renderer/input failures, since interactive controls
may no longer function. Continue draining output during task shutdown so a
logging failure cannot leave children blocked on full pipes. If a storage
failure also prevents failure metadata from being written, report through any
functioning output channel and return an error; do not claim successful capture.

The Python pool parent should be the single event emitter; workers send small
updates to it through an internal queue. Flush events promptly, and account
for buffering by Python/environment-manager launchers.

Start with these messages: conversion started, worker allowance, member
started/reused/completed, stream planning/writing/verification, elapsed time,
failure with recording/stream context, cancellation, and final totals. Include
the retained log location in failed-task diagnostics.

Add a separate program-progress event containing stage, completed, optional
total, and unit. Show members completed first; later add per-stream records or
bytes for long-running members. The reader already exposes record and encoded
byte totals. Do not turn member count into an apparently precise time estimate:
members vary in size and conversion has planning, writing, and verification
passes. Throttle updates to the display cadence and emit stage completion.

**Acceptance:** live visibility before process exit, preserved raw logs,
stderr/stdout flooding without deadlock, malformed/partial frames, bounded
display memory, and final-event delivery on success, failure, and cancellation.

### 3. Published Rust dependency and Python packaging

**Current evidence:** [rust/Cargo.toml](rust/Cargo.toml) packages Rust sources,
tests, and guides, but no Python implementation. `$npy` resolves `python3` and
launches `-m scientific_workflow_reader.npy`; resolution only checks the
executable, not whether its environment contains the reader and NumPy.
Ordinary Python tasks already support system, venv, mamba, conda, uv, and Poetry
through [config/python.rs](rust/src/config/python.rs), but `$npy` bypasses that
selection. The checkout wrapper in
[recording_to_npy.py](python/scripts/recording_to_npy.py) locates `../src`, which
does not exist in the published Rust package.

**Conclusion:** adding the registry Rust dependency currently does not supply
the standard Python tools. Cargo resolves Rust packages; a Python dependency
needs an explicit distribution/runtime arrangement. See
[Cargo dependency sources](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html).

**Accepted decision 2:** use A, the separately installed Python package.
This supersedes the original proposal to bundle Python sources in the Rust
crate. Users prepare their environment and install the compatible
`scientific-workflow-reader[npy]` package, including NumPy. Workflow launches
the installed converter and checks its prerequisites; automatic environment
creation, dependency installation, and bundled converter sources are outside
this pass.

Add clear NPY setup instructions to the repository and Rust READMEs, with
matching detail in the Python README: Python 3.14+, environment creation and
activation, installation of the compatible reader with its `npy` extra, an
import/version check, and running Workflow in the selected environment. State
explicitly that adding the Cargo dependency does not install the Python tools.
Keep package/version instructions synchronized with the chosen release.

**Accepted decision 3:** `$npy` uses the Python interpreter selected by the
environment inherited when Workflow starts. Users activate their prepared
environment before launching Workflow. Resolve and report the interpreter
path, then validate imports and version compatibility before scientific work
starts, with actionable diagnostics. No additional `$npy` environment override
is planned for this pass; existing user-authored Python task declarations keep
their current environment configuration.

Use bold, firm setup wording, for example: **Activate the environment containing
Workflow's Python package and NumPy before running Workflow. Repeat this in
every new shell session before launching the application.** Also emphasize
**Python 3.14 or newer is required for Workflow's Python tools** and **adding
the Rust dependency does not install the Python tools**. Interpret the user's
phrase "wrong words" as a request for strong wording, pending correction.

Keep user-managed installation separate from execution, and perform executable
probes in Runtime preflight before scientific work starts so `Study::load`
remains effect-free. Do not hide interpreter execution in JSON parsing.

Distribute standard reusable tools through the Python package. Keep the
model-specific attractor plotter an example; user-authored scripts continue to
use the existing Python task
declaration. Test a separate downstream project against an unpacked `.crate`
without repository paths or checkout `PYTHONPATH` assistance, including a
missing-NumPy case and a configured virtual environment.

### 4. UI colors, spacing, and pause/resume

**Colors and layout:** [ui/state.rs](rust/src/ui/state.rs) retains plain strings
for messages, and `render_messages` in [ui/terminal.rs](rust/src/ui/terminal.rs)
adds no severity styling. Store structured messages with level, source, time,
and text. Use muted debug, neutral info, yellow warning, red error, and green
success styling, with textual labels so meaning survives plain output.

The Study panel has a fixed height of four, leaving two dense content rows.
Split identity/elapsed, status counts, and output location into spaced rows or
responsive columns. Give long output paths their own row. Adapt panel heights
to small terminals and select visible messages by wrapped display lines so a
long older message cannot push the newest message out of view.

There is also a semantic issue: `DashboardState::snapshot` filters tasks to the
most recently started `(replicate, phase)`, and the Study header computes its
counts from that filtered list. Parallel replicates can therefore be running
outside the visible counts. Compute study totals across all tasks and label
phase-local counts explicitly.

**Accepted decision 6:** use one combined, scrollable task view grouped by
replicate and phase, with active groups in stable declaration order. Preserve
selection and scroll position as events arrive. Completed groups disappear
entirely from the task panel; do not retain collapsed summaries or expandable
completed groups. Report their outcomes in Messages before removing them,
with replicate/phase identity, outcome counts, and useful output or log paths.
Messages identify their source task where applicable. Keep study-wide totals
inclusive of completed work even after its groups disappear. Provide message
scrolling so users can inspect retained outcomes; make any history limit clear.
Do not let event arrival order choose the only visible scope.

**Pause/resume:** [ui/command.rs](rust/src/ui/command.rs) supports only `exit`,
`exit --force`, and interrupt. Runtime's control port is cancellation-only.
Add Runtime-owned running/pause-requested/paused/cancelling states and `pause`
and `resume` commands. Stop new admission and pause execution units at safe
boundaries between complete steps; let persistence drain accepted records.
Never claim the run is paused while a step or uncooperative program is still
executing. Resume should continue the same in-memory run, not create a new
execution or imply checkpoint recovery after process exit.

For Workflow's standard Python tools, add cooperative pause/cancel checkpoints to the
parent and workers, using a separate control channel from diagnostic events.
For arbitrary external programs, initially define pause as stopping admission
while active programs finish, with an explicit “pausing; waiting for program”
status. Supporting immediate suspension of arbitrary process trees is a
separate platform-specific feature.

Use a wakeable control primitive; cancellation must wake paused workers.
**Accepted decision 7:** all execution timers stop immediately when pause is
requested and resume together on resume. This includes task/phase elapsed
time, timeout budgets, scheduling-delay countdowns, and ETA, even while a step
or external program is still running toward a pause boundary. Do not wait for
individual task or whole-phase pause acknowledgements to freeze these clocks.
No task or phase consumes timeout budget during the pause interval.

Add a clearly labeled **Total time** master clock in the Study panel. It
measures elapsed wall time from Workflow startup and keeps advancing throughout
pause requests and acknowledged pauses. Separate this clock from all pausable
execution timing; resuming must neither reset it nor add pause time back into
execution budgets. Keep actual task status honest while work reaches safe
boundaries, and keep input, rendering, log draining, and cancellation responsive.
Current deadlines use elapsed wall time in
[runtime/execution.rs](rust/src/runtime/execution.rs), so changing displayed
values alone is insufficient: scheduling and timeout calculations must use
the same pause-aware execution clock.

**Acceptance:** repeated commands, pause during steps and conversion, resume
without duplicated records, exit while paused, failure during pause, timeout
accounting, concurrent replicates, wrapped messages, narrow-terminal rendering,
headless operation, and terminal restoration. Use buffer-render tests and a
manual PTY check for the layout changes.

### 5. Additional architectural, efficiency, and release findings

- **Process-tree cleanup is a prerequisite for multiprocessing.** Runtime
  currently calls `child.kill()` and `child.wait()` on the immediate process.
  That does not establish cleanup of environment-manager descendants or future
  converter workers. For the accepted Linux-only scope, add process-group
  ownership and cooperative shutdown
  with a bounded escalation path, and retain resource permits until descendants
  are stopped. `exit --force` currently exits the Rust process directly; include
  its child-cleanup semantics in this work.

- **Accepted decision 8: Linux is the supported platform for this pass.**
  Prominently state **Workflow currently supports Linux only. macOS and native
  Windows support are planned future work and are not supported or validated
  by this release** in the repository and Rust READMEs, with consistent scope
  in the Python setup guide and architecture/test documentation. Use Linux
  setup examples and qualification tests. Keep platform-specific process and
  filesystem operations behind narrow internal boundaries. Add future-support
  annotations at those boundaries during implementation, pointing to the
  macOS and Windows work listed under decision 8. Do not claim non-Linux support
  merely because parts compile or run there.

- **Order task-start events before worker progress.** `spawn_task` starts the
  worker before publishing `TaskStarted`. A fast worker can publish progress
  first. This is a source-level ordering risk, not a reproduced test failure.
  Use a startup handshake and handle publication failure without detaching a
  worker. Add an observer test that deliberately exercises fast startup.

- **Measure converter memory before raising concurrency.** `_FieldScan`
  retains per-record numeric metadata, projection metadata, empty-path sets,
  and JSON lengths. `_NumericPlan` keeps a shape per record even for fixed
  fields. `reader._read_chunk` holds chunk bytes, split lines, and decoded
  records. Memory-mapped output does not make total conversion memory constant.
  Compact repeated shape metadata and spill large planning tables if needed.
  Both `_stream_plan` and the write pass verify and decode the source; output
  hashing and reopening add further reads. Benchmark these stages separately
  before removing work, and preserve the existing integrity-before-publication
  guarantees.

- **Reduce presentation overhead on hot paths.** Each member observation
  publishes progress synchronously through the UI state mutex. Coalesce by
  task and refresh interval, while always publishing final progress. Snapshot
  creation also clones all task rows before filtering to the active phase;
  filter before cloning and maintain aggregate counters separately. Benchmark
  short steps, many members, and large parameter sweeps.

- **Track aggregate persistence costs.** Persistence creates one writer
  thread per member, with per-stream byte limits. These useful local bounds
  still multiply with members and tasks. Measure total threads, queued bytes,
  and chunk buffers on large ensembles before considering a shared writer pool
  or study-wide storage budget. Preserve per-recording FIFO order and failure
  isolation if changing ownership.

- **Fix Python CI dependency setup.** The Python job in
  [.github/workflows/ci.yml](.github/workflows/ci.yml) runs test discovery without
  installing the `npy` extra, while `test_npy.py` imports NumPy unconditionally.
  Install `./python[npy]` for converter tests. Separately test the core reader
  without optional dependencies, and test an installed wheel outside the source
  checkout. Local passing tests do not prove a clean CI environment works.

- **Synchronize the release baseline.** `rust/Cargo.toml` and
  [compatibility.json](protocol/compatibility.json) identify Rust 0.13.4, while
  README installation examples, architecture/test guides, the attractor guide,
  and the latest changelog entry still identify 0.13.3. Decide whether 0.13.4
  is the intended upcoming or released baseline and align the affected claims.
  The documented Python Git pin was inspected locally and does contain reader
  0.4.2; no pin-version mismatch was found.

### Suggested implementation order and validation

1. Correct CI setup, release-documentation drift, startup event ordering, and
   process-tree ownership.
2. Add the Python launch/package contract and early prerequisite diagnostics.
3. Add live program logs/progress, severity-aware messages, and the Study
   layout/count fixes.
4. Benchmark conversion; implement bounded member multiprocessing with
   deterministic publication and resource-aware cancellation.
5. Add pause/resume across Runtime, execution units, and standard Python tools.

Keep runtime control and events owned by Runtime, durable program logs owned
by Persistence, presentation owned by UI, and conversion semantics owned by
the Python converter. Update the owning `api.md` files, architecture guide,
Python/Rust guides, examples, and test map with each implementation. Change
the NPY or recording protocol version only if its stored contract changes;
internal scheduling or message colors alone do not require a format bump.

Validation performed for this scan:

- `cargo test --workspace --all-targets --all-features --locked --offline --quiet`
  passed: 114 tests across the workspace test targets.
- `PYTHONPATH=python/src python3 -m unittest discover -s python/tests -q`
  passed: 22 tests.
- No performance benchmark, fresh-environment package installation, process-tree
  cancellation reproduction, or interactive terminal validation was performed.
  Performance improvements above are hypotheses to measure; source-level
  limitations are identified separately from passing existing coverage.

## Decision discussion — 2026-09-05

The user accepted the logging/progress, UI/pause, and additional findings in
sections 2, 4, and 5 as the direction of work. Decision 2 below replaces the
original bundling recommendation with a separately installed Python package
and README setup instructions. Discuss remaining policy choices one per turn.

1. **Python baseline — accepted.** Require Python 3.14 or newer for Workflow's
   reader and converter. Update package metadata, installation guides, Runtime
   prerequisite checks, and CI during implementation. Prominently warn that
   older versions are unsupported and recommend the latest stable patch
   release. User-authored Python tasks may retain their own interpreter
   requirements. This is a recorded decision; implementation is still pending.
2. **Rust/Python integration — accepted: A.** Keep Workflow's Python tools in
   a separately installed package. Instruct users to set up NPY in the READMEs,
   including the environment, Python baseline, compatible package with its
   NumPy extra, and verification. Users provision the environment; Workflow
   checks and uses it. No bundled sources or automatic installation in this pass.
3. **Environment setup and selection — accepted: active environment.** Setup
   is user-managed. `$npy` uses the launch environment's interpreter, with early
   version/import validation. Prominently remind users to activate that
   environment before launching Workflow, including in new shell sessions.
   Bold important requirements and use firm wording. No new `$npy`-specific
   environment override is planned for this pass.
4. **Worker allocation — accepted: automatic.** Use
   `min(study_thread_budget, recording_count)` workers and reserve that allowance
   in the shared Runtime budget before launch. Concurrent replicates must not
   exceed the combined budget; a full-budget converter makes other compute work
   wait. Keep serial execution for one worker and add no separate NPY worker
   setting initially. Benchmark memory use before finalizing implementation.
5. **Logging failure behavior — accepted.** Required stdout/stderr log-write
   or unexpected pipe-read failures fail the affected task under the configured
   failure policy. Malformed/unsupported progress frames are preserved and
   warned about without failing scientific work; excess display updates are
   coalesced. Dashboard renderer/input failure retains the existing run-failure
   and cleanup behavior. If failure metadata cannot be written, report through
   any functioning output channel and return an error.
6. **Concurrent-task UI layout — accepted, with completed groups removed.**
   Show a combined, scrollable view of active replicate/phase groups in stable
   order. Completed groups disappear entirely, with their outcomes reported in
   Messages; do not show collapsed completed summaries. Retain accurate
   study-wide totals, identify message sources, and preserve navigation state.
7. **Pause-time accounting — accepted: all execution timers freeze.** Stop
   task/phase elapsed clocks, timeout budgets, scheduling-delay countdowns, and
   ETA immediately on pause request, including during partial pause. Resume
   them together without charging paused time. Add a master **Total time**
   clock in the Study panel that continuously counts wall time through pauses.
   This supersedes the proposed per-task/phase acknowledgement-based timing.
8. **Process-control platform support — accepted: Linux only for now.**
   Clearly document Linux as the supported Workflow platform in the READMEs
   and related guides. Implement and qualify Linux child-process cleanup in
   this pass. Preserve annotations for future macOS and native Windows work;
   neither platform is part of this pass's support or validation promise.
   Future-work inspection: macOS is a moderate extension of the Unix path;
   Windows needs a broader portability pass, not just process cleanup. The
   current executable resolver checks literal command names without `.exe`
   expansion, and `$npy` resolves `python3`. Persistence opens directories with
   ordinary `File::open`, locks directory handles, and synchronizes directories;
   these operations need an explicit Windows implementation and durability
   review. Windows directory handles require platform-specific opening flags
   ([Microsoft directory-handle documentation](https://learn.microsoft.com/en-us/windows/win32/fileio/obtaining-a-handle-to-a-directory)).
   Many program/timeout tests are Unix-only shell fixtures. Add equivalent
   portable coverage, process-tree tests, NPY mapped-file cleanup tests, and
   terminal verification before claiming Windows support. Unix process groups
   can serve Linux/macOS; Windows needs Job Object lifecycle handling. No native
   macOS or Windows tests were run during this inspection. For a future macOS
   pass, add native CI and verify process groups, multiprocessing, directory
   locking/synchronization, and terminal behavior. For native Windows, also
   implement executable/interpreter discovery and filesystem/process adapters
   before adding a support claim. Leave explicit future-platform annotations
   at the relevant implementation boundaries when those files are changed.
9. **Release versions — pending.** Resolve the 0.13.3/0.13.4 documentation
   baseline and version the planned changes consistently.
