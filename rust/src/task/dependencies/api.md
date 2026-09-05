# Task dependency API

## Basic API

Canonical module: `scientific_workflow::task::dependencies`. Task owns the
immutable result contract; Runtime supplies only completed, correlated declared
dependencies. Results own strings/PathBuf and are Clone + Debug + Send + Sync.
Rust callers normally borrow them through `InitializationContext::dependencies()
-> &Dependencies`. Context borrows cannot outlive initialization; clone a result
or copy its path to retain it. No dependency on Runtime summary types exists.

`Dependencies` methods:

| Method | Contract |
|---|---|
| `from_json(Value) -> Result<Self, DependencyError>` | Own and structurally validate a snapshot; no filesystem access |
| `load(&Path) -> Result<Self, DependencyError>` | Read a JSON snapshot and validate it atomically |
| `from_env() -> Result<Self, DependencyError>` | Load WORKFLOW_DEPENDENCIES_PATH; no cwd discovery |
| `recordings() -> Selection<RecordingDependency>` | Completed execution-unit member recordings |
| `programs() -> Selection<ProgramDependency>` | External and Python program artifacts |
| `npy_batches() -> Selection<NpyDependency>` | Aggregate converted batches |
| `raw_json() -> &Value` | Borrow all original metadata, including extensions |

Every result exposes `phase() -> &str`, `task() -> &str`, and `directory() -> &Path`.
`RecordingDependency` additionally exposes `execution_unit() -> &str`,
`member() -> &str`, `final_iteration() -> u64`. Its directory is the member
recording root. `ProgramDependency` exposes `executable() -> &Path` and
`python_script() -> Option<&Path>`; directory is `<task>/artifacts`, not the log
root. `NpyDependency::directory()` is the batch manifest directory.

`Selection<'a,T>` owns a list of borrowed results; Clone + Debug. Consuming
`in_phase(&str)` and `task(&str)` intersect exact filters. Recording selections
also accept `execution_unit(&str)` and `member(&str)`. Repeated conflicting filters
produce no matches. `one()` returns exactly one `&'a T`; `optional()` returns
`Option<&'a T>` but still rejects multiple matches. `iter()` is an exact-size
iterator over all matching borrowed results in snapshot/plan order.

Selections perform no I/O, cancellation, or implicit first-match choice. They
never broaden Runtime's scope. Ordinary tasks see same-global-configuration
prerequisites; the converter sees transitive recordings across configurations.
NPY batches remain aggregate; selecting a batch does not filter its members.

## Advanced API

`Dependency` is a sealed trait implemented only by the three result types.
`phase()`, `task()`, and `description() -> String` support generic selectors and
source-rich diagnostics. Applications cannot implement it.

`DependencyError` is non-exhaustive and implements Error + Send + Sync:
`MissingEnvironment { variable }`; `Io { path, source }`;
`Invalid(String)`; `Missing { selection }`;
`Ambiguous { selection, matches: Vec<String> }`. Match lists identify phase/task
and member where applicable. Loads preserve I/O/JSON context and return no
partial collection. Unknown workload kinds are preserved in raw_json but excluded
from typed selections. Unknown extra keys are tolerated. Known variants require
valid fields, absolute paths, nonempty identifiers, unique phase/task/member
identities, and u64 final iterations. Snapshot validation does not establish file
existence or scientific correctness; use Persistence/NPY readers for that.

The serialized array contract remains unchanged. Changing dependencies() from
raw Value is a breaking API change in 0.13.5; existing JSON parsers can explicitly
use dependencies().raw_json() while migrating. No implicit Deref compatibility
alias hides this change.

## Example

```rust,ignore
let recording = context.dependencies().recordings()
    .execution_unit("initialize").member("initialization").one()?;
let decoders = JsonPayloadDecoderRegistry::new()
    .with_json_field::<Vec<f64>>("population")?;
let state = StoredStateSeriesReader::open_completed_recording(
    recording.directory(), decoders,
)?.read_latest_state_from_stream("checkpoint")?;
```

For multiple producers, add `.in_phase("prepare")` or `.task(identity)`.

## Not API

Parsing structs, storage vectors, Runtime summary conversion, scope correlation,
and task scheduling are private. No public writer, artifact registry, environment
manager, or scheduler handle is introduced.
