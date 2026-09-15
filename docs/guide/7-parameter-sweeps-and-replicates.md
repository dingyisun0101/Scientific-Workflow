# 7. Parameter sweeps and replicates

**Outcome:** expand experiments through JSON without writing loops that assemble
parameter products. Start with chapter 3's population project; its `Constants`
accepts `initial_population` and `steps`.

## Two independent axes

Replace `wf_configs/parameters.json` with:

```json
{
  "population": {
    "initial_population": {"$sweep": [10, 20]},
    "steps": {"$sweep": [5, 10, 20]}
  }
}
```

The one population task declaration now becomes six tasks:

| Task order | Initial population | Steps | Final population |
| --- | --- | --- | --- |
| 1 | 10 | 5 | 15 |
| 2 | 10 | 10 | 20 |
| 3 | 10 | 20 | 30 |
| 4 | 20 | 5 | 25 |
| 5 | 20 | 10 | 30 |
| 6 | 20 | 20 | 40 |

The first declared axis varies slowest. To admit two tasks together, set the
`simulate` phase's `max_concurrency` to 2 and choose an adequate root thread
budget. Expansion defines the work; scheduling decides when it runs.

## Correlated cases

Use cases when two choices belong together rather than forming all combinations:

```json
{
  "population": {
    "$cases": [
      {"initial_population": 10, "steps": 5},
      {"initial_population": 20, "steps": 10}
    ]
  }
}
```

This creates two tasks, with final populations 15 and 30. Cases must have the
same flattened field set. Fixed siblings cannot overlap those fields. A `$cases`
object is terminal: its fixed siblings and alternatives cannot contain further
selection markers.

## Nested alternatives

For a model whose constants support an optional `noise` object:

```json
{
  "model": {
    "steps": 1000,
    "noise": {
      "$sweep": [
        null,
        {
          "size": {"$sweep": [2, 4]},
          "strength": {"$sweep": [0.1, 0.8]}
        }
      ]
    }
  }
}
```

This creates five local choices: one no-noise baseline and four grid points.
It is a configuration pattern for a compatible model, not an extra field the
chapter 3 population model accepts. That model would correctly reject it.

## Local versus global scope

A top-level parameter section matching a selected execution-unit key is local
to that unit. Other top-level parameters are global. Local choices duplicate
only that unit's task; global choices instantiate the phase graph per global
configuration. Programs receive resolved global settings through snapshots.

For example, two global `temperature` choices and three local population
choices create six population tasks, plus one copy of each ordinary program
task per global configuration. `$npy` aggregates prerequisite recordings across
global configurations once per replicate. Adding a global value does not
silently insert it into a unit's `Constants`; use the documented project/context
accessors if the model needs shared settings.

## Replicates and seeds

Add these root settings to `study.json`:

```json
"seed": 42,
"replicates": {
  "count": 5,
  "scheduling": "parallel",
  "failure_policy": "finish_all"
}
```

This is a fragment to merge into the root object. The six-task population sweep
now has 30 scientific tasks across five replicates, sharing the global thread
budget. The deterministic population model gives identical results for matching
parameters: a master seed does not make a deterministic model random.

Stochastic execution units explicitly request purpose-named seeds through their
initialization context. Program/Python tasks instead declare a task-level
`"seed":{"purpose":"sampling"}` and receive `WORKFLOW_TASK_SEED`.
That request requires the root master seed. Derived values remain stable across
scheduling changes; changing scientific inputs can change scientific identity.

## Common mistakes

- `[10, 20]` is a literal array, not a sweep. Use `{"$sweep":[10,20]}`.
- A `$sweep` object cannot have sibling properties, and choices cannot be empty.
- Do not place nested selection markers inside `$cases` or hide them in arrays
  within sweep alternatives.
- Keep a unit's constants under its registered key, not a phase name.
- Count global choices × local choices × replicates before launching a large study.

See the [Config expansion contract](../../rust/src/config/api.md#nested-alternatives-and-independent-axes)
for ordering and validation details.

---

**Previous:** [6. State and recording](6-state-and-recording.md) · **Guide index:** [Documentation](../README.md)

**Next:** [8. Phases and dependencies](8-phases-and-dependencies.md)
