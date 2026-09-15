# Instructions for AI agents using Scientific Workflow

**All AI agents must read this file before creating, configuring, running,
analyzing, or modifying a Workflow-based project.** Then read the project's own
instructions and the specific contracts needed for the task. For changes to
Workflow itself, also follow the repository's [maintainer instructions](../AGENTS.md).

These instructions distill integration patterns from Dispatcher and OmniFluid.
They describe reusable approaches, not dependencies on those projects or a
requirement to copy their scientific settings. Use the contracts of the Workflow
version the application actually consumes; older applications may use different
runtime policies or companion versions.

## 1. Start with scientific intent and existing components

Identify the scientific question, independent variables, controls, replicate
count, initialization, completion criteria, required observations, and desired
analysis artifacts. Preserve specified units, algorithms, tolerances, and seeds.
Ask only about missing scientific decisions that cannot be inferred from the
project. Do not silently invent them.

Inspect the existing study JSON, registered execution-unit keys, typed constants,
state providers, analysis scripts, and one working example. Reuse these first.
For a new experiment, prefer in order:

1. Change an existing project's JSON.
2. Add a project instance using existing units and analysis components.
3. Extend a reusable scientific component or add a thin study-owned adapter.
4. Propose a framework change only when its supported contracts cannot express
   the required behavior.

A new parameter combination or report setting is not a reason to generate a new
Rust crate, Python launcher, configuration language, scheduler, or recording
format. New science can require code; routine orchestration should stay in JSON.

## 2. Keep ownership explicit

| Owner | Responsibilities |
| --- | --- |
| Workflow | Parse and capture configuration; expand tasks; compile dependencies; allocate compute; schedule, pause, cancel, and report progress; record member state; verify and convert recordings. |
| Scientific library | Own state semantics, numerical algorithms, scientific validation, and model-specific observations. |
| Study or instance | Define its scientific protocol, preparation recipe, model policy, dependency handoff, measurements, and comparisons. |
| Selector or entry point | Select an existing study and pass its project root to the ordinary runner. |
| Analysis adapter | Add scientific names and interpretation over verified data, without reimplementing storage or conversion. |

A multi-study selector must not accumulate scientific preparation or analysis.
Independent studies need not share an abstraction merely because their code
looks similar. Conversely, instances that differ only in configuration should
use one production runner and registered model.

Use `scientific_workflow::run(&Path)` for ordinary Rust applications. Keep Rust
filesystem parameters as `&Path` or `PathBuf`. Do not manually assemble Runtime,
persistence sessions, output ordinals, or a second execution dashboard.

## 3. Author intent, not generated wiring

Keep the required files at their supported locations:

```text
project/
├── wf_configs/
│   ├── study.json
│   ├── parameters.json
│   └── states/          optional project-owned schemas
└── scripts/             optional thin analysis entry points
```

- `study.json` owns the task graph and operational policy. Declare
  `workflow_schema`, `threads`, `compute`, and nonempty `phases`.
- `parameters.json` owns scientific constants, preparation recipes, sweeps, and
  custom analysis settings. Keep each independent setting in one authoritative
  location; avoid redundant values that can disagree.
- A unit's registered key selects its same-named local constants section.
  Other top-level parameter keys are shared globally.
- Project-owned schemas are registered in `paths.states` and live beneath
  `wf_configs/`. Omit task `state` only when a linked standard provider exists.
- Use strict JSON and strict typed constants, normally with
  `#[serde(deny_unknown_fields)]`. Do not hide typos behind permissive parsing.

Generated inputs from an earlier phase belong in that task's output and a
study-owned handoff manifest. Do not paste generated absolute paths, hashes,
artifact descriptors, dependency references, or task ordinals into authored
parameters. Intentionally supplied external datasets are different: select them
through the model's documented input contract and retain their provenance.

Do not rewrite authored JSON during a run to inject preparation results. Workflow
has already captured the configuration. Programs read resolved snapshots using
[project accessors](../rust/src/task/api.md), not raw source parameter files.

