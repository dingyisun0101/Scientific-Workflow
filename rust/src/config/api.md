# Config API

This guide documents the `scientific-workflow` 0.13.2 subsystem contract.

The `config` subsystem is the sole reader and parser of project JSON. One load
captures `wf_configs/study.json`, every named state schema declared by
an optional `study.json.paths.states`, and the canonical `wf_configs/parameters.json` in a
central immutable `Config`. The parameters
document is the one arbitrary nested namespace for every user-project setting:
global sweeps, execution unit constants and local sweeps, plotting settings,
validation tolerances, and external-program options. Config also resolves environment-managed Python
declarations into concrete invocations. Applications author files rather than
constructing Rust configuration objects.

Config never discovers execution unit types, initializes an execution unit, creates output,
schedules work, or persists state. Study owns cross-domain binding; Runtime
owns effects.

## Basic API

The ordinary user-facing API is this project layout:

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
for a valid project. Config derives both reserved document paths. Execution unit tasks do
not name parameter files: their stable execution unit key selects the same-named
top-level section in `parameters.json`. An execution-unit task may explicitly
select a named project schema or omit `state` for later resolution through its
registered unit's standard provider. Missing declared documents, unknown
explicit state keys, and execution unit sections fail loading before output exists.

`wf_configs/states/` is recommended for readable organization, but it is not a
required directory. State-schema documents may be placed anywhere beneath
`wf_configs/`; each project-owned one must be registered with its
project-root-relative path in `study.json.paths.states`. A state path outside
`wf_configs/` is rejected. A project using only linked standard providers needs
no state-schema file or `paths` object.

### `wf_configs/study.json`

The root object has required `workflow_schema`, positive-integer `threads`, and
nonempty `phases` fields plus optional `paths`, `seed`, `replicates`, and
`persistence` objects. `workflow_schema` is independent of the crate version;
this release accepts exactly generation `1` and rejects missing or unknown
generations before assembly:

```json
{
  "workflow_schema": 1,
  "threads": 16,
  "seed": 42,
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
        {"execution_unit": "population", "state": "population"}
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

- `workflow_schema` is required and must equal `1`. It versions the authored
  project grammar and is retained in the frozen snapshot supplied to external
  tasks. An omitted or unsupported generation is never interpreted as the
  current grammar implicitly.
- `threads` is the required positive global compute budget. There is no
  default, host-CPU inference, or environment override. Runtime gives its one
  shared execution-unit pool exactly this many workers and admits external
  tasks only while enough global permits are available.
- `seed` is an optional unsigned 64-bit master seed. Config parses it once;
  Runtime uses it only to derive explicitly requested task-scoped values. It
  places execution-unit derivation behind the immutable initialization context
  and supplies a declared program/Python derivation through the child-process
  environment. Deterministic tasks need not declare it. Config rejects a
  program seed request when the master seed is absent; an execution unit that
  requests a derived seed fails clearly during initialization when it is absent.
- `paths.states` defaults to an empty map and maps nonblank, whitespace-exact semantic state names to JSON
  paths. Paths must be project-root-relative, resolve once during Config
  loading, and identify captured `.json` documents beneath the canonical
  `wf_configs/` root. Multiple execution units may share one state key, while one project
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
- each ordinary phase must contain at least one task. The reserved `$npy`
  phase is the sole exception: it declares no `tasks` and must declare at
  least one prerequisite in `after`.
- `max_concurrency` defaults to `1` and must be positive.
- `start_interval_ms` defaults to zero and is the minimum delay between
  successive task admissions within that phase; the first eligible task is
  admitted immediately. Phase and task `timeout_ms` are optional nonnegative
  millisecond counts.
- each task is exactly one of:
  - an execution-unit task with nonblank `execution_unit`, optional nonblank
    `state`, and optional `timeout_ms`; or
  - a program task with required `program`, optional `args`, optional `seed`,
    optional `resources`, and optional `timeout_ms`, for example
    `{"program":"bin/analyze","args":["--publication"],"resources":{"threads":4}}`; or
  - a Python task with one nested `python` object and optional task-level
    `seed`, `resources`, and `timeout_ms`.
- A phase named exactly `$npy` is Workflow's standard post-processing request.
  Its complete minimal form is `"$npy":{"after":["simulate"]}`. Authors do
  not declare a converter task, executable, paths, or arguments. Config
  synthesizes one aggregate converter task and rejects authored `tasks` on
  this reserved phase. Runtime runs that task once per replicate and supplies
  all transitively prerequisite execution-unit recordings across every global
  configuration; the converter ignores prerequisite program workspaces.
- `resources` is the strict object `{"threads":N}`. It is valid only for a
  program or Python task, defaults to one thread when omitted, and must request
  a positive count no greater than top-level `threads`. The resolved count is
  both the number of global permits held for the child's lifetime and the
  value supplied to its thread-count environment variables. Execution units
  do not accept this field because all of them share the one global Rayon pool.
- A program/Python seed request is the strict object
  `{"seed":{"purpose":"target-initial-conditions"}}`. `purpose` must be
  nonempty and have no surrounding whitespace. The request requires the
  top-level master seed. Config retains only the semantic purpose; Runtime
  derives one task-scoped value using replicate, inferred task identity,
  program kind, and purpose. The field is invalid on an execution-unit task,
  whose implementation requests only the seeds it actually needs through
  `InitializationContext`.
- `args` is an array of opaque strings passed directly to the executable. No
  shell parses the program or arguments.
- a project-relative program is resolved against the project root. A
  one-component command may resolve through `PATH`. The resolved target must
  be an executable regular file (including execute permission on Unix).
  Resolution occurs during loading, so Runtime never searches again.
- the canonical project root, configuration documents, resolved executables,
  Python scripts, and environment paths must be valid UTF-8 because their
  exact values cross language-neutral JSON snapshot or provenance boundaries.
  Config rejects them during preflight rather than applying lossy conversion
  during execution.
- Execution unit keys remain opaque until Study matches them to linked `#[execution_unit]`
  registrations.
