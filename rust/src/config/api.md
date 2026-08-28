# Config API

The `config` subsystem is the sole reader and parser of project JSON. One load
captures `wf_configs/study.json`, every named state schema declared by
`study.json.paths.states`, and the canonical `wf_configs/parameters.json` in a
central immutable `Config`. The parameters
document is the one arbitrary nested namespace for every user-project setting:
model constants and sweeps, plotting settings, validation tolerances, and
external-program options. Config also resolves environment-managed Python
declarations into concrete invocations. Applications author files rather than
constructing Rust configuration objects.

Config never discovers model types, initializes a model, creates output,
schedules work, or persists state. Study owns cross-domain binding; Runtime
owns effects.

## Basic API

`scientific_workflow::config::basic` intentionally exports no Rust symbols.
Its ordinary user-facing API is this project layout:

```text
<project-root>/
└── wf_configs/
    ├── study.json
    ├── parameters.json
    └── states/
        ├── population.json
        └── environment.json
```

The user passes only `project_root: &Path` to `scientific_workflow::run`.
The required `wf_configs/` directory identifies that path as a Workflow project
root; `wf_configs/study.json` and `wf_configs/parameters.json` are both required
for a valid project. Config derives both reserved document paths. Model tasks do
not name parameter files: their stable model key selects the same-named
top-level section in `parameters.json`. Each model task explicitly selects a
named state schema. Missing declared documents, state keys, and model sections
fail loading before output exists.

`wf_configs/states/` is recommended for readable organization, but it is not a
required directory. State-schema documents may be placed anywhere beneath
`wf_configs/`; each one must be registered with its project-root-relative path
in `study.json.paths.states`. A state path outside `wf_configs/` is rejected.

### `wf_configs/study.json`

The root object has required `paths` and `phases` objects and two optional
objects:

```json
{
  "paths": {
    "states": {
      "population": "wf_configs/states/population.json"
    }
  },
  "replicates": {
    "count": 1,
    "scheduling": "sequential",
    "failure_policy": "fail_fast"
  },
  "persistence": {
    "chunk_target_mb": 64,
    "queue_capacity_mb": 64
  },
  "phases": {
    "simulate": {
      "after": [],
      "tasks": [
        {"model": "population", "state": "population"}
      ],
      "max_concurrency": 1,
      "start_interval_ms": 0,
      "timeout_ms": 60000,
      "failure_policy": "fail_fast"
    }
  }
}
```

Unknown properties are rejected at every Workflow-owned level.

- `paths.states` maps nonblank, whitespace-exact semantic state names to JSON
  paths. Paths must be project-root-relative, resolve once during Config
  loading, and identify captured `.json` documents beneath the canonical
  `wf_configs/` root. Multiple models may share one state key, while one project
  may declare any number of schemas.
- `replicates.count` defaults to `1` and must be positive.
- `replicates.scheduling` is `"sequential"` by default or `"parallel"`.
- replicate and phase `failure_policy` are `"fail_fast"` by default or
  `"finish_all"`.
- `persistence.chunk_target_mb` defaults to `64`, must be a positive integer,
  and uses decimal megabytes: one MB is exactly 1,000,000 bytes. Config checks
  conversion overflow and supplies the resulting byte target internally.
- `persistence.queue_capacity_mb` follows the same positive-integer decimal-MB
  rule and defaults to `64`. It bounds queued encoded data for backpressure.
  There is no backend selector while only the automatic local backend exists.
- no JSON persistence-size setting accepts bytes. Config converts both MB
  values once and rejects conversion overflow before Study is published.
- `phases` must contain at least one phase. Phase keys must be nonempty and
  have no surrounding whitespace.
- `after` defaults to empty. Dependencies must exist, be unique, differ from
  the containing phase, and form an acyclic graph.
- each phase must contain at least one task.
- `max_concurrency` defaults to `1` and must be positive.
- `start_interval_ms` defaults to zero and is the minimum delay between
  successive task admissions within that phase; the first eligible task is
  admitted immediately. Phase and task `timeout_ms` are optional nonnegative
  millisecond counts.
- each task is exactly one of:
  - a model task with nonblank `model`, required nonblank `state`, and optional
    `timeout_ms`; or
  - a program task with required `program`, optional `args`, and optional
    `timeout_ms`, for example
    `{"program":"bin/analyze","args":["--publication"]}`; or
  - a Python task with one nested `python` object and optional task-level
    `timeout_ms`.
- `args` is an array of opaque strings passed directly to the executable. No
  shell parses the program or arguments.
- a project-relative program is resolved against the project root. A
  one-component command may resolve through `PATH`. The resolved target must
  be an executable regular file (including execute permission on Unix).
  Resolution occurs during loading, so Runtime never searches again.
- Model keys remain opaque until Study matches them to linked `#[model]`
  registrations.
- Model `state` keys are resolved during effect-free assembly. The selected
  parsed document is validated by State, bound to that exact model task, and
  recorded in task provenance. There is no implicit model-name fallback or
  single global schema.

