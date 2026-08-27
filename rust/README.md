# Scientific Workflow

Scientific Workflow is a Rust library for building reproducible, inspectable
scientific programs. It supplies the infrastructure that tends to be rebuilt
around simulations and numerical experiments: strict configuration expansion,
typed scientific state, deterministic task declarations, bounded recording,
checkpoint reconstruction, immutable artifacts, execution directories, RNG
provenance, lifecycle records, and terminal progress reporting.

The crate does not provide a particular scientific model. Instead, it gives a
model-owning application a set of small boundaries that compose into a complete
workflow while leaving equations, numerical methods, and domain validation in
the application that understands them.

> **Release status:** this crate is test software. Public API behavior may
> change between releases until a stable 1.0 line is announced. Treat every
> version update as a coordinated migration.

## Current refactor boundary

The target application workflow has only four authored inputs:

1. a scientific state and its schema;
2. a writer describing scientifically meaningful observations;
3. typed task definitions; and
4. `study.json`, `config/state.json`, and referenced task input documents below
   `config/inputs/`.

The new `state`, `writer`, `task`, and `config` module boundaries implement
that model now. The current `study`, `configuration`, `execution`, and
`storage` APIs remain temporarily available while `runtime` and `record` are
migrated. They are compatibility surfaces, not the recommended architecture
for new integrations.

The vocabulary used by the new boundary is precise: `study.json` is the
**study manifest**, `config/state.json` is the **state schema document**, an
application JSON file below `config/inputs/` is a **task input document**, one
expanded concrete value is a **resolved task input**, and its typed Rust value
is one set of **model constants**.

## Installation

Scientific Workflow 0.10 requires Rust 1.97 or newer. Use the registry release
in application crates:

```toml
[dependencies]
scientific-workflow = "0.10.0"
```

The dependency intentionally has no Cargo features. All public modules use one
implementation and one dependency graph; applications import only the module
boundaries they need. Commit the application's `Cargo.lock` when it is an
executable so every deployment resolves the same compatible release.

### Migrating from 0.9

Version 0.10 introduces application-owned whole-phase completion examination:

1. Attach validation directly to a phase with
   `PhaseBuilder::examine_completion`. Return `PhaseCompletion::Complete` only
   after validating the entire result represented by that phase.
2. Remove `StudyBuilder::satisfied_phase_verifier`. Exact selection and
   dependency-inclusive selection now consume the same cached phase verdict.
3. Completion examination is enabled by default. Use
   `StudyBuilder::without_completion_examination` for an explicit launch that
   must invoke every selected phase.
4. Treat `PhaseCompletion::Incomplete` as a whole-phase decision only. Workflow
   warns and invokes the phase normally; its workloads remain solely
   responsible for validation, reuse, cleanup, and continuation within it.
5. Study plans and records now use formats v2. Plans declare whether each phase
   has an examiner; records distinguish executed and reused phase disposition.

Version 0.10 provides no compatibility alias for the removed verifier.

### Migrating from 0.8

Version 0.9 is a breaking configuration-vocabulary release:

1. Rename the root `phase_group` object to `components` and every component's
   `phase` object to `workloads`. Keep `global`, `shared`, `$sweep`, and
   `$cases` unchanged.
2. Select one configuration workload with
   `workload(component, workload)`, and call `combinations()` on that
   `WorkloadConfiguration`. The former phase-oriented Rust API is removed;
   components deliberately have no expansion API.
3. Replace resolved-configuration identity accessors `phase_group()`,
   `group_ordinal()`, `phase()`, and `phase_ordinal()` with `component()`,
   `component_ordinal()`, `workload()`, and `workload_ordinal()`.
4. In `study.json`, replace replicate `execution` with `scheduling` and `seed`
   with `base_seed`. The corresponding Rust accessors are `scheduling()` and
   `base_seed()`.
5. Configuration provenance now records `component`, `workload`,
   `component_ordinal`, and `workload_ordinal` under `configuration_identity`.
6. Put truly study-wide parameters in `global`. Put parameters shared by
   related workloads in `components.<component>.shared`, and workload-local
   parameters in `components.<component>.workloads.<workload>`.

