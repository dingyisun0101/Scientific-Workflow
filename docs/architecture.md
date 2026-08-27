# Workflow architecture

This is the first-time map of the Workflow repository: what users author, how
one run moves through the system, where each responsibility lives, and what
every source file does.

## User workflow

An application author supplies only scientific Rust models and project JSON:

```text
<project-root>/
├── src/
│   ├── main.rs                 calls scientific_workflow::run(&Path)
│   └── <models>.rs             registered ScientificModel implementations
├── study.json                  phases, tasks, replicates, operational policy
└── config/
    ├── state.json              canonical state schema
    └── inputs/
        └── <model>.json        constants, sweeps, or correlated cases
```

Each model directly owns its canonical `SystemState` and is linked by a stable
semantic key:

```rust,ignore
#[scientific_workflow::model("population")]
impl ScientificModel for PopulationModel {
    // Constants, initialization, canonical state, completion, and one step.
}
```

The executable needs no registry or orchestration builder:

```rust,ignore
fn main() -> Result<(), scientific_workflow::WorkflowError> {
    scientific_workflow::run(std::path::Path::new("."))
}
```

The application never constructs tasks, phases, a Study, Runtime, output
paths, persistence, progress counters, messages, or worker threads.

## First-level modules

The current first-level library modules are:

- `state`: canonical scientific state, schema, time, payloads, and in-memory
  series;
- `observation`: application-authored scientific selection, cadence, units,
  and private encoding;
- `task`: the `ScientificModel` contract, linked registration, and uniform
  private execution;
- `config`: sole project-JSON reader/parser and sole typed constants supplier;
- `study`: effect-free coordinator of all declared intent and preflight;
- `runtime`: sole coordinator of active execution and output creation;
- `persistence`: automatic durable recordings and verified reconstruction;
- `prelude`: central aggregation of module-owned API tiers; and
- `error`: the complete-workflow error boundary.

There is currently no first-level UI module. Progress can later be inferred
from runtime-owned task/state boundaries without changing model contracts.
Legacy `writer`, `storage`, `execution`, `artifact`, and `rng_record`
modules have been removed; their surviving responsibilities are owned by
`observation`, `persistence`, or `runtime`.

## End-to-end flow

```text
project root: &Path
        │
        ▼
config
  strict JSON + duplicate keys + paths + defaults + expansion
        │ private ProjectSpecification / ResolvedTaskInput
        ▼
study
  state semantics + model discovery + constants decode
  + one-time observation-plan binding + identities + phases
        │ immutable Study
        ▼
runtime
  execution directory + replicates + scheduling + cancellation
  + model initialization and stepping
        │ automatic observation boundaries
        ▼
persistence
  private bounded writer + atomic metadata/chunks + verified reads
```

Study is the ultimate coordinator of declared intent. Runtime is the ultimate
coordinator of active execution. Config never discovers Rust model types.
Study never creates output or initializes a model. Runtime never reparses or
rebinds declarations. Persistence never decides scientific observation
meaning.

## API tiers and dependency direction

Each first-level subsystem root defines inline `basic` and `advanced`
scope-management modules. There are no `mod.rs`, `basic.rs`, or
`advanced.rs` files.

- `module::basic` is the ordinary application surface.
- `module::advanced` publicly re-exports Basic and adds only deliberate
  advanced-user contracts.
- The same Advanced scope may carry `pub(crate)` exports for peer modules;
  crate-visible internals do not become public.
- `prelude::basic` and `prelude::advanced` aggregate these canonical module
  exports but own no behavior.

Current public surface:

```text
basic
├── state construction and manipulation
├── ObservationPlan / ObservationStream
├── ScientificModel / TaskResult / #[model]
├── run(&Path)
└── WorkflowError

advanced additions
├── state schema inspection and deliberate maintenance
├── ConfigError
├── Study / StudyError
├── execute(Study), RuntimeError, and successful summaries
└── completed-recording readers, decoders, timing, and PersistenceError
```

Dependency direction is one-way:

```text
observation ──► state
persistence ─► observation + state
task        ──► config + observation + state
study       ──► config + observation + persistence + state + task
runtime     ──► config + persistence + state + study + task

prelude aggregates supported tiers
error   composes StudyError + RuntimeError
```

Peers import another subsystem through its `advanced` boundary. Config does
not depend on task: it treats model keys as opaque. Study owns the cross-domain
match. Task asks a config-owned resolved input for one complete typed constants
value. Runtime receives only a fully preflighted Study.

## Source tree and file responsibilities

