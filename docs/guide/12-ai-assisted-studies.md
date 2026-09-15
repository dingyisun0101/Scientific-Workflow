# 12. AI-assisted studies

**Outcome:** describe routine experiments in ordinary language and have a coding
agent translate them into project JSON using documented, integrated components.
You need a project that already builds and a successful small study.

**All AI agents must read [the Workflow agent instructions](../AGENTS.md)
before working on the project.**

## Set up once

Give the agent a compact project map:

- Required `wf_configs/study.json` and `parameters.json` paths.
- Registered execution-unit keys, their constants, and supported state schemas.
- Existing analysis scripts, their settings, inputs, and outputs.
- A known-good small study and the expected scientific result.
- This guide, the [configuration chapter](4-project-configuration.md), and the
  project's own instructions and validation commands.

Document accepted units, ranges, and scientific assumptions beside model
parameters. Workflow validates types and its grammar; it cannot infer the
scientific meaning of a parameter called `rate`.

## A request that becomes JSON

With the chapter 9 project already working, tell the agent:

> Use the existing population model and plotting script. Sweep initial
> population over 10 and 20 and steps over 5, 10, and 20. Run five replicates per
> combination, convert all recordings to NumPy, and plot population against
> iteration. Use four total compute threads. Keep the scientific implementation
> unchanged. Show the expanded task counts and validate the edited configuration.

The parameter change is:

```json
{
  "population": {
    "initial_population": {"$sweep": [10, 20]},
    "steps": {"$sweep": [5, 10, 20]}
  },
  "plot": {"filename": "population.svg", "dpi": 180}
}
```

The agent also sets root `threads` to 4, adds `"replicates":{"count":5}`,
and retains the simulation → NPY → plot phases. Setting simulation
`max_concurrency` to 2 permits two scientific tasks at a time under auto mode.
Omit `active_phases` to select every phase and omit `reuse_from` for a fresh run.

Expect 30 population tasks, five aggregate NPY tasks, and five plotting tasks:
40 planned tasks across five replicates. The existing script creates one figure
per replicate with six population curves. Replicates run sequentially unless
`scheduling` is explicitly set to `parallel`. No new Rust or Python is needed
for this request because all required scientific components already exist.

## Why this can save tokens

A configuration edit is smaller than generating scheduling loops, file I/O,
parameter-product assembly, and subprocess coordination. The agent can read
only the relevant guide and model contract, then modify a few JSON fields.
This reduces code and context it needs to inspect and debug. Actual token savings
depend on the model, the project documentation, and the requested change.

Keep one canonical settings reference and a known-good example available to the
agent. Avoid pasting the complete implementation into every conversation.
When a failure names a specific field or subsystem, supply that error and its
owning contract rather than asking the agent to redesign the workflow.

## Review the generated study

1. Check that execution-unit keys and script paths exist in your project.
2. Count global choices, local combinations, and replicates.
3. Check parameter units, ranges, seeds, and expected outputs.
4. Parse JSON and run application preflight or the documented validation path.
5. Run a bounded study in the dashboard and inspect one verified result before
   expanding the experiment.

A JSON parser catches syntax errors, not scientific mistakes or missing linked
units. Agents should use documented APIs and preserve strict validation rather
than inventing unsupported settings.

## When code is still necessary

A new model, observation policy, payload type, or analysis algorithm can require
scientific implementation work. Expose reusable choices through model constants
and script settings so later studies return to JSON edits. Do not promise that
an arbitrary verbal request can always be expressed by existing components.

The [agent instructions](../AGENTS.md) defines where to look for APIs and
how to keep application work within supported extension points.

---

**Previous:** [11. Reusing results](11-reusing-results.md) · **Guide index:** [Documentation](../README.md)

**Next:** [Worked examples](../examples.md) · [Reference documentation](../README.md#reference)
