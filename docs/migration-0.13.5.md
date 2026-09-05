# Migrating to Rust 0.13.5 and Python 0.4.3

**THIS PATCH RELEASE CONTAINS BREAKING API CHANGES.** The patch increments were
explicitly requested for this coordinated refactor. Rust dependency selection,
Python imports, Python requirements, and boundary recording compatibility change.
There are no compatibility aliases for the old Python package or namespace.

## 1. Upgrade and activate the environment

Follow [Linux setup](setup.md). Update every direct Rust dependency and application
lockfile to 0.13.5. Replace the Python distribution `scientific-workflow-reader`
with `scientific-workflow` 0.4.3 and keep `[npy]` where conversion/readback is used.
Replace `scientific_workflow_reader` imports with `scientific_workflow`. Git users
pin the coordinated `v0.13.5` tag, including `#subdirectory=python`.

**Python 3.14+ is required. Activate the environment before every Workflow launch.**
`$npy` no longer executes a bundled script using an arbitrary discovered Python;
it imports the installed companion from active `PATH` Python. Rust cannot install
or activate that environment for you.

## 2. Replace manual dependency parsing

`InitializationContext::dependencies()` now returns
`&scientific_workflow::task::dependencies::Dependencies`, replacing `&serde_json::Value`.
To bridge an existing parser temporarily, pass `context.dependencies().raw_json()`.
The preferred replacement validates the contract once and selects explicitly:

```rust,ignore
let source = context.dependencies()
    .recordings()
    .in_phase("initialize")
    .execution_unit("initialize")
    .member("initialization")
    .one()?;
let directory = source.directory(); // &Path
```

`one()` requires exactly one source; `optional()` allows zero or one; `iter()`
returns all selected sources. Multiple matches are always an error for one/optional.
Selectors intersect. Use task identity when phase/unit/member still leave ambiguity.
Unknown kinds and extra metadata remain available through `raw_json()`.
See the [full type and error contract](../rust/src/task/dependencies/api.md).

Python programs use the equivalent interface:

```python
from scientific_workflow.dependencies import Dependencies
from scientific_workflow.npy import open_npy_batch

batch_source = Dependencies.from_env().npy_batches().one()
batch = open_npy_batch(batch_source.directory)
```

Rust callers retain `ExecutionUnit`, `MemberView`, `run`, and `Study` authoring.
Private runtime control adds no required pause methods to scientific units.

## 3. Use standard project accessors

**REQUIRED LAYOUT: `<study>/wf_configs/study.json` and `parameters.json`.
Do not rename or relocate these files.** Rust `task::project` and Python
`scientific_workflow.project` expose focused accessors for project/output paths
and resolved parameters. Program tasks read Workflow's resolved snapshot;
manual parsing of authored parameters can bypass sweeps and overrides.
There is no new context object to build or thread through application code.

Program dependencies point at the producer's `artifacts` directory. Recording
dependencies point at a specific member recording. NPY dependencies point at a
verified batch root. These paths have different meanings; do not interchange them.

## 4. Replace boundary-sampling sentinels

Replace `.every_iterations(u64::MAX)?` used as an initial/final approximation with
`.initial_and_final()`. The first and final observations are retained, with one
record when they refer to the same iteration. Intermediate observations are
omitted even at the maximum iteration. The last sampling configuration call wins.

Recordings using this policy require **format 8 readers**. Periodic-only recording
writers remain on format 7. Upgrade downstream readers before producing boundary
recordings. NPY format stays v2; consumers still hard-coded to v1 already need
migration independently of this refactor.

## 5. Read complete numeric series through the official reader

Keep `open_npy_conversion`/`open_npy_batch`, then call
`conversion.coordinates(stream)` and `conversion.series(stream, field, logical_path)`.
Pure numeric fields omit `logical_path`; structured fields select a manifest-defined
numeric projection. Fixed series expose `.values`; ragged series expose packed
`.data`, `.offsets`, and `.shapes`. Physical times may be `None`.

Views reuse cached, read-only memory maps. Applications retain scientific units,
axis interpretation, plotting, and domain validation. Avoid reopening and
reverifying each file per record, reconstructing the generic NPY layout, or
changing a v1 format constant without adapting its data model.

## 6. Update operational expectations

Converter processes share the study budget across replicates. Pause freezes all
execution budgets while Total time keeps advancing. Long Rust calls and arbitrary
programs remain cooperative boundaries. Completed groups disappear from the table;
Messages include terminal outcomes and phase counts. Complete raw stdout/stderr
logs remain durable, and failure to preserve them fails the task.

For live diagnostics use `scientific_workflow.reporting` explicitly. Regular
stderr is preserved without being interpreted as an event. Its logging adapter
is opt-in, uses standard Python logging, and does not require NumPy.

## Validate the integration

Run the [dependency pipeline](../examples/dependency_pipeline/README.md) for an
executable initialization → simulation → NPY → analysis example. Run downstream
Rust tests with the updated lockfile, then exercise analysis with the installed
Python companion and a real NPY v2 batch. Consult each downstream root
`refactor.md` for repository-specific findings and validation limits.