```text
workflow/
├── AGENTS.md                         local/private working rules (gitignored)
├── README.md                         repository entry and user outcome
├── docs/
│   ├── architecture.md               this complete ownership/tree guide
│   └── tests.md                      validation responsibilities and commands
├── macros/
│   ├── Cargo.toml                    proc-macro package declaration
│   └── src/lib.rs                    #[model] validation and registration expansion
├── rust/
│   ├── Cargo.toml                    primary library package/dependencies
│   ├── README.md                     complete Rust user procedure
│   ├── src/
│   │   ├── lib.rs                    module declarations; run/macro/error exports
│   │   ├── clock.rs                  private UTC formatting and duration conversion
│   │   ├── error.rs                  crate-level WorkflowError composition
│   │   ├── prelude.rs                inline Basic/Advanced central aggregation
│   │   ├── prelude/api.md            exhaustive prelude export contract
│   │   │
│   │   ├── state.rs                  state module root and inline API tiers
│   │   ├── state/api.md              exhaustive state API and examples
│   │   ├── state/error.rs            schema/state/time/series error enums
│   │   ├── state/field.rs            immutable field metadata
│   │   ├── state/schema.rs           Path loader and schema semantic authority
│   │   ├── state/state.rs            heterogeneous payload owner and tuple borrows
│   │   ├── state/time.rs             StateTime and checked advancement
│   │   ├── state/series.rs           ordered in-memory SystemState collection
│   │   └── state/value.rs            private erased payload/type/Serde adapter
│   │   │
│   │   ├── observation.rs            observation root and inline API tiers
│   │   ├── observation/api.md        exhaustive declaration API and example
│   │   ├── observation/error.rs      declaration/binding/encoding errors
│   │   ├── observation/plan.rs       public plan + private schema-bound plan
│   │   ├── observation/stream.rs     public stream + private bound stream
│   │   ├── observation/sampling.rs   private cadence decision
│   │   ├── observation/state_observation.rs checked borrowed state view
│   │   ├── observation/encoding.rs   canonical owned encoded records
│   │   └── observation/session.rs    cadence state and final-state deduplication
│   │   │
│   │   ├── task.rs                   task root and inline API tiers
│   │   ├── task/api.md               exhaustive model contract and example
│   │   ├── task/model.rs             ScientificModel and direct-state requirements
│   │   ├── task/result.rs            boxed application error alias
│   │   ├── task/catalog.rs           linked registrations and sorted validation
│   │   ├── task/definition.rs        type-erased constants/preflight/execution bridge
│   │   └── task/execution.rs         host port and model invariant enforcement
│   │   │
│   │   ├── config.rs                 config root; empty Basic, error-only Advanced
│   │   ├── config/api.md             complete project grammar/error contract
│   │   ├── config/error.rs           owned contextual ConfigError
│   │   ├── config/document.rs        strict JSON and duplicate-key parser
│   │   ├── config/manifest.rs        study grammar, defaults, dependency checks
│   │   ├── config/expansion.rs       deterministic $sweep/$cases compiler
│   │   ├── config/input.rs           private resolved input and typed decode
│   │   ├── config/specification.rs   one-root loading transaction
│   │   └── config/tests/config_workflow.rs internal compiler/grammar tests
│   │   │
│   │   ├── study.rs                  study root; empty Basic, Study/Error Advanced
│   │   ├── study/api.md              exhaustive Study API and example
│   │   ├── study/error.rs            binding/preflight StudyError
│   │   ├── study/compilation.rs      project-to-Study composition
│   │   ├── study/plan.rs             public Study + private phases/tasks/policies
│   │   └── study/tests/study_workflow.rs internal binding/runtime tests
│   │   │
│   │   ├── runtime.rs                runtime root and inline API tiers
│   │   ├── runtime/api.md            run/execute/summary/error contract
│   │   ├── runtime/error.rs          active execution RuntimeError
│   │   ├── runtime/output.rs         private unique execution/replicate directories
│   │   ├── runtime/host.rs           task-host to persistence-session adapter
│   │   ├── runtime/execution.rs      replicate/phase/task schedulers and run entry
│   │   └── runtime/summary.rs        successful immutable RunSummary tree
│   │   │
│   │   ├── persistence.rs            persistence root; empty Basic, read Advanced
│   │   ├── persistence/api.md        complete settings/read/error contract
│   │   ├── persistence/plan.rs       private effective operational settings
│   │   ├── persistence/session.rs    private automatic Runtime lifecycle port
│   │   ├── persistence/local.rs      private local recording coordinator/lease
│   │   ├── persistence/local/error.rs exact read/write persistence failures
│   │   ├── persistence/local/jsonl_format.rs metadata/chunk wire structures
│   │   ├── persistence/local/queued_state_writer.rs bounded async chunk writer
│   │   ├── persistence/local/stored_state_series_reader.rs verified reconstruction
│   │   ├── persistence/local/json_payload_decoder.rs decoder registry/contracts
│   │   ├── persistence/local/json_payload_decoder/string.rs String decoder
│   │   ├── persistence/local/json_payload_decoder/vec_f64.rs Vec<f64> decoder
│   │   └── persistence/tests/
│   │       ├── persistence_workflow.rs end-to-end local write/read contract
│   │       ├── persistence_resilience.rs fault/integrity tests
│   │       └── python_reader_conformance.rs Rust/Python format compatibility
│   └── tests/
│       ├── state_workflow.rs          downstream state API/ownership tests
│       ├── analysis_workflow.rs       in-memory series analysis tests
│       ├── observation_workflow.rs    public observation declaration tests
│       ├── task_workflow.rs           downstream ScientificModel API tests
│       └── fixtures/*.json            canonical state-schema fixtures
└── examples/attractor_2d/
    ├── Cargo.toml / Cargo.lock         standalone example package
    ├── src/main.rs                     one run(&Path) call
    ├── src/hopf_model.rs               registered state-owning model
    ├── study.json                      phases/tasks/replicates
    └── config/                         schema and model constants
```

