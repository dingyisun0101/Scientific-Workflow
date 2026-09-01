# Two-dimensional attractor study

This example is the release-qualified end-to-end project for
`scientific-workflow` 0.13.1 and `scientific-workflow-reader` 0.4.0.

This is a complete small scientific project rather than a collection of API
fragments. Rust owns the stateful Hopf model, JSON owns the study and all
configuration, Workflow's standard `$npy` phase converts the completed
recordings, and a final Python task turns those arrays into an SVG figure.

```text
attractor_2d/
├── Cargo.toml
├── src/
│   ├── main.rs                 one scientific_workflow::run(&Path) call
│   └── hopf_model.rs          Hopf model registered as one execution unit
├── wf_configs/                 required Workflow configuration root
│   ├── study.json              simulate, reserved $npy, then plot
│   ├── parameters.json         execution-unit and plotting parameters
│   └── states/                 recommended schema grouping
│       └── attractor.json      canonical scientific state schema
└── scripts/
    └── plot.py                 directly declared Python task
```

## Scientific workload

`HopfModel` is the downstream application's scientific model. Workflow itself
intentionally has no narrower "model" abstraction: implementing
`ExecutionUnit` makes this model a unit that Workflow can validate, initialize,
schedule, observe, and advance. This README therefore calls the concrete
application object a **model** and uses **execution unit** only for the generic
Workflow contract through which it is managed.

`HopfModel` directly owns its canonical `SystemState`. It initializes the
two-dimensional `point` and derived `radius`, advances both values and physical
time in `step`, and reports completion through its configured iteration count.
Initial assembly uses `initialize_payload`, so an accidental second
initialization fails instead of silently becoming replacement.
The execution unit retains its decoded `AttractorConstants` directly rather than
duplicating the same immutable values across runtime fields.
Its `member(0)` method exposes a `MemberView` containing a direct immutable state
borrow, completion, target iteration, and the stable `hopf-attractor` member
identity. `member_count()` is one, so this is the single-member form of the
same API an ensemble uses. Inside the
execution unit, `step` obtains `point` and `radius` together with the typed tuple call
`borrow_payloads_mut::<(Vec<f64>, f64)>(("point", "radius"))`; state fields are
not generated or accessed by a macro. The `#[execution_unit("attractor")]` attribute is
only the automatic registration link to `wf_configs/study.json`.
For presentation only, `HopfModel::DEMONSTRATION_STEP_DELAY` sleeps for one
millisecond after every successful step so the automatic dashboard remains
visible long enough to inspect. It is deliberately implementation-owned rather
than project configuration because it is not scientific input. Do not remove
it from the bundled example: the workload otherwise finishes too quickly to
demonstrate live concurrent progress. It never changes scientific time or
persisted constants.

Its custom `ObservationPlan` records:

- the phase-space trajectory every 10 iterations;
- radius every 5 iterations; and
- a combined checkpoint every 1,000 iterations.

The `attractor` section of `wf_configs/parameters.json` sweeps three growth-rate
values and two angular frequencies. Config selects that section from the
registered execution unit key and expands its Cartesian product into six execution-unit tasks;
Runtime executes up to three concurrently and automatically records every
observation stream. The phase's `start_interval_ms: 2000` setting waits two
seconds between successive task admissions (the first eligible task starts
immediately). This phase-owned scheduling delay is separate from the execution unit's
one-millisecond per-step presentation delay. With the supplied workload, a
normal run takes roughly 16 seconds, subject to machine and IO overhead.
Top-level `study.json.workflow_schema: 1` selects the supported authored
configuration grammar. Top-level `study.json.threads` is the required global
compute budget. Workflow owns one shared execution-unit pool of that size; the
phase's `max_concurrency` controls task admission and does not create additional
model pools. The synthesized `$npy` converter and final Python plot task each
reserve one external-task thread after simulation has completed.

### How parameters reach `AttractorConstants`

Parameter wiring uses two distinct kinds of key matching:

```text
#[execution_unit("attractor")]
          |
          +-- study.json task: {"execution_unit":"attractor","state":"attractor"}
          |
          `-- parameters.json["attractor"]
                            |
                            +-- expand mu $sweep (3 values)
                            +-- expand angular_frequency $sweep (2 values)
                            `-- six complete JSON objects
                                         |
                              Serde field-name matching
                                         |
                                  AttractorConstants
                                         |
                  HopfModel::initialize(constants, schema, context)
                                         |
                           HopfModel { state, constants }
                                         |
                    member(0) -> MemberView("hopf-attractor", &state, ...)
```

The stable execution unit key comes from `#[execution_unit("attractor")]`. The execution unit task's
`"execution_unit": "attractor"` selects that linked Rust implementation, and Config
uses the same key to select the top-level `attractor` section of
`wf_configs/parameters.json`. No parameter filename or Rust type name is
inferred. The task's separate `"state": "attractor"` value selects the named
schema registered in `study.json.paths.states`; it does not select parameters,
and execution unit and state keys are not required to have the same spelling.

