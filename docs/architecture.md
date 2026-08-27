# Workflow architecture

This is the first-time map of the Workflow repository: what users author, how
one run moves through the system, where each responsibility lives, and what
every source file does.

## User workflow

An application author supplies scientific Rust models and/or executable
programs plus project JSON:

```text
<project-root>/
├── src/
│   ├── main.rs                 calls scientific_workflow::run(&Path)
│   └── <models>.rs             registered ScientificModel implementations
├── scripts/                     optional executable and `.py` task programs
├── study.json                  phases, tasks, replicates, operational policy
└── config/
    ├── state.json              canonical state schema
    └── parameters.json         every custom-project parameter namespace
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
paths, persistence, progress counters, messages, or worker threads. Any
executable can be a task by declaring `program`; a `.py` file can declare its
environment manager inside nested `python`. Neither needs a Rust wrapper or
registration.

## First-level modules

The current first-level library modules are:

- `state`: canonical scientific state, schema, time, payloads, and in-memory
  series;
- `observation`: application-authored scientific selection, cadence, units,
  and private encoding;
- `task`: generic model/program workload abstraction (including Python
  lowering), `ScientificModel`
  contract, linked model registration, and uniform private execution;
- `config`: sole reader/parser of all project JSON, immutable central snapshot,
  executable/Python-environment resolution, and sole typed model-constants
  supplier;
- `study`: effect-free coordinator of all declared intent and preflight;
- `runtime`: sole coordinator of active execution and output creation;
- `persistence`: automatic durable recordings and verified reconstruction;
- `ui`: automatic terminal presentation of Runtime-owned progress facts;
- `prelude`: central aggregation of module-owned API tiers; and
- `error`: the complete-workflow error boundary.

Legacy `writer`, `storage`, `execution`, `artifact`, and `rng_record`
modules have been removed; their surviving responsibilities are owned by
`observation`, `persistence`, or `runtime`.

## End-to-end flow

```text
project root: &Path
        │
        ▼
crate facade: run(&Path)
  owns complete-workflow composition and error conversion
        │ Study::load
        ▼
config
  all project JSON captured once + strict Workflow grammar
  + paths + defaults + expansion + executable/Python resolution
        │ private Config / ProjectSpecification / ResolvedTask
        ▼
study
  retained Config + state semantics + model discovery + constants decode
  + generic program/Python tasks + one-time observation-plan binding
  + identities + phases
        │ immutable Study returned to the crate facade
        ▼
runtime::execute
  execution directory + replicates + scheduling + cancellation
  + model initialization/stepping or direct program/Python invocation
        ├──── model observation boundaries ────────► persistence
        │                                             private bounded writer
        │                                             + atomic metadata/chunks
        │                                             + verified reads
        ├──── program snapshot/log/status/artifacts ─► persistence
        │
        └──── lifecycle/progress facts ─────────────► ui
                                                      Ratatui + exit cancellation
```

The crate facade owns only the transition from project root to Study to
Runtime. Study is the ultimate coordinator of declared intent. Runtime is the
ultimate coordinator of active execution. Config never discovers Rust model
types. Study never creates output or initializes a model. Study and Runtime
never reparse the captured documents. Persistence never decides scientific
observation meaning.

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
├── crate-facade run(&Path)
└── WorkflowError

advanced additions
├── state schema inspection and deliberate maintenance
├── ConfigError
├── Study / StudyError
├── execute(Study), RuntimeError, and successful summaries including TaskRunKind
└── completed-recording readers, decoders, timing, and PersistenceError
```

Dependency direction is one-way:

```text
observation ──► state
persistence ─► observation + state
task        ──► config + observation + state
study       ──► config + observation + persistence + state + task + ui plan
runtime     ──► config + persistence + state + study + task + ui events
error       ──► StudyError + RuntimeError
crate facade──► study + runtime + error

prelude aggregates supported tiers
```

