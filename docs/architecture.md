# Workflow architecture

This is the first-time map of the Workflow repository: what users author, how
one run moves through the system, where each responsibility lives, and what
every source file does.

## User workflow

An application author supplies scientific Rust execution units and/or executable
programs plus project JSON:

```text
<project-root>/
├── src/
│   ├── main.rs                 calls scientific_workflow::run(&Path)
│   └── <units>.rs              registered ExecutionUnit implementations
├── scripts/                     optional executable and `.py` task programs
└── wf_configs/                 required Workflow configuration root
    ├── study.json              phases, tasks, optional seed, operational policy
    ├── parameters.json         every custom-project parameter namespace
    └── states/                 recommended, optional schema grouping
        ├── population.json
        └── environment.json
```

The presence of `wf_configs/` identifies the root passed to `run`; a valid
project requires `wf_configs/study.json` and `wf_configs/parameters.json`.
The `states/` subdirectory is organizational convention, not grammar. A state
schema may use any path beneath `wf_configs/`, but its project-root-relative
path must be registered explicitly in `study.json.paths.states`. State paths
outside the canonical `wf_configs/` boundary are rejected.

Each registered unit exposes one or more independently stateful members and is
linked by a stable semantic key:

```rust,ignore
#[scientific_workflow::execution_unit("population")]
impl ExecutionUnit for PopulationUnit {
    // Constants, initialization, member views, and one coordinated step.
}
```

The attribute only submits registration metadata. It does not create the
unit, generate state fields, wrap member state, or change field access.
A standalone implementation owns an ordinary `SystemState` field and returns
one `MemberView`; an ensemble owns multiple members and returns one stable view
per member. Implementations access coupled payloads through typed tuple borrowing such
as `borrow_payloads_mut::<(Position, Velocity)>(("position", "velocity"))`.

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
- `task`: generic scientific/program workload abstraction (including Python
  lowering), `ExecutionUnit`/`MemberView` contract, linked registration, and
  uniform private execution;
- `config`: sole reader/parser of all project JSON, immutable central snapshot,
  executable/Python-environment resolution, and sole typed execution unit-constants
  supplier;
- `study`: effect-free coordinator of all declared intent and preflight;
- `runtime`: sole coordinator of active execution and output creation;
- `persistence`: automatic durable recordings and verified reconstruction;
- `ui`: automatic terminal presentation of Runtime-owned progress facts;
- `prelude`: the ordinary execution-unit authoring imports; and
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
  retained Config + named state semantics + execution unit discovery + constants decode
  + generic program/Python tasks + one-time observation-plan binding
  + identities + phases
        │ immutable Study returned to the crate facade
        ▼
runtime::execute
  execution directory + replicates + scheduling + cancellation
  + immutable initialization context + execution-unit stepping
  + direct program/Python invocation
        ├──── per-member observation boundaries ────► persistence
        │                                             one private bounded writer per member
        │                                             + atomic metadata/chunks
        │                                             + verified reads
        ├──── program snapshot/log/status/artifacts ─► persistence
        │
        └──── lifecycle/progress facts ─────────────► ui
                                                      Ratatui + exit cancellation
```

The crate facade owns only the transition from project root to Study to
Runtime. Study is the ultimate coordinator of declared intent. Runtime is the
ultimate coordinator of active execution. Config never discovers Rust execution unit
types or performs execution.

During `Study::load`, Config is the sole subsystem that discovers, reads, and
parses authored project JSON. State's standalone public `&Path` loader remains
a deliberate independent-use exception; composed Workflow passes State
already parsed values. Persistence alone owns the Workflow recording format,
its writes, and verified reconstruction. Runtime may create high-level
execution/replicate directories and launch external programs; external
programs own their declared domain artifacts, and UI owns terminal output.
Other subsystems consume typed or validated in-memory values instead of
reopening configuration or interpreting persistence files.
Study never creates output or initializes an execution unit. Study and Runtime never
reparse the captured documents. Persistence never decides scientific
observation meaning.

## Public API and dependency direction

Each public subsystem exposes one API at its module root. The single `prelude`
contains only ordinary execution-unit authoring types and crate conveniences;
inspection, embedding, and completed-recording APIs remain at their owning
module roots. Crate-visible peer contracts do not become public. Every type or
trait appearing in a peer signature is explicitly re-exported by its owning
module root, so subsystem coupling is nameable and auditable rather than hidden
behind inference or private-module reach-through.

Current public surface:

```text
ordinary crate root and prelude
├── state construction and manipulation
├── ObservationPlan / ObservationStream
├── ExecutionUnit / InitializationContext / MemberView / SeedError / UnitResult
│   / #[execution_unit]
├── crate-facade run(&Path)
└── WorkflowError