The complete grammars and startup example appear below. Version 0.9 provides
no aliases for the former phase-oriented names.

## What problem the crate solves

A scientific executable usually does more than evaluate equations. It must
decide which parameter combinations exist, create stable identities for runs,
schedule independent work, expose progress, record large states without
unbounded memory growth, resume interrupted output, retain the inputs that made
a result, and reject incomplete or contradictory data.

Those concerns are easy to mix together. A configuration object starts opening
files, a scheduler learns about model state, a recording layer starts deciding
when a trajectory has converged, and several callers implement slightly
different versions of the same path or provenance rules. The resulting program
may still produce numbers, but it becomes difficult to explain exactly which
inputs produced them or which layer owns a failure.

Scientific Workflow separates those responsibilities. A target program has
the following flow:

```text
project root: &Path
        │
        ├── study.json                 study manifest
        └── config/
            ├── state.json             state schema document
            └── inputs/*.json          task input documents
        │
        ▼
ProjectSpecification                  strict central parsing and expansion
        │
        ├── resolved task input ──► typed model constants
        └── effective phase and replicate policy
        │
        ▼
runtime plan ──► Task ──► ScientificModel + Writer
        │
        ▼
recording + source/input provenance ──► verified readback or StateSeries
```

Config loads and validates declarations but does not execute work or create
output. Task owns typed scientific behavior but not paths or lifecycle. The
future runtime composes these boundaries and infers identities, scheduling,
progress, messages, and recording administration. The application remains the
owner of scientific meaning without manually wiring those mechanics.

## Design philosophy

### One owner for each concern

Each public module has one primary responsibility. Orchestration belongs to
`study`, scientific observation definitions belong to `writer`, durable state
belongs to `storage`, filesystem run identity belongs to `execution`, and
scientific values belong to `state`. The same behavior should not be
implemented again in a neighboring layer.

This is more than code organization. It makes failures attributable. A bad
parameter document is a configuration error; an incompatible state payload is
a state error; an altered artifact is an artifact error; and a failed workload
is represented by the study lifecycle. Callers do not have to infer which
subsystem rejected an operation.

### Reuse before invention

Applications should first use an existing Scientific Workflow API, then an
appropriate third-party API, before creating another implementation. If a
needed capability genuinely belongs to an existing boundary but is missing,
the preferred change is a small explicit addition to that boundary. New
application-level behavior is appropriate only when the application owns its
semantics.

This rule keeps validation, path handling, task identity, persistence, and
provenance consistent across a program. It also keeps public APIs narrow:
sharing an implementation does not require merging the responsibilities of the
modules that call it.

### Scientific meaning stays downstream

The crate knows how to store a typed state, but not what a population, energy,
or field means. It knows how to execute a task, but not which solver should run.
It knows how to enumerate a parameter sweep, but not whether a parameter is
physically valid. Domain equations, invariants, stopping rules, scientific
transformations, and interpretation remain in model-owned code.

### Deterministic declarations, explicit effects

Configuration combinations, task registration order, phase dependencies,
metadata, and plan serialization are deterministic. Effects such as creating
an execution directory, publishing an artifact, starting a workload, or
writing a recording happen through explicit calls. Loading configuration does
not start work or create output.

### Fail closed at durable boundaries

Source documents are validated before objects are published. Duplicate JSON
keys are rejected instead of silently overwritten. State schemas are checked
before payloads are accepted. Artifacts are verified by content digest.
Continuation requires a compatible, internally consistent recording. Derived
JSON is written through a temporary file and atomically installed.

The goal is to prefer a contextual error over a plausible but ambiguous
scientific result.

### Provenance is part of the result

Configured task helpers retain the complete resolved configuration and named
path table in task metadata. Study plans record the declaration before work
runs, and study records retain task metadata with lifecycle facts. Storage can
also retain caller metadata and RNG records. Provenance is therefore available
without asking a renderer or reconstructing command-line state after the fact.

### Bounded work and bounded memory

Phase concurrency and prepared-work queues are explicit. State writers use
bounded buffering and sealed chunks instead of retaining an entire trajectory.
Large studies can therefore choose resource ceilings without changing their
scientific workloads.

