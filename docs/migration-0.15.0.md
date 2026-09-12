# Rust 0.15.0 / Python 0.5.0 migration

This is an unreleased coordinated update. The dependency decision and final
publication remain pending. Scientific APIs and recording formats 7/8 are
unchanged; runtime policies and finalized JSON handling change.

## Disk guard

Existing studies default to pausing when the output filesystem is 95% occupied.
Set `"disk": {"pause_at_percent": 90}` to change the threshold, or explicitly
set the value to `null` to bypass monitoring. Recovery automatically releases
the disk pause two percentage points below the threshold, preserving a separate
manual pause. The guard works in headless runs. Sampling failure cancels active
work and returns `RuntimeError::DiskMonitor`.

Disk pause is sampled, not a disk-space reservation. Execution units finish their
current host call before pausing; NPY workers use cooperative checkpoints, and
non-cooperative Unix programs are suspended through their owned process groups.

## NPY resources

The reserved `$npy` phase accepts `threads` bounded by the global study budget
and `mode` equal to `fixed` (default) or `auto`. Auto increases admission from one
worker by one every 250 ms of active time. It targets the worker limit, not CPU
or RAM utilization. Install the coordinated Python 0.5.0 companion with its NPY
extra; older companion CLIs do not support the worker-mode argument.

## Cleanup and execution identity

The ordinary `run(&Path)` facade recognizes `--clean` in process arguments before
an optional `--` delimiter. Application argument parsers must allow that flag to
reach Workflow. Advanced `runtime::execute` does not inspect arguments or clean.
Cleanup removes the complete standard output directory contents after preflight,
with shared/exclusive project and output locks protecting active runs. Symlink
roots and protected input/reuse paths are rejected. Partial deletion is not
rolled back if a filesystem operation fails. Older releases do not participate
in these locks.

Execution names now use `execution-YYYYMMDDTHHMMSS.nnnnnnnnnZ`, with collision
suffixes when necessary. Consumers must use returned paths rather than parse
PID/sequence names. Existing execution paths remain usable for explicit reuse.
Every physical `log.txt` line now begins with an RFC 3339 UTC timestamp.

## Reuse and JSON

Compute allocation, scheduling, timeouts, failure policy, buffering, disk policy,
and Python launcher environments no longer invalidate completed work. Scientific
parameters, schemas, seeds, programs/scripts, arguments, replicate count, and
phase dependencies remain strict. If a resource choice changes scientific
meaning, represent it in scientific parameters.

Source JSON is captured once; editing source files affects subsequent loads.
Generated task input JSON is checked against captured byte digests during
execution and before success. Changed or missing input JSON fails the task.
Active recording metadata and control IPC retain their lifecycle updates;
finalized metadata and successful receipts cannot be replaced through Workflow
writers. Matching NPY batches are verified and returned unchanged. Conflicting
or corrupt batches require a new output directory instead of overwriting the
committed batch manifest. There are no compatibility aliases restoring the old
execution names or finalized JSON rewrite behavior.

## Dashboard

Task rows show allocated threads, and Usage shows the total across all active
replicates against the global budget. This is allocation, not CPU utilization or
the number of OS background threads. Paused tasks retain their reservations.
PageUp/PageDown move by the visible task-page capacity and clamp after resizing.
