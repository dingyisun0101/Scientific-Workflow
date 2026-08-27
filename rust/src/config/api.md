# Config API

The `config` subsystem is the sole reader and parser of Workflow project JSON.
It resolves a project root into strict study declarations, one centrally parsed
state schema, and complete model-constant values. These compilation values are
crate-private inputs to Study; applications interact with config by authoring
files, not by constructing Rust configuration objects.

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
    └── inputs/
        └── <model>.json
```

The user passes only `project_root: &Path` to `scientific_workflow::run`.
Config derives every other path. Inputs must be relative `.json` paths rooted
under `config/inputs`; absolute paths, parent traversal, non-normal components,
non-JSON paths, and symlink escapes are rejected.

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
        {
          "model": "population",
          "input": "inputs/population.json",
          "timeout_ms": 30000
        }
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
- each task has exactly a nonblank `model`, an `input` path, and optional
  `timeout_ms`. Model keys remain opaque until Study matches them to linked
  `#[model]` registrations.

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

All documents undergo duplicate-key detection before typed decoding. Loading
is failure-atomic with respect to output: no directory, model, task thread, or
persistence session exists yet.

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

Config reads `study.json`, `config/state.json`, and referenced input files as
part of that call. Any config failure returns before Runtime creates output.

## Not API

Strict-value parsing, `ProjectSpecification`, `StudyManifest`, phase and
replicate policies, persistence settings, `ResolvedTaskInput`, typed decoding,
document caching, path canonicalization, and `$sweep`/`$cases` expansion are
private compilation machinery. Peer subsystems reach the required crate-visible
types through `config::advanced`; downstream applications cannot construct or
inspect them.

A replacement config implementation must preserve the file grammar, typed
`Path` containment, duplicate-key rejection, deterministic expansion,
centralized one-pass parsing, complete constants decoding, contextual errors,
and the no-output-before-Study boundary.
