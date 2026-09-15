# 10. Running and monitoring

**Outcome:** operate and diagnose a study, including long runs and resource pauses.
Start with a working study; chapter 4 contains the complete parameter tables.

![Scientific Workflow dashboard with task progress and resource usage](../assets/UI-3.png)

## Read the dashboard

| Area | What it tells you |
| --- | --- |
| Study | Wall-clock total time, lifecycle counts, replicates, and phase progress. |
| Tasks | Active groups, per-task status, progress, thread allocation, elapsed time, and ETA. |
| Messages | Recent outcomes and diagnostics; complete history is in execution `log.txt`. |
| Usage | Allocated compute threads and sampled CPU, RAM, and disk usage. |
| Command | Interactive pause, resume, exit, and forced-exit controls. |

Allocation is not utilization: `THREADS 72/72` does not mean every CPU is busy.
The screenshot is an example study, not a performance benchmark.

## Execution and diagnostics

Run the application inside `tmux new -s workflow` or `screen -S workflow`.
The dashboard requires terminal stdin and stderr; redirected or headless runs
are rejected before output creation or cleanup. Detach from the multiplexer
when needed and reattach to interact with the dashboard.

Disk usage pauses work at 95% by default. Free space to at most 93%, then type
`resume` and Enter. Recovery alone never resumes work; the dashboard reminds
you when space is sufficient. Early resume commands are rejected.

The study `threads` budget is shared across tasks and replicates. `$npy` gets at
most `min(study threads, distinct source recordings)` worker processes, with one
native numeric-library thread per worker. The default `auto` mode ramps from one
worker by one every 250 ms of active time. Leave `$npy.mode` and `$npy.threads`
unset unless you need a lower limit to reserve resources for other tasks.
More workers trade memory for throughput; see [qualification measurements](../tests.md).

The dashboard combines active phase groups in plan order. **Completed, failed,
and cancelled groups disappear. Inspect Messages for their outcomes and counts.**
The global summary counts every planned task. Messages retain the latest 100
entries; full program stdout/stderr logs remain in the program task directory.
Page Up/Down scroll tasks. Messages always show the newest entries; read or
follow the complete live history in `<execution>/log.txt`. The Usage section
under Messages reports CPU, RAM, and execution-filesystem disk occupation.
Severity appears in text and color. See [UI controls](../../rust/src/ui/api.md)
for pause and exit keys.

Pause freezes execution timers and timeout budgets immediately. Rust work parks
at initialization/step boundaries; the standard converter acknowledges its safe
points. An arbitrary external program may continue until completion. The Study
**Total time** clock measures wall time and continues throughout every pause.
Ordinary exit waits for cleanup; forced exit restores the terminal and kills
owned process groups, but can leave incomplete recordings.

Raw-log failures are task failures. Invalid diagnostic frames stay in raw logs
and produce bounded warnings; they do not invalidate scientific computation.
Use the opt-in [Python reporting helpers](../../python/src/scientific_workflow/api.md)
for progress and standard logging. Imports do not configure the root logger.

## Run cleanup and timestamps

The ordinary `run(&Path)` facade recognizes `--clean` before an optional `--`
argument delimiter, for example `cargo run -- --clean`. After complete study,
reuse, and Python preflight, it clears `<project-root>/output` before creating
the execution. Other arguments remain application-owned. `runtime::execute`
does not inspect process arguments and does not clean.

All runs hold shared advisory locks on the project and output directories; a
cleaning run holds both exclusively until execution and presentation finish. Cleanup rejects
an output-root symlink/file, a conflicting active run, or a target containing
configuration, a resolved executable/script, a selected reuse source, or any
imported task/member/NPY directory. Child symlinks are removed without following
them. Deletion errors abort startup; already removed entries are not restored.
The output directory itself is preserved. Older Workflow versions do not
participate in this project-lock protocol.

Execution directories use `execution-YYYYMMDDTHHMMSS.nnnnnnnnnZ`, UTC, with an
additional numeric suffix only on collision. Atomic directory creation prevents
reuse even if the clock repeats. Names are invocation identities; scientific
task identity is unchanged. Every physical `log.txt` line receives an absolute
RFC 3339 UTC timestamp when appended, including buffered and multiline messages.
Execution requires the dashboard; launch inside `screen` or `tmux` for long runs.

## Disk guard and NPY resource policy

`study.json` accepts optional `"disk": {"pause_at_percent": 95}`. The default
is 95; a finite numeric threshold must be greater than 0 and at most 100. Use
`null` as the threshold to explicitly bypass the guard. Unknown fields and
invalid values are rejected during Config preflight.

