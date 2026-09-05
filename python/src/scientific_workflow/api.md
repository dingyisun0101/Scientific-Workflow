# Python companion API

Python distribution/import: `scientific-workflow` / `scientific_workflow`, version
0.4.3. Python 3.14+; Linux is the supported execution platform. The base package
has no runtime dependencies; `[npy]` installs NumPy. Imports have no environment,
logging, working-directory, subprocess, or output-creation side effects.

## Basic API

### Recording reader

`scientific_workflow.reader.open_completed_recording(directory, decoders=None)`
returns a `RecordingReader` after metadata validation and successful-lifecycle
validation. Directory parameters accept `str | pathlib.Path`. Root reexports
remain available for the reader, state containers, and recording errors; the old
`scientific_workflow_reader` import package is not provided.

`RecordingReader(directory, decoders=None)` validates metadata but permits
inspection of incomplete recordings. Properties: `directory`, `format_version`
(the file's actual 7 or 8), `stream_names` (ordered tuple), `user_metadata`,
`terminal_metadata`, `timing`. Methods: `stream_record_count(stream)`,
`stream_encoded_bytes(stream)`, `read_stream(stream)`, `read_all_streams()`,
`read_latest(stream)`, `iter_verified_records(stream)`. Complete reads are
transactional; the iterator verifies a bounded chunk before yielding and may
fail later after prior chunks were yielded. Decoders are mappings from field
name to callable; ordinary JSON values are the default. See the Python README
for integrity/lifecycle details and custom-decoder examples.

`FORMAT_NAME` identifies scientific-workflow-jsonl; `FORMAT_VERSION` is the
highest supported version, 8. Writers remain Rust-owned. `Decoder` is the decoder
type alias. `StateField`, `StateRecord`, `StateSeries` are frozen containers;
application-supplied payloads retain their own mutability. All recording error
classes are exported at package root: `RecordingError`, `MetadataError`,
`IntegrityError`, `RecordError`, `DecoderError`, `RecordingNotCompleteError`,
`UnknownStreamError`. They preserve contextual stream/file information; no
scientific partial series is returned by complete reads.

### Dependencies

`scientific_workflow.dependencies.Dependencies(snapshot)` validates an owned
copy of a dependency JSON array. `load(path)` reads an explicit snapshot;
`from_env()` requires WORKFLOW_DEPENDENCIES_PATH. Both are class methods.
`recordings()`, `programs()`, `npy_batches()` return typed `Selection` objects.
`raw_json()` returns an independent mutable JSON copy including unknown kinds.

Result dataclasses are frozen and contain paths as `Path`:

| Type | Public attributes |
|---|---|
| `RecordingDependency` | phase, task, execution_unit, member, final_iteration, directory |
| `ProgramDependency` | phase, task, directory, executable, python_script (Path or None) |
| `NpyDependency` | phase, task, directory |

Acquire results from Dependencies; direct dataclass construction does not validate
an external snapshot. Program directory means `<task>/artifacts`; recording
means member root; NPY means aggregate batch root. Runtime already selected the
replicate/configuration scope. NPY batches may contain several global configurations.

`Selection.in_phase(key)` and `.task(identity)` return new intersections.
`.execution_unit(key)` and `.member(identity)` filter recordings; they match
nothing on other result types. `.one()` requires exactly one result;
`.optional()` allows zero or one but rejects ambiguity. `.iter()` and Python
iteration enumerate all matches in deterministic snapshot order. All lookups are
pure and perform no scientific I/O. Selection references keep results alive.

`DependencyError(ValueError)` covers malformed snapshots and read/environment
failures. `MissingDependencyError` means zero matches for one().
`AmbiguousDependencyError` exposes `selection` and `matches` and identifies all
matching phase/task/member sources. Known kinds require valid fields, unique
identifiers, absolute paths and u64 iterations; unknown extension keys/kinds are
preserved. File existence and scientific correctness are checked by the reader.

### Standard project accessors

**REQUIRED LAYOUT:** declarations remain at `<study>/wf_configs/study.json` and
`parameters.json`. Runtime creates per-program `workflow-config.json` and
`workflow-dependencies.json` beside `artifacts/`, `stdout.log`, and `stderr.log`.
**Do not rename or relocate required files.** There is no heuristic discovery.

`scientific_workflow.project` exports:

- `project_root() -> Path`: verify WORKFLOW_PROJECT_ROOT is an absolute directory.
- `output_directory() -> Path`: verify WORKFLOW_TASK_OUTPUT, the artifacts directory.
- `study_path(root) -> Path`: require `<root>/wf_configs/study.json`; no parsing.
- `parameters(section=None, *, snapshot=None) -> object`: load resolved parameters
  from WORKFLOW_CONFIG_PATH or an explicit runtime snapshot. Return all parameters
  or one exact top-level section. Do not reread unresolved source declarations.
- `ProjectLayoutError(ValueError)`: identifies the required variable/file/layout
  and chains underlying read/parse failures.

Accessors synchronously read files/environment. They do not create files, mutate
cwd, configure logging, activate environments or implement a second resolver.
Use explicit paths outside a Workflow program; from-env calls require the launch
contract. There is no ProgramContext.

### NPY readers and whole-series views

`scientific_workflow.npy` requires the `[npy]` extra.
`open_npy_batch(directory) -> NpyBatch` verifies the batch and every member.
`open_npy_conversion(directory) -> NpyConversion` verifies one member. Acquire
objects through these functions; direct constructors do not establish integrity.
Both require the standard manifest directory, not an individual `.npy` path.

`NpyBatch` attributes: `directory`, `manifest`, `members` (ordered tuple).
`NpyConversion` attributes: `directory`, `manifest`, `stream_names` (tuple),
`execution_unit` (provenance key or None). Methods:

| Method | Result |
|---|---|
| `array(relative_path)` | Cached read-only memory map of a declared component |
| `field(stream, field)` | Field representation metadata |
| `reconstruct(stream, field, record)` | One exact numeric or JSON fallback record |
| `projection(stream, field, logical_path, record)` | One structured numeric projection record |
| `coordinates(stream)` | Tuple (iterations, physical_times); absent physical times are None |
| `series(stream, field, logical_path=None)` | Cached FixedSeries or RaggedSeries |

Omit logical_path for wholly numeric fields. Structured fields require an exact
projection path, including JSON-pointer escaping. Missing or ambiguous projections
raise `NpyConversionError`. Opening verifies checksums/layout up front; series
access does not repeat that complete validation or reconstruct every JSON record.
Callers must not mutate metadata dictionaries or source files after opening.

`NumericSeries = FixedSeries | RaggedSeries`. Both frozen dataclasses expose
`iterations`, optional `physical_times` (None when absent), `len(series)`, and `record(index)`.
`FixedSeries.values` is a read-only array with a leading record axis.
`RaggedSeries.data`, `.offsets`, `.shapes` are read-only components; record()
slices and reshapes one record in C order, including empty shapes. Indices must
be Python integers in [0, len); booleans, negative and out-of-range indices raise
IndexError. Views retain references to maps and remain usable while retained;
there is no explicit close API. Reading pages may incur filesystem I/O. Access
is read-only and has no cancellation or publication effects.

## Advanced API

`convert_recording(recording_directory, output_directory=None)` verifies and
converts a completed recording, returning its manifest. Default output is the
recording sibling suffixed `-npy`. Conflicts fail; verified matching output is
reused. Successful publication is atomic after complete validation. Source files
must remain immutable. NPY_FORMAT and NPY_BATCH_FORMAT remain v2;
MANIFEST_FILE is `manifest.json`. NpyConversionError(ValueError) reports storage
contract failures; recording errors and I/O failures retain their own types.

`convert_workflow_dependencies(dependencies_path, output_directory)` converts
completed prerequisite recordings and publishes a batch in stable source order.
The installed CLI `scientific-workflow-to-npy` and `python -m
scientific_workflow.npy` call `main()`. Consult `--help` for CLI flags; public
conversion call shapes remain unchanged. No supported Python recording writer
or runtime scheduler/control handle is exposed.

Optional dependencies are imported only by their owning module. OF/Dispatcher
retain domain adapters, scientific validation, statistics and plotting. This
package owns generic wire/layout mechanics.

## Example

```python
from scientific_workflow.dependencies import Dependencies
from scientific_workflow.npy import open_npy_batch
from scientific_workflow.project import parameters

settings = parameters("analysis")
batch_path = Dependencies.from_env().npy_batches().one().directory
batch = open_npy_batch(batch_path)
for member in batch.members:
    if member.execution_unit == "simulation":
        signal = member.series("statistics", "stats", "/energy")
        print(signal.iterations, signal.record(0))
```

The example runs in a standard Workflow-launched analysis task. Outside Workflow,
use Dependencies.load(explicit_snapshot) and parameters(snapshot=explicit_path).

## Not API

Underscore-prefixed validators, planners, writers, framing helpers, cache layout,
worker orchestration and temporary naming are internal. Recording and NPY wire
schemas are separately versioned contracts; implementation internals are not.

## Reporting reference

`scientific_workflow.reporting.log(message: str, *, level="info")` emits one
flushed prefixed event to stderr. Levels: debug/info/warning/error/success.
`progress(stage: str, completed: int, total: int | None = None, *, unit="records")`
emits counts; nonempty stage/unit and u64 bounds are required. Invalid input or
frames over 16 KiB raise ValueError before output; stderr write/flush errors
propagate. Calls serialize threads in one process but do not synchronize unrelated
processes. Outside Workflow, output remains prefixed stderr lines.

`WorkflowHandler(logging.Handler)` follows standard Handler construction,
level/filter/formatter/close behavior and overrides emit(record). It maps standard
logging levels to Workflow severities and formats through the installed formatter.
`install_logging(logger=None, *, level=logging.INFO) -> WorkflowHandler` attaches
one handler idempotently to that logger (root when omitted). It sets handler level,
not logger level; callers retain logging policy. Remove through
logger.removeHandler(handler) and handler.close(). Imports do not configure logging.
Each process configures its own logging; converter workers use a bounded queue to
the parent emitter rather than writing progress directly. See program-events-v1.

## Converter execution and publication

`convert_workflow_dependencies` uses min(WORKFLOW_THREADS, unique recordings),
with a standalone default of one. Rust supplies/reserves the allowance across
replicates. Spawn workers own their verified reader and arrays; threadpoolctl
limits native numeric pools to one thread each. There is no new mandatory public
argument. Progress reports planning, writing, verification, member reuse/completion,
and batch totals. Completion order never changes manifest member order.

Linux directory flock serializes competing publishers. Unique temporary paths
avoid same-process collisions. Failure terminates/joins workers, publishes no
success batch, and retains individually verified members for retry. Staging
folders left by abrupt termination are not published data. Successful metadata
publication uses atomic replacement. Private control checkpoints freeze work at
record/hash/job boundaries; parent acknowledgement requires all active jobs to
be paused or complete. Admission stops during pause. Cancel while paused wakes
and terminates work; raw log draining in Rust continues throughout.

State container detail: StateField(name, description=None) exposes those fields;
StateRecord(iteration, physical_time, values) exposes them, and create() wraps
values in a read-only MappingProxyType. StateSeries(stream, fields, records)
implements len, indexing/slicing, iteration, and an iterations tuple property.
These frozen containers do not deep-freeze decoded application payloads.
