# 9. Python analysis and visualization

**Outcome:** turn the first population study into a simulation-to-figure pipeline.
Install the NumPy companion as described in chapter 2. This chapter adds a
Python plotting component once; subsequent supported changes use JSON.

## Extend the first study

Retain the chapter 3 model and state schema. Use this complete `study.json`:

```json
{
  "workflow_schema": 1,
  "threads": 2,
  "compute": {"mode": "auto"},
  "paths": {"states": {"population": "wf_configs/states/population.json"}},
  "phases": {
    "simulate": {
      "tasks": [{"execution_unit": "population", "state": "population"}],
      "max_concurrency": 2
    },
    "$npy": {"after": ["simulate"]},
    "plot": {
      "after": ["$npy"],
      "tasks": [{"python": {
        "script": "scripts/plot.py",
        "environment": {"manager": "system"}
      }}]
    }
  }
}
```

Omitting the earlier `active_phases: [0]` is intentional: run all three phases.
Use this complete `parameters.json`:

```json
{
  "population": {
    "initial_population": {"$sweep": [10, 20]},
    "steps": 5
  },
  "plot": {"filename": "population.svg", "dpi": 180}
}
```

## Add the plotting component

In the activated environment, install the plotting library and create the script
directory:

```sh
python -m pip install matplotlib
mkdir -p scripts
```

Save the following as `scripts/plot.py`:

```python
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
from scientific_workflow.dependencies import Dependencies
from scientific_workflow.npy import open_npy_batch
from scientific_workflow.project import output_directory, parameters

settings = parameters("plot")
batch = open_npy_batch(Dependencies.from_env().npy_batches().one().directory)
fig, ax = plt.subplots()
for member in batch.members:
    if member.execution_unit != "population":
        continue
    series = member.series("state", "population")
    values = [int(series.record(i).item()) for i in range(len(series))]
    ax.plot(series.iterations, values, label=f"initial population {values[0]}")
ax.set(xlabel="Iteration", ylabel="Population")
ax.legend()
fig.savefig(output_directory() / settings["filename"], dpi=settings["dpi"])
plt.close(fig)
```

Run `cargo run` from the project root inside the terminal session. Expect two
simulation tasks, one aggregate conversion task, and one plotting task. The
plot task's `artifacts/population.svg` contains lines from 10 to 15 and 20 to 25.
Type `exit` after completion. If you changed stream names in chapter 6, either
restore the default observation plan for this example or update `member.series`
to request the corresponding stream and field.

The script reads resolved settings from Workflow's snapshot and finds the batch
through dependencies. It never parses raw source JSON or guesses task paths.
`output_directory()` provides the task's artifact directory, not the project root.

## Python environments and arbitrary programs

`python.environment` is required. Use `system` for the selected interpreter,
`venv` with a directory path, `mamba` or `conda` with an environment name, or
`uv` or `poetry` with a project directory. The complete manager fields are in
[chapter 4](4-project-configuration.md#python-task-environments).
The standard `$npy` converter always uses the activated `python3` from `PATH`;
it does not inherit a plotting task's manager declaration.

An ordinary executable task has a different shape:

```json
{"program": "bin/analyze", "args": ["--publication"], "resources": {"threads": 2}}
```

The executable must exist and be executable. Arguments are passed directly:
`$HOME`, pipes, and wildcard characters are not shell expressions. Python script
arguments instead belong in `python.args`.

External tasks run in isolated `artifacts/` working directories. Workflow supplies
resolved config/dependency snapshots and `WORKFLOW_*` path variables, captures
stdout/stderr, and records exit status. Use project helpers for paths rather
than relying on the current directory being the application root.

## NumPy settings and exclusions

`$npy` takes prerequisite phases, not authored converter tasks. It converts all
transitive execution-unit recordings, ignoring prerequisite program workspaces.
It uses one aggregate task per replicate. Automatic worker admission ramps up
within the shared budget; chapter 10 explains allocation and memory tradeoffs.

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

## Reading without conversion

For simple scalar inspection, the Python core reader can read completed raw
recordings directly, as shown in chapters 3 and 6. NumPy conversion adds
manifest-directed array representations and reusable batch access. Preserve the
verified conversion object when repeatedly accessing a series.

For a larger model and complete visualization, see the
[attractor example](../../examples/attractor_2d/README.md). For all reader and
conversion APIs, see the [Python reference](../../python/src/scientific_workflow/api.md).

---

**Previous:** [8. Phases and dependencies](8-phases-and-dependencies.md) · **Guide index:** [Documentation](../README.md)

**Next:** [10. Running and monitoring](10-running-and-monitoring.md)
