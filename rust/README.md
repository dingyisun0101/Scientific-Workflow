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

Scientific Workflow separates those responsibilities. A typical program has
the following flow:

```text
fixed.json + sweep.json       paths.json
          │                       │
          ▼                       ▼
  ConfigurationSpace         ProjectPaths
          │                       │
          └────── application maps combinations ──────┐
                                                       ▼
                                              Study → Phase → Task
                                                       │
                          application-owned workload ──┤
                                                       ▼
       artifacts + RNG records + typed SystemState + ExecutionScope
                                                       │
                                                       ▼
                         bounded Storage → completed recording/checkpoint
                                                       │
                                                       ▼
                                   reader or in-memory StateSeries analysis
```

Every arrow is explicit. Configuration does not silently execute work. A task
does not silently choose a filesystem location. Storage does not decide model
semantics. The application connects the pieces and therefore remains the owner
of scientific meaning.

## Design philosophy

### One owner for each concern

Each public module has one primary responsibility. Orchestration belongs to
`study`, durable state belongs to `storage`, filesystem run identity belongs to
`execution`, and scientific values belong to `system_state`. The same behavior
should not be implemented again in a neighboring layer.

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

Rust workloads cannot be forcibly stopped safely. Cancellation, task timeouts,
and phase deadlines are cooperative: the scheduler requests cancellation and
the workload observes it through `TaskContext`. This makes the control contract
honest and avoids pretending that arbitrary scientific code can be terminated
without cleanup.

## Public modules

### `configuration`: strict experiment inputs

The `configuration` module turns validated JSON documents into immutable,
deterministic scientific inputs.

`ConfigurationSpace` reads a directory containing `fixed.json` and
`sweep.json`. Fixed leaves are shared by every combination. Swept leaves are
expanded either as a Cartesian product or as explicit cases.
`ResolvedConfiguration` represents one combination and supports exact JSON
Pointer lookup, typed decoding, iteration over terminal keys, and reconstruction
of the complete nested document.

The module also provides `ProjectPaths`. It strictly loads the conventional
`config/paths.json`, rejects duplicate or blank entries, preserves declaration
order and original bytes, and resolves relative values lexically against a
project root. It does not canonicalize targets, expand shell syntax, or require
paths to exist; those policies remain explicit application decisions.

An input tree normally looks like this:

```text
study/
└── config/
    ├── fixed.json
    ├── sweep.json
    └── paths.json
```

Example fixed and sweep documents:

```json
{
  "solver": { "time_step": 0.01 },
  "maximum_iterations": 10000
}
```

```json
{
  "mode": "cartesian",
  "axes": {
    "temperature": { "values": [280.0, 300.0] },
    "seed": { "values": [7, 11] }
  }
}
```

```rust,no_run
use scientific_workflow::configuration::{ConfigurationSpace, ProjectPaths};

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let space = ConfigurationSpace::load("study/config")?;
let paths = ProjectPaths::load("study")?;

for configuration in space.combinations() {
    let (temperature, seed): (f64, u64) =
        configuration.decode_values(("/temperature", "/seed"))?;
    println!(
        "combination={} temperature={temperature} seed={seed}",
        configuration.ordinal()
    );
}

let recording_root = paths.resolve_path("recordings")?;
println!("recordings: {}", recording_root.display());
# Ok(())
# }
```

Configuration does not register tasks, choose concurrency, create output, or
validate domain-specific physics. It only defines and expands inputs.

### `study`: orchestration and lifecycle

The `study` module is the control plane. Its vocabulary is deliberately small:

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

A `Phase` owns a deterministic list of tasks and phase-local execution policy:
maximum active tasks, prepared queue capacity, start interval, per-task timeout,
phase deadline, dependencies, failure behavior, and optional confirmation.

A `Task` owns identity, category, label, mode, immutable metadata, and exactly
one application workload. Progress and one-shot tasks share the same type.
`Task::completed` represents work that the application has independently
verified as already satisfied.

`Task::one_shot_for_configuration` and
`Task::progress_for_configuration` provide the conventional task identity and
attach the complete resolved configuration. `with_project_paths` adds the named
path table. These helpers prevent each application from inventing a different
configuration-to-task convention.