### Cooperative orchestration

Rust workloads cannot be forcibly stopped safely. In the transitional study
API, the scheduler requests cancellation and the legacy workload observes it
through `study::TaskContext`. In the new task boundary, runtime exposes the
same cooperative state through `TaskExecutionHost` and checks it between model
steps. Neither contract pretends that arbitrary Rust code can be interrupted
mid-call without cleanup.

## Public modules

### `config`: central project declarations and constants supply

The target `config` subsystem is the only Workflow code that opens or parses
project declaration JSON. Ordinary users call no config Rust API; they author
the file grammar. Runtime will accept only a borrowed `project_root: &Path`
and load this conventional layout:

```text
<project-root>/
├── study.json
└── config/
    ├── state.json
    └── inputs/
        └── run.json
```

`study.json` declares replicate policy, phases, task definition keys, input
references, and optional execution/display policy. Task input documents hold
application-owned values. Exact `{"$sweep":[...]}` markers create independent
Cartesian choices; `$cases` creates correlated alternatives; ordinary arrays
remain literal. Config expands these internally and runtime creates one task
invocation per resulting constants value. Users never call `combinations()`
or decode individual JSON keys.

The ordinary `config::basic` scope is intentionally empty. The read-only
`config::advanced` integration scope exposes `ProjectSpecification` and its
effective manifest, phases, source documents, state schema document, and
`ResolvedTaskInput` values. Task consumes `ResolvedTaskInput` directly and
asks config to decode the complete declared constants type. See
`src/config/api.md` for the exhaustive manifest grammar, selection grammar,
defaults, path containment, errors, examples, and replacement contract.

### `configuration`: transitional experiment inputs

This older module serves the unmigrated study/execution/example stack. It is
not an alias for `config`, and its manual workload selection,
`combinations()`, JSON Pointer decoding, and named-path APIs are intentionally
absent from the target boundary.

The `configuration` module validates three independent input concerns: the
study manifest, study-wide scientific parameters, and named paths:

```text
study/
├── study.json             replicate policy and application-owned settings
└── config/
    ├── parameters.json    scientific parameters and selections
    └── paths.json         named filesystem paths
```

`StudySettings` strictly loads `study.json`, validates Workflow's replicate
policy, and exposes the already-parsed application object through one typed
API. `StudyConfiguration` independently loads `config/parameters.json`. Calling
`workload(component, workload)` returns a `WorkloadConfiguration`, whose combinations
automatically compose the global, component-shared, and workload-local scopes.
`ResolvedConfiguration` supports exact JSON Pointer lookup, typed decoding,
terminal-key iteration, nested-document reconstruction, and scoped ordinals.

#### Complete `study.json` grammar

The root contains the required `replicate_settings` object and an optional
`application` object. `replicate_settings` contains exactly four required
fields:

```json
{
  "replicate_settings": {
    "replicates": 4,
    "scheduling": "parallel",
    "failure_policy": "finish_all",
    "base_seed": 1101
  },
  "application": {
    "schema": "my-program.study.v1",
    "protocol": "calibration",
    "enabled_phases": ["prepare", "run"]
  }
}
```

| Field | Accepted values | Meaning |
|---|---|---|
| `replicates` | positive JSON integer representable as `u64` | Number of isolated executions of the complete program |
| `scheduling` | `"sequential"` or `"parallel"` | Whether the controller awaits each child before starting the next or starts one child per replicate immediately |
| `failure_policy` | `"fail_fast"` or `"finish_all"` | Whether a failure stops future sequential children/terminates active parallel children, or lets every replicate finish |
| `base_seed` | JSON integer representable as `u64` | Study-level source for lazy namespace-separated seeds |

Unknown Workflow-level fields and unknown, missing, duplicated, mistyped, or
zero-valued replicate fields are rejected. There are no replicate defaults:
the source document always states the complete process contract. The optional
`application` value must be an object. Workflow preserves it without assigning
meaning to its fields.

The user program decodes that object from the already-parsed manifest. This
eliminates application-specific companion settings files and a second
filesystem read:

