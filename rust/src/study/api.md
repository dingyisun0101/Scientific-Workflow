# Study API

The `study` subsystem compiles parsed project declarations and linked Rust
models into immutable, completely preflighted scientific intent. It is the
ultimate coordinator of declared intent, not an active scheduler.

Study never opens a project file directly: `config` alone performs file IO and
JSON parsing. Study never creates output, starts a thread, initializes a model,
or records a state. It combines supported advanced boundaries from config,
state, and task.

## Basic API

`scientific_workflow::study::basic` intentionally exports no symbols.

The ordinary user-facing Study API is the `study.json` grammar documented in
`config/api.md`. Users call `scientific_workflow::run(project_root: &Path)` and
do not load, construct, mutate, or execute a Study themselves.

An empty Basic scope is intentional API minimization, not an unfinished
implementation. Manual phase builders, task builders, selectors, completion
examiners, metadata maps, identities, and scheduling contexts are not part of
the supported workflow.

## Advanced API

`study::advanced` is a strict superset of the empty Basic scope.

### `study::advanced::Study`

`Study` is an immutable, clone-cheap, `Send + Sync` plan. It owns an `Arc` to a
validated project specification, shared state schema, inferred output root, and
ordered `StudyPhase` values. Cloning does not duplicate documents, model
definitions, schemas, or resolved constants.

Construction:

- `Study::load(project_root: &Path) -> Result<Study, StudyError>` asks config to
  load all declarations, discovers linked `#[model]` registrations, validates
  the catalog, semantically validates the state schema, binds every task input,
  and performs full preflight. It canonicalizes only through config. No output
  is created and no model is initialized.
- `Study::load_with_catalog(project_root: &Path, catalog: &ModelCatalog)` does
  the same work with an explicit immutable catalog. It is intended for tests,
  embedded applications, and replacement discovery adapters. The catalog is
  borrowed only for the call; the resulting Study owns type-erased task
  definitions.

Inspection:

- `project_root() -> &Path` returns config's canonical project root.
- `output_root() -> &Path` returns the inferred `<project-root>/output`. Loading
  does not create it.
- `state_schema() -> &SystemStateSchema` borrows the exact schema allocation
  against which writers and display fields were preflighted.
- `phases() -> &[StudyPhase]` returns manifest declaration order. Runtime may
  derive a stable topological execution order without changing this view.
- `replicate_policy() -> ReplicatePolicy` returns config's copyable effective
  replicate policy.
- `source_documents() -> &[ProjectDocument]` returns exact config-owned source
  documents in deterministic first-use order for provenance.

`Debug` prints bounded project/output roots and the phase count, never model
captures or constants. Study performs no blocking beyond config file loading
and pure preflight work. Any error is failure-atomic with respect to output.

### `study::advanced::StudyPhase`

`StudyPhase` is a cloneable immutable view compiled from one manifest phase.
It exposes:

- `name() -> &str`: stable manifest phase key;
- `dependencies()`: exact-size iterator over dependency keys in authored order;
- `tasks() -> &[StudyTask]`: bound invocations in deterministic input-expansion
  order;
- `max_concurrency() -> usize`: positive effective active-task bound;
- `start_interval() -> Duration`: delay between task admissions;
- `timeout() -> Option<Duration>`: optional cooperative phase timeout; and
- `failure_policy() -> FailurePolicy`: fail-fast or finish-all sibling policy.

Phase values own their names/dependency strings and task array. Accessors borrow
them without allocation or mutation. Config already verified nonempty tasks,
positive concurrency, dependency existence, uniqueness, and acyclicity.

### `study::advanced::StudyTask`

`StudyTask` is one registered model bound to one `ResolvedTaskInput`. It is
cloneable because the resolved input and erased definition are shared handles.
It exposes:

- `identity() -> &str`: inferred stable identity formatted from phase key,
  global plan ordinal, model key, and input expansion ordinal;
- `label() -> &str`: inferred human-readable `<model> #<ordinal>` label;
- `model() -> &str`: exact stable model key;
- `input() -> &ResolvedTaskInput`: config-owned resolved constants/provenance;
- `display_fields()`: exact-size iterator over already validated selected state
  fields; and
- `timeout() -> Option<Duration>`: optional cooperative invocation timeout.

It exposes no output path, mutable lifecycle, executor, writer session, progress
counter, or manual completion operation. The internal task definition and
numeric output ordinal are crate-private runtime inputs.

### `study::advanced::StudyError`

`StudyError` is non-exhaustive and preserves source chains. Variants are:

- `Config`: project loading/grammar/expansion failure;
- `State`: state-schema semantic failure;
- `ModelCatalog`: invalid or duplicate compiled registration;
- `UnknownModel { phase, model }`: manifest key has no linked registration;
- `ModelPreflight { phase, model, ordinal, source }`: constants decoding,
  writer declaration, or writer/schema binding failed; and
- `UnknownDisplayField { phase, model, field }`: display selection is absent
  from the state schema; and
- `TaskIdentityOverflow`: the total expanded plan exceeds the stable `u64`
  identity space.

Every error occurs before output creation and before model initialization.
Callers may correct code or JSON and retry without cleaning a partial run.

## Example

Ordinary applications do not mention a Study:

```rust,no_run
use std::path::Path;

fn main() -> Result<(), scientific_workflow::WorkflowError> {
    scientific_workflow::run(Path::new("."))
}
```

An advanced host may inspect preflighted intent and then execute it:

```rust,no_run
use std::path::Path;
use scientific_workflow::runtime::advanced::execute;
use scientific_workflow::study::advanced::Study;

# fn inspect() -> Result<(), Box<dyn std::error::Error>> {
let study = Study::load(Path::new("."))?;
for phase in study.phases() {
    println!("{}: {} tasks", phase.name(), phase.tasks().len());
}
let summary = execute(study)?;
println!("{}", summary.output_directory().display());
# Ok(())
# }
```

## Not API

`study::compilation`, `StudyInner`, internal task definitions, global output
ordinals, and structure constructors are private. Identity formatting is a
Study-owned deterministic mechanism, not permission for applications to parse
identity strings. Inventory iteration order is explicitly not observable:
`ModelCatalog` sorts keys before Study uses them.

Replacement Study implementations must remain output-free, must use config's
typed decode rather than parsing raw JSON, must validate every model reference
before publication, and must produce immutable runtime input.