- An explicit execution unit `state` key is resolved during effect-free
  assembly. Its parsed document is validated by State, bound to that exact
  task, and recorded in provenance. If the field is omitted, Config retains
  that omission and Study asks the registered unit for its standard provider.
  There is no execution-unit-name fallback or global schema registry. An empty
  string is invalid; “no project state” means the field is absent.

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
  "resources": {"threads": 4},
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
execution unit tasks by their explicit `state` key.

This section is optional. When a task omits `state`, Config reads no replacement
schema from disk; Study resolves the linked static provider after matching the
execution-unit registration. Provider bytes are code-owned data rather than a
project configuration document and therefore do not appear in Config's
snapshot.

### `wf_configs/parameters.json`

This required root object contains every custom-project parameter. Users write
only the parameters and selection markers; there is no scope declaration,
reference syntax, phase selector, or configuration ordinal:

```json
{
  "species": {"$sweep": [200, 400]},
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

Config infers scope from `study.json`. Every top-level key named by an
execution-unit task is that unit's local constants section. All other
top-level values form the shared parameter object. Config first expands the
shared object into global configurations, then instantiates every task in every
phase once per configuration. Within each task copy, a selected execution-unit
section is expanded independently, so its `$sweep` and `$cases` markers remain
local to that unit.

For `{"execution_unit":"population","state":"population"}` or
`{"execution_unit":"population"}`, Config selects the `population` section,
expands it locally, and decodes each result as one complete
`ExecutionUnit::Constants`. The `species` sweep above therefore duplicates the
whole phase graph, while the `growth` sweep duplicates only `population` tasks.
Config owns two expansion markers wherever selection is allowed:

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
Expansion order is object declaration order, then choice order. Each global
configuration is internal correlation state, never user-authored syntax. Each
concrete local result becomes one internal resolved execution unit-parameter
value and is decoded as one complete owned constants value during Study
preflight.

### Central configuration snapshot

State schemas and the entire arbitrary parameters namespace use the same
strict parser, duplicate-key rules, immutable snapshot, and lookup graph.
Additional JSON found beneath `wf_configs/` (other than the separately
captured reserved `study.json`) is still captured and validated for forward
compatibility, but the supported ordinary layout puts every custom project
parameter in `parameters.json` rather than fragmenting it across files.

Every task is bound to a deterministic language-neutral snapshot in which the
shared selections are resolved. Generic program and Python tasks receive that
snapshot through Runtime:

```json
{
  "study": {
    "workflow_schema": 1,
    "threads": 16,
    "paths": {"states": {"population": "wf_configs/states/population.json"}},
    "phases": {"simulate": {"tasks": [{"execution_unit":"population","state":"population"}]}}
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
failure-atomic with respect to output: no directory, execution unit, task thread, or
persistence session exists yet. Later edits on disk do not affect the retained
Study or an execution made from it.

## Advanced API

The module root exports only `ConfigError` to downstream crates. The same
module is the named crate-visible peer boundary used by Study and Runtime;
peer types are listed below so a Config replacement does not depend on type
inference or private implementation paths.

### `config::ConfigError`

`ConfigError` is a non-exhaustive owned error enum:

- `Read { path, source }` retains the attempted `PathBuf` and IO source;
- `Parse { path, source }` reports invalid JSON syntax;
- `DuplicateKey { path, key }` rejects repeated keys at any nesting depth;
- `InvalidDocument { path, pointer, reason }` identifies a grammar/default or
  selection violation with a JSON Pointer;
- `PathOutsideConfig { path, config_root }` rejects unsafe discovered-document
  containment, including symlink escapes;
- `NonUtf8Path { path, context }` rejects a canonical path that could not be
  represented exactly in a later JSON snapshot, dependency document, or
  recording provenance value;
- `InvalidProgram { path, reason }` reports an unsafe, missing, non-executable
  program/manager or invalid Python script/environment declaration;
- `UnknownDependency { phase, dependency }` reports a missing phase edge;
- `UnknownState { phase, execution unit, state }` reports an execution-unit task whose selector
  does not name a declared state schema;
- `ExpansionOverflow { path }` prevents unrepresentable or unallocatable
  combination products; and
- `DecodeExecutionUnitConstants { execution_unit, path, ordinal, source }` contextualizes a
  Serde mismatch between one expanded parameter combination and its execution unit
  constants type.

Paths and explanatory strings are owned, so an error remains useful after the
loading transaction ends. IO, JSON, and constants-decoding variants preserve
their source chains. The enum exposes no partial project graph.

### Crate-visible peer API

These contracts are crate-private:

- `ProjectSpecification` is Config's completed loading result. It provides the
  canonical roots, retained `Config`, parsed manifest, named state documents,
  and fully resolved task declarations consumed by Study.
- `Config` is the immutable central graph. Its typed lookup/decode operations
  supply already-parsed configuration to Study; it performs no later disk IO.
- `ConfigSnapshot` is clone-cheap and retains both deterministic
  complete-project JSON bytes and the corresponding global-resolved parameters
  value. Runtime uses the bytes for external programs and the value for
  execution-unit provenance without retaining Config's parsing and lookup
  interface.
- `StateSchemaDocument` names one semantic state key and borrows its canonical
  source path and already-parsed JSON value.
- `StudyManifest`, `PhaseSpecification`, `PersistenceSpecification`,
  `ReplicatePolicy`, `ReplicateScheduling`, and `FailurePolicy` are the strict
  resolved procedural and operational views used during Study assembly.
- `ResolvedTask`, `ResolvedExecutionUnitParameters`, and `ResolvedProgramTask` carry
  complete execution unit constants/provenance or direct program/Python launch intent.
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
creates output. A phase may then declare an execution-unit task followed by a direct
Python task:

```json
{
  "workflow_schema": 1,
  "threads": 16,
  "paths": {"states": {"population":"wf_configs/states/population.json"}},
  "phases": {
    "simulate": {
      "tasks": [{"execution_unit":"population","state":"population"}]
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
resolution, optional explicit per-task state selection, execution unit-key
parameter selection, typed `Path` containment, duplicate-key
rejection, deterministic expansion,
centralized one-pass parsing, immutable complete-project snapshots, complete
constants decoding, direct executable and Python-environment resolution,
contextual errors, and the no-output-before-Study boundary.