```rust,no_run
use serde::Deserialize;
use scientific_workflow::configuration::StudySettings;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplicationSettings {
    schema: String,
    protocol: String,
    enabled_phases: Vec<String>,
}

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let study = StudySettings::load("study")?;
let application: ApplicationSettings = study.application()?;
# let _ = (application.schema, application.protocol, application.enabled_phases);
# Ok(())
# }
```

The requested type owns its grammar and validation. Applications needing an
untyped object can request `serde_json::Value`; Workflow exposes no second,
overlapping key-access interface.

`ReplicateExecutor` consumes the validated settings plus an output root chosen
through `ProjectPaths`. The first process is the controller and starts the same
executable once per replicate. Each child re-enters the same call and receives
a `ReplicateContext`; only that branch constructs and runs the scientific
study:

```rust,no_run
use scientific_workflow::prelude::basic::*;

# fn run_one_replicate(_replicate: &ReplicateContext) -> Result<(), Box<dyn std::error::Error>> {
# Ok(())
# }
# fn main() -> Result<(), Box<dyn std::error::Error>> {
let study_root = std::path::Path::new("study");
let settings = StudySettings::load(study_root)?;
let output_root = ProjectPaths::load(study_root)?.resolve_path("output_root")?;

let Some(replicate) =
    ReplicateExecutor::new(settings.replicate_settings(), output_root)
        .dispatch_current_executable()?
else {
    return Ok(()); // Controller completed all child processes.
};

run_one_replicate(&replicate)?;
# Ok(())
# }
```

Replicate indices are zero-based. The controller exclusively creates
`output_root/replicate_0`, `replicate_1`, and so on; an existing directory is a
hard error rather than an overwrite. Original command-line arguments and stdio
are inherited. In parallel mode there is deliberately no process pool or
machine-resource scheduler: one child is started for every declared replicate.
Applications should use plain or hidden study display when concurrent children
would otherwise compete for an interactive terminal.

`ReplicateContext::execution_scope()` is the existing output scope and should
be passed to APIs that require an `ExecutionScope`.
`ReplicateContext::seed_deriver()` reuses the library's versioned
`ReplicateSeedDeriver`; seed material is calculated only when the application
requests a named random stream. A deterministic study need not derive a seed.

#### Complete `parameters.json` grammar

The root has exactly two fields. `global` is a parameter scope.
`components` is indexed by stable application-defined string keys:

```json
{
  "global": {
    "species": 128
  },
  "components": {
    "models": {
      "shared": {
        "maximum_iterations": 10000
      },
      "workloads": {
        "mean-field": {
          "solver": {"time_step": 0.01}
        },
        "lattice": {
          "shape": [64],
          "boundary": "periodic"
        }
      }
    }
  }
}
```

Each component has exactly `shared` and `workloads`. `workloads` is indexed by
stable application-defined workload keys. Only `global`, `components`,
`shared`, `workloads`, `$sweep`, and `$cases` are grammar names; keys such as
`models` and `mean-field` have no built-in scientific meaning.

Every ordinary JSON value is literal, including arrays. An array can therefore
represent a shape, field list, recording definition, matrix, or any other
domain value without escaping. Cartesian selection requires an explicit
object containing exactly `$sweep`:

```json
{
  "temperature": {"$sweep": [280.0, 300.0]},
  "shape": {"$sweep": [[64], [128], [64, 64]]},
  "fields": ["abundance", "space"]
}
```

All `$sweep` markers in one scope form a Cartesian product. Choices may be any
JSON values, including arrays and objects, but every object choice for one
marker must flatten to the same key set.

Correlated alternatives use one scope-level `$cases` array:

```json
{
  "solver": "rk4",
  "$cases": [
    {"temperature": 280.0, "time_step": 0.02},
    {"temperature": 300.0, "time_step": 0.01}
  ]
}
```

Every case must be a nonempty object with the same flattened key set. A scope
cannot mix `$cases` and `$sweep`; ordinary sibling values are shared by all
cases.

#### Scope composition and ownership

For one selected workload, expansion is:

```text
global selections × component shared selections × workload-local selections
```

Declare a parameter at the lowest scope whose complete set of descendants must
share or cross it. A truly study-wide value belongs in `global`; a value shared
only by a related set of workloads belongs in that component's `shared`; all
other values belong in their consuming workload.

