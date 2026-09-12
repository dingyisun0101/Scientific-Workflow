# Linux setup and operation

> **Release candidate:** Rust 0.15.0 / Python 0.5.0 is prepared but not published.
> Dependency agreement and publication are pending; published examples still
> consume Rust 0.14.2 with Python 0.4.5.

## Required platform and layout

**LINUX IS THE ONLY SUPPORTED PLATFORM. Windows and macOS are future work.**
The runtime owns Linux process groups; conversion uses POSIX file locks. Porting
requires process-tree, locking, terminal, and cancellation qualification.

**Use Rust 1.97+ and Python 3.14+. Python 3.10–3.13 are unsupported for Workflow's
Python utilities and `$npy`. Upgrade before installing.** Arbitrary external
program tasks may use their own interpreter requirements.

**DO NOT RENAME OR MOVE THE REQUIRED CONFIGURATION FILES.** A study root contains
`wf_configs/study.json` and `wf_configs/parameters.json`; state schemas referenced
by the manifest normally live under `wf_configs/states/`. Accessors assume this
layout; they do not search for alternative names. Pass the study root to
`scientific_workflow::run(&Path)`. See the [beginner guide](../rust/getting-started.md)
for the manifest grammar and a first Rust execution unit.

## Install the coordinated release

```sh
# Activate your existing Python 3.14+ environment first.
python -m pip install --upgrade pip
python -m pip install \
  'scientific-workflow[npy] @ git+https://github.com/dingyisun0101/Scientific-Workflow.git@v0.15.0#subdirectory=python'
cargo add scientific-workflow@0.15.0
```

The tag contains Rust 0.15.0 and Python companion 0.5.0. Without `$npy` or NumPy
readback, omit `[npy]` to install the dependency-free Python core. Cargo only
installs Rust dependencies. **Workflow does not create, activate, or populate a
Python environment. You must install the Python package yourself.**

**ACTIVATE THE ENVIRONMENT BEFORE EVERY LAUNCH, INCLUDING EVERY NEW SHELL:**

```sh
# Activate the same existing Python environment; do not stack environments.
python3 -c 'import sys, scientific_workflow, numpy, threadpoolctl; print(sys.executable, scientific_workflow.__version__)'
cargo run --release
```

`$npy` selects `python3` from the active `PATH`, preserving virtual-environment
identity. Preflight verifies Python 3.14+, companion 0.5.0, NumPy, and threadpoolctl
before scientific work begins. A project-local `python3` does not override this
selection. An explicitly configured generic Python program retains its separate
interpreter configuration. See [Config](../rust/src/config/api.md).

For development from a checkout, use `python -m pip install -e './python[npy]'`
in that activated environment, then run
`cargo run -p workflow-dependency-pipeline` from the repository root.

## Execution and diagnostics

The study `threads` budget is shared across tasks and replicates. `$npy` gets at
most `min(study threads, distinct source recordings)` worker processes, with one
native numeric-library thread per worker. There is no user worker-count knob.
More workers trade memory for throughput; see [qualification measurements](tests.md).

The dashboard combines active phase groups in plan order. **Completed, failed,
and cancelled groups disappear. Inspect Messages for their outcomes and counts.**
The global summary counts every planned task. Messages retain the latest 100
entries; full program stdout/stderr logs remain in the program task directory.
Page Up/Down scroll tasks. Messages always show the newest entries; read or
follow the complete live history in `<execution>/log.txt`. The Usage section
under Messages reports CPU, RAM, and execution-filesystem disk occupation.
Severity appears in text and color. See [UI controls](../rust/src/ui/api.md)
for pause and exit keys.

Pause freezes execution timers and timeout budgets immediately. Rust work parks
at initialization/step boundaries; the standard converter acknowledges its safe
points. An arbitrary external program may continue until completion. The Study
**Total time** clock measures wall time and continues throughout every pause.
Ordinary exit waits for cleanup; forced exit restores the terminal and kills
owned process groups, but can leave incomplete recordings.

Raw-log failures are task failures. Invalid diagnostic frames stay in raw logs
and produce bounded warnings; they do not invalidate scientific computation.
Use the opt-in [Python reporting helpers](../python/src/scientific_workflow/api.md)
for progress and standard logging. Imports do not configure the root logger.

## Troubleshooting and reference

- Prerequisite error: activate the correct environment and repeat the import
  command above; verify `python3` resolves inside it and reports 0.5.0.
- Missing dependency: check phase prerequisites and selector filters.
  Ambiguous dependency: add phase/task/member filters; `.optional()` also rejects
  multiple matches. Do not silently choose the first result.
- Missing parameter: use the resolved snapshot accessors inside program tasks;
  raw source JSON does not include resolved sweeps and overrides.
- Interrupted conversion: retry the same output. Verified completed members are
  reused; a failed batch has no success manifest. Remove orphan private staging
  directories only after confirming no converter still owns that destination.
- Reader incompatibility: boundary recordings require format 8 support. Ordinary
  periodic recordings remain format 7; NPY manifests remain v2.

Continue with the [complete Python API](../python/src/scientific_workflow/api.md),
[typed Rust dependencies](../rust/src/task/dependencies/api.md),
[worked pipeline](../examples/dependency_pipeline/README.md), and
[migration instructions](migration-0.13.5.md).

## Stream exclusions in NumPy conversion

```json
"$npy": {
  "after": ["evolve"],
  "exclude_streams": ["checkpoint"]
}
```

Only the reserved `$npy` phase accepts `exclude_streams`. Omission or an empty
list converts all streams. Entries must be distinct, nonempty strings without
surrounding whitespace. Matching is exact and case-sensitive; names absent from
a particular recording have no effect. Exclusions apply to all prerequisite
members, including transitive prerequisites. Excluded chunks are not read or
verified; recording metadata and all included chunks remain verified. Raw
recordings, checkpoint production, member identities, and phase indices do not
change. If every stream is excluded, a valid metadata-only member dataset is
published.

Both member and batch manifests retain a sorted `exclude_streams` list. Legacy
v2 manifests without this field mean no exclusions. Reuse requires identical
filters and source metadata; use a different output directory for a different
selection. Serial and parallel conversion have the same filtering semantics.
No shell interpretation or glob matching is performed.
