# Two-dimensional attractor study

This is a complete small scientific project rather than a collection of API
fragments. Rust owns the stateful Hopf model, JSON owns the study and all
configuration, and a final Python task turns the completed recordings into an
SVG figure.

```text
attractor_2d/
├── Cargo.toml
├── src/
│   ├── main.rs                 one scientific_workflow::run(&Path) call
│   └── hopf_model.rs           registered one-model execution unit
├── wf_configs/                 required Workflow configuration root
│   ├── study.json              simulate phase followed by plot phase
│   ├── parameters.json         every model and plotting parameter
│   └── states/                 recommended schema grouping
│       └── attractor.json      canonical scientific state schema
└── scripts/
    └── plot.py                 directly declared Python task
```

## Scientific workload

`HopfModel` directly owns its canonical `SystemState`. It initializes the
two-dimensional `point` and derived `radius`, advances both values and physical
time in `step`, and reports completion through its configured iteration count.
Initial assembly uses `initialize_payload`, so an accidental second
initialization fails instead of silently becoming replacement.
The model retains its decoded `AttractorConstants` directly rather than
duplicating the same immutable values across runtime fields.
Its `model(0)` method exposes a `ModelView` containing a direct immutable state
borrow, completion, target iteration, and the stable `hopf-attractor` member
identity. `model_count()` is one, so this is the standalone-model form of the
same API an ensemble uses. Inside the
model, `step` obtains `point` and `radius` together with the typed tuple call
`borrow_payloads_mut::<(Vec<f64>, f64)>(("point", "radius"))`; model fields are
not generated or accessed by a macro. The `#[execution_unit("attractor")]` attribute is
only the automatic registration link to `wf_configs/study.json`.
For presentation only, `HopfModel::DEMONSTRATION_STEP_DELAY` sleeps for one
millisecond after every successful step so the automatic dashboard remains
visible long enough to inspect. It is deliberately implementation-owned rather
than project configuration because it is not scientific input. Do not remove
it from the bundled example: the workload otherwise finishes too quickly to
demonstrate live concurrent progress. It never changes scientific time or
persisted model constants.

Its custom `ObservationPlan` records:

- the phase-space trajectory every 10 iterations;
- radius every 5 iterations; and
- a combined checkpoint every 1,000 iterations.

The `attractor` section of `wf_configs/parameters.json` sweeps three growth-rate
values and two angular frequencies. Config selects that section from the
registered model key and expands its Cartesian product into six model tasks;
Runtime executes up to three concurrently and automatically records every
observation stream. The phase's `start_interval_ms: 2000` setting waits two
seconds between successive task admissions (the first eligible task starts
immediately). This phase-owned scheduling delay is separate from the model's
one-millisecond per-step presentation delay. With the supplied workload, a
normal run takes roughly 16 seconds, subject to machine and IO overhead.

### How parameters reach `AttractorConstants`

Parameter wiring uses two distinct kinds of key matching:

```text
#[execution_unit("attractor")]
          |
          +-- study.json task: {"model":"attractor","state":"attractor"}
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
                    model(0) -> ModelView("hopf-attractor", &state, ...)
```

The stable model key comes from `#[execution_unit("attractor")]`. The model task's
`"model": "attractor"` selects that linked Rust implementation, and Config
uses the same key to select the top-level `attractor` section of
`wf_configs/parameters.json`. No parameter filename or Rust type name is
inferred. The task's separate `"state": "attractor"` value selects the named
schema registered in `study.json.paths.states`; it does not select parameters,
and model and state keys are not required to have the same spelling.

Within the selected parameter object, Serde matches JSON property names such
as `initial_point`, `step_count`, and `angular_frequency` to the identically
named fields of `AttractorConstants`. Consequently, `#[derive(Deserialize)]` is
required on that type. `#[serde(deny_unknown_fields)]` makes obsolete,
misspelled, or otherwise unconsumed properties fail during effect-free Study
preflight instead of being silently ignored.

Config expands the independent `mu` and `angular_frequency` `$sweep` markers
into their deterministic three-by-two Cartesian product before decoding. Study
deserializes each complete object once to validate its constants type and to
call `HopfModel::observation_plan` during preflight. Task deserializes a fresh,
equivalent owned value from the retained immutable JSON when that model task
actually executes. This second decode lets the constants type remain local to
the execution thread rather than requiring it to be shared between planning
and Runtime. `HopfModel::initialize` consumes that value and retains it directly
beside the model-owned `SystemState`. Workflow manages `HopfModel` through the
same execution-unit lifecycle it uses for an ensemble; the difference is only
that this example exposes one model rather than several. An ensemble would
return one stable `ModelView` per internal model and perform its shared or
parallel advancement inside the same `step()` method.

Runtime also passes an `InitializationContext`. This attractor is deterministic,
so it names that argument `_context` and does not require a top-level study
seed. A stochastic unit would call `shared_seed(purpose)` for unit-wide random
work or `model_seed(identity, purpose)` for one model. Workflow would then
record each actual derived seed with the corresponding model metadata.

## Python plot phase

The `plot` phase depends on `simulate` and declares `scripts/plot.py` directly:

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
completed simulation outputs, and isolated artifact directory through the
standard `WORKFLOW_*` contract.

The plotter retrieves its visual settings from the `plot` section of
`wf_configs/parameters.json`, opens every dependency recording with the official
verified `scientific_workflow_reader`, and produces:

```text
output/plots/
├── attractor-sweep.svg
└── plot-summary.json
```

For every dependency it verifies the current recording provenance: model kind
and key, explicitly selected `attractor` state key, parameter ordinal, and the
canonical `wf_configs/parameters.json` source. The plot summary retains the
state key and ordinal so the generated artifact remains traceable to the
assembled task rather than inferring state from the model name. Reader metadata
is consumed through its immutable mapping interface; the example does not rely
on a concrete mutable dictionary representation. These provenance fields are
the `scientific-workflow-attractor-plot-v2` summary shape.

An installed `scientific-workflow-reader` is used normally. When this example
runs from the repository checkout, the script falls back to the adjacent
reader source tree so a clean checkout needs only Python 3.10 or newer.

## Run

From the repository root:

```bash
cargo run --manifest-path examples/attractor_2d/Cargo.toml
```

Workflow creates a unique Rust execution beneath `examples/attractor_2d/output`.
Python owns its configured `output/plots` destination directly. When stdin and
stderr are interactive, the automatic Ratatui dashboard shows task rows,
progress, timing, messages, and an `exit` command. The task section refreshes
for each phase and shows that phase only; redirected runs use plain lifecycle
lines. The model and plotter do not construct tasks, phases,
persistence sessions, progress counters, or message channels.

The omitted replicate, timeout, UI, and persistence settings use Workflow's
validated defaults. The project authors scientifically meaningful observation
cadence, parameter sweep, plotting choices, one justified concurrency bound,
the two-second inter-task admission delay. The explicitly non-scientific
per-step demonstration delay belongs to the example implementation itself.

The example tests also load the project through `Study::load`, proving that the
required `wf_configs/` root, canonical reserved documents, named-state path,
model `state` selector, typed constants, Python environment, and complete
effect-free preflight remain synchronized with the current crate specification.