The merged scopes cannot contain overlapping paths. There is intentionally no
shadowing or fallback precedence: `/seed` cannot be declared globally and
again in a contributing component or workload. Parameters with the same short name
but different meanings should use explicit namespaces.

Global selections vary slowest, followed by component selections and
workload-local selections. Each result exposes `global_ordinal()`,
`component_ordinal()`, and `workload_ordinal()` in addition to the flattened
`ordinal()`. Because a workload never expands sibling workload selections,
editing an unrelated workload cannot
multiply or renumber its tasks.

#### Loading and decoding

```rust,no_run
use scientific_workflow::configuration::{ProjectPaths, StudyConfiguration};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let study = StudyConfiguration::load("study")?;
let configurations = study.workload("models", "mean-field")?;
let paths = ProjectPaths::load("study")?;

for configuration in configurations.combinations() {
    let (species, maximum): (usize, u64) =
        configuration.decode_values(("/species", "/maximum_iterations"))?;
    println!(
        "global={} component={} workload={} species={species} maximum={maximum}",
        configuration.global_ordinal(),
        configuration.component_ordinal(),
        configuration.workload_ordinal(),
    );
}

println!("recordings: {}", paths.resolve_path("recordings")?.display());
# Ok(())
# }
```

Loading validates the entire registry before returning any workload. It rejects
duplicate JSON keys, missing or unknown grammar fields, blank keys, malformed
or empty selections, mismatched cases, overlapping paths, and combination
count overflow. `source_json()` preserves the exact validated parameter bytes.

`ProjectPaths` strictly loads `config/paths.json`, rejects duplicate or blank
entries, preserves declaration order and original bytes, and resolves relative
values lexically against the study root. It does not canonicalize targets,
expand shell syntax, or require paths to exist.

The transitional configuration module validates process-level replicate policy but does not execute
it. It does not register tasks, create output, choose phase concurrency, or
validate domain-specific physics. Applications explicitly pass settings to the
execution API and map selected parameter combinations into workloads.

### `task`: typed scientific work with inferred mechanics

The new `task` boundary exposes only `Task`, `ScientificModel`, and
`TaskResult` to ordinary application code. A canonical `ScientificModel`
directly owns its `SystemState`, initializes it from one typed constants value,
performs one observable transition in `step`, and optionally reports a target
iteration. A stateful task binds that model to a writer factory:

```rust,no_run
use scientific_workflow::prelude::basic::*;
use serde::Deserialize;

#[derive(Deserialize)]
struct Constants {
    initial: u64,
    steps: u64,
}

struct Model {
    state: SystemState,
    remaining: u64,
    target: u64,
}

impl ScientificModel for Model {
    type Constants = Constants;

    fn initialize(constants: Constants, schema: &SystemStateSchema) -> TaskResult<Self> {
        let mut state = schema.create_empty_state(StateTime::from_iteration(0));
        state.initialize_payload("population", constants.initial)?;
        Ok(Self { state, remaining: constants.steps, target: constants.steps })
    }

    fn state(&self) -> &SystemState { &self.state }
    fn is_complete(&self) -> bool { self.remaining == 0 }

    fn step(&mut self) -> TaskResult {
        *self.state.payload_mut::<u64>("population")? += 1;
        self.state.advance_time(None)?;
        self.remaining -= 1;
        Ok(())
    }

    fn target_iteration(&self) -> Option<u64> { Some(self.target) }
}

let task = Task::stateful::<Model, _>(|_| Ok(Writer::all_fields()));
# let _ = task;
```

Config supplies typed constants from the resolved task input. Runtime loads
the centrally parsed `config/state.json` into state, validates the writer
before model initialization, observes the initial state and every
successful step, and publishes routine lifecycle/progress messages. Task code
receives no generic context, paths, identity, progress counter, message
callback, record session, or completion control. `Task::one_shot` covers typed
work that genuinely needs neither state nor a writer. See `src/task/api.md` for
the complete ownership and advanced-host contracts.

### `study`: transitional orchestration and lifecycle

The current `study` module remains the transitional control plane while the
future runtime is migrated onto the new `task` boundary. Its legacy vocabulary
is deliberately small:

```text
Study
└── Phase
    └── Task
        └── workload(&TaskContext) -> TaskResult
```

A `Study` owns the phase graph, validates dependencies, selects phases, starts
the renderer, coordinates cancellation, and writes a durable `StudyRecord`.
`StudyPlan` is the serializable declaration available before execution, while
`StudySummary` and `PhaseSummary` describe the outcome.

A `Phase` owns a deterministic list of tasks and workload-local execution policy:
maximum active tasks, prepared queue capacity, start interval, per-task timeout,
phase deadline, dependencies, failure behavior, optional confirmation, and an
optional application-owned whole-phase completion examiner.

Completion examination is deliberately phase-level. A complete result reuses
the whole phase and satisfies it as an omitted dependency. Missing output runs
normally. Incomplete output emits a warning and also runs normally. Invalid
output fails before execution begins. Workflow never infers how individual
tasks inside an incomplete phase should validate, reuse, clean up, or resume.
Those decisions remain entirely with the phase's application workloads.

A `study::Task` owns identity, category, label, mode, immutable metadata, and
exactly one application workload. It is distinct from the new `task::Task`.
Progress and one-shot tasks share the same legacy type. `Task::completed`
represents work that the application has independently
verified as already satisfied.

`Task::one_shot_for_configuration` and
`Task::progress_for_configuration` provide the conventional task identity and
attach the complete resolved configuration. `with_project_paths` adds the named
path table. These helpers prevent each application from inventing a different
configuration-to-task convention.

```rust,no_run
use scientific_workflow::configuration::{ProjectPaths, StudyConfiguration};
use scientific_workflow::prelude::study::*;

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let study_configuration = StudyConfiguration::load("study")?;
let configurations = study_configuration.workload("models", "dynamics")?;
let paths = ProjectPaths::load("study")?;
let tasks = configurations.combinations().map({
    let paths = paths.clone();
    move |configuration| {
        let provenance = configuration.clone();
        let paths = paths.clone();
        Task::progress_for_configuration("simulation", &provenance, move |context| {
            let maximum: u64 = configuration.decode_value("/maximum_iterations")?;
            context.set_target_iteration(maximum)?;
            for iteration in 0..=maximum {
                if !context.should_continue(iteration)? {
                    break;
                }
                // Advance application-owned scientific state here.
            }
            Ok(())
        })
        .with_project_paths(&paths)
    }
});

let phase = Phase::builder(1, "simulations")
    .tasks(tasks)
    .max_active_tasks(4)
    .prepared_task_queue_capacity(4)
    .examine_completion(|| {
        // Validate the complete application-owned phase result here.
        PhaseCompletion::Missing
    })
    .build()?;

let summary = Study::builder("study-record.json")
    .phase(phase)
    .build()?
    .run()?;
assert!(summary.is_success());
# Ok(())
# }
```

The only communication channel supplied to this legacy workload is
`study::TaskContext`. It
reports progress and human-readable detail, exposes identity and metadata, and
observes cooperative cancellation. It does not provide hidden filesystem,
network, subprocess, state, or artifact capabilities.

The terminal renderer is centralized so worker threads never compete for
stdout. Automatic, interactive, plain, and hidden display modes change
presentation without changing scheduling semantics.

An examiner is a small read-only closure and may capture application paths and
expected configuration directly:

```rust,no_run
use scientific_workflow::prelude::study::*;

# fn validate_result() -> Result<bool, String> { Ok(false) }
# fn main() -> Result<(), StudyError> {
let phase = Phase::builder(20, "model dynamics")
    .task(Task::one_shot("model", "model", |_| Ok(())))
    .examine_completion(|| match validate_result() {
        Ok(true) => PhaseCompletion::Complete,
        Ok(false) => PhaseCompletion::incomplete("partial model output exists"),
        Err(reason) => PhaseCompletion::invalid(reason),
    })
    .build()?;

Study::builder("study-record.json")
    .phase(phase)
    .hidden()
    .build()?
    .run_phases([20])?;
# Ok(())
# }
```