owning module roots
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

prelude aggregates ordinary authoring contracts
```

Peers import another subsystem through its module root. Config does not depend
on task: it treats execution unit keys as opaque. Study owns the cross-domain
match. Task asks config-owned resolved execution unit parameters for one complete typed
constants value or delegates one resolved program. Runtime receives only a
fully preflighted Study and its retained Config.

## Source tree and file responsibilities

```text
workflow/
├── README.md                         repository entry and user outcome
├── docs/
│   ├── architecture.md               this complete ownership/tree guide
│   └── tests.md                      validation responsibilities and commands
├── macros/
│   ├── Cargo.toml                    proc-macro package declaration
│   └── src/lib.rs                    registration attribute expansion
├── rust/
│   ├── Cargo.toml                    primary library package/dependencies
│   ├── README.md                     complete Rust user procedure
│   ├── src/
│   │   ├── lib.rs                    module declarations; run/macro/error exports
│   │   ├── clock.rs                  private UTC formatting and duration conversion
│   │   ├── error.rs                  private facade-error implementation
│   │   ├── error/api.md              complete facade-error contract
│   │   ├── error/workflow.rs         WorkflowError composition
│   │   ├── prelude.rs                ordinary authoring aggregation
│   │   ├── prelude/api.md            exhaustive prelude export contract
│   │   │
│   │   ├── state.rs                  state public root and peer exports
│   │   ├── state/api.md              exhaustive state API and examples
│   │   ├── state/error.rs            schema/state/time/series error enums
│   │   ├── state/field.rs            immutable field metadata
│   │   ├── state/schema.rs           Path loader and schema semantic authority
│   │   ├── state/state.rs            heterogeneous payload owner and tuple borrows
│   │   ├── state/time.rs             StateTime and checked advancement
│   │   ├── state/series.rs           ordered in-memory SystemState collection
│   │   └── state/value.rs            private erased payload/type/Serde adapter
│   │   │
│   │   ├── observation.rs            observation public root and peer exports
│   │   ├── observation/api.md        exhaustive declaration API and example
│   │   ├── observation/error.rs      declaration/binding/encoding errors
│   │   ├── observation/plan.rs       public plan + private schema-bound plan
│   │   ├── observation/stream.rs     public stream + private bound stream
│   │   ├── observation/sampling.rs   private cadence decision
│   │   ├── observation/state_observation.rs checked borrowed state view
│   │   ├── observation/encoding.rs   canonical owned encoded records
│   │   ├── observation/session.rs    cadence state and final-state deduplication
│   │   └── observation/tests/observation_workflow.rs internal binding/session tests
│   │   │
│   │   ├── task.rs                   private task internals and root re-exports
│   │   ├── task/api.md               exhaustive execution-unit contract and example
│   │   ├── task/unit.rs              ExecutionUnit and borrowed MemberView contract
│   │   ├── task/result.rs            boxed application error alias
│   │   ├── task/catalog.rs           linked registrations and sorted validation
│   │   ├── task/definition.rs        type-erased execution unit/program execution definitions
│   │   ├── task/execution.rs         host port and execution unit invariant enforcement
│   │   └── task/tests/task_workflow.rs private catalog/execution contract tests
│   │   │
│   │   ├── config.rs                 config root; public ConfigError + peer exports
│   │   ├── config/api.md             complete project grammar/error contract
│   │   ├── config/error.rs           owned contextual ConfigError
│   │   ├── config/document.rs        strict JSON and duplicate-key parser
│   │   ├── config/store.rs           central immutable all-document Config snapshot
│   │   ├── config/manifest.rs        study grammar, defaults, dependency checks
│   │   ├── config/expansion.rs       deterministic $sweep/$cases compiler
│   │   ├── config/parameters.rs      resolved execution unit parameters + typed decode
│   │   ├── config/program.rs         validated resolved executable declaration
│   │   ├── config/python.rs          nested Python environment validation/lowering
│   │   ├── config/specification.rs   one-root loading transaction
│   │   └── config/tests/config_workflow.rs internal compiler/grammar tests
│   │   │
│   │   ├── study.rs                  public Study/StudyError root + peer exports
│   │   ├── study/api.md              exhaustive Study API and example
│   │   ├── study/error.rs            binding/preflight StudyError
│   │   ├── study/compilation.rs      project-to-Study composition
│   │   ├── study/plan.rs             public Study + private phases/tasks/policies
│   │   └── study/tests/study_workflow.rs internal binding/runtime tests
│   │   │
│   │   ├── runtime.rs                public execution/summary/error root
│   │   ├── runtime/api.md            execute/summary/error contract
│   │   ├── runtime/error.rs          active execution RuntimeError
│   │   ├── runtime/output.rs         private unique execution/replicate directories
│   │   ├── runtime/host.rs           execution unit/program execution and persistence adapter
│   │   ├── runtime/execution.rs      Study-only replicate/phase/task schedulers
│   │   ├── runtime/summary.rs        successful immutable RunSummary tree
│   │   └── runtime/tests/runtime_workflow.rs private scheduler/lifecycle tests
│   │   │
│   │   ├── ui.rs                     private UI root
│   │   ├── ui/api.md                 automatic presentation contract
│   │   ├── ui/plan.rs                private Study-owned inferred UI policy
│   │   ├── ui/event.rs               borrowed Runtime-to-UI fact vocabulary
│   │   ├── ui/command.rs             former command editor and exact exit parser
│   │   ├── ui/state.rs               event-reduced rows/messages/status snapshot
│   │   ├── ui/session.rs             renderer thread and cancellation bridge
│   │   └── ui/terminal.rs            Ratatui dashboard + plain noninteractive mode
│   │   │
│   │   ├── persistence.rs            public completed-recording read root
│   │   ├── persistence/api.md        complete settings/read/error contract
│   │   ├── persistence/plan.rs       private effective operational settings
│   │   ├── persistence/session.rs    member recording/program workspace lifecycle
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
│       ├── task_workflow.rs           downstream ExecutionUnit API tests
│       └── fixtures/*.json            canonical state-schema fixtures
├── python/
│   ├── pyproject.toml                 reader package metadata and build policy
│   ├── README.md / LICENSE            Python reader guide and license
│   ├── src/scientific_workflow_reader/
│   │   ├── __init__.py                supported reader exports
│   │   ├── errors.py                  typed verification/read failures
│   │   ├── state.py                  read-only field/record/series containers
│   │   ├── reader.py                  format-v7 validation and reconstruction
│   │   └── py.typed                   typing marker
│   └── tests/
│       ├── test_reader.py             reader/integrity behavior
│       ├── roundtrip_bridge.py        Rust/Python conformance helper
│       └── fixtures/                  shared valid/invalid format fixtures
└── examples/attractor_2d/
    ├── Cargo.toml / Cargo.lock         standalone example package
    ├── src/main.rs                     one run(&Path) call
    ├── src/hopf_model.rs              domain model implementing ExecutionUnit
    ├── wf_configs/
    │   ├── study.json                  swept simulation then Python plot phase
    │   ├── parameters.json             execution unit sweeps + plotter settings
    │   └── states/attractor.json       named `attractor` state schema
    └── scripts/plot.py                 direct verified-recording SVG task
```

## Subsystem details

### State

`SystemStateSchema::load_json_template(&Path)` is the public construction
boundary. Config uses a crate-private in-memory equivalent so Study does not
reread named state documents. Persistence reconstructs schemas directly from
decoded ordered field metadata rather than serializing and reparsing JSON.
Observation borrows serializable payloads through a crate-visible State port,
not slot internals. `SystemState` owns fixed heterogeneous slots and
`StateTime`; application execution units mutate payloads and advance time.
Specialized module-root inspection exposes field metadata and schema identity. The old generic
schema-source adapter was removed with the persistence builder that needed it.

### Observation

An execution unit's `preflight(&Constants, &SystemStateSchema)` owns its domain
validation and defaults to an all-fields plan. Study calls it once, trusts a
successful result, and stores the exact schema-bound plan. Runtime does not
call it again. Private sessions select due streams, borrow/encode selected
payloads, and deduplicate the final iteration. The resulting canonical owned
JSON record is the explicit handoff to Persistence: encoding occurs while the
mutable, potentially non-`Sync` state is synchronously borrowed, then the bytes
may cross to an asynchronous writer. Observation owns no paths, buffers,
files, or lifecycle; Persistence owns no field-selection or cadence policy.

### Task

`ExecutionUnit` is the irreducible user contract for stateful Rust science.
It represents either a single stateful member or a coordinated, positive,
stable collection of members. Task itself is generic: Study may instead bind a resolved executable with
direct arguments and no public Rust adapter. Config lowers a nested Python
script/environment declaration to that same executable boundary. Study and
Runtime decode equivalent constants instances independently from Config's same
retained JSON value, so the constants type itself need not be `Send` or `Sync`.
The unit initializes from its task-bound selected schema and a Workflow-owned
`InitializationContext`, then exposes stable
`MemberView`s. Each view names one independently complete member and directly
borrows that member's state. The unit performs one coordinated `step`; a normal
execution unit is the one-view case, while an ensemble may synchronize or parallelize
members internally. Task enforces stable count/order/identity/state/schema,
monotonic completion and targets, and progress by at least one incomplete member.
The macro submits immutable registration metadata; the private catalog
rejects bad or duplicate keys and ignores linker order.
State definition and access remain macro-free: each execution unit owns the concrete
`SystemState`, exposes it through its member view, and uses the state's typed single- or
tuple-payload borrowing methods directly.
Task passes a semantic borrowed `ProgramTaskInvocation` through its execution
host port, so Runtime does not depend on Config's resolved-program
representation.
Deterministic units ignore the initialization context. Stochastic units request
stable purpose-named shared or per-member seeds only when needed. Seed
derivation incorporates the optional study master seed, replicate, task,
execution-unit key, scope, member identity, and purpose without an order-sensitive
counter. Runtime validates member-scoped requests against the initialized views;
Persistence records shared requests plus the applicable member requests and
their actual derived values in each member's metadata.

### Config

Config canonicalizes the project and required `wf_configs` roots and parses
`wf_configs/study.json`, every other JSON document beneath `wf_configs/`
(including all named state schemas), and the complete arbitrary
`wf_configs/parameters.json` namespace with duplicate-key rejection. One
clone-cheap immutable Config retains the entire value graph.
The optional top-level `study.json.seed` is the sole master randomness input
owned by Workflow. Config parses it once and Study retains it as immutable
intent; neither layer draws random values.
`wf_configs/study.json.paths.states` maps semantic state keys to configuration documents; every
execution unit task explicitly selects one key. An execution unit key automatically selects its
same-name parameter section; no per-task parameter path exists. Config expands
selections deterministically, resolves program paths and Python scripts/environment
managers once, and creates a deterministic language-neutral snapshot for
external tasks. Reserved Workflow documents and arbitrary application
documents use the same lookup graph. Runtime retains only a clone-cheap
`ConfigSnapshot` byte handle for an active task, not Config's typed lookup and
parsing interface. The downstream-public Rust API is only `ConfigError`;
closed peer types are explicitly named through the same owning scope.

### Study

`Study::load(&Path)` performs all cross-domain checks before output: every
named state's semantics, every execution unit task's state lookup, linked registration
validation, execution unit-key resolution, constants decoding, and
observation/task-schema binding over Config's already-resolved generic program
and Python tasks. It retains
the central Config and infers stable identities, labels, the output root, and
private operational policy. Public inspection is limited to project/output
roots; phases, tasks, schema, resolved parameters, and policies exist only for
Runtime. Its crate-visible Runtime view exposes compiled execution and semantic
provenance facts without exposing Config or Task descriptors.

### Runtime

The crate-level `run(&Path)` loads a Study and passes it to
`runtime::execute(Study)`. Runtime has no project-root or loading
entry point: it consumes only complete immutable intent. Runtime alone creates
`output/execution-<pid>-<sequence>`, isolated
`replicate-NNNNNN` directories, and deterministic task recording paths. It
topologically schedules generic tasks, applies concurrency/start intervals and
fail-fast/finish-all policy, checks cooperative cancellation between execution-unit
steps, directly starts programs and resolved Python launchers without a shell,
and returns deterministic successful summaries. Parallel replicate workers
report completion as it occurs, allowing replicate-level fail-fast to cancel
active siblings promptly without changing ascending successful-summary order.
Worker completion timestamps, rather than scheduler harvest time, determine
task deadline outcomes; phase timeouts apply only while work remains. A
blocking user `step` cannot be forcibly killed safely, but cancellation
observed when it returns prevents a successful final recording transition. An
external child is killed and reaped on observed cancellation. A task panic is
caught while its host is alive so any active member recording becomes durably
failed before the existing `TaskPanicked { task }` error is returned.
Runtime passes semantic execution unit provenance into Persistence and does not author
durable metadata JSON, backend names, or local format fields.

### Persistence

Persistence is constructed and run only by Runtime. The member-recording constructor
accepts an inferred destination, the already-bound observation plan, semantic provenance,
and one effective shared chunk/queue policy. A bounded worker owns all stream
writes and commits immutable JSONL chunks plus one authoritative
`metadata.json` lifecycle. There is no writer builder, per-stream storage
override, public flush, resume/continuation path, or completion handle.
Persistence alone converts execution-unit provenance and effective policy into exact
durable metadata, including the current local-backend field.

Users author every persistence size as a positive integer decimal MB, with one
MB equal to 1,000,000 bytes. The `wf_configs/study.json` fields are
`persistence.chunk_target_mb` and `persistence.queue_capacity_mb`; no JSON
persistence-size field is byte-addressed. Config alone checks and converts
both values to internal byte counts, and effective provenance records those
exact byte values.

Specialized users can only open completed recordings with a decoder registry.
Readers verify metadata, sizes, hashes, framing, ordering, schema, decoding,
and StateSeries invariants before publishing owned results. A future local or
remote adapter belongs behind the private `PersistenceSession`, not in execution unit
or task APIs. A separate private program session creates an isolated artifacts
directory, freezes central config and dependency JSON, captures stdout/stderr,
records generic-program or Python launcher provenance, and atomically publishes
and directory-synchronizes running/complete/failed metadata. Initial and final
member-observation failures best-effort terminalize an active recording as
failed before preserving the triggering error. Latest-state reconstruction
validates every record and descriptor fact in the newest chunk, not only its
last line. The workspace remains the external task's
default working area, but Python or another external program owns its
domain-specific IO and may write to a safe project-relative destination from
`wf_configs/parameters.json`; the attractor plotter uses `output/plots`.

Member-recording provenance calls the selected combination
`parameter_ordinal` and its canonical `wf_configs/parameters.json` document
`parameter_source`; `state` records the named schema selector bound during
assembly. The removed per-task input-file vocabulary is not retained on disk.
When an execution unit requested Workflow-derived seeds, the recording's
`user_metadata.workflow.seed_derivation` stores the versioned algorithm,
master seed, and actual applicable shared/per-member requests. Requests for a
different ensemble member are intentionally absent from that member's metadata.

### UI

Study owns a private zero-configuration `UiPlan`; its current effective policy
is inferred rather than authored in `wf_configs/study.json`. Runtime constructs one
clone-cheap `UiSession` after creating the execution output and publishes
borrowed execution, replicate, phase, task, iteration, outcome, and path facts.
Execution unit progress comes from the same host boundaries already used for automatic
persistence. Programs publish generic lifecycle facts without invented
iteration values, so neither workload supplies UI code or values.

Interactive stdin and stderr select the Ratatui/Crossterm alternate-screen
dashboard; noninteractive runs deliberately select UI's stable plain lifecycle
renderer. UI is the sole presentation interface, so failure to start or
initialize the selected renderer, poll terminal input, draw, or write plain
output is a fatal panic rather than cancellation or fallback. Renderer health
is checked from Runtime-facing publication, scheduler, and final-join
boundaries, while the terminal lease restores process state during unwinding.
The dashboard
owns a phase-scoped declaration-ordered task panel, progress gauges/spinners,
elapsed/ETA fields, a bounded message panel, and the former command editor.
Every phase-start event replaces the visible task set. Replicate and phase
appear once in the panel title; rows contain only task-specific information.
Exact lowercase `exit` or Ctrl+C requests cooperative Runtime cancellation,
stops further admission, and waits for active execution unit/program cleanup before
terminal restoration. One private refresh thread retains presentation facts
but never scientific payloads. It exposes no downstream API. Early phase,
replicate, or execution termination closes affected
unadmitted task rows as skipped; cancellation text does not invent whether its
source was user input, failure policy, or a deadline.

### Error and prelude integration

`WorkflowError` composes the effect-free `StudyError` and active
`RuntimeError` stages without absorbing either subsystem's detailed
vocabulary. The crate root re-exports that type and owns the sole ordinary
`run(&Path)` facade. Its transparent variants forward subsystem display and
source chains, retain `Send + Sync`, and do not absorb fatal UI renderer panics.

The single prelude aggregates ordinary state, observation, and execution-unit
contracts plus the crate-owned `run`, `execution_unit`, and `WorkflowError`
conveniences. It owns no behavior or alternative implementation path.

## Core invariants

1. Filesystem APIs borrow `&Path` and retain `PathBuf`; raw strings are not
   path parameters.
2. Config is the sole project JSON reader/parser, central immutable snapshot,
   executable/Python-environment resolver, and typed constants supplier.
3. Study completes all execution-unit/constants/observation and program/Python binding
   before output and retains the exact Config snapshot.
4. Execution unit registration keys are authored stable semantics, never Rust type names.
5. Execution units are initialized only during active Runtime execution.
6. An execution unit exposes a stable positive set of member identities, and
   each member directly owns one stable canonical `SystemState`.
7. Every successful unit `step` strictly advances at least one incomplete
   member and cannot advance a completed member.
8. Execution-unit preflight is deterministic, side-effect-free, evaluated
   once, and returns an observation plan bound to the task's selected schema.
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
14. UI consumes only Runtime facts, activates automatically as the sole
    presentation interface, and panics on failure of its selected renderer.
15. Scientific execution-unit and external-program tasks share phase, dependency, timeout, failure,
    summary, persistence-workspace, and UI lifecycle semantics without forcing
    fake state or iteration onto programs.
16. A Python environment is declared inside its task's `python` object and is
    completely resolved during Study loading; there is no global environment
    registry or runtime environment discovery.
17. Config is the only composed-Workflow reader/parser of authored project
    JSON; Runtime never reparses derived config values.
18. Persistence is the only owner of Workflow recording IO and format
    interpretation; Runtime owns only scope orchestration around that boundary.
19. Closed subsystem coupling is expressed through explicitly named
    `pub(crate)` exports at each owning module root; peers do not import
    another subsystem's private modules or depend on inference-only return
    types.
20. Config preflight rejects non-UTF-8 canonical project, configuration,
    executable, script, and environment paths before any exact path must cross
    a language-neutral JSON snapshot or provenance boundary; Runtime and
    Persistence never apply lossy path conversion.

## Replacement boundaries

- A state replacement preserves typed ownership, schema identity, time
  ordering, direct reconstruction from parsed values/field metadata, and
  serialization borrows.
- An observation replacement preserves declaration meaning, one-time binding,
  cadence, clone-free borrowing, deterministic encoded order, and the owned
  canonical-record handoff to Persistence.
- A task replacement preserves config-owned constants decode, stable
  execution-unit/execution unit/state boundaries, independent execution unit observation and
  completion, and generic program
  delegation without public adapters.
- A config replacement preserves the grammar, typed-path containment,
  named-state lookup, explicit task selection, duplicate-key rejection,
  deterministic expansion, all-document immutable
  snapshots, direct-program and nested Python-environment resolution, and
  centralized parsing, and the clone-cheap frozen snapshot handle.
- A Study replacement remains effect-free and performs complete binding before
  publishing immutable intent through its narrow Runtime view.
- A Runtime replacement consumes only Study, preserves policy and summary
  order, owns scope orchestration, and passes semantic rather than formatted
  persistence provenance.
- A persistence replacement remains behind the private session and preserves
  bounded execution unit submission, program workspace isolation/snapshots/logs,
  ownership of durable metadata/format concerns, terminal evidence/provenance,
  and verified execution unit reads.
- A UI replacement remains downstream of Runtime, requires no execution unit/config
  participation, handles concurrent publishers, restores terminal state while
  unwinding, and treats renderer failure as fatal rather than cancellation.
