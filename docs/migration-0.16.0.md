# Rust 0.16.0 / Python 0.6.0 migration

This release supersedes Rust 0.15.x / Python 0.5.x. Scientific execution-unit
APIs, raw recording formats 7/8, and `workflow_schema: 1` remain supported.
Task reuse requires current verified receipts, and standard conversion produces
NPY v3. No legacy receipt adapter or compatibility alias restores old reuse.

## Install the coordinated release

Use current patches Rust `scientific-workflow = "0.16.1"` and Python
`scientific-workflow[npy]==0.6.1` in the environment used by the runner.
The migration contract originated in 0.16.0 / 0.6.0 and is unchanged. Rust
0.16.1 accepts stable companions `>=0.6,<0.7` for conversion and reuse; the minimum
remains 0.6.0. The initial Rust 0.16.0 release checked exactly Python 0.6.0.
The registration macro remains published version 0.2.1.

## Captured inputs and reuse

Every completed source task owns `workflow-config.json`, `workflow-dependencies.json`, and
`workflow-inputs.json` in its output directory. The input manifest records
canonical filenames and SHA-256 checksums of the exact snapshot bytes.
Unit snapshots resolve their selected local constants alongside correlated
global settings; program snapshots retain the resolved global configuration. Program
metadata `scientific-workflow-program-v2` and completed receipts
`scientific-workflow-task-result.v2` retain the same references.

Program tasks monitor these saved copies during execution. Rust units validate
them at completion boundaries. All task kinds verify them before successful
completion, supported snapshot reads, and reuse. Reused placeholders refer to
the original verified source directory; they do not copy snapshots. Editing source JSON affects future
runs; an active execution retains its captured configuration. Changing saved
snapshot bytes, including whitespace, fails integrity verification.

Integrity and compatibility are separate. Reuse still compares scientific
parameters, schemas, seeds, program/script identities, arguments, replicate
count, and dependencies. Allowed resource, scheduling, phase-selection, and
Python-environment changes do not require equal whole-snapshot hashes.
Executable or scientific-script source contents are not fingerprinted.

Only current receipts qualify for reuse. Rerun required prerequisites when an
older or missing receipt is encountered. Workflow does not reconstruct legacy
completion, migrate historical results, or reinterpret archived data. Downstream
analysis packages own historical result compatibility.

## NPY v3

Member and batch manifests use `scientific-workflow-npy.v3` and
`scientific-workflow-npy-batch.v3`. A scalar series has shape `(records,)`;
an actual length-one vector retains shape `(records, 1)`.

Valid values use direct numeric storage only when a supported dtype preserves
them. Otherwise the existing JSON-byte fallback retains exact values. Such a
field remains reconstructable but has no direct whole-field numeric series.
Explicit numeric envelopes reject incorrect Boolean kinds, invalid integer
values, and floating-point overflow; ordinary rounding to declared float
precision remains valid. Verified readers check coordinate roles and layouts.

Reconvert raw recordings into a new destination for corrected v3 results.
Existing v2 conversions cannot satisfy current conversion reuse and are not
overwritten. Downstream packages such as DSES Analysis must explicitly support v2
and v3 according to their stored semantics. Lost integer precision in an old
conversion can only be recovered from the original data, not its rounded array.

See [the v3 protocol](../protocol/npy-v3.md) for the complete representation
contract and [reuse guidance](guide/11-reusing-results.md) for phase selection.

## Worker startup and dashboard

Conversion follows Python's default or an embedding application's selected
process-start method, with the existing private override retained. On supported
Linux/Python 3.14 this normally uses forkserver. Direct parallel batch calls
from scripts require an import-safe `if __name__ == "__main__":` entry point.
Standard CLI/module entry points provide it. Current control-file configuration
is passed explicitly to workers so pause/cancel remains tied to that batch.

The centered bold title is `SCIENTIFIC WORKFLOW`. CPU usage uses a two-second
rolling average; resources and normal dashboard redraws refresh every second.
Editing, commands, paging, and resize feedback remain responsive. The terminal
application controls font size. Runtime's disk guard remains independent.

`--clean` initializes the required dashboard before deleting prior output.
Startup errors preserve existing results and restore the terminal. Failures
after cleanup starts retain ordinary runtime behavior.

## State cloning and stricter boundaries

`SystemState::clone` and `StateSeries::clone` invoke payload-defined Rust `Clone`.
Payload backing storage may be copied or shared. Independent scientific branches
require payload semantics that isolate subsequent mutations; immutable shared
structures can remain shared safely.

Members must expose distinct, stable state instances. Stored recording paths
reject empty and current-directory components; stored sampling intervals use
tagged objects. Interpreter paths are validated after preserving their symlinks.
Correlated cases compare actual escaped JSON paths, including slash-containing
keys. These checks enforce existing contracts rather than adding new authored
configuration fields.
