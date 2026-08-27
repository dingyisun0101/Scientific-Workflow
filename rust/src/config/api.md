# Config API

The `config` subsystem is the sole reader and parser of every project JSON
document. One load captures `study.json` plus every `.json` file recursively
beneath `config/` in a central immutable `Config`. It then resolves the
Workflow-owned declarations needed by Study while retaining arbitrary
application documents for generic tasks. It also resolves environment-managed
Python declarations into concrete program invocations. Applications interact
with config by authoring files, not by constructing Rust configuration objects.

Config never discovers model types, initializes a model, creates output,
schedules work, or persists state. Study owns cross-domain binding; Runtime
owns effects.

## Basic API

`scientific_workflow::config::basic` intentionally exports no Rust symbols.
Its ordinary user-facing API is this project layout:

```text
<project-root>/
├── study.json
└── config/
    ├── state.json
    ├── inputs/
    │   └── <model>.json
    └── <arbitrary-name>.json
```

The user passes only `project_root: &Path` to `scientific_workflow::run`.
Config derives every other path. Model inputs must be relative `.json` paths
rooted under `config/inputs`; absolute paths, parent traversal, non-normal
components, non-JSON paths, and symlink escapes are rejected. Other JSON files
may use any nested location beneath `config/` and have application-defined
content.

### `study.json`

The root object has one required `phases` object and two optional objects:

```json
{
  "replicates": {
    "count": 1,
    "scheduling": "sequential",
    "failure_policy": "fail_fast"
  },
  "persistence": {
    "chunk_target_bytes": 67108864,
    "queue_capacity_bytes": 67108864
  },
  "phases": {
    "simulate": {
      "after": [],
      "tasks": [
        {"model": "population", "input": "inputs/population.json"}
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

- `replicates.count` defaults to `1` and must be positive.
- `replicates.scheduling` is `"sequential"` by default or `"parallel"`.
- replicate and phase `failure_policy` are `"fail_fast"` by default or
  `"finish_all"`.
- both persistence byte settings default independently to 64 MiB and must be
  positive integers. There is no backend selector while only the automatic
  local backend exists.
- `phases` must contain at least one phase. Phase keys must be nonempty and
  have no surrounding whitespace.
- `after` defaults to empty. Dependencies must exist, be unique, differ from
  the containing phase, and form an acyclic graph.
- each phase must contain at least one task.
- `max_concurrency` defaults to `1` and must be positive.
- `start_interval_ms` defaults to zero. Phase and task `timeout_ms` are
  optional nonnegative millisecond counts.
- each task is exactly one of:
  - a model task with nonblank `model`, required `input`, and optional
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

### `config/state.json`

Config parses this document once and rejects duplicate JSON keys. State owns
its semantic grammar; the current shape is an ordered `fields` array whose
entries contain `name` and optional `description`. Study passes the already
parsed value to state validation without rereading the file.

### Task input documents

A task input document is application-owned data decoded as the selected
`ScientificModel::Constants`. Config does not assign scientific meaning to its
ordinary fields. It does own two reserved expansion markers:

- `{"$sweep": [a, b, ...]}` selects independent alternatives at that object
  position. Multiple sweeps form a deterministic Cartesian product.
- a root or nested object may contain `"$cases": [{...}, {...}]` alongside
  fixed fields. Cases are correlated alternatives and must share the same
  flattened field set without overlapping fixed values.

Choices and cases must be nonempty. A `$sweep` object has no siblings.
Reserved markers cannot occur inside a choice/case, and unknown `$...` keys
are rejected. Ordinary arrays are literal constants, not implicit sweeps.
Expansion order is object declaration order, then choice order. Each concrete
result becomes one internal task input and is decoded as one complete owned
constants value during Study preflight.

### Arbitrary configuration documents

Every `.json` file recursively beneath `config/` belongs to the same captured
configuration graph, whether or not Workflow assigns it a typed meaning.
Reserved documents such as `state.json` and referenced model inputs therefore
use the same parser, duplicate-key rules, immutable snapshot, and lookup
namespace as application documents such as `plot.json` or
`analysis/publication.json`.

Generic program and Python tasks receive a deterministic language-neutral
snapshot:

```json
{
  "study": {"phases": {}},
  "config": {
    "analysis/publication.json": {"dpi": 300},
    "state.json": {"fields": []}
  }
}
```

The top-level `study` value is the captured root manifest. `config` maps exact
forward-slash-normalized paths relative to `config/` to their values, sorted
lexicographically. Programs retrieve the keys they understand from this one
snapshot. Config does not require a schema for arbitrary documents.

All documents undergo duplicate-key detection when `Study::load` captures the
project, including currently unreferenced arbitrary documents. Loading is
failure-atomic with respect to output: no directory, model, task thread, or
persistence session exists yet. Later edits on disk do not affect the retained
Study or an execution made from it.

## Advanced API

`config::advanced` is the strict public superset of the empty Basic scope. It
exports only `ConfigError`. Project specifications, manifests, policies,
resolved task inputs, source documents, and expansion machinery are
crate-private because applications cannot use them without duplicating Study.

### `config::advanced::ConfigError`

`ConfigError` is a non-exhaustive owned error enum:

- `Read { path, source }` retains the attempted `PathBuf` and IO source;
- `Parse { path, source }` reports invalid JSON syntax;
- `DuplicateKey { path, key }` rejects repeated keys at any nesting depth;
- `InvalidDocument { path, pointer, reason }` identifies a grammar/default or
  selection violation with a JSON Pointer;
- `PathOutsideConfig { path, config_root }` rejects unsafe input resolution;
- `InvalidProgram { path, reason }` reports an unsafe, missing, non-executable
  program/manager or invalid Python script/environment declaration;
- `UnknownDependency { phase, dependency }` reports a missing phase edge;
- `ExpansionOverflow { path }` prevents unrepresentable or unallocatable
  combination products; and
- `DecodeModelConstants { model, path, ordinal, source }` contextualizes a
  Serde mismatch between one expanded input and its model constants type.

Paths and explanatory strings are owned, so an error remains useful after the
loading transaction ends. IO, JSON, and constants-decoding variants preserve
their source chains. The enum exposes no partial project graph.

## Example

Given the layout above, an ordinary application does not call config:

```rust,no_run
use std::path::Path;

fn main() -> Result<(), scientific_workflow::WorkflowError> {
    scientific_workflow::run(Path::new("."))
}
```

Config reads `study.json` and every JSON document beneath `config/` as part of
that call. Any config failure returns before Runtime creates output. A phase
may then declare a model task followed by a direct Python task:

```json
{
  "phases": {
    "simulate": {
      "tasks": [{"model":"population","input":"inputs/population.json"}]
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

Strict-value parsing, `Config`, `ProjectSpecification`, `StudyManifest`, phase
and replicate policies, persistence settings, `ResolvedTaskInput`,
`ResolvedProgramTask`, Python declaration/environment types, manager command
lowering, typed decoding, document caching, snapshot serialization, path
canonicalization, and `$sweep`/`$cases` expansion are private compilation
machinery. Peer subsystems reach the required crate-visible types through
`config::advanced`; downstream applications cannot construct or inspect them.

A replacement config implementation must preserve the file grammar, typed
`Path` containment, duplicate-key rejection, deterministic expansion,
centralized one-pass parsing, immutable complete-project snapshots, complete
constants decoding, direct executable and Python-environment resolution,
contextual errors, and the no-output-before-Study boundary.
