# Study API

The `study` subsystem is the ultimate coordinator of declared intent. It asks
config to load one project root, asks state to validate the centrally parsed
schema, discovers linked models, binds each expanded constants value to its
model, binds each observation plan to the schema, and produces one immutable
execution plan.

Study creates no output, starts no worker, initializes no model, and writes no
recording. Runtime alone owns active effects.

## Basic API

`scientific_workflow::study::basic` intentionally exports no Rust symbols.
Ordinary users express Study intent in `study.json` and call
`scientific_workflow::run(project_root: &Path)`. They do not construct phases,
tasks, identities, persistence plans, registries, or Study builders.

## Advanced API

`study::advanced` is the strict superset of Basic and exposes exactly
`Study` and `StudyError`.

### `study::advanced::Study`

`Study` is an immutable, clone-cheap plan backed by `Arc`. It owns the shared
state schema, internal phases/tasks, resolved constants, schema-bound
observation plans, inferred output root, replicate policy, and effective
persistence/UI settings. None of those internal planning types is public.

- `Study::load(project_root: &Path) -> Result<Study, StudyError>` performs the
  complete effect-free loading and preflight transaction. Config canonicalizes
  paths and parses JSON. Study validates registration keys and duplicates,
  resolves every manifest model key, decodes every concrete constants value,
  calls each model's side-effect-free `observation_plan` exactly once, and
  binds that plan to the shared schema. It does not call
  `ScientificModel::initialize`.
- `project_root() -> &Path` returns config's retained canonical root.
- `output_root() -> &Path` returns the inferred
  `<canonical-project-root>/output`. The path need not exist until Runtime
  starts.

`Clone` increments one reference count; it does not reread files, repeat
preflight, clone scientific payloads, or duplicate constants documents.
`Debug` prints bounded root/plan information and never model captures or raw
constants.

There is no public manual-catalog loader, phase accessor, task accessor, state
schema accessor, replicate-policy accessor, persistence-plan accessor, source
document view, UI-plan accessor, or mutable Study operation. Advanced consumers that need to run
a preloaded Study pass it to `runtime::advanced::execute`; successful runtime
summaries provide output paths and task results.

### `study::advanced::StudyError`

`StudyError` is a non-exhaustive owned error enum:

- `Config(ConfigError)` preserves project loading, grammar, path, expansion,
  or constants-decoding failure;
- `State(StateError)` preserves state-schema semantic failure;
- `InvalidModelRegistration { reason }` reports an invalid or duplicate
  linked `#[model]` key without exposing the private catalog type;
- `UnknownModel { phase, model }` reports a manifest key with no linked
  registration;
- `ModelPreflight { phase, model, ordinal, source }` contextualizes constants
  decoding, observation declaration, or schema binding for one concrete task;
  and
- `TaskIdentityOverflow` prevents a plan whose deterministic global task
  ordinal cannot fit in `u64`.

Every variant occurs before output creation and model initialization. Source
errors remain available through `std::error::Error::source`. A failed load
publishes no partial Study, so the user can correct code or JSON and retry
without cleaning output.

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
use scientific_workflow::runtime::advanced::execute;
use scientific_workflow::study::advanced::Study;

# fn host() -> Result<(), Box<dyn std::error::Error>> {
let study = Study::load(Path::new("."))?;
println!("planned output root: {}", study.output_root().display());
let summary = execute(study)?;
println!("actual execution: {}", summary.output_directory().display());
# Ok(())
# }
```

## Not API

`ProjectSpecification`, explicit model catalogs, `StudyInner`,
`StudyPhase`, `StudyTask`, resolved inputs, type-erased task definitions,
bound observation plans, replicate/persistence policies, UI plan, global
output ordinals, identity/label formats, and topological planning data are private.
Runtime obtains them through crate-visible `study::advanced` exports.

A replacement Study must remain output-free, consume config exactly once,
perform complete model/constants/observation preflight, infer deterministic
identities and roots, retain immutable execution intent, and expose no mutable
lifecycle to applications.