### Python tasks

A Python task keeps all Python-specific intent inside `python`, while generic
Workflow lifecycle policy remains on the containing task:

```json
{
  "python": {
    "script": "scripts/analyze.py",
    "environment": {
      "manager": "mamba",
      "name": "DSES"
    },
    "args": ["--publication"]
  },
  "timeout_ms": 300000
}
```

`script` is an absolute or project-relative `.py` regular file. It need not
have operating-system execute permission because the resolved interpreter
opens it. Traversal components are rejected. `args` defaults to empty and is
passed after the script without shell parsing. Top-level `args` remains valid
only for a generic program task.

`environment` is required inside every Python declaration. There is no global
registry, alias, hidden active environment, or mutable runtime selection. Its
strict manager forms are:

- `{"manager":"system"}` resolves `python3` through `PATH`;
- `{"manager":"system","executable":"tools/python"}` uses an explicit
  interpreter path or command;
- `{"manager":"venv","path":".venv"}` resolves that environment's
  platform-specific Python executable;
- `{"manager":"mamba","name":"DSES"}` and
  `{"manager":"conda","name":"DSES"}` resolve the manager and lower to
  `run -n DSES python <script>`;
- `{"manager":"uv","project":"python"}` lowers to
  `uv run --project <resolved-project> python <script>`; and
- `{"manager":"poetry","project":"python"}` lowers to
  `poetry --directory <resolved-project> run python <script>`.

`system`, `mamba`, `conda`, `uv`, and `poetry` accept an optional `executable`
path/command; otherwise Config resolves the conventional command through the
project root or `PATH`. Environment names must be nonblank and cannot begin
with `-`. Virtual-environment and project paths must resolve to directories.
Every manager/interpreter is resolved to an executable regular file during
`Study::load`; Runtime performs no environment discovery.

### State-schema documents

Every path in `study.json.paths.states` names a JSON document captured beneath
`wf_configs/`. The recommended `wf_configs/states/` directory is optional; for
example, both `wf_configs/states/population.json` and
`wf_configs/population.json` are valid when their exact project-root-relative
paths are declared. Config parses each document once and rejects duplicate JSON
keys. State owns its semantic grammar; the current shape is an ordered `fields`
array whose entries contain `name` and optional `description`. Study passes each
already parsed value to State validation without rereading the file, then binds
model tasks by their explicit `state` key.

### `wf_configs/parameters.json`

This required root object contains every custom-project parameter, grouped by
the stable key of its consumer:

```json
{
  "population": {
    "initial": 10,
    "growth": {"$sweep": [0.1, 0.2]}
  },
  "plot": {
    "output_directory": "output/plots",
    "dpi": 180
  }
}
```

For `{"model":"population","state":"population"}`, Config selects only the
`population` section, expands it, and decodes each result as one complete
`ScientificModel::Constants`. Other sections remain arbitrary and are available
to external programs through the frozen central snapshot. Config owns two
expansion markers inside a selected model section:

- `{"$sweep": [a, b, ...]}` selects independent alternatives at that object
  position. Multiple sweeps form a deterministic Cartesian product.
- a root or nested object may contain `"$cases": [{...}, {...}]` alongside
  fixed fields. Cases are correlated alternatives and must share the same
  flattened field set without overlapping fixed values. Fixed siblings and
  case values cannot contain further selection markers: a `$cases` object is
  the terminal correlated choice at that subtree.

Choices and cases must be nonempty. A `$sweep` object has no siblings.
Reserved markers cannot occur inside a choice/case, and unknown `$...` keys
are rejected. Ordinary arrays are literal constants, not implicit sweeps.
Expansion order is object declaration order, then choice order. Each concrete
result becomes one internal resolved model-parameter value and is decoded as
one complete owned constants value during Study preflight.

### Central configuration snapshot

State schemas and the entire arbitrary parameters namespace use the same
strict parser, duplicate-key rules, immutable snapshot, and lookup graph.
Additional JSON found beneath `wf_configs/` (other than the separately
captured reserved `study.json`) is still captured and validated for forward
compatibility, but the supported ordinary layout puts every custom project
parameter in `parameters.json` rather than fragmenting it across files.

Generic program and Python tasks receive a deterministic language-neutral
snapshot:

```json
{
  "study": {
    "paths": {"states": {"population": "wf_configs/states/population.json"}},
    "phases": {"simulate": {"tasks": [{"model":"population","state":"population"}]}}
  },
  "config": {
    "parameters.json": {
      "population": {"growth": 0.1},
      "plot": {"dpi": 300}
    },
    "states/population.json": {"fields": [{"name":"population"}]}
  }
}
```

The top-level `study` value is the captured reserved manifest. The logical
`config` object maps exact forward-slash-normalized paths relative to
`wf_configs/` to their values, excluding `study.json` and sorting keys
lexicographically. The logical key remains `config` so external tasks consume a
stable configuration namespace rather than a source-directory name. Relative
document names must be valid UTF-8; Config rejects them instead of collapsing
distinct names through lossy conversion. JSON file symlinks retain their
authored relative snapshot key, but their canonical target must remain beneath
`wf_configs/`. Directory symlinks are not traversed.
Programs retrieve the keys they understand from this one
snapshot. Config does not require a schema for arbitrary documents.