## Subsystem details

### State

`SystemStateSchema::load_json_template(&Path)` is the public construction
boundary. Config uses a crate-private in-memory equivalent so Study does not
reread `state.json`. `SystemState` owns fixed heterogeneous slots and
`StateTime`; application models mutate payloads and advance time. Advanced
inspection exposes field metadata and schema identity. The old generic
schema-source adapter was removed with the persistence builder that needed it.

### Observation

A model's `observation_plan(&Constants)` defaults to all fields. Study calls
it once during preflight and stores the exact schema-bound plan. Runtime does
not call it again. Private sessions select due streams, borrow/encode selected
payloads, and deduplicate the final iteration. Observation owns no paths,
buffers, files, or lifecycle.

### Task

`ScientificModel` is the irreducible user contract. The model consumes typed
constants, initializes from the shared schema, directly owns the stable state
returned by `state()`, reports completion, and performs one iteration-advancing
`step`. Task enforces what Rust can observe: state address/schema stability,
strict iteration progress, and monotonic optional target. The macro submits
immutable registration metadata; the private catalog rejects bad/duplicate
keys and ignores linker order.

### Config

Config canonicalizes the project and `config` roots, parses each required JSON
file with duplicate-key rejection, validates the exact study grammar and safe
input containment, caches parsed input values, and expands selections
deterministically. It retains only values needed by Study; exact source bytes,
duplicate public graph views, and a cloneable top-level `Arc` were removed.
The public Advanced API is only `ConfigError`.

### Study

`Study::load(&Path)` performs all cross-domain checks before output: state
semantics, linked registration validation, model-key resolution, constants
decoding, and observation/schema binding. It infers stable identities, labels,
the output root, and private operational policy. Public inspection is limited
to project/output roots; phases, tasks, schema, resolved inputs, and policies
exist only for Runtime.

### Runtime

`run(&Path)` loads then executes; `execute(Study)` is the advanced split
boundary. Runtime alone creates `output/execution-<pid>-<sequence>`, isolated
`replicate-NNNNNN` directories, and deterministic task recording paths. It
topologically schedules phases, applies concurrency/start intervals and
fail-fast/finish-all policy, checks cooperative cancellation between model
steps, and returns deterministic successful summaries. A blocking user
`step` cannot be forcibly killed safely.

### Persistence

Persistence is constructed and run only by Runtime. One internal constructor
accepts an inferred destination, the already-bound observation plan, provenance,
and one effective shared chunk/queue policy. A bounded worker owns all stream
writes and commits immutable JSONL chunks plus one authoritative
`metadata.json` lifecycle. There is no writer builder, per-stream storage
override, public flush, resume/continuation path, or completion handle.

Advanced users can only open completed recordings with a decoder registry.
Readers verify metadata, sizes, hashes, framing, ordering, schema, decoding,
and StateSeries invariants before publishing owned results. A future local or
remote adapter belongs behind the private `PersistenceSession`, not in model
or task APIs.

## Core invariants

1. Filesystem APIs borrow `&Path` and retain `PathBuf`; raw strings are not
   path parameters.
2. Config is the sole project JSON reader/parser and typed constants supplier.
3. Study completes all model/constants/observation binding before output.
4. Model registration keys are authored stable semantics, never Rust type names.
5. Models are initialized only during active Runtime execution.
6. A model directly owns one stable canonical `SystemState`.
7. Every successful `step` strictly advances scientific iteration.
8. Observation plans are deterministic, side-effect-free, evaluated once, and
   stored schema-bound.
9. Runtime alone creates output and owns scheduling/cancellation.
10. Persistence writing is private, automatic, bounded, and finalized by
    Runtime; application code cannot flush, resume, or complete it.
11. Effective paths, identities, labels, and operational defaults are inferred
    whenever one safe deterministic answer exists.
12. Public APIs contain irreducible user intent or a deliberate read/embedding
    contract, not internal flexibility for hypothetical use.

## Replacement boundaries

- A state replacement preserves typed ownership, schema identity, time
  ordering, and serialization borrows.
- An observation replacement preserves declaration meaning, one-time binding,
  cadence, clone-free borrowing, and deterministic encoded order.
- A task replacement preserves config-owned constants decode, direct state
  ownership checks, and automatic observation boundaries.
- A config replacement preserves the grammar, typed-path containment,
  duplicate-key rejection, deterministic expansion, and centralized parsing.
- A Study replacement remains effect-free and performs complete binding before
  publishing immutable intent.
- A Runtime replacement consumes only Study, preserves policy and summary
  order, and owns all output/effects.
- A persistence replacement remains behind the private session and preserves
  bounded submission, terminal evidence/provenance, and verified reads.