```rust,no_run
use scientific_workflow::configuration::{ConfigurationSpace, ProjectPaths};
use scientific_workflow::prelude::study::*;

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let configurations = ConfigurationSpace::load("study/config")?;
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
    .build()?;

let summary = Study::builder("study-record.json")
    .phase(phase)
    .build()?
    .run()?;
assert!(summary.is_success());
# Ok(())
# }
```

The only communication channel supplied to a workload is `TaskContext`. It
reports progress and human-readable detail, exposes identity and metadata, and
observes cooperative cancellation. It does not provide hidden filesystem,
network, subprocess, state, or artifact capabilities.

The terminal renderer is centralized so worker threads never compete for
stdout. Automatic, interactive, plain, and hidden display modes change
presentation without changing scheduling semantics.

### `system_state`: typed scientific state

The `system_state` module defines heterogeneous state schemas and values.
`SystemStateSchema` declares the ordered fields in a complete scientific state.
Each `StateFieldSchema` owns a stable name and payload specification.
`SystemState` stores values that have been checked against that schema, and
`SimulationTime` gives iteration and optional physical-time identity.

Payload support is type-erased at the storage boundary but remains validated by
its specification. This permits one state to contain several scientific value
types without reducing everything to an untyped JSON object. Tuple helpers make
common multi-field states ergonomic while preserving the same underlying
schema checks.

This module owns in-memory shape and value compatibility. It does not choose
equations, mutate a model, schedule observation, or write files.

### `time_series`: ordered in-memory observations

The `time_series` module stores an ordered collection of compatible
`SystemState` values for analysis that should remain in memory. `StateSeries`
owns the observations, while `StateSeriesView` provides borrowed access without
copying payloads.

Insertion checks schema and time ordering. The module is useful for small
trajectories, derived windows, and analysis inputs where filesystem persistence
would be unnecessary. It intentionally has no codecs, background writer,
sampling policy, or execution-directory behavior; durable or large trajectories
belong in `storage`.

### `storage`: durable bounded recordings

The `storage` module turns complete typed states into recoverable on-disk
recordings. `SystemStateWriterBuilder` configures streams, sampling intervals,
buffer limits, time-axis metadata, and caller metadata. `SystemStateWriter`
accepts states, writes bounded chunks, maintains recording metadata, and seals a
terminal result.

Recordings distinguish running, complete, and failed lifecycle states.
Continuation validates the existing metadata, schemas, chunks, checksums, and
checkpoint authority before appending. An unpublished tail can be discarded
only through the explicit continuation rules; sealed history is not silently
rewritten.

`CompletedRecording` represents a successfully finalized result.
`StoredStateSeriesReader` reconstructs verified streams and checkpoints, and
payload decoder APIs allow caller-owned types to participate in JSON-backed
storage without making the storage layer understand their scientific meaning.

Storage owns persistence integrity and reconstruction. The application still
owns what to record, when an observation is scientifically meaningful, and why
a run terminates.

### `execution`: filesystem identity for runs

The `execution` module owns collision-resistant run directories through
`ExecutionScope`. A caller can create a named scope, create a generated scope,
or open an existing scope according to the API being used. Child paths are
validated so semantic task identifiers cannot accidentally become unsafe path
traversals.

An execution scope is only a filesystem lifecycle boundary. It does not create
tasks, interpret configuration, define recording schemas, or select scientific
parameters.

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

The module records provenance only. It does not generate random numbers or
claim that two different algorithms are interchangeable merely because they
share a seed.

### `prelude`: narrow imports, no new behavior

The prelude is split by responsibility:

- `prelude::basics` re-exports configuration, state, storage, execution,
  artifact, and RNG primitives used by scientific code.
- `prelude::study` re-exports orchestration types used at the application
  boundary.

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

The crate does not provide a universal model trait, solver registry, distributed
queue, cloud service, database, dataframe abstraction, plotting API, or domain
ontology. It does not decide whether a simulation is correct. It provides
auditable infrastructure around scientific work while keeping scientific
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