## 4. Distinguish global sweeps, local tasks, and members

Choose the expansion level before editing JSON:

| Scientific intent | Representation |
| --- | --- |
| Repeat the complete protocol for a shared parameter | A global `$sweep` outside registered unit sections. |
| Vary one model while retaining common preparation | A local `$sweep` inside that unit's constants. |
| Couple specific parameter choices | Terminal `$cases`, or explicit complete alternatives under `$sweep`. |
| Baseline plus independent axes inside an optional feature | A `$sweep` containing `null` and an object with nested axes, when the model accepts that optional field. |
| Several states deliberately sharing one scientific lifecycle or random stream | Members of an existing ensemble contract. |
| Independent repeated experiments | Workflow `replicates`, with the model's documented seed derivation. |

Ordinary arrays are literal model values. They do not create Workflow tasks.
A model may interpret an array as ensemble members; that is model semantics,
not another Workflow expansion rule. Do not turn members into independent tasks
if doing so changes shared randomness, initialization, or the comparison design.
Do not force independent models into an ensemble solely to reduce task count.

A local evolution sweep can consume one shared initialization checkpoint. A
global sweep duplicates the phase graph and can change that pairing. Validate
scope explicitly before promising paired controls.

Count global configurations, local task choices, replicates, and members
separately. `$npy` creates one aggregate task per replicate across prerequisite
recordings. Preserve declaration order and full-precision parameter identities.
Do not add duplicate baseline cases or an artificial Boolean control axis.

See [sweeps and replicates](guide/7-parameter-sweeps-and-replicates.md) for
Cartesian ordering, nested alternatives, and the restrictions on `$cases`.

## 5. Use native phases and typed dependencies

Common patterns include:

```text
prepare → simulate → scientific export → $npy
initialize → evolve → $npy → measure → compare → report
prepare → reference model → derive new inputs → target model → export → $npy
```

These are examples, not fixed phase names. Declare the actual producer phases
in `after`; do not sequence them with shell scripts or subprocess loops.
Independent animation and measurement tasks can both depend on `$npy`; a report
can depend on both plus a comparison task.

Use Rust `InitializationContext::dependencies()` or
`Dependencies::from_env()`, and Python `Dependencies.from_env()`. Select the
producer by phase, task, execution unit, and member as appropriate:

- `.one()` means exactly one result; absence and ambiguity are errors.
- `.optional()` permits absence but still rejects ambiguity.
- `.iter()` means intentional aggregation of all selected results.

Do not scan output trees, parse human-readable logs, or select the newest
execution by modification time inside Workflow tasks. Do not guess a recording
path from a model name. Global configuration correlation does not eliminate
ambiguity among multiple local producers.

Read preparation manifests from the selected program dependency's artifact
directory. Read checkpoints through the verified recording API. Validate the
handoff's format, dimensions, and scientific invariants before constructing the
consumer's state. Reject incompatible data rather than selecting the first file.

## 6. Add adapters only at scientific boundaries

If a model already accepts the desired JSON and dependencies, register and use
it directly. Do not introduce an instance-specific wrapper for each sweep.

A thin adapter is appropriate when an earlier phase creates inputs that are not
available during configuration loading. Its authored constants contain recipes
and model policy. Its `preflight` validates known facts and declares observations
without reading nonexistent future output. Its `initialize` selects the generated
handoff, validates the newly available facts, constructs the underlying model,
and delegates member access and stepping.

Preserve the model's standard schema provider, state ownership, completion,
observation meaning, and member identities. Do not modify an upstream model to
teach it a private study's manifest. Upstream crate changes require explicit
user authorization; do not substitute a local patch for a published dependency.

## 7. Make seeds and pairing scientifically meaningful

Use the study master seed and purpose-named derived seeds. Rust units request
seeds through their initialization context. Program/Python tasks declare
`seed.purpose` and consume `WORKFLOW_TASK_SEED`. Domain-specific child derivations
must use stable semantic identities and record their actual seeds and derivation
provenance. Do not fall back to host entropy or reseed from task start order.