Peers import another subsystem through its `advanced` boundary. Config does
not depend on task: it treats model keys as opaque. Study owns the cross-domain
match. Task asks config-owned resolved model parameters for one complete typed
constants value or delegates one resolved program. Runtime receives only a fully
preflighted Study and its retained Config.

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
│   │   ├── error.rs                  error root and inline API tiers
│   │   ├── error/api.md              complete facade-error contract
│   │   ├── error/workflow.rs         WorkflowError composition
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
│   │   ├── task/definition.rs        type-erased model/program execution definitions
│   │   └── task/execution.rs         host port and model invariant enforcement
│   │   │
│   │   ├── config.rs                 config root; empty Basic, error-only Advanced
│   │   ├── config/api.md             complete project grammar/error contract
│   │   ├── config/error.rs           owned contextual ConfigError
│   │   ├── config/document.rs        strict JSON and duplicate-key parser
│   │   ├── config/store.rs           central immutable all-document Config snapshot
│   │   ├── config/manifest.rs        study grammar, defaults, dependency checks
│   │   ├── config/expansion.rs       deterministic $sweep/$cases compiler
│   │   ├── config/parameters.rs      resolved model parameters + typed decode
│   │   ├── config/program.rs         validated resolved executable declaration
│   │   ├── config/python.rs          nested Python environment validation/lowering
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
│   │   ├── runtime.rs                runtime root; empty Basic, execute Advanced
│   │   ├── runtime/api.md            execute/summary/error contract
│   │   ├── runtime/error.rs          active execution RuntimeError
│   │   ├── runtime/output.rs         private unique execution/replicate directories
│   │   ├── runtime/host.rs           model/program execution and persistence adapter
│   │   ├── runtime/execution.rs      Study-only replicate/phase/task schedulers
│   │   └── runtime/summary.rs        successful immutable RunSummary tree
│   │   │
│   │   ├── ui.rs                     UI root; empty public Basic/Advanced tiers
│   │   ├── ui/api.md                 automatic presentation contract
│   │   ├── ui/plan.rs                private Study-owned inferred UI policy
│   │   ├── ui/event.rs               borrowed Runtime-to-UI fact vocabulary
│   │   ├── ui/command.rs             former command editor and exact exit parser
│   │   ├── ui/state.rs               event-reduced rows/messages/status snapshot
│   │   ├── ui/session.rs             renderer thread and cancellation bridge
│   │   └── ui/terminal.rs            Ratatui dashboard + plain fallback
│   │   │
│   │   ├── persistence.rs            persistence root; empty Basic, read Advanced
│   │   ├── persistence/api.md        complete settings/read/error contract
│   │   ├── persistence/plan.rs       private effective operational settings
│   │   ├── persistence/session.rs    model recording/program workspace lifecycle
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
│       ├── integration_surface.rs    facade/module/prelude boundary tests
│       ├── state_workflow.rs          downstream state API/ownership tests
│       ├── analysis_workflow.rs       in-memory series analysis tests
│       ├── observation_workflow.rs    public observation declaration tests
│       ├── task_workflow.rs           downstream ScientificModel API tests
│       └── fixtures/*.json            canonical state-schema fixtures
└── examples/attractor_2d/
    ├── Cargo.toml / Cargo.lock         standalone example package
    ├── src/main.rs                     one run(&Path) call
    ├── src/hopf_model.rs               registered state-owning model
    ├── study.json                      swept simulation then Python plot phase
    ├── config/state.json               canonical model state schema
    ├── config/parameters.json          model sweeps + plotter settings
    └── scripts/plot.py                 direct verified-recording SVG task
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

`ScientificModel` is the irreducible user contract for stateful Rust science.
Task itself is generic: Study may instead bind a resolved executable with
direct arguments and no public Rust adapter. Config lowers a nested Python
script/environment declaration to that same executable boundary. A model
consumes typed constants, initializes from the shared schema, and directly owns
the stable state returned by `state()`. It reports completion and performs one
iteration-advancing `step`. Task enforces what Rust can observe: state address/schema stability,
strict iteration progress, and monotonic optional target. The macro submits
immutable registration metadata; the private catalog rejects bad/duplicate
keys and ignores linker order.

### Config

Config canonicalizes the project and `config` roots and parses `study.json`,
`config/state.json`, and the complete arbitrary `config/parameters.json`
namespace with duplicate-key rejection. One clone-cheap immutable Config
retains the entire value graph. A model key automatically selects its same-name
parameter section; no manifest input path exists. Config expands selections
deterministically, resolves program paths and Python
scripts/environment managers once, and creates a
deterministic language-neutral snapshot for external tasks. Reserved Workflow
documents and arbitrary application documents use the same lookup graph. The
public Advanced API is only `ConfigError`.

### Study

`Study::load(&Path)` performs all cross-domain checks before output: state
semantics, linked registration validation, model-key resolution, generic
program and Python-environment resolution, constants decoding, and
observation/schema binding. It retains
the central Config, infers stable identities, labels, the output root, and
private operational policy. Public inspection is limited
to project/output roots; phases, tasks, schema, resolved parameters, and policies
exist only for Runtime.

### Runtime

The crate-level `run(&Path)` loads a Study and passes it to
`runtime::advanced::execute(Study)`. Runtime has no project-root or loading
entry point: it consumes only complete immutable intent. Runtime alone creates
`output/execution-<pid>-<sequence>`, isolated
`replicate-NNNNNN` directories, and deterministic task recording paths. It
topologically schedules generic tasks, applies concurrency/start intervals and
fail-fast/finish-all policy, checks cooperative cancellation between model
steps, directly starts programs and resolved Python launchers without a shell,
and returns deterministic successful summaries. A blocking user `step` cannot
be forcibly killed safely; an external child is killed and reaped on observed
cancellation.

### Persistence

Persistence is constructed and run only by Runtime. The model constructor
accepts an inferred destination, the already-bound observation plan, provenance,
and one effective shared chunk/queue policy. A bounded worker owns all stream
writes and commits immutable JSONL chunks plus one authoritative
`metadata.json` lifecycle. There is no writer builder, per-stream storage
override, public flush, resume/continuation path, or completion handle.

Users author every persistence size as a positive integer decimal MB, with one
MB equal to 1,000,000 bytes. The `study.json` fields are
`persistence.chunk_target_mb` and `persistence.queue_capacity_mb`; no JSON
persistence-size field is byte-addressed. Config alone checks and converts
both values to internal byte counts, and effective provenance records those
exact byte values.

Advanced users can only open completed recordings with a decoder registry.
Readers verify metadata, sizes, hashes, framing, ordering, schema, decoding,
and StateSeries invariants before publishing owned results. A future local or
remote adapter belongs behind the private `PersistenceSession`, not in model
or task APIs. A separate private program session creates an isolated artifacts
directory, freezes central config and dependency JSON, captures stdout/stderr,
records generic-program or Python launcher provenance, and atomically publishes
running/complete/failed metadata. The workspace remains the external task's
default working area, but Python or another external program owns its
domain-specific IO and may write to a safe project-relative destination from
`parameters.json`; the attractor plotter uses `output/plots`.

Model recording provenance calls the selected combination
`parameter_ordinal` and its canonical `parameters.json` document
`parameter_source`; the removed per-task input-file vocabulary is not retained
on disk.

### UI

Study owns a private zero-configuration `UiPlan`; its current effective policy
is inferred rather than authored in `study.json`. Runtime constructs one
clone-cheap `UiSession` after creating the execution output and publishes
borrowed execution, replicate, phase, task, iteration, outcome, and path facts.
Model progress comes from the same host boundaries already used for automatic
persistence. Programs publish generic lifecycle facts without invented
iteration values, so neither workload supplies UI code or values.

Interactive stdin and stderr select the Ratatui/Crossterm alternate-screen
dashboard; noninteractive runs use stable plain lifecycle lines. The dashboard
owns declaration-ordered persistent task rows, progress gauges/spinners,
elapsed/ETA fields, a bounded message panel, and the former command editor.
Exact lowercase `exit` or Ctrl+C requests cooperative Runtime cancellation,
stops further admission, and waits for active model/program cleanup before
terminal restoration. One private refresh thread retains presentation facts
but never scientific payloads. Its public Basic and Advanced tiers remain
empty.

### Error and prelude integration

`error::basic::WorkflowError` composes the effect-free `StudyError` and active
`RuntimeError` stages without absorbing either subsystem's detailed
vocabulary. The crate root re-exports that type and owns the sole ordinary
`run(&Path)` facade. `error::advanced` is currently the same supported set.

Prelude Basic aggregates all subsystem Basic scopes plus the crate-owned
`run`, `model`, and `WorkflowError` conveniences. Prelude Advanced is its
strict superset and aggregates every subsystem Advanced scope. Neither prelude
owns behavior or creates an alternative canonical implementation path.

## Core invariants

1. Filesystem APIs borrow `&Path` and retain `PathBuf`; raw strings are not
   path parameters.
2. Config is the sole project JSON reader/parser, central immutable snapshot,
   executable/Python-environment resolver, and typed constants supplier.
3. Study completes all model/constants/observation and program/Python binding
   before output and retains the exact Config snapshot.
4. Model registration keys are authored stable semantics, never Rust type names.
5. Models are initialized only during active Runtime execution.
6. A model directly owns one stable canonical `SystemState`.
7. Every successful `step` strictly advances scientific iteration.
8. Observation plans are deterministic, side-effect-free, evaluated once, and
   stored schema-bound.
9. The crate facade alone turns a project root into a Study and then invokes
   Runtime; Runtime accepts only a completed Study.
10. Runtime alone creates output and owns scheduling/cancellation and external
    process invocation.
11. Persistence writing is private, automatic, bounded, and finalized by
    Runtime; application code cannot flush, resume, or complete it.
12. Effective paths, identities, labels, and operational defaults are inferred
    whenever one safe deterministic answer exists.
13. Public APIs contain irreducible user intent or a deliberate read/embedding
    contract, not internal flexibility for hypothetical use.
14. UI consumes only Runtime facts, activates automatically, and never turns a
    presentation failure into scientific failure.
15. Model and external-program tasks share phase, dependency, timeout, failure,
    summary, persistence-workspace, and UI lifecycle semantics without forcing
    fake state or iteration onto programs.
16. A Python environment is declared inside its task's `python` object and is
    completely resolved during Study loading; there is no global environment
    registry or runtime environment discovery.

## Replacement boundaries

- A state replacement preserves typed ownership, schema identity, time
  ordering, and serialization borrows.
- An observation replacement preserves declaration meaning, one-time binding,
  cadence, clone-free borrowing, and deterministic encoded order.
- A task replacement preserves config-owned constants decode, direct state
  ownership checks, automatic observation boundaries, and generic program
  delegation without public adapters.
- A config replacement preserves the grammar, typed-path containment,
  duplicate-key rejection, deterministic expansion, all-document immutable
  snapshots, direct-program and nested Python-environment resolution, and
  centralized parsing.
- A Study replacement remains effect-free and performs complete binding before
  publishing immutable intent.
- A Runtime replacement consumes only Study, preserves policy and summary
  order, and owns all output/effects.
- A persistence replacement remains behind the private session and preserves
  bounded model submission, program workspace isolation/snapshots/logs,
  terminal evidence/provenance, and verified model reads.
- A UI replacement remains downstream of Runtime, requires no model/config
  participation, handles concurrent publishers, and stays best-effort.