Use `.without_completion_examination()` on `Study::builder(...)` only when the
launch intentionally wants every selected phase invoked. This switch does not
define overwrite behavior and does not make existing application output safe
to replace.

### `state`: typed scientific state and in-memory series

The `state` module defines heterogeneous state schemas and values.
`SystemStateSchema` declares the ordered fields in a complete scientific state.
Each advanced `StateFieldSchema` describes a stable name, template position,
and optional scientific description.
`SystemState` stores values that have been checked against that schema, and
`StateTime` gives iteration and optional physical-time identity.

Payload support is type-erased at the storage boundary but remains validated by
its specification. This permits one state to contain several scientific value
types without reducing everything to an untyped JSON object. Tuple helpers make
common multi-field states ergonomic while preserving the same underlying
schema checks.

This module owns in-memory shape and value compatibility. It does not choose
equations, mutate a model, schedule observation, or write files.

The same module stores an ordered collection of compatible `SystemState`
values for analysis that should remain in memory. `StateSeries` owns the
observations; borrowing `&StateSeries` provides access without copying
payloads or introducing a separate view type.

Insertion checks schema and time ordering. The module is useful for small
trajectories, derived windows, and analysis inputs where filesystem persistence
would be unnecessary. It intentionally has no codecs, background writer,
sampling policy, or execution-directory behavior; durable or large trajectories
belong at the `writer` and `storage` boundaries.

### `writer`: inferred scientific observation

The `writer` module lets application code express only scientific output
intent. `Writer::all_fields()` infers one stream named `state`, selects every
schema field, and samples every iteration. `Writer::fields(...)` keeps that
default stream while narrowing its payloads. Applications use named `Stream`
values only when several scientifically distinct outputs or cadences are
needed.

Axis names, schema order, checkpoint eligibility, final-state deduplication,
stream directories, paths, buffering, provenance, and lifecycle are inferred
or owned outside the basic API. The only basic types are `Writer`, `Stream`,
and `WriterError`. `writer::advanced` adds schema-bound descriptors, checked
borrowed observations, owned encoded handoffs, and a replaceable sink port;
the concrete writer session remains private.

### `storage`: durable bounded recordings

The `storage` module turns complete typed states into recoverable on-disk
recordings. During migration to the target `record` subsystem,
`SystemStateWriterBuilder` accepts one application `Writer`, optional caller
metadata, and transitional persistence tuning. It binds the writer to the
state schema and infers axis names, canonical field order, checkpoint coverage,
stream directories, and a safe default buffering policy. `SystemStateWriter`
accepts states, writes bounded chunks, maintains recording metadata, and seals
a terminal result.

Filesystem APIs use typed paths. Builders retain an owned `PathBuf`, while
recording and reader inspection returns borrowed `&Path` values. Logical
stream names never double as filesystem paths.

Recordings distinguish running, complete, and failed lifecycle states.
Continuation validates the existing metadata, schemas, chunks, checksums, and
checkpoint authority before appending. An unpublished tail can be discarded
only through the explicit continuation rules; sealed history is not silently
rewritten.

`CompletedRecording` represents a successfully finalized result.
`StoredStateSeriesReader` reconstructs verified streams and checkpoints, and
payload decoder APIs allow caller-owned types to participate in JSON-backed
storage without making the storage layer understand their scientific meaning.

Storage owns persistence integrity and reconstruction. The application owns
what to record and any non-default cadence through `Writer`; the runtime will
own why and when a run reaches its terminal lifecycle state.

### `execution`: replicate isolation and filesystem identity

The `execution` module owns `ReplicateExecutor`, `ReplicateContext`, and
collision-resistant run directories through `ExecutionScope`. Replicate
dispatch uses the operating system's process API directly: the controller
starts the current executable, preserves its arguments, supplies one internal
worker index, and observes the resulting status. It does not introduce a
second task scheduler or process-pool abstraction.

A caller can also create a named scope, create a generated scope, or open an
existing scope directly. Child paths are validated so semantic task
identifiers cannot accidentally become unsafe path traversals.

An execution scope is only a filesystem lifecycle boundary. Replicate dispatch
does not create study tasks, interpret scientific parameters, define recording
schemas, or choose machine-level CPU and memory limits.