Reusing one completed initialization can establish identical starting state.
It does not imply identical stochastic evolution: separate tasks can receive
different derived seeds even with the same master seed. Verify initial state,
model settings, coordinates, and required random streams before labeling an
analysis paired or claiming exact replay.

Checkpoint continuation is a scientific model capability, distinct from reusing
completed Workflow phases. It may require RNG state or recorded seeds, iteration,
physical time, topology, and other model state. Do not claim restart support from
the existence of a file named `checkpoint` alone.

## 8. Record once and consume verified data

Models declare state fields and observation plans; Workflow owns recording I/O.
Record only the scientific streams required for analysis and recovery. Use
boundary sampling for initialization/final checkpoints where appropriate, and
periodic streams for evolving observations. Observation cadence must not silently
change dynamics or random-number consumption.

Use the task-free `$npy` phase for standard conversion. Never add a second
converter, manually interpret raw chunks, reconstruct dtype maps from filenames,
or bypass checksums. Read batches with `open_npy_batch` and retain the verified
conversion object in domain adapters.

Use numeric series/projections for numerical work; use verified decoded records
when the scientific operation requires them. Preserve fixed/ragged shapes,
offsets, dtypes, empty arrays, and stream coordinates. Do not repeatedly reopen,
verify, or decode a complete recording for every field or animation frame.

`exclude_streams` affects conversion only. Keep raw recording policy separate.
If topology or checkpoints were excluded, mark dependent metrics unavailable or
reject the analysis that requires them. Do not report absent data as zero or
silently reopen raw checkpoints to defeat the selected conversion boundary.

## 9. Separate measurement, comparison, and presentation

- **Measurement** consumes verified scientific data and publishes numerical
  results with units, definitions, source identities, and coordinates.
- **Comparison** validates baseline/variant pairing, aligns coordinates, and
  computes scientific differences without depending on plotting code.
- **Presentation** consumes those products to render figures, videos, or HTML.

Use thin project scripts that call reusable scientific analysis modules. Read
settings through `scientific_workflow.project.parameters`, obtain artifacts
through project/dependency accessors, and report progress through
`scientific_workflow.reporting`. Do not implement a custom progress protocol.

Align different streams by recorded iteration or physical time, never by row
number alone. State explicitly whether an observation at iteration n describes
state n or the transition n → n+1. Prefer recorded diagnostic fields over
re-evaluating today's model configuration to depict a historical run.

Keep scales and case ordering consistent across comparisons. Playback FPS is
not simulation time and does not authorize dropping recorded frames. Preserve
requested frames, units, coordinate transforms, and numerical results when
optimizing a renderer. Label missing observations and incomplete grids explicitly.

## 10. Preserve outputs and provenance

Default to the task artifact directory from project accessors. If a project
publishes a convenient analysis hierarchy, isolate it by execution, replicate,
and task/stage, and preserve the authoritative dependency path. Keep this
publication policy application-owned; it is not a new Workflow output setting.
Never move or rewrite Workflow's raw recordings, captured snapshots, receipts,
or conversion manifests to manufacture a successful or reusable result.

Version scientific handoff, measurement, and report formats when their meaning
changes. Preserve source task/member identities, resolved scientific settings,
seeds, coordinates, and relevant verification evidence. Use explicit migration
for old inputs; do not silently reinterpret old data under a new schema.

Keep generated plots, videos, and executed notebooks out of source control
unless the project explicitly tracks them as fixtures or documentation assets.
Preserve input assets. Keep source notebooks clean and save executed copies
separately according to the project's convention.

For new project names, follow the existing local convention. Structured names
may use hyphens within words, underscores between fields, and `key=value`
qualifiers. Such names never replace Workflow's physical output identities.
Numeric prefixes in phase names are labels; `active_phases` uses dependency-order
indices, not the number written in a phase name.

## 11. Respect resource and runtime policy