Runtime samples the output filesystem before admitting work and every 250 ms
of wall time. At the threshold it requests a disk pause and tells the user to
free space, then type `resume` and Enter. Usage must reach
`max(0, threshold - 2)` percent before that command can succeed. Recovery alone
never resumes work: a second reminder and the dashboard status indicate when
space is sufficient. The explicit command clears the disk and manual pauses;
an early command is rejected without being queued. The shared active clock
freezes while either pause holds. Cancellation wakes paused work. Sampling or monitor
startup failure produces `RuntimeError::DiskMonitor { path, source }`; an active
monitor failure cancels work and is returned after joining workers. The guard
is stopped before the terminal's final user-input wait.

Execution units pause between host calls; cooperative NPY tasks acknowledge
through control IPC. For non-cooperative Unix programs, disk pause sends SIGSTOP
to the owned process group and recovery sends SIGCONT. Cleanup resumes a stopped
group before terminating it. Disk enforcement is sampled and cooperative calls
may take time to return, so the threshold is a pause trigger, not reserved free
space or a guarantee that in-flight writes cannot fill the filesystem.

The `$npy` phase accepts `"threads": 4` and `"mode": "auto"` (or `"fixed"`).
Leave the worker limit unset unless reserving resources for other tasks requires
a lower limit. Threads default to `study.threads` and must be a positive integer no larger than
that global limit. Mode defaults to `auto`; these fields are invalid on ordinary
phases. Runtime reserves `min(phase.threads, study.threads, recording_count)`
permits through the existing shared budget, including across replicates. Each
conversion worker limits native numerical pools to one thread. Fixed mode admits
workers up to that allowance immediately; auto begins at one and increases the
admission limit by one every 250 ms of active time until the allowance is reached.
Short batches may finish before reaching the limit. This does not measure CPU
or RAM utilization, and the full allowance remains reserved during ramp-up.

Python's `convert_workflow_dependencies` adds the optional keyword
`worker_mode="auto"`; `"fixed"` selects immediate admission. Other values raise
`NpyConversionError` before output creation. The Workflow CLI accepts
`--worker-mode=fixed|auto` with `--workflow-dependencies`; ordinary single-recording
conversion rejects it. Mode does not change result or reuse identity.

## Thread display and task pages

Runtime publishes a complete snapshot of active task allocations, serialized
across replicate schedulers and sampled at most every 50 ms, with immediate
publication after activation and at phase/execution completion. Internal pools
report their actual current Rayon pool sizes, including automatic rebalancing;
external programs and NPY report their reserved compute allowance. One private
working-task registry ties display entries to resource-lease lifetime.

The task table includes a `threads` column. Usage shows `THREADS allocated/budget`
across all running tasks, including tasks on other pages and in other replicates.
These are allocated compute threads, not measured CPU activity or every OS
thread. Paused tasks retain their allocation; pending and completed tasks count
as zero. NPY auto ramp-up reports the reserved allowance. The private
`ThreadAllocations { allocations, budget }` event carries the whole allocation
set so UI never combines partial rebalancing updates into an inflated total.

PageUp/PageDown move by the task table's visible data-row capacity, excluding
borders and its header. The footer no longer replaces a task row. The title
shows the current page and page count. Resizing recalculates capacity and clamps
to a valid page; an existing first-row anchor is retained where possible, and a
disappearing active-group anchor resets to the first page. All rows on a full
page are usable, and the final page may contain fewer rows.

## Choose scheduling deliberately

A phase's `max_concurrency` limits admitted tasks. Root `threads` limits their
combined compute allocation across replicates. `start_interval_ms` spaces task
starts; it does not slow individual model steps. Parallel replicates still share
the same global budget.

Use `fail_fast` to stop sibling work after an error, or `finish_all` to let
remaining sibling tasks or replicates finish. Neither policy turns a failure
into success or permits a consumer to ignore failed prerequisites. Timeouts and
admission intervals use the pause-aware active clock; the Study Total time
continues as wall time.

## After interruption

Inspect task status and raw logs before interpreting artifacts. Incomplete raw
recordings are not completed results. A converter retry may reuse verified
completed members, but a failed batch has no success manifest. Use chapter 11
for study-level reuse; it does not append to interrupted scientific recordings.

---

**Previous:** [9. Python analysis and visualization](9-python-analysis-and-visualization.md) · **Guide index:** [Documentation](../README.md)

**Next:** [11. Reusing results](11-reusing-results.md)
