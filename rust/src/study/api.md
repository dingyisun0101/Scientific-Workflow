# Study API

The `study` subsystem is the ultimate coordinator of declared intent. It asks
Config to capture one project root and fully resolve its declarations, retains
that central immutable Config, asks State to validate the centrally parsed
named schemas, discovers linked execution units, binds each Config-expanded constants
value to its registered execution unit and resolved explicit-or-provided schema, binds its observation plan
to that schema, and produces one immutable execution plan of generic tasks.
Executable paths, Python environments, parameter selection, and expansion are
Config responsibilities completed before Study performs this cross-domain
binding.

Study creates no output, starts no worker, initializes no execution unit, and writes no
recording. Runtime alone owns active effects.

## Ordinary use

Ordinary users express Study intent in `wf_configs/study.json` and call
`scientific_workflow::run(project_root: &Path)`. They do not construct phases,
tasks, identities, persistence plans, registries, or Study builders.

## Public Rust API

The module root exposes exactly `Study` and `StudyError`.

### `study::Study`

`Study` is an immutable, clone-cheap plan backed by `Arc`. It owns the shared
central Config, task-bound state schemas, internal phases/tasks, resolved constants,
resolved executable/script/environment paths, schema-bound observation plans,
inferred output root, optional master seed, replicate policy, and effective persistence/UI settings. None of those
internal planning types is public. `Study` is `Send + Sync`; clones may be
moved or shared across host threads without mutable planning state.

- `Study::load(project_root: &Path) -> Result<Study, StudyError>` performs the
  complete effect-free loading and preflight transaction. Config first
  canonicalizes paths, parses JSON, expands execution unit parameters, and resolves
  programs, Python scripts, interpreters, and environment managers. Study then
  validates registration keys and duplicates, resolves every execution unit key,
  validates every named state document, resolves an omitted task state through
  `ExecutionUnit::standard_state_schema`, validates and caches each static
  provider once per provider ID, retains the optional top-level master
  seed, decodes every concrete constants value, calls each execution unit's
  side-effect-free `preflight` exactly once, trusts its domain validation, and binds
  that plan to the resolved schema. An explicit project state takes precedence
  over a standard provider. It does not
  call `ExecutionUnit::initialize`. Loading is synchronous and may block on
  ordinary configuration reads and executable metadata/resolution, but starts
  no worker thread and creates no output.
- `project_root() -> &Path` returns config's retained canonical root.
- `output_root() -> &Path` returns the inferred
  `<canonical-project-root>/output`. The path need not exist until Runtime
  starts.

`Clone` increments shared reference counts; it does not reread files, repeat
preflight, clone scientific payloads, or duplicate constants documents.
`Debug` prints bounded root/plan information and never execution unit captures or raw
constants.

There is no public manual-catalog loader, phase accessor, task accessor, state
schema map/accessor, replicate-policy accessor, persistence-plan accessor, source
document view, UI-plan accessor, or mutable Study operation. Embedding consumers that need to run
a preloaded Study pass it to `runtime::execute`; successful runtime
summaries provide output paths and task results.

### Crate-visible Runtime view

Runtime consumes a deliberately narrow crate-private peer API:

- `Study` supplies replicate/phase policy, persistence and UI plans, and a
  clone-cheap `ConfigSnapshot`. Runtime receives frozen program bytes without
  retaining Config's parsing or typed-lookup interface.
- `StudyPhase` supplies only its semantic name, dependencies, admission policy,
  and compiled task slice.
- `StudyTask` supplies its stable identity/label, timeout, generic execution
  definition, semantic execution unit provenance, and program summary facts. Runtime no
  longer reaches through it to Task descriptors or Config-resolved types.

This view is crate-visible, not downstream-public. Its explicit names and
semantics form the replacement contract between effect-free assembly and active
execution.

### `study::StudyError`

`StudyError` is a non-exhaustive owned error enum:

- `Config(ConfigError)` preserves project loading, grammar, path, expansion,
  or constants-decoding failure;
- `State { state, path, source }` identifies the semantic manifest key and
  canonical source path of a rejected named schema while preserving its
  original `StateError`;
- `ProvidedState { provider, source }` identifies an invalid static provider
  document while preserving its original `StateError`;
- `InvalidStateSchemaProvider { provider, reason }` rejects an empty or
  whitespace-padded provider ID, or the same ID supplying different bytes;
- `MissingStateSchema { phase, execution_unit }` reports a task that omitted
  project `state` when its registered unit supplies no standard provider;
- `InvalidExecutionUnitRegistration { reason }` reports an invalid or duplicate
  linked `#[execution_unit]` key without exposing the private catalog type;
- `UnknownExecutionUnit { phase, execution_unit }` reports a manifest key with no linked
  registration;
- `ExecutionUnitPreflight { phase, execution_unit, ordinal, source }` contextualizes constants
  decoding, observation declaration, or schema binding for one concrete task;
  and
- `TaskIdentityOverflow` prevents a plan whose deterministic global task
  ordinal cannot fit in `u64`.

Every variant occurs before output creation and execution unit initialization. An
invalid program or Python declaration is reported through the wrapped
`ConfigError`.
Source errors remain available through `std::error::Error::source`. A failed load
publishes no partial Study, so the user can correct code or JSON and retry
without cleaning output. `StudyError` is `Send + Sync` and retains no borrow of
the failed loading transaction.

## Example

Ordinary execution does not mention `Study`:

```rust,no_run
use std::path::Path;

fn main() -> Result<(), scientific_workflow::WorkflowError> {
    scientific_workflow::run(Path::new("."))
}
```

An embedding host may split preflight from execution and inspect only the
stable roots:

```rust,no_run
use std::path::Path;
use scientific_workflow::runtime::execute;
use scientific_workflow::study::Study;

# fn host() -> Result<(), Box<dyn std::error::Error>> {
let study = Study::load(Path::new("."))?;
println!("planned output root: {}", study.output_root().display());
let summary = execute(study)?;
println!("actual execution: {}", summary.output_directory().display());
# Ok(())
# }
```

## Not API

`ProjectSpecification`, explicit execution unit catalogs, `StudyInner`, central
`Config`, resolved execution unit parameters/programs and Python launchers,
type-erased task definitions,
named and task-bound state schemas, bound observation plans,
replicate/persistence policies, UI plan, global
output ordinals, identity/label formats, and topological planning data are private.
`StudyPhase` and `StudyTask` are the named crate-visible Runtime view described
above; their backing representation remains private.

A replacement Study must remain output-free, consume and retain Config exactly
once, perform complete state/execution unit/constants/observation binding over Config's
already-resolved program/Python tasks, infer deterministic identities and
roots, retain immutable execution intent, and expose no mutable lifecycle to
applications. Runtime must be able to execute the retained snapshot after
project JSON changes without rereading it.