Use the application's installed release and coordinated Python companion.
Activate the project's chosen environment before launching; do not stack a new
virtual environment on an existing one or assume Cargo installs Python.

Use the root thread budget and declared compute mode. Automatic unit allocation
requires a truthful `THREAD_COUNT_INVARIANT` promise. Isolated units need fixed
resources. Do not create competing pools around runs or infer scientific
parallelism from dashboard thread allocations. Use phase concurrency and start
intervals to control admission; retain the default NPY worker policy unless the
study needs an explicit limit.

Estimate retained data and peak record size before large runs. A single large
state record must fit the recording queue's supported limits; a chunk-size target
does not divide a scientific record. Per-stream queues, payloads, converter
workers, and analysis arrays all contribute to memory. Use a small scientific
run plus a representative-size record check when scale makes this material.

Workflow 0.15 requires an interactive dashboard; use `screen` or `tmux` for long
runs. The default disk pause requires freeing space and explicitly typing
`resume`. Do not disable protection to hide a storage-planning problem.

Omit `active_phases` for a full run. Use supported selection and `reuse_from`
for compatible completed prerequisites. Do not add custom `--skip-simulation`
or filename-existence shortcuts. Resource changes can preserve reuse; scientific
inputs and relevant conversion filters must satisfy the documented compatibility
checks. Never assume changing analysis JSON or a script guarantees reuse.
`--clean` removes all output contents after guarded preflight, not just one run.

## 12. Validate the claim, then report the evidence

For configuration and integration work:

1. Parse JSON and load the study with the same linked registrations as the runner.
2. Verify phase order, task counts, scope, model constants, member grouping, and
   dependency selection. A syntax-only parser is insufficient.
3. Run a bounded copy of the project in a real terminal, retaining its graph and
   scientific features. Reduce size and duration in the copy, not production
   configurations or saved results.
4. Verify completion and selected scientific values through supported readers.
   Check the relevant handoff, NPY content, and analysis artifacts.
5. For changes to randomness, observation, or compute allocation, test the
   applicable replay, pairing, cadence-independence, or worker-count invariants.
6. For scientific changes, add the relevant analytical, conservation, convergence,
   or statistical check. A successful pipeline does not validate the physics.

Use proportional validation: a prose/link change needs documentation checks;
a scientific implementation change needs scientific evidence. Do not execute a
production-scale study merely to check a configuration edit. Do not claim a small
run proves full-scale memory, storage, runtime, or scientific convergence.

In the handoff, state what changed, task/member counts when relevant, versions,
checks and measured results, artifact locations, and unresolved limits. Update
associated documentation and keep completed evidence distinguishable from plans.

## Read only the references needed next

| Task | Reference |
| --- | --- |
| First integration | [First study](guide/3-first-study.md), [scientific models](guide/5-scientific-models.md) |
| JSON field, default, or validation | [Configuration guide](guide/4-project-configuration.md), [Config contract](../rust/src/config/api.md) |
| Scope, cases, and replicates | [Sweep guide](guide/7-parameter-sweeps-and-replicates.md) |
| State and sampling | [Recording guide](guide/6-state-and-recording.md), [State](../rust/src/state/api.md), [Observation](../rust/src/observation/api.md) |
| Generated inputs and checkpoints | [Dependency guide](guide/8-phases-and-dependencies.md), [typed dependencies](../rust/src/task/dependencies/api.md) |
| Python, NPY, and analysis | [Analysis guide](guide/9-python-analysis-and-visualization.md), [Python API](../python/src/scientific_workflow/api.md) |
| Run control, resources, and reuse | [Operations](guide/10-running-and-monitoring.md), [reuse](guide/11-reusing-results.md) |
| Verbal request to JSON | [AI-assisted study example](guide/12-ai-assisted-studies.md) |
| Framework maintenance | [Architecture](architecture.md), [tests](tests.md), owning subsystem `api.md` |

Load the relevant model contract and one working example before exploring
implementation details. Keep project instructions linked to this file so agents
can find the authoritative guidance without rereading unrelated source trees.