All documents undergo duplicate-key detection when `Study::load` captures the
project, including currently unreferenced arbitrary documents. Loading is
failure-atomic with respect to output: no directory, model, task thread, or
persistence session exists yet. Later edits on disk do not affect the retained
Study or an execution made from it.

## Advanced API

`config::advanced` is the strict public superset of the empty Basic scope. It
exports only `ConfigError` to downstream crates. The same module is also the
named crate-visible peer boundary used by Study and Runtime; peer types are
listed below so a Config replacement does not depend on type inference or
private implementation paths.

### `config::advanced::ConfigError`

`ConfigError` is a non-exhaustive owned error enum:

- `Read { path, source }` retains the attempted `PathBuf` and IO source;
- `Parse { path, source }` reports invalid JSON syntax;
- `DuplicateKey { path, key }` rejects repeated keys at any nesting depth;
- `InvalidDocument { path, pointer, reason }` identifies a grammar/default or
  selection violation with a JSON Pointer;
- `PathOutsideConfig { path, config_root }` rejects unsafe discovered-document
  containment, including symlink escapes;
- `InvalidProgram { path, reason }` reports an unsafe, missing, non-executable
  program/manager or invalid Python script/environment declaration;
- `UnknownDependency { phase, dependency }` reports a missing phase edge;
- `UnknownState { phase, model, state }` reports a model task whose selector
  does not name a declared state schema;
- `ExpansionOverflow { path }` prevents unrepresentable or unallocatable
  combination products; and
- `DecodeModelConstants { model, path, ordinal, source }` contextualizes a
  Serde mismatch between one expanded parameter combination and its model
  constants type.

Paths and explanatory strings are owned, so an error remains useful after the
loading transaction ends. IO, JSON, and constants-decoding variants preserve
their source chains. The enum exposes no partial project graph.

### Crate-visible peer API

These `pub(crate)` contracts are available only through `config::advanced`:

- `ProjectSpecification` is Config's completed loading result. It provides the
  canonical roots, retained `Config`, parsed manifest, named state documents,
  and fully resolved task declarations consumed by Study.
- `Config` is the immutable central graph. Its typed lookup/decode operations
  supply already-parsed configuration to Study; it performs no later disk IO.
- `ConfigSnapshot` is a clone-cheap `Arc<[u8]>` handle. `bytes()` borrows the
  deterministic complete-project JSON supplied to external programs, allowing
  each runtime task to retain the frozen bytes without retaining Config's
  parsing and lookup interface.
- `StateSchemaDocument` names one semantic state key and borrows its canonical
  source path and already-parsed JSON value.
- `StudyManifest`, `PhaseSpecification`, `PersistenceSpecification`,
  `ReplicatePolicy`, `ReplicateScheduling`, and `FailurePolicy` are the strict
  resolved procedural and operational views used during Study assembly.
- `ResolvedTask`, `ResolvedModelParameters`, and `ResolvedProgramTask` carry
  complete model constants/provenance or direct program/Python launch intent.
  Their accessors expose semantic facts, not Config's document representation.

These are closed subsystem-to-subsystem contracts, not a downstream builder
API. Their concrete storage and parsing machinery may change, but their owning
module, names, and semantics must remain synchronized with peer consumers.

## Example

Given the layout above, an ordinary application does not call config:

```rust,no_run
use std::path::Path;

fn main() -> Result<(), scientific_workflow::WorkflowError> {
    scientific_workflow::run(Path::new("."))
}
```

Config reads `wf_configs/study.json` and every other JSON document beneath
`wf_configs/` as part of that call. Any config failure returns before Runtime
creates output. A phase may then declare a model task followed by a direct
Python task:

```json
{
  "paths": {"states": {"population":"wf_configs/states/population.json"}},
  "phases": {
    "simulate": {
      "tasks": [{"model":"population","state":"population"}]
    },
    "plot": {
      "after": ["simulate"],
      "tasks": [{
        "python": {
          "script": "scripts/plot.py",
          "environment": {"manager":"system"}
        }
      }]
    }
  }
}
```

## Not API

Strict-value parsing, Python declaration/environment representation, manager
command lowering, document caching, snapshot serialization, path
canonicalization, and `$sweep`/`$cases` expansion are private compilation
machinery. The named crate-visible types above are peer API rather than freely
replaceable internals; downstream applications still cannot construct or
inspect them.

A replacement config implementation must preserve the required `wf_configs`
project boundary, canonical manifest and parameters files, named state-path
resolution, explicit per-model state
selection, model-key parameter selection, typed `Path` containment, duplicate-key
rejection, deterministic expansion,
centralized one-pass parsing, immutable complete-project snapshots, complete
constants decoding, direct executable and Python-environment resolution,
contextual errors, and the no-output-before-Study boundary.