Serde is Rust's standard serialization and deserialization framework; here
Workflow uses its deserialization direction to convert JSON into a Rust value.
Within the selected parameter object, Serde matches JSON property names such
as `initial_point`, `step_count`, and `angular_frequency` to the identically
named fields of `AttractorConstants`. Consequently, `#[derive(Deserialize)]` is
required on that type. `#[serde(deny_unknown_fields)]` makes obsolete,
misspelled, or otherwise unconsumed properties fail during effect-free Study
preflight instead of being silently ignored.

Config expands the independent `mu` and `angular_frequency` `$sweep` markers
into their deterministic three-by-two Cartesian product before decoding. Study
deserializes each complete object once to validate its constants type and to
call `HopfModel::preflight` during preflight. This model-owned hook runs during
effect-free `Study::load`, before Runtime creates output or executes anything.
This model needs no additional domain check and therefore does not inspect the
schema argument. Its observation builders validate stream names, field
selections, and sampling intervals while producing the `ObservationPlan` that
tells Workflow what to record and at what cadence. Workflow then binds that
plan to the selected schema, which verifies that the named fields exist.
Returning an error at either stage rejects the study. The hook must therefore
remain side-effect free.

Task deserializes a fresh,
equivalent owned value from the retained immutable JSON when that execution unit task
actually executes. This second decode lets the constants type remain local to
the execution thread rather than requiring it to be shared between planning
and Runtime. `HopfModel::initialize` consumes that value and retains it directly
beside the model-owned `SystemState`. Workflow manages `HopfModel` through the
same execution-unit lifecycle it uses for an ensemble; the difference is only
that this example exposes one member rather than several. An ensemble would
return one stable `MemberView` per internal member and perform its shared or
parallel advancement inside the same `step()` method.

Runtime also passes an `InitializationContext`. This attractor is deterministic,
so it names that argument `_context` and does not require a top-level study
seed. A stochastic unit would call `shared_seed(purpose)` for unit-wide random
work or `member_seed(identity, purpose)` for one member. Workflow would then
record each actual derived seed with the corresponding member metadata.

## Standard NPY and Python plot phases

The reserved `$npy` phase needs only its prerequisite:

```json
"$npy": {"after": ["simulate"]}
```

Workflow synthesizes the conversion task, gives it every transitively
prerequisite execution-unit recording for the same global configuration, and
writes a `scientific-workflow-npy-batch.v1` manifest plus C-contiguous arrays
inside that task's artifact directory. Project authors provide no converter
script, paths, arguments, or duplicated recording selectors.

The `plot` phase depends on `$npy` and declares `scripts/plot.py` directly:

```json
{
  "python": {
    "script": "scripts/plot.py",
    "environment": {"manager": "system"}
  }
}
```

No Rust closure or caller wraps Python. Workflow resolves `python3` during
Study loading, captures stdout/stderr, and supplies the central configuration,
completed conversion output, and isolated artifact directory through the
standard `WORKFLOW_*` contract.

The plotter retrieves its visual settings from the `plot` section of
`wf_configs/parameters.json`, reads only the verified processed manifests and
memory-mapped `.npy` arrays, and produces:

```text
output/plots/
├── attractor-sweep.svg
└── plot-summary.json
```

For every processed member it verifies the retained recording provenance: execution unit kind
and key, explicitly selected `attractor` state key, parameter ordinal, and the
canonical `wf_configs/parameters.json` source. The plot summary retains the
state key and ordinal so the generated artifact remains traceable to the
assembled task rather than inferring state from the execution unit name. The
plotter never opens JSONL chunks itself. These provenance fields are the
`scientific-workflow-attractor-plot-v2` summary shape.

## Run

From the repository root:

```bash
python -m pip install "./python[npy]"
cargo run -p attractor-2d
```

Workflow creates a unique Rust execution beneath `examples/attractor_2d/output`.
Python owns its configured `output/plots` destination directly. When stdin and
stderr are interactive, the automatic Ratatui dashboard shows task rows,
progress, timing, messages, and an `exit` command. The task section refreshes
for each phase and shows that phase only; redirected runs use plain lifecycle
lines. The execution unit and plotter do not construct tasks, phases,
persistence sessions, progress counters, or message channels.

The omitted replicate, timeout, UI, and persistence settings use Workflow's
validated defaults. The project authors scientifically meaningful observation
cadence, parameter sweep, plotting choices, one justified concurrency bound,
the two-second inter-task admission delay. The explicitly non-scientific
per-step demonstration delay belongs to the example implementation itself.

`Study::load` performs complete effect-free validation of the required
`wf_configs/` root, reserved documents, named-state path, execution-unit
selector, typed constants, Python environment, and `$npy` launcher before
Runtime creates output.