### `artifact`: immutable verified inputs

The `artifact` module publishes immutable byte payloads under content-derived
identity. `persist_artifact` writes or reuses exact content,
`ArtifactDescriptor` records its identity, and `load_verified_artifact` checks
the stored bytes before returning them.

This is intended for scientific inputs and derived products whose exact bytes
matter. It prevents a path with familiar spelling from silently referring to
different content. Artifact publication does not decide how bytes are encoded
or what they mean; those remain caller responsibilities.

### `rng_record`: random-source provenance

The `rng_record` module stores validated descriptions of caller-owned random
number generators. `RngRecord` captures namespace, implementation identity,
version, seed material, and optional method-specific metadata, and can be
inserted into recording metadata under `RNG_RECORDS_METADATA_KEY`.

`ReplicateContext::seed_deriver()` supplies a `ReplicateSeedDeriver` initialized
from `study.json` and the worker's zero-based replicate index. The deriver
lazily derives a named stream seed. Its versioned, domain-separated SHA-256 contract
is independent of request order, process scheduling, and sequential versus
parallel execution. Every `DerivedSeed` carries the exact `RngRecord` that
should accompany persisted output:

```rust
use scientific_workflow::rng_record::ReplicateSeedDeriver;

let replicate = ReplicateSeedDeriver::new(1101, 3);
let matrix = replicate.derive("matrix")?;
let pairing = replicate.derive("pairing/task-17")?;
assert_ne!(matrix.value(), pairing.value());
# Ok::<(), scientific_workflow::rng_record::RngRecordError>(())
```

The module does not construct an RNG, sample distributions, or claim that two
algorithms are interchangeable merely because they share a seed. Applications
use the derived `u64` with their selected RNG implementation and retain the
provided provenance record.

### `prelude`: narrow imports, no new behavior

The prelude uses tiered scope management:

- `prelude::basic` re-exports the complete ordinary state, writer, and task
  tiers. It also temporarily contains legacy configuration, storage,
  execution, artifact, and RNG primitives. The target `config::basic` tier is
  aggregated but intentionally contributes no names.
- `prelude::advanced` is a strict superset that adds supported state
  inspection, writer-backend, task-runtime, and read-only config integration
  contracts.
- `prelude::study` temporarily re-exports legacy orchestration types during
  runtime migration; it is not a third long-term tier.

Prelude modules are aliases for canonical public types. They do not compile a
second implementation, wrap behavior, or introduce another ownership layer.

## Plans, records, and recordings are different

The crate deliberately uses three related but distinct durable concepts:

- A **study plan** describes what phases and tasks were declared, in what order,
  with which scheduling policy and immutable metadata.
- A **study record** describes what happened during an execution: lifecycle
  status, timestamps, duration, progress, and the same task provenance.
- A **scientific recording** contains typed state streams, chunks, checkpoints,
  schemas, and scientific metadata.

Keeping them separate avoids treating scheduler status as scientific state or
forcing a large state recording into a small orchestration record.

## What Scientific Workflow intentionally does not do

The crate provides the narrow `ScientificModel` execution contract, but no
universal solver abstraction, distributed queue, cloud service, database,
dataframe abstraction, plotting API, or domain ontology. It does not decide
whether a simulation is correct. It provides auditable infrastructure around scientific
authority in the code that owns the model.

That restraint is part of the design: a small set of well-owned tools is easier
to compose, test, and trust than a framework that attempts to absorb every
layer of a scientific application.

## Failure model

Public fallible operations return contextual error enums. Errors retain paths,
keys, ordinals, task identities, schema details, and underlying sources where
appropriate. Builders validate before publishing immutable objects. Durable
operations avoid presenting partial output as success.

Workload errors are preserved as task failures. Panics are contained at the
scheduler boundary and reported as failed execution rather than successful
completion. Cancellation remains distinct from scientific success.

## Compatibility expectations

Before 1.0, release notes and public type signatures are the compatibility
contract. Persisted formats carry explicit format or generator identities where
their interpretation must survive software changes. Consumers should pin a
crate version, retain provenance with results, and validate migrations against
representative recordings rather than assuming an API bump is behaviorally
neutral.
