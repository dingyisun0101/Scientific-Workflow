# Scientific Workflow: Rust Architecture

## Scope and status

This is the authoritative clean-slate design for the Rust crate. Superseded
alternatives are intentionally absent.

Current scope:

- Rust only; Python and bridging are later modules.
- JSON persistence only; protobuf is out of scope.
- No backward compatibility or legacy support.
- Payloads may be any `Serialize + Clone + Send + 'static` Rust value.
- Production tests live under `tests/`, never inside module files.
- Additional built-in payload decoders are deferred until core development is
  complete.

Implemented and verified:

- `configuration`: standard three-file parameter/path loading, deterministic
  task expansion, complete shared task handles, exact sweep selection, named
  paths, and exact export;
- `project`: conventional four-file `ScientificProject` loading including
  `config/state.json`;
- `execution`: automatic generated/named execution scopes and task paths;
- `reporting`: parameter-identified atomic progress tracking and exclusive
  centralized terminal rendering for parallel tasks;
- `system_state`: mutable simulation-owned heterogeneous state;
- `time_series`: eager in-memory analysis collection;
- storage format, borrowed encoder, bounded writer, decoder registry, two
  default decoders, and eager reader.

The run-level storage facade is implemented and public. It connects private
encoders and writers, owns the sole `metadata.json` lifecycle, and is available
together with every supported crate API through `scientific_workflow::prelude`.

## End-to-end workflow

    simulation owns and mutates one complete SystemState
        -> SystemStateWriter::observe_state checks every typed stream sampling interval
        -> non-due streams return without payload access
        -> due JsonStateRecordEncoder borrows only selected fields
        -> one owned EncodedStateRecord is produced
        -> StateWriterWorker::submit_record applies bounded backpressure
        -> worker appends indivisible records to byte-targeted chunks
        -> writer facade commits chunk descriptors and completion metadata

    completed metadata.json and immutable chunks
        -> StoredStateSeriesReader validates metadata and selects a stream
        -> each JSONL record is read into borrowed raw field slices
        -> reader looks up the decoder registered for each exact key
        -> decoder converts only that raw field into its concrete payload
        -> reader assembles SystemState values
        -> reader returns one complete StateSeries

`StateSeries` is an analysis result, not a runtime writer buffer. The persisted
stream is the authoritative sampled history.

## Parameter configuration

The next proposed module reads two conventional JSON files: `fixed.json`
declares values shared by every task, while `sweep.json` declares values that
vary across tasks. This module defines and resolves parameter combinations; it
does not execute tasks, allocate output directories, write recording metadata,
or manage threads.

The agreed public module name is `configuration`. It covers the complete
three-file project configuration while leaving execution and persistence to
later layers. `parameters` would exclude `paths.json`, `constants` would
misdescribe swept values, and `orchestration layer` would imply task execution that this
module deliberately does not perform. A swept value is constant during one
task but is scientifically a parameter across the complete study. The public
model distinguishes:

- `ParameterSpace`: the validated, immutable study definition loaded from both
  files; and
- `TaskParameters`: one resolved task's immutable union of all fixed values and
  one selected value from every sweep axis; and
- `ProjectPaths`: the immutable named path table resolved relative to the
  project root; and
- `TaskConfig`: a cheap owned full-task handle combining `TaskParameters` with
  shared `ProjectPaths` for direct movement into worker queues.

The public namespace is therefore:

```text
scientific_workflow::configuration::{
    ProjectConfig,
    ParameterSpace,
    TaskConfig,
    TaskConfigIter,
    MatchingTaskConfigIter,
    TaskParameters,
    ProjectPaths,
    ConfigurationError,
}
```

`ParameterSpace` should own parsed JSON once. Each `TaskParameters` can retain a
shared owner plus compact selected-value indices, avoiding a clone of every
JSON value for every generated task. Iteration and indexed task lookup must be
deterministic, and task-count multiplication must be checked for overflow.

The agreed standard project layout is:

```text
project-root/
└── config/
    ├── fixed.json
    ├── sweep.json
    └── paths.json
```

`ProjectConfig::load(project_root)` should locate this exact directory and own
the resulting `ParameterSpace` and `ProjectPaths`. The lower-level types remain
independently useful, but callers should not normally assemble standard paths
manually.

The primary task API lives on `ProjectConfig`:

```rust
for task in project.task_configs() {
    submit(task)?;
}

for task in project.task_configs_matching("temperature", 300.0)? {
    submit(task)?;
}
```

`task_configs()` lazily emits the full Cartesian product or the exact explicit
case list. Filtering one sweep key retains every combination of all other axes.
`unique_task_config_matching` succeeds only when that one selector identifies
exactly one task; zero and multiple matches are separate errors. Fixed constants
and paths are deliberately invalid selection keys.

`fixed.json` remains a plain object:

```json
{
  "lattice_shape": [1024, 1024],
  "physical_time_increment": 0.01
}
```

The agreed `sweep.json` contract is tagged. Cartesian mode preserves declared
axis order and changes the last axis fastest:

```json
{
  "mode": "cartesian",
  "axes": [
    {"name": "temperature", "values": [280.0, 300.0, 320.0]},
    {"name": "seed", "values": [1, 2, 3]}
  ]
}
```

Explicit correlated cases use:

```json
{
  "mode": "cases",
  "cases": [
    {"temperature": 280.0, "physical_time_increment": 0.1},
    {"temperature": 300.0, "physical_time_increment": 0.05}
  ]
}
```

Every fixed value, Cartesian candidate, and explicit case value may be any JSON
value, including an object or another array. No Cartesian axes means one
fixed-only task; an individual axis with no candidates is rejected. Explicit
cases must share one key set. Fixed and swept keys are disjoint so `fixed.json`
remains semantically truthful and lookup never needs override precedence.
Duplicate object keys and structurally invalid roots are rejected with path
and key context.

`paths.json` is a separate plain object whose values are path strings. It
contains shared named input roots, data locations, and output roots; it does not
contain per-task output paths derived later by orchestration layer. Relative paths are
resolved against `project_root` on access, without canonicalizing or requiring
the target to exist. The original string remains authoritative for JSON
round-trip. Environment-variable and tilde expansion are initially out of
scope because they make resolution and reproducibility host-dependent.

The lower-level parameter output is a dict-like resolved task rather than an
application-defined Rust configuration structure. The normal project-level
output is `TaskConfig`, which pairs that parameter view with shared paths. The
API is:

```text
space = ParameterSpace::load(directory)
space.task_count()
space.task(index)
space.tasks()

task.task_ordinal()
task.value(key) -> Option<&serde_json::Value>
task.require_value(key) -> Result<&serde_json::Value, ConfigurationError>
task.decode_value<T>(key) -> Result<T, ConfigurationError>
task.contains(key) / keys() / iter() / len() / is_empty()
task.to_json() -> Result<String, ConfigurationError>
```

`TaskParameters` is the logical read-only union of every fixed entry and the
selected sweep entries for one task. The application uses the exact JSON names
as lookup keys and decodes each required value to its concrete type once before
entering a hot loop. Resolved JSON output is useful for provenance and later
orchestration layer metadata, but this module remains independent of storage
recordings.

`TaskParameters` should be cheap to produce and clone. It stores an `Arc` to the
immutable parsed parameter space plus one task ordinal; it does not allocate a
merged map or clone JSON values. Cartesian selection is calculated from
precomputed axis lengths and strides. Explicit-case selection refers directly
to the chosen shared case. Materialization occurs only when a caller explicitly
decodes an owned value or serializes the resolved task.

`TaskConfig` adds only a cheap `ProjectPaths` shared handle. It is owned rather
than lifetime-borrowed so task schedulers can move it through ordinary thread and
work queues while the parsed JSON and path allocations remain shared. It
delegates task ordinal and typed parameter lookup and adds direct named-path
resolution. It does not allocate a merged parameter/path dictionary.

### JSON names and Rust access

JSON keys are preserved exactly and become the runtime lookup names. The
program uses them directly through the read-only dictionary API:

```rust
let physical_time_increment = task.decode_value::<f64>("physical_time_increment")?;
let temperature = task.decode_value::<f64>("temperature")?;
let seed = task.decode_value::<u64>("seed")?;
```

This is the chosen dynamic boundary. Rust cannot turn runtime strings into
compile-time local variables or struct fields, and a trait cannot require
implementors to contain fields named `fixed`, `sweep`, and `paths`. No custom
`ParameterSpace` trait, derive macro, application configuration structure, or
compile-time code generator is required.

The library-owned `ParameterSpace` performs the scientifically central
operation: deterministically combine the fixed map with every Cartesian
selection or explicit case and expose each combination as `TaskParameters`.
`ProjectPaths` follows a similar read-only named lookup model but adds raw-path
and project-root-relative resolution methods; paths are not merged into task
parameters.

### Exact JSON round-trip

Two meanings of “exact” must remain distinct. Semantic round-trip means the
same JSON values, object membership, array order, number values, and strings.
Byte-exact round-trip additionally preserves whitespace, indentation, key
presentation order, and number spelling such as `1.0` versus `1.00`.

The immutable loader must satisfy both: retain the original bytes
of all three source files alongside their validated parsed forms. Re-exporting
or copying the source configuration writes those bytes unchanged. Parsed
inspection and task expansion use the validated representation. A resolved
`TaskParameters::to_json` document is derived data and therefore guarantees a
deterministic canonical semantic representation, not byte identity with either
input file.

`TaskParameters` does not implement Rust's panicking `Index<&str>` syntax.
Missing configuration keys return contextual `ConfigurationError` through the
fallible lookup methods.

### ConfigurationError

The configuration module's non-exhaustive public error type is implemented
first so every later loader and lookup shares one stable contextual failure
boundary. It contains:

- source read, JSON parse, and exact-export write errors with preserved
  filesystem or Serde sources;
- semantic document rejection and exact duplicate-key context;
- fixed/sweep key collisions and checked Cartesian task-count overflow;
- task-ordinal bounds, missing task parameters, and typed decode failures;
- unknown sweep selectors, selector encoding failures, no-match selection, and
  ambiguous unique selection; and
- missing named project paths.

It never owns a resolved task map or JSON payload. Paths, keys, and task indices
are owned only on failure paths so errors remain useful after configuration
objects are dropped.

#### Reference

```text
ProjectConfig loader/exporter
    -> ConfigurationError::{ReadConfigurationFile,
                            ParseConfigurationFile,
                            WriteConfigurationFile}

ParameterSpace validation and expansion
    -> ConfigurationError::{InvalidConfigurationDocument,
                            DuplicateConfigurationKey,
                            FixedSweepKeyConflict,
                            TaskCountOverflow,
                            TaskOrdinalOutOfBounds}

TaskParameters lookup and conversion
    -> ConfigurationError::{UnknownTaskParameter,
                            DecodeTaskParameter,
                            SerializeTaskParameters}

ProjectConfig task selection
    -> ConfigurationError::{UnknownSweepParameter,
                            EncodeTaskSelection,
                            NoMatchingTaskConfiguration,
                            AmbiguousTaskConfiguration}

ProjectPaths lookup
    -> ConfigurationError::UnknownProjectPath
```

### ParameterSpace

Public cheap-clone handle to one validated immutable `fixed.json` plus
`sweep.json` definition. Its shared allocation retains exact source bytes,
declaration-ordered values, lookup indexes, sweep storage, precomputed
Cartesian strides, and the checked `u64` task count.

#### ParameterSpace::load

Reads the two standard files from a supplied `config/` directory, strictly
parses duplicate-preserving JSON, validates the tagged sweep, rejects
fixed/sweep overlap, and builds all indexes before publishing a space.

##### Reference

```text
ProjectConfig::load -> ParameterSpace::load(project_root/config)
direct parameter-only workflows -> ParameterSpace::load(config_directory)
```

#### ParameterSpace::configuration_directory

Returns the directory exactly as supplied without filesystem
canonicalization.

##### Reference

```text
diagnostics and ProjectConfig inspection
```

#### ParameterSpace::fixed_source_json

Borrows the original validated `fixed.json` bytes unchanged.

##### Reference

```text
ProjectConfig exact source export and byte-round-trip tests
```

#### ParameterSpace::sweep_source_json

Borrows the original validated `sweep.json` bytes unchanged.

##### Reference

```text
ProjectConfig exact source export and byte-round-trip tests
```

#### ParameterSpace::fixed_parameter_count

Returns the number of fixed names.

##### Reference

```text
configuration inspection, Debug, and integrated workflow logs
```

#### ParameterSpace::sweep_parameter_count

Returns the number of selected sweep names in every resolved task.

##### Reference

```text
configuration inspection, Debug, and integrated workflow logs
```

#### ParameterSpace::parameter_count

Returns the fixed-plus-swept dictionary size for every resolved task.

##### Reference

```text
orchestration layer validation and task-shape reporting
```

#### ParameterSpace::task_count

Returns the checked Cartesian product or explicit-case count as `u64`.

##### Reference

```text
orchestration layer task allocation, task bounds validation, tasks(), and Debug
```

#### ParameterSpace::contains_parameter

Checks both disjoint key indexes without resolving a task.

##### Reference

```text
orchestration layer preflight and configuration inspection
```

#### ParameterSpace::fixed_keys

Iterates exact fixed keys in source declaration order.

##### Reference

```text
configuration inspection and deterministic metadata
```

#### ParameterSpace::sweep_keys

Iterates exact axis or first-case keys in source declaration order.

##### Reference

```text
configuration inspection and deterministic metadata
```

#### ParameterSpace::task

Bounds-checks one ordinal and returns a cheap `TaskParameters` owner sharing the
space. It never clones JSON values or materializes a merged map.

##### Reference

```text
orchestration layer indexed task assignment and restart lookup
```

#### ParameterSpace::tasks

Returns an owning iterator over increasing deterministic task ordinals.

##### Reference

```text
sequential or parallel orchestration layer task enumeration
```

#### ParameterSpace::clone / Debug

Clone increments one `Arc`; Debug prints only directory and counts.

##### Reference

```text
shared orchestration layer ownership and bounded diagnostics
```

### TaskParameters

One immutable dict-like logical union of all fixed values and one sweep
selection. It stores only the shared space and `u64` task ordinal.

#### TaskParameters::task_ordinal

Returns the stable zero-based ordinal under the loaded sweep definition.

##### Reference

```text
orchestration layer identity, output naming, errors, and provenance
```

#### TaskParameters::value

Looks up an exact key and returns a borrowed `serde_json::Value`. Fixed lookup
uses its index; Cartesian lookup calculates one mixed-radix digit from the
ordinal and precomputed stride; explicit lookup borrows the selected case.

##### Reference

```text
application zero-copy JSON inspection, require_value, decode_value, and iter
```

#### TaskParameters::require_value

Adds task-and-key context to a missing lookup.

##### Reference

```text
application required-configuration access and decode_value
```

#### TaskParameters::decode_value<T>

Deserializes `T` directly from the borrowed JSON value without cloning the
generic JSON tree first. The returned domain value is owned and may allocate.

##### Reference

```text
simulation setup before numerical hot loops
```

#### TaskParameters::contains

Checks both fixed and sweep indexes for an exact key.

##### Reference

```text
optional application parameters and orchestration layer preflight
```

#### TaskParameters::len / is_empty

Reports resolved dictionary size or emptiness without iteration.

##### Reference

```text
SerializeMap length, Debug, and configuration inspection
```

#### TaskParameters::keys

Iterates fixed keys first and sweep keys second, preserving declaration order
inside each group.

##### Reference

```text
application inspection, iter, and deterministic task JSON
```

#### TaskParameters::iter

Pairs every ordered key with its borrowed resolved value.

##### Reference

```text
ResolvedTaskRef::serialize and application dictionary traversal
```

#### TaskParameters::to_json

Serializes borrowed entries directly as one compact deterministic resolved
object without first allocating a merged `serde_json::Map`.

##### Reference

```text
orchestration layer provenance, task manifests, and semantic round-trip tests
```

#### TaskParameters::clone / Debug

Clone copies one ordinal and increments one `Arc`; Debug never traverses or
prints parameter values.

##### Reference

```text
task handoff between orchestration layer threads and bounded diagnostics
```

### TaskParametersIter

Owning fused iterator over the half-open ordinal range. It keeps the shared
space alive even if the originating `ParameterSpace` handle is dropped.

#### TaskParametersIter::next / size_hint

`next` creates one cheap task owner. `size_hint` is exact when the remaining
`u64` count fits the platform `usize` and otherwise reports a saturated lower
bound with no upper bound.

##### Reference

```text
ParameterSpace::tasks -> orchestration layer task enumeration
```

#### TaskParametersIter::clone / Debug

Clone preserves an independent cursor; Debug reports only its ordinal range.

##### Reference

```text
iterator composition and bounded diagnostics
```

### Parameter parsing internals

`ParameterSpaceInner`, `NamedValue`, `SweepPlan`, and `SweepAxis` own the shared
validated representation. `SweepPlan::{keys,key_count,task_count,value}` unify
Cartesian and explicit lookup. `SweepKey` and `SweepKeys` provide one
allocation-free exact-size key iterator. `ResolvedTaskRef::serialize` streams
the logical dictionary by reference.

`StrictValue` is a private duplicate-preserving JSON syntax tree.
`StrictValue::{into_json,duplicate_key}` convert accepted values and detect the
first repeated exact object key at any nesting depth. Its Serde visitor retains
ordered object pairs instead of collecting a map. The private read, parse,
mode-specific validation, collision, object-field, name, and error helpers are
called only by `ParameterSpace::load` and do not form public extension points.

#### Reference

```text
ParameterSpace::load -> strict parser and validation helpers
TaskParameters::value -> SweepPlan::value
ParameterSpace/TaskParameters key iteration -> SweepPlan::keys -> SweepKeys
TaskParameters::to_json -> ResolvedTaskRef::serialize
```

### ProjectPaths

Public cheap-clone immutable dictionary loaded from the standard
`project-root/config/paths.json`. Each value must be a non-empty path string.
The shared allocation retains the supplied project root, source path, exact
source bytes, declaration-ordered entries, and exact-key lookup index.

#### ProjectPaths::load

Derives `config/paths.json` beneath the supplied project root, reuses the strict
recursive duplicate-aware parser, and validates a string-only root object.
Loading performs no path expansion, canonicalization, metadata lookup, or
existence check.

##### Reference

```text
ProjectConfig::load -> ProjectPaths::load(project_root)
direct path-only workflows -> ProjectPaths::load(project_root)
```

#### ProjectPaths::project_root

Returns the root exactly as supplied at load time.

##### Reference

```text
resolve_path, ProjectConfig inspection, and Debug
```

#### ProjectPaths::source_path

Returns the derived `project-root/config/paths.json` path.

##### Reference

```text
diagnostics, ProjectConfig exact export, and Debug
```

#### ProjectPaths::source_json

Borrows the complete original validated source bytes unchanged.

##### Reference

```text
ProjectConfig exact source export and byte-round-trip tests
```

#### ProjectPaths::len / is_empty

Reports the declaration count or whether no path names are configured.

##### Reference

```text
configuration inspection, Debug, and integrated workflow logs
```

#### ProjectPaths::contains

Checks the exact case-sensitive name index.

##### Reference

```text
optional project resources and orchestration layer preflight
```

#### ProjectPaths::path

Returns a borrowed unresolved `Path` exactly represented by the JSON string, or
`None` for an unknown key. It performs no allocation or filesystem operation.

##### Reference

```text
application raw-path inspection and require_path
```

#### ProjectPaths::require_path

Adds the exact missing name to `ConfigurationError::UnknownProjectPath`.

##### Reference

```text
application required-path access and resolve_path
```

#### ProjectPaths::resolve_path

Returns absolute configured paths unchanged and lexically joins relative paths
to the supplied project root. It deliberately does not canonicalize, normalize,
open, or require the result to exist.

##### Reference

```text
orchestration layer input/output root setup and application resource lookup
```

#### ProjectPaths::keys

Iterates exact names in JSON declaration order.

##### Reference

```text
configuration inspection and deterministic metadata
```

#### ProjectPaths::iter

Iterates declaration-ordered names and borrowed unresolved paths.

##### Reference

```text
application dictionary traversal and configuration reporting
```

#### ProjectPaths::clone / Debug

Clone increments one `Arc`; Debug prints only root, source path, and entry
count.

##### Reference

```text
shared task ownership and bounded diagnostics
```

### ProjectPathsInner and PathEntry

Private shared storage and one declaration-ordered name/path pair. They expose
no extension boundary and are accessed only through `ProjectPaths`.

#### Reference

```text
ProjectPaths::load -> ProjectPathsInner / PathEntry
all ProjectPaths accessors -> shared immutable storage
```

### ProjectConfig

Normal public facade for one standard `project-root/config/` directory. It owns
the exact supplied root plus compatible `ParameterSpace` and `ProjectPaths`
handles loaded from that root. It coordinates files but does not merge paths
into task parameters or perform task execution.

#### ProjectConfig::load

Loads and validates `fixed.json`, `sweep.json`, and `paths.json` as one complete
read-only operation. Failure drops any earlier validated component and never
returns a partial facade.

##### Reference

```text
application and orchestration layer startup -> ProjectConfig::load(project_root)
```

#### ProjectConfig::project_root

Returns the root exactly as supplied without canonicalization.

##### Reference

```text
orchestration layer project identity, path setup, and Debug
```

#### ProjectConfig::configuration_directory

Returns the standard `config/` directory retained by the parameter space.

##### Reference

```text
configuration diagnostics and project inspection
```

#### ProjectConfig::parameters

Borrows the validated parameter-space handle.

##### Reference

```text
orchestration layer task_count/task/tasks access
```

#### ProjectConfig::paths

Borrows the validated project-path dictionary.

##### Reference

```text
orchestration layer and simulation resource-path access
```

#### ProjectConfig::task_count

Returns the validated number of complete task configurations.

##### Reference

```text
orchestration layer mission planning -> ProjectConfig::task_count
ScientificProject::task_count -> ProjectConfig::task_count
```

#### ProjectConfig::task_config

Returns one complete cheap task handle by stable ordinal, sharing both parsed
parameter and path storage.

##### Reference

```text
indexed mission selection -> ProjectConfig::task_config
ScientificProject::task_config -> ProjectConfig::task_config
```

#### ProjectConfig::task_configs

Returns an owning lazy iterator over the full Cartesian product or explicit
case list in canonical task-ordinal order.

##### Reference

```text
all-mission construction -> ProjectConfig::task_configs
ScientificProject::task_configs -> ProjectConfig::task_configs
attractor_2d main loop -> ScientificProject::task_configs
```

#### ProjectConfig::task_configs_matching

Converts one caller value to JSON once, requires an exact sweep key, and lazily
retains every task with that exact selected value. Other axes remain
unconstrained.

##### Reference

```text
parameter-subset orchestrating -> ProjectConfig::task_configs_matching
ScientificProject::task_configs_matching -> ProjectConfig::task_configs_matching
configuration_workflow exact filtering
```

#### ProjectConfig::unique_task_config_matching

Returns exactly one matching task or distinguishes no match from an ambiguous
multi-task match. It never silently selects the first result.

##### Reference

```text
single-mission lookup -> ProjectConfig::unique_task_config_matching
ScientificProject::unique_task_config_matching -> ProjectConfig::unique_task_config_matching
configuration_workflow unique/no-match/ambiguity cases
```

#### ProjectConfig::into_parts

Consumes only the facade and moves out both cheap shared component handles. No
source bytes or parsed values are cloned.

##### Reference

```text
applications that transfer parameter and path ownership independently
```

#### ProjectConfig::write_source_config

Creates a destination project root when absent, exclusively creates its
standard `config/` directory, and writes the three validated original byte
sequences through exclusive files. Every file and both affected directories
are synchronized. An existing `config/` entry is rejected atomically by
directory creation and is never overwritten. A failure after directory
creation may retain the new partial configuration as diagnostic evidence.

##### Reference

```text
configuration replication and byte-exact three-file round-trip tests
```

#### ProjectConfig::clone / Debug

Clone copies the root and cheap-clones both shared component handles. Debug
prints only the root and parameter/task/path counts.

##### Reference

```text
shared project setup and bounded diagnostics
```

### TaskConfig

Owned complete task handle containing one `TaskParameters` handle and one
`ProjectPaths` handle. Both fields are `Arc`-backed; cloning or moving the task
does not clone JSON values, create a merged map, or duplicate the path table.

#### TaskConfig::task_ordinal

Delegates the stable deterministic ordinal.

##### Reference

```text
orchestration layer mission ordering and output naming -> TaskConfig::task_ordinal
attractor_2d recording directory -> TaskConfig::task_ordinal
```

#### TaskConfig::parameters

Borrows the underlying parameter-only view for APIs such as writer metadata.

##### Reference

```text
SystemStateWriterBuilder::with_task_parameters -> TaskConfig::parameters
```

#### TaskConfig::paths

Borrows the shared complete path dictionary for lower-level path inspection.

##### Reference

```text
advanced multi-path task setup -> TaskConfig::paths
```

#### TaskConfig::value / require_value / decode_value

Delegate clone-free raw lookup and explicit typed decoding to
`TaskParameters`.

##### Reference

```text
application task assembly -> TaskConfig::decode_value
generic task inspection -> TaskConfig::value / require_value
```

#### TaskConfig::resolve_path

Delegates named lexical path resolution to the shared `ProjectPaths` handle.

##### Reference

```text
task resource and output setup -> TaskConfig::resolve_path
```

#### TaskConfig::clone / Debug

Clone increments two shared owners and copies one task ordinal. Debug exposes
only identity and bounded counts.

##### Reference

```text
orchestration layer work-queue ownership -> TaskConfig::clone/move
bounded diagnostics -> TaskConfig::Debug
```

### TaskConfigIter

Owning lazy adapter over `TaskParametersIter` that attaches a shared path handle
to each yielded task.

#### TaskConfigIter::next / size_hint

Preserves canonical ordinal order and the lower-level iterator's count bounds.

##### Reference

```text
ProjectConfig::task_configs -> TaskConfigIter
all-task orchestration layer loop -> TaskConfigIter::next
attractor_2d Rayon par_bridge -> TaskConfigIter::next
```

#### TaskConfigIter::clone / Debug

Clone preserves an independent cursor over shared sources. Debug contains no
configuration values.

##### Reference

```text
independent mission enumeration and diagnostics
```

### MatchingTaskConfigIter

Owns one encoded exact JSON selector and lazily filters complete tasks without
materializing a result list.

#### MatchingTaskConfigIter::next / size_hint

Yields canonical-order matches; the lower bound is zero because future matches
cannot be known without testing task selections.

##### Reference

```text
ProjectConfig::task_configs_matching -> MatchingTaskConfigIter
ProjectConfig::unique_task_config_matching -> MatchingTaskConfigIter::next
```

#### MatchingTaskConfigIter::Debug

Reports the key and source iterator but deliberately omits the possibly large
selector value.

##### Reference

```text
bounded configuration diagnostics
```

### ScientificProject

`ScientificProject` is the immutable four-file project facade. It owns a
`ProjectConfig` and the `SystemStateSchema` loaded from mandatory
`config/state.json`. Cloning shares all component allocations.

#### ScientificProject::load

Loads `fixed.json`, `sweep.json`, `paths.json`, and `state.json` from the
conventional `config/` directory.

##### Reference

```text
application startup -> ScientificProject::load(project_root)
attractor_2d::main -> ScientificProject::load
dependent-model crate project bootstrap -> ScientificProject::load
```

#### ScientificProject::project_root

Returns the supplied project root without canonicalization.

##### Reference

```text
diagnostics and execution-root resolution -> ScientificProject::project_root
```

#### ScientificProject::configuration_directory

Returns the standard `config/` directory.

##### Reference

```text
startup logging and configuration provenance -> ScientificProject::configuration_directory
```

#### ScientificProject::parameters

Borrows the deterministic fixed-plus-sweep parameter space.

##### Reference

```text
task enumeration and indexed task lookup -> ScientificProject::parameters
```

#### ScientificProject::paths

Borrows the named runtime-path dictionary.

##### Reference

```text
recording and input path resolution -> ScientificProject::paths
```

#### ScientificProject::resolve_path

Convenience delegation for one project-wide named path.

##### Reference

```text
attractor execution-root setup -> ScientificProject::resolve_path
```

#### ScientificProject::task_count / task_config / task_configs

Delegate complete task planning to the lower-level `ProjectConfig` without
requiring callers to traverse component accessors.

##### Reference

```text
normal project task enumeration -> ScientificProject::task_configs
indexed project task lookup -> ScientificProject::task_config
orchestration layer capacity planning -> ScientificProject::task_count
```

#### ScientificProject::task_configs_matching / unique_task_config_matching

Delegate exact sweep selection while retaining the project's state-schema
facade.

##### Reference

```text
filtered scientific-project orchestrating and unique mission lookup
```

#### ScientificProject::state_schema

Borrows the shared state schema loaded from `config/state.json`.

##### Reference

```text
model assembly and writer configuration -> ScientificProject::state_schema
```

#### ScientificProject::configuration

Borrows the lower-level three-file configuration facade.

##### Reference

```text
exact parameter/path source export -> ScientificProject::configuration
```

#### ScientificProject::into_parts

Consumes the facade and returns owned configuration and schema handles without
cloning their shared internals.

##### Reference

```text
consumer ownership separation -> ScientificProject::into_parts
```

#### ScientificProject::clone / Debug

Clone shares parsed allocations. Debug reports only roots and bounded counts.

##### Reference

```text
shared project ownership and startup diagnostics
```

### ScientificProjectError

Wraps either `ConfigurationError` or `StateError` without discarding its source
chain.

#### Reference

```text
ScientificProject::load -> ScientificProjectError
```

### Project export helpers

Private helpers create the destination root and standard directory, write and
synchronize one exclusively created file, synchronize directory entries, and
construct the shared contextual IO error. They operate only on exact paths
derived by `ProjectConfig::write_source_config`.

#### Reference

```text
ProjectConfig::write_source_config -> create_destination_root
                                  -> create_configuration_directory
                                  -> write_source_file (three times)
                                  -> sync_directory (config and project root)
                                  -> write_error on failure
```

### configuration facade

`src/configuration.rs` is the documented public boundary. It keeps the four
implementation files private and re-exports only `ConfigurationError`,
`ParameterSpace`, `TaskParameters`, `TaskParametersIter`, `ProjectPaths`,
`ProjectConfig`, `TaskConfig`, `TaskConfigIter`, and
`MatchingTaskConfigIter`. Its crate-level example demonstrates complete lazy
task generation, named path resolution, and typed lookup.

#### Reference

```text
crate root -> pub mod configuration
prelude -> explicit configuration type re-exports
consumer callers -> scientific_workflow::configuration::{...}
```

## Planned WorkflowRuntime

`WorkflowRuntime` is the single operational owner for one scientific program
execution. It owns first-class phases and tasks, generic bounded phase/task
scheduling, display, and cooperative cancellation for registered work. It does
not own scientific state, storage writers, execution directories, task I/O,
subprocesses, machine resource isolation, or the application's scientific
protocol. Applications declare phase dependencies and task workloads; the
runtime validates, schedules, and displays that declared structure without
inferring scientific dependencies.

The responsibility split is:

```text
application
    owns scientific phase/task definitions, dependencies, workloads,
    all task I/O, subprocesses, completion verification, and result handling
        -> WorkflowRuntime
           owns one or more phases, bounded scheduling, display, and
           cancellation for registered work
               -> Phase
                  owns one or more Tasks and its queue/concurrency policy
                      -> task-local TaskContext is passed into work
```

`ExecutionScope` retains its existing narrower meaning: a filesystem scope for
recordings and artifacts used directly by task code. The runtime does not
create or manage execution scopes.

### Runtime ownership and lifecycle

Only one `WorkflowRuntime` may be active in a process, including hidden-output
operation. Its process-wide lease covers its scheduler, renderer, reporting
state, and cancellation source. It owns no task storage or child-process
infrastructure.

The runtime follows a declare-select-run lifecycle:

1. The application constructs at least one nonempty `Phase`.
2. Tasks are added to a `PhaseBuilder`, never directly to
   `WorkflowRuntimeBuilder` or a live runtime.
3. `WorkflowRuntimeBuilder::build` validates phase IDs, task identities,
   dependencies, display projections, queue/concurrency limits, and display
   policy.
4. `run_phases`, `run_phases_exact`, or `run_phases_with_dependencies` selects
   and validates at least one phase before any reporting or work starts.
5. The selected run acquires the process-wide lease, starts its private
   renderer and event receiver, and executes phases in validated dependency
   order.
6. Within an eligible phase, the runtime prepares work through a bounded queue
   and executes tasks concurrently up to the phase-local limit.
7. Successful runtime completion requires every selected task to be completed
   or explicitly reused. Failure preserves observed states, prevents dependent
   phases from starting, and shuts down owned infrastructure in order.

The runtime implements only the generic scheduling implied by declared phases,
dependencies, and limits. It never derives a dependency from parameter values,
paths, output names, or scientific results. Flat parameter sweeps, compiled
study graphs, nested ensembles, and one-off programs can all declare the same
runtime hierarchy.

### First-class phases

`Phase` is the mandatory top-level work unit beneath a runtime:

```text
WorkflowRuntime
└── Phase (one or more)
    └── Task (one or more)
```

A phase has a stable `PhaseId`, an automatically generated or caller-supplied
human label, one or more tasks, zero or more declared predecessor `PhaseId`
values, `max_concurrent_workloads`, and `queue_capacity`. Empty runtimes, empty
phase selections, empty phases, duplicate phase IDs, unknown dependencies, and
dependency cycles are rejected before execution.

Selected phases execute in validated dependency order. A dependent phase
starts only after all required predecessors are verified as satisfied. Tasks
within one eligible phase execute concurrently. The initial implementation uses
phase barriers; concurrent independent phases require a future explicit policy
rather than occurring implicitly.

Phase selection has two explicit modes:

```text
run_phases_exact([2, 4, 5])
run_phases_with_dependencies([2, 4, 5])
```

Exact selection rejects an omitted unsatisfied prerequisite. Dependency-
inclusive selection adds required predecessors. A previously completed phase
counts as satisfied only through application-provided result verification, not
because a directory happens to exist. The concise `run_phases` convenience has
exact-selection semantics and never starts additional scientific work
silently.

A phase is also the renderer's interactive display unit. Only the active
phase's header, tasks, and phase-local messages occupy the live display; phases
are not rendered as simultaneous sections and therefore need neither an
independent `SectionId` nor positional splitters.

The intended construction and selection shape is:

```rust,ignore
let simulation = Phase::builder(2, "simulation")
    .progress_tasks_from_project(&project, "simulation")
    .display_tasks_by("simulation", ["mu"])
    .max_concurrent_workloads(4)
    .queue_capacity(8)
    .build()?;

let validation = Phase::builder(4, "validation")
    .activity(validation_task)
    .depends_on(2)
    .max_concurrent_workloads(1)
    .queue_capacity(1)
    .build()?;

let runtime = WorkflowRuntime::builder()
    .phases([simulation, validation])
    .build()?;

runtime.run_phases([2, 4])?;
```

This is directional API design rather than a commitment to the exact nested
builder syntax. The semantic commitments are mandatory phase ownership,
configuration-driven task generation, bounded work preparation/concurrency,
and explicit phase selection.

### First-class tasks and identity

A task exists independently of reporting and belongs to exactly one phase:

```text
Task
├── id: TaskId
├── key: TaskKey { phase_id, task_id }
├── kind/namespace
├── complete structured parameter view
├── generated display label
├── display kind
│   ├── Progress { initial_iteration, target_iteration? }
│   └── Activity
└── workload factory
```

`TaskKey` is the exact runtime lookup key. `TaskId` must be unique within its
phase; qualifying it with `PhaseId` permits the same scientific configuration
to participate in simulation, validation, and processing phases without an
identity collision. Labels are generated presentation and never serve as
lookup keys. Duplicate labels are valid when exact task keys differ.

`Progress` is used for iterative work. `Activity` is used for work that runs
once and only changes lifecycle state, such as creating an artifact, validating
a recording, or executing a processor. Activity rows never invent an iteration
counter or progress bar.

The lightweight task declaration exists before rendering. Its workload factory
materializes expensive executable state only after the task enters the phase's
bounded prepared-work queue. The runtime invokes work with a `TaskContext`
containing the appropriate progress/activity handle, cancellation observation,
and retained task configuration. The workload itself owns every read, write,
recording, artifact, network operation, and subprocess needed by that task.

### Automatic configuration task generation

`ProjectConfig` remains the sole authority for fixed/sweep parsing and
deterministic task expansion. The concise
`PhaseBuilder::{progress,activity}_tasks_from_{project,configuration}` helpers
adapt each existing cheap `TaskConfig` into one managed task; the runtime does
not implement a second Cartesian product, merged parameter map, or cloned JSON
identity. This avoids a second task-set registration builder while preserving
the central `config -> tasks -> phase -> runtime` procedure.

Each generated task retains:

- its configuration task ordinal for deterministic order;
- a shared `TaskConfig` handle;
- every resolved fixed parameter;
- every selected sweep parameter; and
- its task kind/namespace, which distinguishes different operations generated
  from the same configuration.

All resolved parameters remain available through task `value`, `require_value`,
and `decode_value` operations. The default row label is generated from the task
kind and every parameter that varies within the phase's generated task set.
Shared fixed values remain accessible through every task's complete identity
and phase-level inspection but are not repeated on every row. Arrays and
objects use compact shape/length plus short-digest labels rather than expanding
their complete contents into terminal output.

Callers may request a shorter display projection such as `display_by(["mu"])`.
Startup accepts it only when that exact parameter subset uniquely distinguishes
the applicable tasks. A collision reports the conflicting exact task keys; it
never produces indistinguishable rows.

`TaskSelector` performs exact partial matching over task kind and any subset of
the complete fixed/sweep parameter view. `unique_task_matching` returns one
task only when the selector has exactly one result; zero matches and ambiguous
matches are distinct contextual errors. Selecting by display text or parsing a
label is unsupported. Omitting a phase from a runtime-wide selector is valid,
but tasks in multiple phases then participate in ambiguity detection.

### Runtime reporting

The interactive renderer is an active-phase projection of the complete runtime
state. The runtime continues to own every selected phase and task, but the live
terminal shows only work belonging to the phase currently crossing its
execution barrier. It has three regions:

- a phase header at the top;
- a stable task region containing only that phase's progress and activity rows;
  and
- a bounded phase-local message region for runtime and task events relevant to
  the active phase.

The header distinguishes selection order from stable identity. For example,
`Phase 2 of 3 — [4] validation` means the second phase in this selected run has
stable `PhaseId(4)`. It also presents phase status, total/running/pending/reused
task counts, the phase's active-work and queue limits, phase-local elapsed time,
and declared dependency state. These are structured runtime values, not text
inferred from labels.

At a phase transition the renderer finalizes and archives the previous phase's
summary, removes its live task and message regions, creates the next phase
header, and installs only the next phase's task rows. Phase-local elapsed time
and messages restart at that boundary. A successful phase may transition
immediately without an artificial viewing delay. A failed phase remains on
screen as the terminal interactive state so the failure and its task context
are not erased.

Runtime-wide messages emitted during a phase are visibly distinguished from
task-scoped messages. The runtime retains only a documented bounded phase
summary. A task that requires durable diagnostics must write its own log or
artifact; the display is never a task's persistence mechanism.

Non-interactive output never clears or rewrites prior lines. It emits uncolored
structured lifecycle records such as `[phase-start]`, `[task]`, and
`[phase-complete]`, including stable phase/task keys where applicable. After a
successful run, the interactive display is replaced by an overall summary of
selected phase and task outcomes rather than leaving the last successful phase
as if it were still active. Hidden mode maintains the same state and lifecycle
semantics without terminal output.

Interactive status flags use terminal-aware color: pending is subdued,
running is active, completed is successful, and failed is an error. Color is a
presentation detail and is never embedded in labels, statuses, plain output,
redirected logs, or persisted data.

`TaskProgress` remains the model-facing handle for iterative work. A separate
`ActivityTask` handle exposes detail, message, completion, failure, status, and
cancellation without iteration methods. Dropping either active handle marks
the reporting task failed, but does not automatically convert a recoverable
storage recording into a terminal failed recording. Reporting lifecycle and
recording lifecycle remain distinct.

The current transient `TaskProgress::set_phase` vocabulary becomes
`set_detail`, avoiding collision with the structural `Phase` type. The current
`ProgressReporter` constructors become private runtime implementation details.
The reporter accepts immutable phase/task views and structured events from the
runtime; it never creates, identifies, owns, selects, schedules, or looks up
phases or tasks. All human-facing messages emitted while a runtime is active
pass through the runtime or a task handle.

### Scheduling boundary, task I/O, and resource containment

`max_concurrent_workloads` limits running workloads within a phase, while
`queue_capacity` limits prepared but not running workloads. Neither limit
describes CPU, memory, process, thread, file, or network resources. Complete
lightweight task declarations remain preregistered so display identity and
ordering are stable even when expensive workload preparation is bounded.

Each task is solely responsible for its scientific execution and I/O. Task
code creates and owns its `ExecutionScope`, storage writers, artifact files,
network clients, subprocesses, thread pools, and any application-specific
backpressure. The runtime invokes the task workload and observes its declared
lifecycle; it does not inspect results, verify receipts, open recordings,
redirect subprocess output, merge writer queues, or infer success from files.
Applications must route any task-owned terminal output through task messages
when an interactive runtime display is active.

Hard resource containment is external to Workflow. The complete application is
launched under an operator-configured service or systemd scope that applies
aggregate CPU, memory, swap, process, and thread limits to the process and its
descendants. Workflow neither translates nor validates those settings. Work
created outside the phase/task interface remains subject to the external scope
but is invisible to runtime scheduling and display.

This yields two deliberately separate guarantees:

- the external service scope contains the entire application; and
- `WorkflowRuntime` schedules and displays only registered phases and tasks.

The runtime does not expose `ResourcePolicy`, `RuntimeClient`, a subprocess
launcher, an IPC protocol, or OS-isolation configuration. Applications such as
Dispatcher retain ownership of Python invocation, systemd integration, worker
protocols, and result verification.

### Implementation sequence

1. Introduce first-class phase/task identity, configuration-driven task
   generation, partial selection, progress/activity kinds, generated labels,
   and reporter observation without duplicating current slot machinery.
2. Introduce `WorkflowRuntime` as the public scheduling and display owner,
   require the runtime/phase/task hierarchy, and add bounded phase scheduling
   and explicit phase selection.
3. Harden the runtime display, cancellation, scheduler shutdown, and public
   documentation while retaining the strict no-I/O boundary.
4. Migrate examples and dependent model runners so they accept only task-local
   handles and never construct reporters or resource scopes.
5. Remove superseded reporting entry points after all in-repository consumers
   use `WorkflowRuntime`; no compatibility layer is required.

State and series equality are not part of this plan. Type-erased scientific
payloads retain the existing `Serialize + Clone + Send + 'static` boundary;
`SystemState` and `StateSeries` do not implement `Eq` or `PartialEq`.

## Current centralized progress reporting

This section records the implemented `ProgressReporter` baseline that the
`WorkflowRuntime` sequence above will migrate. Where its public ownership model
conflicts with the planned runtime, the runtime plan is authoritative.

The `reporting` module is the sole human-facing terminal owner during parallel
scientific execution. It is operational observation, not scientific state:
`SystemState::simulation_time()` remains authoritative and callers explicitly
synchronize its absolute iteration into a task-local atomic counter after a
successful model transition.

The vocabulary is intentionally distinct:

- **task ordinal** is assigned automatically by deterministic configuration
  expansion and is used only for stable ordering and path organization;
- **task identity** is an exact caller-selected combination of parameter
  key/value pairs; and
- **task label** is the deterministic compact JSON rendering of that identity
  for terminal output.

Default identity contains all sweep keys in declaration order. Applications may
select any fixed/swept parameter combination, but it must uniquely distinguish
every generated task. Paths cannot be identity fields. Duplicate or unknown
keys and colliding identities fail before any renderer or worker starts.

One `ProgressReporter` owns a registry of per-task slots and one renderer
thread. Workers update only per-slot atomics for iteration and status; the
renderer polls at a bounded interval. Infrequent detail strings and messages use
locks or a channel outside the numerical counter path. Interactive stderr uses
Indicatif 0.18.6 as a private rendering backend. After the exclusive lease
reaches the renderer thread and before any bar is created, terminal mode clears
stderr's terminal exactly once through `console::Term`; redirected stderr
receives stable line-oriented transitions and is never cleared. Hidden mode
retains lifecycle validation for tests or embedding without rendering.

Every configured task receives a bar immediately, including tasks that remain
pending because the worker pool is smaller than the parameter space. The
renderer forces each initial bar state once to prevent Indicatif's global draw
throttle from starving later rows during synchronized initialization. A task's
elapsed clock resets on its `Pending -> Running` transition, excluding queue
wait from execution time. Known targets render precise elapsed time and ETA;
unknown targets retain elapsed time but label ETA as unknown.

A pseudo-terminal validation of `attractor_2d` captures all three identities
(`mu=-0.25`, `mu=0.25`, and `mu=1.0`) simultaneously, with each completed row
showing `elapsed` and `ETA`. This specifically guards the observed failure mode
where global draw throttling materialized only the earlier rows.

Only one reporting session may exist in a process at a time. A static lease is
acquired after identity validation and released only after renderer shutdown.
Workers, models, writers, and application orchestration must not call terminal
printing APIs while the lease is active; all messages flow through the
reporter. `ProgressReporter::report_error` is the post-session/startup error
boundary used by a binary's `main`.

### ProgressReporterBuilder

Owns a cheap cloned `ProjectConfig`, optional identity keys, and output policy
until validation and renderer startup.

#### ProgressReporterBuilder::identify_tasks_by

Replaces default all-sweep identity with an ordered parameter-key combination.
Uniqueness is validated across the complete task space at `start`.

##### Reference

```text
attractor_2d identity policy -> identify_tasks_by(["mu"])
multi-axis projects -> identify_tasks_by(["temperature", "seed"])
```

#### ProgressReporterBuilder::terminal / plain / hidden

Override automatic stderr detection. Terminal selects cursor-controlled bars,
plain selects stable lifecycle lines, and hidden suppresses output without
disabling counters or lifecycle validation.

##### Reference

```text
interactive application override -> terminal
CI/log capture -> plain
reporting_workflow and embedding -> hidden
```

#### ProgressReporterBuilder::start

Validates identity keys and uniqueness, allocates one slot per automatically
ordered task, acquires the process-wide lease, and starts one renderer thread.
No task is started and no scientific model is constructed by this method.

##### Reference

```text
application reporting startup -> ProgressReporterBuilder::start
```

### ProgressReporter

Non-clone owner of one reporting session, renderer thread, and complete task
registry. Its shared inner registry is borrowed by parallel `start_task` calls;
the session itself retains exclusive finalization authority.

#### ProgressReporter::for_project

Creates the normal builder from `ScientificProject` and delegates its lower
configuration handle.

##### Reference

```text
attractor_2d and conventional applications -> ProgressReporter::for_project
```

#### ProgressReporter::for_configuration

Creates a builder for parameter/path-only workflows without requiring a state
schema.

##### Reference

```text
orchestration layer planning and fixed-only reporting tests -> for_configuration
```

#### ProgressReporter::start_task

Derives ordinal and identity from `TaskConfig`, verifies membership, atomically
transitions the slot from pending to running, and installs absolute initial and
optional target iterations. Callers never provide an ordinal or label.

##### Reference

```text
parallel task closure startup -> ProgressReporter::start_task
```

#### ProgressReporter::report

Routes one application-wide human message through the sole renderer.

##### Reference

```text
infrequent execution announcements -> ProgressReporter::report
```

#### ProgressReporter::summary

Returns a non-blocking aggregate snapshot of pending, running, completed, and
failed slot counts without stopping reporting.

##### Reference

```text
monitoring and reporting_workflow assertions -> ProgressReporter::summary
```

#### ProgressReporter::complete

Requires every registered task to be completed, emits the final success message
and summary, joins the renderer, and releases terminal ownership. Pending,
running, or failed tasks make successful completion an error after orderly
renderer shutdown.

##### Reference

```text
successful application termination -> ProgressReporter::complete
```

#### ProgressReporter::fail

Emits an unsuccessful final summary while preserving each task's observed
status, then joins the renderer and releases terminal ownership.

##### Reference

```text
parallel execution error boundary -> ProgressReporter::fail
```

#### ProgressReporter::report_error

Writes a single standardized error after no live reporter owns the terminal,
covering startup failure and the binary process boundary.

##### Reference

```text
binary error boundaries and reporting_workflow -> ProgressReporter::report_error
```

#### ProgressReporter::Debug / Drop

Debug exposes only identity keys and aggregate counts. Dropping an unfinished
reporter asks the renderer to publish failure, joins it, and releases the lease.

##### Reference

```text
bounded diagnostics and panic/early-return cleanup
```

### TaskProgress

Non-clone worker-local handle to one task slot. Its hot-path iteration update is
an atomic maximum operation. Dropping an active handle marks the task failed,
so ordinary `?` propagation cannot leave it permanently running.

#### TaskProgress::identity

Borrows the exact parameter-derived identity without cloning JSON values.

##### Reference

```text
task-local messages and diagnostics -> TaskProgress::identity
```

#### TaskProgress::current_iteration / target_iteration / status

Read atomic task progress and lifecycle state. Targets are optional and support
open-ended or convergence-driven scientific work.

##### Reference

```text
monitoring, tests, and custom non-terminal presentation
```

#### TaskProgress::set_iteration

Synchronizes to an authoritative absolute simulation iteration without locking
or allocation. Regressions and movement beyond a known target are rejected
without changing the counter.

##### Reference

```text
successful model transition -> SystemState::simulation_time -> set_iteration
attractor_2d::record_model -> TaskProgress::set_iteration
```

#### TaskProgress::set_detail

Updates an infrequent human-readable detail such as `evolving`, `finalizing
storage`, or `validating recording`. It is not intended for hot-loop metrics.

##### Reference

```text
application workflow boundary changes -> TaskProgress::set_detail
```

#### TaskProgress::report

Routes one identity-prefixed task message through the sole renderer.

##### Reference

```text
infrequent task events -> TaskProgress::report
```

#### TaskProgress::complete / fail

Consume the task handle and commit one terminal status. Completion means the
caller-defined complete workflow—including persistence and validation when
applicable—succeeded, not merely that numerical iteration ended. When a target
is known, completion also requires the current absolute iteration to equal it;
open-ended successful work uses no target.

##### Reference

```text
task closure success -> TaskProgress::complete
handled task failure -> TaskProgress::fail
unhandled early return -> TaskProgress::Drop -> failed
```

#### TaskProgress::Debug / Drop

Debug exposes bounded identity/progress facts. Drop marks an active handle
failed with phase `interrupted`.

##### Reference

```text
parallel worker diagnostics and failure-safe cleanup
```

### TaskIdentity

Cheap clone retaining a `TaskConfig`, shared identity-key list, and rendered
label. It never clones parameter JSON values.

#### TaskIdentity::label / len / is_empty / value / iter

Expose deterministic display text and clone-free inspection of the exact
parameter combination in configured key order.

##### Reference

```text
renderer labels, task messages, tests, and custom presentation
```

### TaskStatus

Public non-exhaustive lifecycle enumeration: `Pending`, `Running`, `Completed`,
`Reused`, and `Failed`. Reused work contributes to successful completed
summary counts without inventing an execution interval. Ordinal ordering is
deliberately not encoded as identity.

##### Reference

```text
TaskProgress::status, ProgressSummary aggregation, renderer output
```

### ProgressSummary

Immutable aggregate captured during or after reporting.

#### ProgressSummary::total / pending / running / completed / failed

Return exact task lifecycle counts.

##### Reference

```text
ProgressReporter::summary/complete/fail and orchestration layer run summaries
```

#### ProgressSummary::is_success

Returns true only when every registered task completed and every other count is
zero.

##### Reference

```text
successful finalization validation and reporting_workflow
```

### ReportingError

Non-exhaustive contextual failures cover configuration propagation, identity
keys and uniqueness, task membership and duplicate starts, iteration bounds,
exclusive session ownership, renderer startup/liveness/panic, and incomplete
success finalization.

##### Reference

```text
ProgressReporterBuilder and ProgressReporter/TaskProgress fallible methods
reporting_workflow reachable failure families
```

### Private reporting infrastructure

`ReporterInner`, `ProgressSlot`, `RenderEvent`, `TerminalLease`, and
`TerminalDisplay` remain private. They implement shared atomics, the infrequent
message channel, process lease, one-time interactive terminal clearing,
complete initial-row materialization, task-relative elapsed/ETA tracking,
Indicatif rendering, plain transitions, and failure-safe shutdown without
becoming extension points.

##### Reference

```text
public reporting facade -> private centralized renderer implementation
```

## Execution scopes

`ExecutionScope` owns one project-execution directory but never creates task
recording children. Those paths remain absent until `SystemStateWriter`
exclusively creates them.

### ExecutionScope::create_generated

Creates the parent root when necessary, captures a UTC timestamp, and
exclusively creates a readable collision-resistant execution directory using
timestamp, process, and atomic sequence components.

#### Reference

```text
ordinary application run -> ExecutionScope::create_generated(recording_root)
attractor_2d::main -> ExecutionScope::create_generated
```

### ExecutionScope::create_named

Validates one safe path component and exclusively creates that named scope.

#### Reference

```text
orchestration layer-owned run identity -> ExecutionScope::create_named
reproducible reference run -> ExecutionScope::create_named
```

### ExecutionScope::open_existing

Read-only opens an existing directory. It does not invent a historical
creation timestamp or modify the scope.

#### Reference

```text
task continuation orchestration -> ExecutionScope::open_existing
```

### ExecutionScope::directory

Returns the scope directory.

#### Reference

```text
execution logging and task-path derivation -> ExecutionScope::directory
```

### ExecutionScope::created_at_utc

Returns the captured RFC 3339 UTC timestamp for a newly created scope, or
`None` for a reopened scope without separate scope metadata.

#### Reference

```text
application execution log -> ExecutionScope::created_at_utc
```

### ExecutionScope::task_recording_directory

Deterministically derives `task-{ordinal:06}` without creating it. The ordinal
is automatic ordering, not scientific identity.

#### Reference

```text
per-task writer setup -> ExecutionScope::task_recording_directory
attractor_2d::record_model -> ExecutionScope::task_recording_directory
```

### ExecutionScope::clone / Debug

Clone copies only small path/timestamp owners. Debug contains no task or
scientific payload data.

#### Reference

```text
shared execution orchestration and bounded diagnostics
```

### ExecutionScopeError

Distinguishes invalid caller names, UTC formatting failure, contextual IO, and
exhausted generated-identity attempts.

#### Reference

```text
ExecutionScope constructors -> ExecutionScopeError
```

### Execution-scope helpers

`validate_name` permits exactly one nonempty normal path component.
`compact_timestamp` removes RFC 3339 punctuation for readable ordered directory
names; exclusive creation, not the timestamp, remains the uniqueness authority.

#### Reference

```text
ExecutionScope::create_named -> validate_name
ExecutionScope::create_generated -> compact_timestamp
```

## Core invariants

1. `SystemStateSchema` fixes keys and order but never persists Rust type names.
2. All states derived from one spec share its immutable allocation through
   `Arc`.
3. The simulation owns and mutates its live `SystemState`, one mutable payload
   borrow at a time.
4. `set`, `take`, writer submission, and decoded insertion transfer ownership;
   they do not clone payloads.
5. `SystemState::clone` and `StateSeries::clone` are explicit deep clones and
   should be avoided in performance-sensitive paths.
6. Encoding borrows payloads but necessarily allocates the owned serialized
   record bytes.
7. Each stream has independent selected keys, sampling interval, directory, queue,
   chunk sequence, and analysis series.
8. One sampled partial state is one indivisible JSONL record.
9. Chunk rollover uses exact framed bytes; no record is split.
10. Writer admission is bounded by a user byte budget and an internal maximum
    of 1,024 accepted but uncommitted records.
11. Full queues block the simulation until capacity becomes available.
12. A run directory contains exactly one structural metadata file.
13. Chunk files contain only compact records with readable field keys.
14. A reader returns a complete series or an error, never a partial series.
15. Reader key lookup and decoder conversion are separate responsibilities.

## Intended file tree

The refactor retains Rust's modern split-module layout: a public module facade
such as `configuration.rs` may own implementation files in a same-named
`configuration/` directory. No `mod.rs` files are used. Small independent
concepts remain single files. Generated build output and virtual environments
are omitted.

```text
workflow/
├── .gitignore
├── README.md
├── audit.md
├── docs/
│   ├── design.md
│   └── tests.md
├── rust/
│   ├── Cargo.toml
│   ├── Cargo.lock
│   ├── LICENSE
│   ├── README.md
│   ├── src/
│   │   ├── lib.rs
│   │   ├── prelude.rs
│   │   ├── clock.rs
│   │   ├── artifact.rs
│   │   ├── project.rs
│   │   ├── rng_record.rs
│   │   ├── configuration.rs
│   │   ├── configuration/
│   │   │   ├── error.rs
│   │   │   ├── parameters.rs
│   │   │   ├── paths.rs
│   │   │   └── project_config.rs
│   │   ├── execution.rs
│   │   ├── execution/
│   │   │   ├── error.rs
│   │   │   └── scope.rs
│   │   ├── runtime.rs
│   │   ├── runtime/
│   │   │   ├── error.rs
│   │   │   ├── phase.rs
│   │   │   ├── task.rs
│   │   │   ├── scheduler.rs
│   │   │   ├── reporting.rs
│   │   │   └── renderer.rs
│   │   ├── system_state.rs
│   │   ├── system_state/
│   │   │   ├── error.rs
│   │   │   ├── schema.rs
│   │   │   ├── state.rs
│   │   │   └── value.rs
│   │   ├── time_series.rs
│   │   ├── time_series/
│   │   │   ├── error.rs
│   │   │   └── state_series.rs
│   │   ├── storage.rs
│   │   └── storage/
│   │       ├── error.rs
│   │       ├── json_payload_decoder.rs
│   │       ├── jsonl_format.rs
│   │       ├── json_state_record_encoder.rs
│   │       ├── queued_state_writer.rs
│   │       ├── stored_state_series_reader.rs
│   │       └── json_payload_decoder/
│   │           ├── string.rs
│   │           └── vec_f64.rs
│   └── tests/
│       ├── fixtures/
│       │   ├── configuration/
│       │   │   ├── cartesian_project/config/
│       │   │   │   ├── fixed.json
│       │   │   │   ├── paths.json
│       │   │   │   ├── state.json
│       │   │   │   └── sweep.json
│       │   │   └── cases_project/config/
│       │   │       ├── fixed.json
│       │   │       ├── paths.json
│       │   │       └── sweep.json
│       │   ├── coupled_state.json
│       │   └── state.json
│       ├── analysis_workflow.rs
│       ├── artifact_workflow.rs
│       ├── configuration_workflow.rs
│       ├── python_reader_conformance.rs
│       ├── resume_workflow.rs
│       ├── rng_record_workflow.rs
│       ├── runtime_workflow.rs
│       ├── state_workflow.rs
│       ├── storage_resilience.rs
│       └── storage_workflow.rs
├── python/
│   ├── pyproject.toml
│   ├── LICENSE
│   ├── README.md
│   ├── src/scientific_workflow_reader/
│   │   ├── __init__.py
│   │   ├── errors.py
│   │   ├── model.py
│   │   ├── py.typed
│   │   └── reader.py
│   └── tests/
│       ├── fixtures/complete/
│       │   ├── metadata.json
│       │   └── streams/signal/chunk-000000.jsonl
│       ├── roundtrip_bridge.py
│       └── test_reader.py
└── examples/
    └── attractor_2d/
        ├── Cargo.toml
        ├── Cargo.lock
        ├── README.md
        ├── design.md
        ├── steps.md
        ├── config/
        │   ├── fixed.json
        │   ├── paths.json
        │   ├── state.json
        │   └── sweep.json
        └── src/
            ├── main.rs
            ├── cross_check.rs
            ├── hopf_model.rs
            ├── recording.rs
            ├── task_execution.rs
            └── validation.rs
```

`runtime` replaces the standalone public `reporting` module and owns all
reporting implementation. `execution` remains intentionally small because its
filesystem-scope responsibility is independent of runtime scheduling. Storage,
state, time-series, configuration, and Python reading retain their established
responsibility boundaries. The excluded development-only `rust/src/main.rs`
is removed rather than carried into the intended package layout.

## System state

### StateFieldSchema

Immutable normalized field declaration: compact index, exact key, and optional
natural-language description. It contains no type or codec tag.

#### StateFieldSchema::new

Private normalized construction during template validation.

##### Reference

    SystemStateSchema::from_template -> StateFieldSchema::new

#### StateFieldSchema::position

Returns template-order slot index.

##### Reference

    SystemState compact slot lookup and metadata inspection

#### StateFieldSchema::name

Returns the exact normalized key.

##### Reference

    encoder field selection and dictionary-style access

#### StateFieldSchema::description

Returns optional documentation without affecting behavior.

##### Reference

    run metadata construction -> StateFieldMetadata

### SystemStateSchema

Cheap cloneable handle to one immutable `Arc`-shared layout containing ordered
fields, key lookup, and template provenance.

#### SystemStateSchema::load_json_template

Loads the first layout from the required JSON template path.

##### Reference

    program initialization -> SystemStateSchema::load_json_template

#### SystemStateSchema::parse

Crate-private reconstruction from metadata bytes using identical validation.

##### Reference

    StoredStateSeriesReader stream schema -> SystemStateSchema::parse

#### SystemStateSchema::create_empty_state

Creates a blank state sharing the layout.

##### Reference

    simulation initialization and StoredStateSeriesReader state assembly

#### SystemStateSchema::to_json_template

Returns normalized pretty JSON for semantic round trips.

##### Reference

    template inspection and tests -> SystemStateSchema::to_json_template

#### SystemStateSchema::template_path

Returns retained template or metadata provenance.

##### Reference

    diagnostics and tests -> SystemStateSchema::template_path

#### SystemStateSchema::field_schemas

Returns declarations in deterministic template order.

##### Reference

    encoder canonicalization and metadata construction

#### SystemStateSchema::len

Returns declared field count.

##### Reference

    SystemState allocation -> SystemStateSchema::len

#### SystemStateSchema::is_empty

Reports whether no fields are declared.

##### Reference

    time-only state inspection and tests

#### SystemStateSchema::field_schema

Looks up one declaration by exact key.

##### Reference

    JsonStateRecordEncoder configuration -> SystemStateSchema::field_schema

#### SystemStateSchema::contains_field

Reports whether a key is declared.

##### Reference

    configuration inspection and tests

#### SystemStateSchema::shares_schema_instance

Performs constant-time `Arc` identity comparison.

##### Reference

    StateSeries::push_state -> SystemStateSchema::shares_schema_instance

#### SystemStateSchema::index_of

Crate-private exact key-to-slot resolution.

##### Reference

    SystemState accessors -> SystemStateSchema::index_of

#### SystemStateSchema::from_template

Private semantic validation and layout construction.

##### Reference

    SystemStateSchema::load_json_template/parse -> SystemStateSchema::from_template

### SimulationTime

Small `Copy` coordinate with mandatory `u64` iteration and optional
finite physical time.

#### SimulationTime::from_iteration

Creates an iteration-only coordinate.

##### Reference

    simulation or reader without physical time -> SimulationTime::from_iteration

#### SimulationTime::from_iteration_and_physical_time

Creates a coordinate only when physical time is finite.

##### Reference

    simulation initialization and StoredStateSeriesReader record reconstruction

#### SimulationTime::iteration

Returns the authoritative iteration coordinate.

##### Reference

    StateSeries ordering, writer ordering, chunk descriptors

#### SimulationTime::physical_time

Returns optional physical time.

##### Reference

    JsonStateRecordEncoder record header and analysis

### SystemState

Simulation-owned fixed-layout dictionary of optional heterogeneous concrete
payloads plus one mutable `SimulationTime`.

#### SystemState::new

Crate-private allocation from a validated specification and time point. It is
the single structural invariant-establishing mechanism: it allocates exactly
one empty, initially type-unbound slot per declared field and is not part of the
consumer API.

##### Reference

    SystemStateSchema::create_empty_state -> SystemState::new

#### SystemState::clone_structure_without_payloads

Creates another blank state sharing the same specification and retaining the
source state's per-slot concrete type definitions without cloning payloads. Its
final allocation is produced from the same structural constructor, but its
contract is different: the caller supplies only a new time, and both field
layout and assembly-established type contracts are inherited. The name is
still potentially confusing because structural `is_empty` means “zero declared
fields,” whereas this method produces a payload-blank state (`is_blank ==
true`).

##### Reference

    assembled state -> SystemState::clone_structure_without_payloads -> typed simulation scratch state
    tests and state reuse

#### SystemState::simulation_time

Returns the complete `Copy` coordinate.

##### Reference

    encoder, writer, series, simulation

#### SystemState::replace_simulation_time

Replaces time and returns the previous coordinate.

##### Reference

    simulation-owned explicit time reset -> SystemState::replace_simulation_time

#### SystemState::advance_simulation_time

Increments iteration by one after a completed model step and optionally adds a physical-time increment
transactionally.

##### Reference

    simulation evolution loop -> SystemState::advance_simulation_time

#### SystemState::schema

Returns the shared immutable spec.

##### Reference

    StateSeries::push_state and JsonStateRecordEncoder compatibility checks

#### SystemState::declared_field_count

Returns structural slot count.

##### Reference

    state inspection and tests

#### SystemState::has_no_declared_fields

Reports whether the layout declares no slots.

##### Reference

    time-only state inspection

#### SystemState::populated_field_count

Counts populated slots.

##### Reference

    simulation diagnostics

#### SystemState::has_no_payloads

Reports whether all declared slots are empty.

##### Reference

    empty-state lifecycle checks

#### SystemState::field_schemas

Returns ordered field declarations.

##### Reference

    dictionary inspection

#### SystemState::contains_payload

Checks whether an exact declared key has a payload.

##### Reference

    simulation conditional access and decoder tests

#### SystemState::payload_has_type<T>

Checks the concrete type in one populated slot.

##### Reference

    runtime type inspection

#### SystemState::insert_payload<T>

Moves a payload into a slot. First insertion establishes the slot's concrete
type definition; same-type replacement returns the previous payload, while a
later different-type insertion is rejected even when the slot is temporarily
empty. Rejection returns the incoming payload through `PayloadInsertError<T>`.

##### Reference

    simulation initialization and JsonPayloadDecoderRegistry::decode_into -> SystemState::insert_payload

#### SystemState::payload<T>

Borrows one concrete payload immutably.

##### Reference

    simulation inspection and analysis

#### SystemState::payload_mut<T>

Borrows one concrete payload mutably through on-demand name resolution.

##### Reference

    simulation evolution and StateSeries::payload_mut_at

#### SystemState::borrow_payloads<Q>

Borrows a tuple of distinct populated payloads immutably. `Q` is written by the
caller as the tuple of expected concrete payload types; the method argument is
the equally sized tuple of field names. The sealed query implementation is
generated internally and is not an end-user concept.

##### Reference

    coupled scientific inspection and multi-input kernels

#### SystemState::borrow_payloads_mut<Q>

Borrows a tuple of distinct populated payloads mutably without moving or
cloning them. It resolves and validates the complete request before returning
any reference and rejects repeated resolved slots. One borrow is intended to
surround an entire coupled kernel or simulation sweep.

##### Reference

    runtime model EcoSystem sweep -> SystemState::borrow_payloads_mut<(SquareLattice, TaxonTable)>
    coupled scientific integrators and solvers

### PayloadTuple

Doc-hidden, sealed public trait required only as the generic mapping behind
`SystemState::borrow_payloads` and `SystemState::borrow_payloads_mut`. A private declarative macro
implements it for heterogeneous tuples of arity two through eight. It is not
re-exported by the prelude and cannot be implemented by consumer crates.

#### PayloadTuple::borrow

Resolves a tuple of field names, validates distinct indices, payload presence,
and retained concrete types, then returns the equally shaped immutable
reference tuple.

##### Reference

    SystemState::borrow_payloads -> PayloadTuple::borrow

#### PayloadTuple::borrow_mut

Performs the same complete preflight before safely separating the slot slice
and returning the equally shaped mutable reference tuple.

##### Reference

    SystemState::borrow_payloads_mut -> PayloadTuple::borrow_mut

#### SystemState::take_payload<T>

Moves one concrete payload out without cloning and restores it on type error.
The now-empty slot retains its assembly-established concrete type definition.

##### Reference

    ownership handoff and allocation reuse

#### SystemState::clear_payload

Drops one payload and reports whether it existed while retaining the field's
concrete type definition.

##### Reference

    explicit working-set cleanup

#### SystemState::clear_all_payloads

Drops every payload while retaining layout, slots, and concrete type
definitions.

##### Reference

    state reuse and cleanup

#### SystemState::serializable

Crate-private borrowed erased-Serde view of one populated payload.

##### Reference

    JsonStateRecordEncoder::encode -> SystemState::serializable

#### SystemState::clone

Deep-clones each populated payload and shares only immutable layout metadata.

##### Reference

    explicit snapshots and StateSeries::clone

### StateError

Non-exhaustive state/template/time/access error vocabulary. It includes
`RepeatedPayloadBorrow` for aliased tuple requests and never owns a scientific
payload.

### PayloadInsertError<T>

Ownership-preserving `SystemState::insert_payload` rejection containing `StateError` and
the unchanged incoming `T`.

#### PayloadInsertError::new

Crate-private rejection construction.

##### Reference

    SystemState::insert_payload validation failure -> PayloadInsertError::new

#### PayloadInsertError::error

Borrows the rejection reason.

##### Reference

    application failure inspection

#### PayloadInsertError::payload

Borrows the unchanged rejected payload.

##### Reference

    application recovery decision

#### PayloadInsertError::into_parts

Returns `(StateError, T)` without cloning.

##### Reference

    decoder insertion handling and caller ownership recovery

### StateValue and ErasedValue

Private boxed type-erasure implementation. `StateValue::{new,type_id,type_name,
is,downcast_ref,downcast_mut,downcast,serializable}` delegate concrete type and
ownership operations to `ErasedValue::{clone_box,as_any,as_any_mut,into_any,
concrete_type_name,as_serialize}`.

#### StateValue and ErasedValue method references

##### Reference

    SystemState::insert_payload/payload/payload_mut/take_payload/serializable/clone
        -> StateValue -> private ErasedValue blanket implementation

### StateSlot

Private storage for one declared field. It pairs the optional owned
`StateValue` with an optional retained `ValueType`. A newly templated slot is
unbound; its first successful payload insertion establishes the type contract.
Removing the payload does not remove that contract.

#### StateSlot::unbound

Creates a payload-empty slot whose concrete type has not yet been established.

##### Reference

    SystemState::new -> StateSlot::unbound

#### StateSlot::empty_like

Creates a payload-empty slot while copying the existing retained type contract.

##### Reference

    SystemState::clone_structure_without_payloads -> StateSlot::empty_like

### ValueType

Private copyable runtime type identity. It retains both `TypeId` for exact
comparison and `type_name` for diagnostics after a slot's payload has been
taken or cleared.

#### ValueType::of<T>

Captures the concrete runtime identity and diagnostic name of `T` when the
first payload is installed.

##### Reference

    SystemState::insert_payload -> ValueType::of

#### ValueType::is<T>

Tests an expected concrete type against a slot's retained type contract.

##### Reference

    SystemState::payload_has_type / validate_slot / insert_payload
        -> ValueType::is

## Time series

### StateSeriesError

Non-exhaustive analysis error with only layout mismatch, non-increasing time,
position bounds, and contextualized field access variants. It contains no IO
or serialization concerns.

### StateSeries

Growable `Vec<SystemState>` analysis collection enforcing shared layout identity
and strictly increasing simulation indices.

#### StateSeries::new

Creates an empty series.

##### Reference

    analysis initialization

#### StateSeries::with_capacity

Creates an empty series with state-owner capacity.

##### Reference

    StoredStateSeriesReader::read_stream_as_state_series and known-size analysis

#### StateSeries::schema

Returns the canonical shared layout.

##### Reference

    StoredStateSeriesReader state assembly and analysis

#### StateSeries::as_view

Returns lightweight copyable `StateSeriesView`.

##### Reference

    borrowed analysis instead of deep clone

#### StateSeries::len / is_empty / capacity

Expose ordinary collection facts.

##### Reference

    analysis and tests

#### StateSeries::reserve

Reserves additional state-owner capacity.

##### Reference

    analysis allocation planning

#### StateSeries::state_at / first_state / last_state / as_state_slice / iter

Provide immutable collection access without cloning.

##### Reference

    analysis traversal and reader verification

#### StateSeries::payload_mut_at<T>

Mutates one typed payload without exposing mutable state time or structure.

##### Reference

    analysis mutation -> SystemState::payload_mut

#### StateSeries::push_state

Moves a state into the collection after layout and time validation.

##### Reference

    analysis construction and StoredStateSeriesReader::read_chunk

#### StateSeries::pop_state

Moves the last state out.

##### Reference

    analysis ownership recovery

#### StateSeries::clear_states

Drops states while retaining vector capacity and canonical spec.

##### Reference

    analysis working-set reuse

#### StateSeries::into_states

Consumes the series and returns its vector allocation.

##### Reference

    consumer ownership transfer

#### StateSeries::clone

Explicitly deep-clones every state payload.

##### Reference

    independent mutable analysis copies only

### StateSeriesView

Copyable borrowed pair of canonical spec and immutable state slice.

#### StateSeriesView::new

Private view construction.

##### Reference

    StateSeries::as_view -> StateSeriesView::new

#### StateSeriesView::schema / len / is_empty / state_at / first_state / last_state / as_state_slice / iter

Expose borrowed collection facts and traversal.

##### Reference

    lightweight analysis paths

### StateSeriesPushError

Owns a `StateSeriesError` and the unchanged rejected `SystemState` in a failure-only
box so failed append never loses payload ownership.

#### StateSeriesPushError::new

Private rejection construction.

##### Reference

    StateSeries::push_state failure -> StateSeriesPushError::new

#### StateSeriesPushError::error / state

Borrow rejection reason and unchanged state.

##### Reference

    caller inspection after failed push

#### StateSeriesPushError::into_parts

Returns `(StateSeriesError, SystemState)` without cloning.

##### Reference

    StoredStateSeriesReader invariant context and caller recovery

## Storage format

Format version 4 stores typed `sampling_interval` scheduling, structurally
separate initial/terminal metadata, and automatic operational timing. Earlier format
versions are intentionally unsupported in this clean-slate stage.

### On-disk layout

    run/
    ├── metadata.json
    ├── signal/
    │   ├── chunk-000000.jsonl
    │   └── chunk-000001.jsonl
    └── space/
        └── chunk-000000.jsonl

One compact record:

    {"iteration":12,"physical_time":0.25,"values":{"values":[1.0,2.0],"label":"sample"}}

`physical_time` is omitted when absent. Field keys remain for readability and exact
decoder orchestrating. Metadata stores schemas, run facts, byte limits, lifecycle,
and chunk descriptors once; no sidecar metadata exists.

### RecordingMetadata

Complete versioned contents of the sole metadata document.

#### RecordingMetadata::running

Creates initial running metadata from time, run attributes, streams, and the
automatically captured creation timestamp. Terminal metadata starts empty.

##### Reference

    SystemStateWriter::create_new_recording -> RecordingMetadata::running

#### RecordingMetadata::validate

Validates structure without filesystem access.

##### Reference

    every metadata commit and StoredStateSeriesReader::open_completed_recording

#### RecordingMetadata::stream / stream_mut

Look up immutable or mutable stream declarations by exact name.

##### Reference

    StoredStateSeriesReader selection and SystemStateWriter chunk bookkeeping

### RecordingStatus

Persisted `Running`, `Complete`, or non-empty-message `Failed` lifecycle.

#### RecordingStatus::validate

Validates lifecycle-specific content.

##### Reference

    RecordingMetadata::validate -> RecordingStatus::validate

### Stored RecordingTiming

Private persisted timing representation containing creation/finalization UTC
timestamps, accumulated active nanoseconds, and continuation count.

#### RecordingTiming::started

Creates timing for a new running recording with no terminal timestamp, zero
duration, and zero continuations.

##### Reference

    RecordingMetadata::running -> RecordingTiming::started

#### RecordingTiming::validate

Requires canonical UTC RFC 3339 timestamps and enforces that running recordings
lack finalization while terminal recordings contain it.

##### Reference

    RecordingMetadata::validate -> RecordingTiming::validate

### Operational clock helpers

`utc_now_rfc3339` formats the host UTC clock, `is_utc_rfc3339` validates the
canonical persisted form, and `duration_nanoseconds` checked-converts monotonic
durations to `u64` nanoseconds.

#### Reference

    recording creation/finalization and ExecutionScope creation -> utc_now_rfc3339
    RecordingTiming::validate -> is_utc_rfc3339
    SystemStateWriter::transition_terminal -> duration_nanoseconds

### RecordFormat

Versioned JSON plus JSON Lines declaration.

#### RecordFormat::json_lines / validate

Construct and validate the only supported encoding pair.

##### Reference

    RecordingMetadata::running/validate -> RecordFormat

### TimeAxisMetadata

Metadata names and optional units for integer and physical coordinates.

#### TimeAxisMetadata::validate

Rejects empty labels and a physical unit without a physical name.

##### Reference

    RecordingMetadata::validate -> TimeAxisMetadata::validate

### StateStreamMetadata

One logical stream's directory, positive typed `sampling_interval`, ordered fields,
byte limits, and committed chunk inventory.

#### StateStreamMetadata::validate

Validates exact names, safe paths, typed sampling intervals and non-zero limits, unique fields,
and ordered non-overlapping chunks.

##### Reference

    RecordingMetadata::validate -> StateStreamMetadata::validate

### StateFieldMetadata

One exact payload key and optional natural-language description.

#### StateFieldMetadata::validate

Rejects empty names and empty present descriptions.

##### Reference

    StateStreamMetadata::validate -> StateFieldMetadata::validate

### ChunkMetadata

Immutable ordinal, filename, record/byte counts, checksum, and iteration range.

#### Compulsory integrity contract

The descriptor's SHA-256 checksum is not optional metadata. Any operation that
claims to validate a sealed chunk or uses that chunk to reconstruct scientific
state must verify both its exact byte count and checksum before exposing decoded
payloads. There is no public unchecked reader, checksum toggle, Cargo feature,
or performance mode that weakens this rule. A mismatch invalidates the read
transaction; previously decoded states remain internal and are discarded.

An operation may avoid opening chunks it does not consume—for example, a
latest-state read does not scan older history. Such chunks are explicitly
unexamined, not implicitly valid or checksum-verified. This distinction keeps
targeted reads efficient without making a false integrity claim.

SHA-256 establishes accidental-corruption detection relative to the committed
descriptor. It does not establish provenance, authenticity, or numerical model
correctness; those require separate mechanisms.

##### Reference

    full-series reconstruction -> verify every consumed chunk checksum
    latest-state reconstruction -> verify the selected newest chunk checksum
    storage resilience tests -> reject byte-count and checksum corruption

#### ChunkMetadata::validate

Validates deterministic naming, non-empty facts, range order, and checksum
syntax.

##### Reference

    StateStreamMetadata::validate and StoredStateSeriesReader integrity verification

### EncodedStateRecord

Non-Clone owner of one complete compact JSON object plus its framing newline and
validated `SimulationTime`.

#### EncodedStateRecord::new

Adds the single framing newline to encoded JSON bytes.

##### Reference

    JsonStateRecordEncoder::encode -> EncodedStateRecord::new

#### EncodedStateRecord::simulation_time / len / bytes

Return temporal coordinate, exact framed length, and borrowed bytes.

##### Reference

    StateWriterWorker admission, ordering, chunk rollover, and append

#### chunk_filename

Returns deterministic `chunk-NNNNNN.jsonl` naming.

##### Reference

    ActiveChunk::create and ChunkMetadata::validate

#### chunk_temp_filename

Returns the corresponding `chunk-NNNNNN.jsonl.tmp` open lifecycle name.

##### Reference

    ActiveChunk creation/recovery and stream-directory inventory

## Storage encoding and writing

### JsonStateRecordEncoder

Crate-private immutable configuration for one stream. It retains only the
stream name and canonical selected keys, borrows selected live payloads, and
produces one owned `EncodedStateRecord` without cloning them. The construction
`SystemStateSchema` is not retained in production.

#### JsonStateRecordEncoder::new

Validates stream name and selected keys, rejects duplicates, and stores keys in
template order.

##### Reference

    SystemStateWriter::create_new_recording -> JsonStateRecordEncoder::new

#### JsonStateRecordEncoder::fields

Iterates selected names in canonical template order for metadata construction.

##### Reference

    SystemStateWriter::create_new_recording -> JsonStateRecordEncoder::fields

#### JsonStateRecordEncoder::encode

Preflights selected slots, serializes borrowed erased payloads into compact JSON,
and ends all borrows before returning the owned record.

The preflight retains the successfully resolved
`&dyn erased_serde::Serialize` values and `ValuesRef` serializes those
cached borrows. This preserves typed `StateAccess` errors and per-field
`EncodeField` context while reducing state lookup from twice to once per key.
The cache contains payload references only and continues using the
encoder's existing canonical field slice for keys, avoiding redundant `&str`
storage. This introduces one small `Vec` allocation per record, so its
performance effect remains a benchmark question rather than an assumed gain.

##### Reference

    SystemStateWriter::observe_state -> JsonStateRecordEncoder::encode -> StateWriterWorker::submit_record

### RecordRef, ValuesRef, and ErasedRef

Private borrowing-only Serde adapters used during encoding.

#### ValuesRef::serialize / ErasedRef::serialize

Serialize selected values in canonical order and delegate to each payload's
existing `Serialize` implementation.

##### Reference

    JsonStateRecordEncoder::encode -> serde_json::to_vec -> private adapters

### StateStreamStorageConfig

Immutable stream name, output directory, non-zero chunk target, and strict
queue-byte budget shared by start and resume.

#### StateStreamStorageConfig::new

Validates configuration without filesystem mutation.

##### Reference

    SystemStateWriter construction -> StateStreamStorageConfig::new

#### StateStreamStorageConfig::create_directory

Creates the configured stream directory without replacing an existing entry.

##### Reference

    SystemStateWriter::create_new_recording -> StateStreamStorageConfig::create_directory

### RecoveredStateStream

Owned append position for one stream: next ordinal, final accepted iteration,
optional active chunk, and optional locator for its latest complete open
record.

#### RecoveredStateStream::empty / latest_open_record

`empty` seeds a new stream. `latest_open_record` exposes only the recovered
locator needed for checkpoint reconstruction before the seed moves into the
worker.

##### Reference

    StateWriterWorker::start_new_recording -> RecoveredStateStream::empty
    SystemStateWriter::continue_recording -> RecoveredStateStream::latest_open_record

### RecoveredUnsealedRecord

Copy-free file range identifying the latest complete JSON object in a recovered
open chunk.

#### RecoveredUnsealedRecord::path / offset / bytes

Borrow the payload path and return the exact byte range used for one checkpoint
decode.

##### Reference

    decode_resume_state -> RecoveredUnsealedRecord::path / offset / bytes

### StateWriterWorker

Non-Clone exclusive recording worker with one bounded FIFO. It receives named
`EncodedStateRecord` values, never a payload or serializer. Private
`StateStreamSink` values preserve independent chunk state for every stream.

#### StateWriterWorker::start_new_recording

Starts one worker after recording startup has created every stream directory
and published initial running metadata.

##### Reference

    SystemStateWriter construction -> StateWriterWorker::start_new_recording

#### StateWriterWorker::recover_state_stream / resume

`recover` checks the sealed ordinal prefix by filename without opening sealed
payloads, then scans at most the highest open payload. It completes a prepared
rename or returns an owned `RecoveredStateStream` containing the valid unprepared file
handle, counters, checksum state, next ordinal, and a small file-range locator
for the latest complete record. The locator avoids copying its encoded bytes
out of the scan buffer.
`resume` transfers all recovered stream seeds directly into one append worker.

##### Reference

    SystemStateWriter::continue_recording -> StateWriterWorker::recover_state_stream
    successful recovery -> StateWriterWorker::continue_recovered_recording

#### StateWriterWorker::submit_record

Consumes a stream name and record, then blocks until the recording-wide record
count and that stream's byte capacity permit FIFO admission. Impossible
oversized records fail immediately.

##### Reference

    SystemStateWriter::observe_state -> StateWriterWorker::submit_record

#### StateWriterWorker::flush_state_stream

Queues a named ordered barrier and blocks until all earlier FIFO work and that
stream's current chunk are durably sealed and described by metadata.

##### Reference

    SystemStateWriter::flush_stream_to_storage -> StateWriterWorker::flush_state_stream

#### StateWriterWorker::finish_recording

Closes admission, drains work, seals every stream's final chunk through the
shared manifest, and joins the worker. Chunk descriptors were already installed
incrementally and are not returned as a duplicate summary.

##### Reference

    SystemStateWriter::complete_recording -> StateWriterWorker::finish_recording
    SystemStateWriter::mark_recording_failed -> StateWriterWorker::finish_recording

#### StateWriterWorker::close_admission / join_worker / drop

Private terminal lifecycle that wakes waiters and prevents detached workers.

##### Reference

    StateWriterWorker::finish_recording and Drop cleanup

#### StateWriterWorker::spawn

Builds per-stream admission state and worker-owned sinks, drops recovery-only
record locators, and transfers every file handle into the sole worker thread.

##### Reference

    StateWriterWorker::start_new_recording -> StateWriterWorker::spawn
    StateWriterWorker::continue_recovered_recording -> StateWriterWorker::spawn

### Work, Shared, QueueState, and StreamQueueState

`Work` is the named record/flush command enum. `Shared` owns the mutex and
condition variables. `QueueState` owns the recording-wide FIFO, count permit,
barrier acknowledgements, lifecycle, and per-stream admission map.
`StreamQueueState` owns one stream's byte limit, outstanding bytes, and final
accepted iteration.

#### Shared::new

Creates an open empty queue with one admission state seeded from each recovered
stream's final accepted iteration.

##### Reference

    StateWriterWorker::start_new_recording -> Shared::new

### StateStreamSink

Worker-owned persistence state for one named stream: directory, chunk target,
next ordinal, and optional active chunk.

#### StateStreamSink::new

Combines immutable stream configuration with a recovered append position.

##### Reference

    StateWriterWorker::spawn -> StateStreamSink::new

#### StateStreamSink::append

Rolls over only before or after complete records, creates an active chunk when
needed, and appends the encoded owner without splitting it.

##### Reference

    StateWriterWorker FIFO record command -> StateStreamSink::append

#### StateStreamSink::flush

Durably seals the stream's current non-empty chunk through the recording
manifest.

##### Reference

    StateWriterWorker flush command or shutdown -> StateStreamSink::flush

### ActiveChunk

Private open-payload owner with incremental SHA-256 and exact counters.

#### ActiveChunk::create

Creates one deterministic `chunk-NNNNNN.jsonl.tmp` payload without replacing
existing output. The open name is a lifecycle state, not a second payload copy:
records are written once into one inode, which sealing later renames.

##### Reference

    writer worker begins a chunk -> ActiveChunk::create

#### ActiveChunk::recovered

Reopens the already validated complete prefix of one unsealed chunk for direct
append without copying its payload.

##### Reference

    recover_open_chunk -> ActiveChunk::recovered

#### ActiveChunk::append

Appends one complete record and updates checksum and facts.

##### Reference

    writer worker FIFO loop -> ActiveChunk::append

#### ActiveChunk::descriptor

Builds the authoritative chunk metadata from incremental counters and a cloned
hasher state without consuming the open chunk.

##### Reference

    ActiveChunk::seal -> ActiveChunk::descriptor

#### ActiveChunk::seal

Synchronizes the open payload and its directory entry, constructs its
descriptor, asks `RecordingManifest::prepare_chunk` to commit that descriptor, then
atomically renames the same inode to `.jsonl` and synchronizes the stream
directory again. Metadata-before-rename ensures a sealed filename can never be
absent from the authoritative inventory. A crash after preparation leaves one
recognizable open payload that recovery can verify and finish.

##### Reference

    chunk rollover or writer finish -> ActiveChunk::seal
    storage_workflow final-name and temporary-file assertions -> successful seal path

#### ActiveChunk::finish_prepared

Completes the crash-interrupted rename for an open chunk whose descriptor was
already committed.

##### Reference

    StateWriterWorker::recover_state_stream prepared tail -> ActiveChunk::finish_prepared

## Payload decoding

### Responsibility split

1. `StoredStateSeriesReader` validates a record and retrieves a raw value by schema key.
2. `JsonPayloadDecoderRegistry` retrieves the decoder registered for the same exact key.
3. `JsonPayloadDecoder<T>` receives only that raw JSON field and returns owned `T`.
4. The registry moves `T` into the matching empty state slot.

A configured decoder entry exists per payload key. A Rust decoder type may be
reused when several keys share the same representation. JsonPayloadDecoderRegistry never perform
key lookup or see sibling fields.

### JsonPayloadDecoder<T>

Thread-safe typed conversion contract from borrowed raw JSON `&str` to owned
`T`, with an owned thread-safe associated error. Compatible closures receive a
blanket implementation.

#### JsonPayloadDecoder::decode_json_payload

Converts exactly one complete raw JSON value.

##### Reference

    JsonPayloadDecoderRegistry erased adapter -> JsonPayloadDecoder::decode_json_payload -> concrete T

### JsonPayloadDecoderRegistry

Non-Clone heterogeneous exact-key registry. Additional entries are permitted so
one registry can cover several streams.

#### JsonPayloadDecoderRegistry::new / with_capacity

Create an empty registry, optionally reserving key capacity.

##### Reference

    analysis setup -> JsonPayloadDecoderRegistry construction

#### JsonPayloadDecoderRegistry::register_for_field

Binds one exact key to one typed decoder, rejecting empty or duplicate keys.

##### Reference

    application reader configuration -> JsonPayloadDecoderRegistry::register_for_field

#### JsonPayloadDecoderRegistry::with_json_field

Fluently registers ordinary `DeserializeOwned + Serialize + Clone + Send`
payloads using their direct Serde JSON representation. Specialized,
configured, or validating conversions continue to use `register_for_field`.

##### Reference

    common reader setup -> JsonPayloadDecoderRegistry::with_json_field -> register_for_field -> serde_json::from_str

#### JsonPayloadDecoderRegistry::len / is_empty / has_decoder_for_field / registered_field_names

Expose registry configuration without decoder internals.

##### Reference

    setup inspection, Debug, and tests

#### JsonPayloadDecoderRegistry::require

Crate-private coverage check for every field in a selected stream.

##### Reference

    StoredStateSeriesReader::read_stream_as_state_series before chunk IO -> JsonPayloadDecoderRegistry::require

#### JsonPayloadDecoderRegistry::decode_into

Crate-private lookup, conversion, ownership transfer, and contextual error
wrapping for one field.

##### Reference

    StoredStateSeriesReader canonical field loop -> JsonPayloadDecoderRegistry::decode_into
        -> JsonPayloadDecoder::decode_json_payload -> SystemState::insert_payload

### TypedDecoder, ErasedPayloadDecoder, and DecoderInsertError

Private type-erasure adapter retaining each decoder's concrete output type. An
unexpected occupied destination is restored transactionally.

#### ErasedPayloadDecoder::decode_into

Performs typed conversion and insertion behind the heterogeneous registry.

##### Reference

    JsonPayloadDecoderRegistry::decode_into -> ErasedPayloadDecoder::decode_into

### JsonVecF64Decoder

Zero-sized default decoder for JSON numeric arrays to owned `Vec<f64>`. It adds
no length, finite-value, or domain validation.

#### JsonVecF64Decoder::decode_json_payload

Calls `serde_json::from_str::<Vec<f64>>` directly on the selected raw field.

##### Reference

    JsonPayloadDecoderRegistry entry for key -> JsonVecF64Decoder::decode_json_payload -> Vec<f64>

### JsonStringDecoder

Zero-sized default decoder for JSON strings to owned `String`. It preserves
content and performs no trimming or normalization.

#### JsonStringDecoder::decode_json_payload

Calls `serde_json::from_str::<String>` with standard escape and Unicode handling.

##### Reference

    JsonPayloadDecoderRegistry entry for key -> JsonStringDecoder::decode_json_payload -> String

Only these two defaults are included during main development. Applications may
register closures or named decoder types for tensors and domain values.

## Storage reading

### StoredStateSeriesReader

All-in-one eager reader owning output root, validated completed metadata, and a
caller-configured `JsonPayloadDecoderRegistry` registry. It is intentionally non-Clone.

#### StoredStateSeriesReader::open_completed_recording

Reads and validates `metadata.json`, requires `RecordingStatus::Complete`, and consumes
the registry.

##### Reference

    analysis startup -> StoredStateSeriesReader::open_completed_recording

#### StoredStateSeriesReader::recording_directory

Returns the supplied root without canonicalization.

##### Reference

    diagnostics and tests

#### StoredStateSeriesReader::stream_names

Iterates stream names in metadata order.

##### Reference

    analysis stream discovery

#### StoredStateSeriesReader::format_version

Returns the validated completed storage format version.

##### Reference

    orchestration layer compatibility diagnostics -> StoredStateSeriesReader::format_version

#### StoredStateSeriesReader::user_metadata

Borrows immutable creation-time user metadata.

##### Reference

    task parameter and model identity validation -> StoredStateSeriesReader::user_metadata

#### StoredStateSeriesReader::terminal_metadata

Borrows completion-time user metadata.

##### Reference

    termination reason and completed iteration validation -> StoredStateSeriesReader::terminal_metadata

#### StoredStateSeriesReader::recording_timing

Borrows validated automatic operational timing.

##### Reference

    completed-run timing analysis -> StoredStateSeriesReader::recording_timing

#### StoredStateSeriesReader::stream_record_count

Sums metadata-declared chunk record counts without opening payload files.

##### Reference

    analysis sampling summary -> StoredStateSeriesReader::stream_record_count
    attractor checkpoint count -> StoredStateSeriesReader::stream_record_count

#### StoredStateSeriesReader::stream_encoded_bytes

Sums exact metadata-declared chunk bytes without opening payload files.

##### Reference

    orchestration layer storage summary -> StoredStateSeriesReader::stream_encoded_bytes

#### StoredStateSeriesReader::read_stream_as_state_series

Checks stream existence and decoder coverage, verifies every chunk, decodes all
states transactionally, and returns one complete `StateSeries`.

##### Reference

    analysis request -> StoredStateSeriesReader::read_stream_as_state_series(stream)

#### StoredStateSeriesReader::read_all_streams_as_state_series

Returns ordered `(stream name, StateSeries)` pairs and drops prior results if a
later stream fails.

##### Reference

    whole-run eager analysis -> StoredStateSeriesReader::read_all_streams_as_state_series

#### StoredStateSeriesReader::read_latest_state_from_stream

Opens only the newest chunk, verifies its byte length and checksum, locates its
final newline-terminated record, reconstructs the stream's partial state, and
checks its iteration against the descriptor. Earlier chunks are not opened.

##### Reference

    final-value analysis -> StoredStateSeriesReader::read_latest_state_from_stream
    attractor checkpoint verification -> StoredStateSeriesReader::read_latest_state_from_stream

#### StoredStateSeriesReader::read_chunk

Private buffered JSONL traversal, size/checksum verification, strict ordering,
state assembly, and descriptor-fact validation.

##### Reference

    StoredStateSeriesReader::read_stream_as_state_series -> StoredStateSeriesReader::read_chunk

### Resume-state reader helpers

`decode_resume_state` is the crate-private checkpoint reconstruction boundary
used by the writer facade. It validates that the selected stream exactly matches
the full `SystemStateSchema`, requires decoder coverage, prefers the last complete line
already recovered from the open chunk, and otherwise seeks directly to the
final line of the highest sealed chunk after verifying that selected chunk's
byte count and checksum. It does not open or validate earlier sealed history.

##### Reference

    SystemStateWriter::continue_recording with checkpoint request -> decode_resume_state

`read_last_sealed_record` verifies the selected chunk and then locates its last
JSONL boundary. Earlier chunks remain outside the operation.

##### Reference

    decode_resume_state without complete open record -> read_last_sealed_record

### BorrowedRecord, BorrowedValues, and BorrowedValuesVisitor

Private record representation borrowing each `RawValue` from one line buffer.
Only small field keys are owned. Duplicate payload keys are rejected.

#### BorrowedValues::deserialize

Starts strict borrowed object parsing.

##### Reference

    serde_json::from_slice in StoredStateSeriesReader::read_chunk

#### BorrowedValuesVisitor::expecting / visit_map

Describe and collect unique keys with borrowed raw value boundaries.

##### Reference

    BorrowedValues::deserialize -> BorrowedValuesVisitor

### StreamTemplateRef

Private borrowed adapter used to reconstruct a stream's shared `SystemStateSchema` from
metadata field declarations.

## Errors

### StorageError

Non-exhaustive storage error vocabulary covering configuration, lifecycle,
metadata, chunk integrity, record structure, state access, encoding, decoder
registration/conversion, series invariants, IO, JSON, accounting, ordering,
queue termination, and worker panic. Decoder and lower-level state/series
errors preserve their source chains. No variant owns scientific payload data.

##### Reference

    every storage Result boundary -> StorageError

## Run-level storage facade

`src/storage.rs` is the only intended consumer entry point for persistence.
Its child modules remain private and it re-exports only reader, decoder, and
error types that form part of the supported public workflow. Low-level
encoding, framing, queue, writer, checksum, and raw metadata types remain
implementation details.

### `TimeAxisMetadata`

Public run-level documentation for the iteration coordinate and optional
physical time. It owns only small labels and units; it never stores a time
sample.

#### `TimeAxisMetadata::new`

Creates an iteration-only declaration. Complete semantic validation is deferred to
run startup so fluent configuration remains infallible.

##### Reference

    SystemStateWriterBuilder::default time -> TimeAxisMetadata::default -> TimeAxisMetadata::new
    consumer run configuration -> TimeAxisMetadata::new

#### `TimeAxisMetadata::with_iteration_unit`

Fluently declares the optional iteration unit.

##### Reference

    consumer run configuration -> TimeAxisMetadata::with_iteration_unit

#### `TimeAxisMetadata::with_physical_time_name`

Fluently declares the optional physical-coordinate name.

##### Reference

    consumer run configuration -> TimeAxisMetadata::with_physical_time_name

#### `TimeAxisMetadata::with_physical_time_unit`

Fluently declares the physical unit; startup requires a physical name.

##### Reference

    consumer run configuration -> TimeAxisMetadata::with_physical_time_unit

#### `TimeAxisMetadata::with_physical_axis`

Declares the physical-coordinate name and unit together for the common case.

##### Reference

    consumer run configuration -> TimeAxisMetadata::with_physical_axis

#### `TimeAxisMetadata::default`

Uses `iteration` with no unit or physical coordinate.

##### Reference

    SystemStateWriterBuilder::new -> TimeAxisMetadata::default

#### `TimeAxisMetadata::into_stored`

Moves public configuration into the private versioned metadata representation.

##### Reference

    SystemStateWriter::create_new_recording -> TimeAxisMetadata::into_stored

### `SamplingInterval`

Public coordinate-aware sampling policy. Its Serde representation is externally
tagged so metadata and project configuration retain the coordinate together
with the interval.

#### `SamplingInterval::Iterations`

Selects iteration zero and every iteration divisible by its `NonZeroU64`
interval.

##### Reference

    stream configuration -> SamplingInterval::Iterations -> writer observation scheduling
    JSON {"iterations":N} -> TaskParameters::decode_value<SamplingInterval>

#### `SamplingInterval::iterations`

Convenience constructor from `u64`; returns `None` for zero rather than
constructing an invalid policy.

##### Reference

    hard-coded consumer stream setup -> SamplingInterval::iterations

#### `SamplingInterval::includes`

Private constant-time scheduling predicate used only after a state is offered
to the writer.

##### Reference

    SystemStateWriter::observe_state -> SamplingInterval::includes

### `StateStreamConfig`

Owns one logical stream's exact selected keys, safe relative directory, and
typed `SamplingInterval`. Explicit declarations also own a soft chunk-byte
target and strict queue-byte budget; concise sampled declarations inherit both
from their writer builder. Typed/non-zero values reject invalid intervals and limits
at the public boundary.

#### `StateStreamConfig::new`

Creates a declaration whose directory initially equals its stream name and
stores the sampling interval used by writer-side observation.

##### Reference

    consumer run construction -> StateStreamConfig::new

#### `StateStreamConfig::sampled`

Private constructor used by the concise builder path. It retains no local
storage limits and therefore requires writer-wide limits during preparation.

##### Reference

    SystemStateWriterBuilder::add_sampled_state_stream -> StateStreamConfig::sampled -> PreparedRecording::from_builder

#### `StateStreamConfig::with_relative_directory`

Overrides the relative output directory. Startup rejects unsafe paths and
directory collisions.

##### Reference

    consumer stream path customization -> StateStreamConfig::with_relative_directory

### `SystemStateWriterBuilder`

Owns unopened run configuration and a cheap shared `SystemStateSchema` handle.

#### `SystemStateWriterBuilder::new`

Creates a builder with default time documentation, empty run metadata, and no
streams.

##### Reference

    SystemStateWriter::builder -> SystemStateWriterBuilder::new
    direct consumer construction -> SystemStateWriterBuilder::new

#### `SystemStateWriterBuilder::with_time_axis_metadata`

Replaces temporal-coordinate documentation.

##### Reference

    consumer run configuration -> SystemStateWriterBuilder::with_time_axis_metadata

#### `SystemStateWriterBuilder::with_user_metadata`

Moves arbitrary JSON-compatible run metadata into the builder. It remains
separate from scientific payload records.

##### Reference

    orchestration layer fixed/sweep provenance -> SystemStateWriterBuilder::with_user_metadata
    simulation run annotations -> SystemStateWriterBuilder::with_user_metadata

#### `SystemStateWriterBuilder::with_task_parameters`

Copies one resolved task's small JSON values into recording metadata and adds
its authoritative task ordinal. A same-named input value is replaced by the
actual index.

##### Reference

    TaskParameters for one simulation -> SystemStateWriterBuilder::with_task_parameters -> metadata.json user_metadata

#### `SystemStateWriterBuilder::with_shared_stream_limits`

Sets the chunk rollover target and bounded-queue budget inherited by concise
periodic stream declarations. An explicit `StateStreamConfig` retains its own
limits and takes precedence.

##### Reference

    recording setup -> SystemStateWriterBuilder::with_shared_stream_limits -> sampled stream preparation

#### `SystemStateWriterBuilder::add_state_stream`

Appends one stream in deterministic metadata order. Cross-stream conflicts are
validated together at startup.

##### Reference

    consumer run configuration -> SystemStateWriterBuilder::add_state_stream

#### `SystemStateWriterBuilder::add_sampled_state_stream`

Adds a name, selected keys, and typed sampling interval while inheriting writer-wide
storage limits. Its relative directory initially equals the stream name.

##### Reference

    common recording setup -> SystemStateWriterBuilder::add_sampled_state_stream -> StateStreamConfig::sampled

#### `SystemStateWriterBuilder::create_new_recording`

Delegates complete validation, exclusive filesystem creation, root leasing,
initial atomic metadata publication, and writer startup to `SystemStateWriter`.

##### Reference

    configured builder -> SystemStateWriterBuilder::create_new_recording -> SystemStateWriter::create_new_recording

#### `SystemStateWriterBuilder::continue_existing_recording`

Explicitly reopens a matching running recording, recovers at most one open chunk
per stream, seeds ordinal/iteration continuation, and starts append worker. It
never makes `start` silently reuse existing output.

##### Reference

    externally managed state restoration -> SystemStateWriterBuilder::continue_existing_recording

#### `SystemStateWriterBuilder::continue_recording_from_latest_checkpoint`

Coordinates recovery with typed checkpoint reconstruction and returns
`(SystemStateWriter, SystemState)`. The selected stream must exactly cover the complete
builder `SystemStateSchema`; decoder outputs populate every slot before writers start.

##### Reference

    runtime model restart -> SystemStateWriterBuilder::continue_recording_from_latest_checkpoint

### `RecordingTiming`

Immutable operational timing exposed after successful completion. It never
represents simulation iteration or physical time.

#### `RecordingTiming::created_at_utc`

Returns the original canonical UTC RFC 3339 creation timestamp.

##### Reference

    CompletedRecording::timing -> RecordingTiming::created_at_utc -> run logging

#### `RecordingTiming::finalized_at_utc`

Returns the successful terminal UTC timestamp.

##### Reference

    CompletedRecording::timing -> RecordingTiming::finalized_at_utc -> run logging

#### `RecordingTiming::active_duration_ns`

Returns exact accumulated monotonic active duration in integer nanoseconds.

##### Reference

    orchestration layer metrics and metadata verification -> RecordingTiming::active_duration_ns

#### `RecordingTiming::active_duration`

Returns the same exact value as `std::time::Duration`.

##### Reference

    human-facing duration formatting -> RecordingTiming::active_duration

#### `RecordingTiming::continuation_count`

Returns the number of successful continuation opens.

##### Reference

    resume diagnostics -> RecordingTiming::continuation_count

### `CompletedRecording`

Immutable durable result created only after a successful terminal commit. It
cannot write or resume the recording.

#### `CompletedRecording::directory`

Returns the completed recording root.

##### Reference

    attractor analysis and orchestration layer handoff -> CompletedRecording::directory

#### `CompletedRecording::timing`

Borrows automatic operational timing.

##### Reference

    application terminal log -> CompletedRecording::timing

#### `CompletedRecording::terminal_metadata`

Borrows the exact terminal metadata committed by the caller.

##### Reference

    dependent-model crate outcome and orchestration layer validation -> CompletedRecording::terminal_metadata

#### `CompletedRecording::stream_summaries`

Returns declaration-ordered aggregate stream facts derived from durable chunk
descriptors without reading payload files.

##### Reference

    dependent-model crate writer statistics and orchestration layer completion validation -> CompletedRecording::stream_summaries

#### `CompletedRecording::stream_summary`

Looks up one aggregate by exact logical stream name.

##### Reference

    application terminal reporting -> CompletedRecording::stream_summary

### `CompletedStreamSummary`

Owns one stream name, chunk count, record count, exact encoded bytes, and
optional first/final iterations.

#### `CompletedStreamSummary::name`

Returns the logical stream name.

##### Reference

    summary logging and lookup verification -> CompletedStreamSummary::name

#### `CompletedStreamSummary::chunk_count`

Returns the number of immutable chunk files.

##### Reference

    storage statistics -> CompletedStreamSummary::chunk_count

#### `CompletedStreamSummary::record_count`

Returns the number of persisted states.

##### Reference

    dependent-model crate TaskOutcome replacement -> CompletedStreamSummary::record_count

#### `CompletedStreamSummary::encoded_bytes`

Returns exact total framed bytes.

##### Reference

    storage metrics -> CompletedStreamSummary::encoded_bytes

#### `CompletedStreamSummary::first_iteration` / `last_iteration`

Return stream range endpoints or `None` for a stream with no records.

##### Reference

    orchestration layer progress/completion validation -> CompletedStreamSummary iteration range

### `SystemStateWriter`

Non-clone exclusive owner of one active writer handle and the sole legal
terminal metadata transition. It never owns or retains a `SystemState`.

#### `SystemStateWriter::builder`

Provides the concise public construction entry point.

##### Reference

    consumer simulation setup -> SystemStateWriter::builder

#### `SystemStateWriter::recording_directory`

Borrows the configured output root.

##### Reference

    diagnostics and run logging -> SystemStateWriter::recording_directory

#### `SystemStateWriter::stream_names`

Iterates names in deterministic declaration order.

##### Reference

    diagnostics and run logging -> SystemStateWriter::stream_names

#### `SystemStateWriter::observe_state`

Reads the offered state's iteration and evaluates every stream sampling interval.
Non-due streams perform no payload lookup or allocation. Due streams borrow
selected payloads for encoding, end those borrows, and submit owned records;
submission is the blocking backpressure boundary. Repeated observation of an
already recorded iteration is an idempotent no-op for that stream.

##### Reference

    model evolution loop -> SystemStateWriter::observe_state
    SystemStateWriter::observe_state -> JsonStateRecordEncoder::encode -> StateWriterWorker::submit_record

#### `SystemStateWriter::flush_stream_to_storage`

Looks up one stream and waits on its ordered durability barrier. The method
returns only after all earlier accepted records are sealed and their descriptor
is durable in `metadata.json`.

##### Reference

    resume-critical checkpoint interval -> SystemStateWriter::flush_stream_to_storage

#### `SystemStateWriter::complete_recording`

Consumes the coordinator, drains all streams, and atomically publishes
`Complete` with automatic terminal timing. It returns `CompletedRecording`.
Descriptors were committed incrementally at each seal. On writer failure it
attempts `Failed` metadata without hiding the first writer error.

##### Reference

    successful simulation termination -> SystemStateWriter::complete_recording

#### `SystemStateWriter::complete_recording_with_terminal_metadata`

Completes without an extra final-state offer and atomically commits a terminal
metadata map in its separate namespace.

##### Reference

    terminal facts without final state -> SystemStateWriter::complete_recording_with_terminal_metadata

#### `SystemStateWriter::complete_recording_with_final_state`

Offers one terminal state to every stream, skips streams that already recorded
the same iteration, records non-aligned endpoints once, then delegates normal drain
and completion. Sampling-interval and endpoint policy therefore remain inside the writer.

##### Reference

    successful model termination -> SystemStateWriter::complete_recording_with_final_state

#### `SystemStateWriter::complete_recording_with_final_state_and_terminal_metadata`

Combines exactly-once final-state sampling with terminal user metadata,
automatic finalization timestamp, monotonic active duration, and successful
status in one lifecycle operation.

##### Reference

    applications with terminal facts -> SystemStateWriter::complete_recording_with_final_state_and_terminal_metadata
    dependent-model crate termination -> SystemStateWriter::complete_recording_with_final_state_and_terminal_metadata

#### `SystemStateWriter::mark_recording_failed`

Consumes the coordinator, drains all streams, and publishes an explicit
non-empty failed reason. A concurrent writer failure takes precedence.

##### Reference

    simulation-level terminal error -> SystemStateWriter::mark_recording_failed

#### `SystemStateWriter::mark_recording_failed_with_terminal_metadata`

Publishes an explicit failure reason together with terminal-only facts and
automatic timing.

##### Reference

    intentional early termination diagnostics -> SystemStateWriter::mark_recording_failed_with_terminal_metadata

#### `SystemStateWriter::create_new_recording`

Privately validates all configuration before mutation, creates and leases the
root, creates stream directories, atomically commits `Running`, and then starts
one recording-wide writer worker.

##### Reference

    SystemStateWriterBuilder::create_new_recording -> SystemStateWriter::create_new_recording

#### `SystemStateWriter::continue_recording`

Privately loads running metadata under the exclusive root lease, rejects
terminal or mismatched output, removes only a known interrupted metadata
replacement temporary, recovers stream seeds, optionally reconstructs a full
checkpoint, preserves creation time, increments continuation count, commits
that continuation fact, and starts the append worker.

##### Reference

    SystemStateWriterBuilder::continue_existing_recording -> SystemStateWriter::continue_recording
    SystemStateWriterBuilder::continue_recording_from_latest_checkpoint -> SystemStateWriter::continue_recording

#### `SystemStateWriter::finish_writer`

Privately drains the sole writer. Its stream sinks commit successful
descriptors directly through `RecordingManifest`.

##### Reference

    SystemStateWriter::complete_recording -> SystemStateWriter::finish_writer
    SystemStateWriter::mark_recording_failed -> SystemStateWriter::finish_writer

### `RecordingManifest`

Private mutex-serialized authority over the sole metadata snapshot. Chunk
workers clone and mutate a candidate, atomically persist it, and replace the
in-memory snapshot only after commit succeeds.

#### `RecordingManifest::prepare_chunk`

Appends the next descriptor and durably publishes it before the writer performs
the open-to-sealed rename.

##### Reference

    ActiveChunk::seal -> RecordingManifest::prepare_chunk

#### `RecordingManifest::transition_terminal`

Publishes terminal status, finalization timestamp, accumulated active duration,
and terminal metadata after all writers drain.

##### Reference

    SystemStateWriter terminal methods -> RecordingManifest::transition_terminal

#### `RecordingManifest::snapshot`

Clones the small durable metadata snapshot after completion.

##### Reference

    SystemStateWriter::completed_recording -> RecordingManifest::snapshot

### `RecordingLease`

Private advisory exclusive lock held on the output directory handle. It creates
no lockfile, is released by the operating system after process death, and stays
owned until all writers are dropped.

#### `RecordingLease::acquire`

Opens and non-blockingly locks the existing output root.

##### Reference

    SystemStateWriter::create_new_recording and SystemStateWriter::continue_recording -> RecordingLease::acquire

### Metadata transaction helpers

`ensure_absent` performs the read-only preflight; `create_root` closes its race
with exclusive creation. `commit_metadata` validates and serializes a snapshot,
while `write_and_replace_metadata` exclusively creates a temporary sibling,
syncs it, renames it over the authoritative file, and syncs the root directory.

Unlike a chunk temporary, the metadata temporary is required for the current
lifecycle guarantee. `Running` metadata already exists when `Complete` or
`Failed` is published. Rewriting that file in place can expose truncation,
mixed old/new bytes, or invalid JSON after a crash. Writing and synchronizing a
complete sibling before atomic rename guarantees that observers see either the
previous complete metadata document or the next complete document.

##### Reference

    SystemStateWriter::create_new_recording -> ensure_absent -> create_root
    SystemStateWriter lifecycle transition -> commit_metadata -> write_and_replace_metadata

## Public API and prelude

The crate provides `scientific_workflow::prelude`, allowing consumer code to
import the complete intended end-user API with:

```rust
use scientific_workflow::prelude::*;
```

The prelude is an explicit, curated list of crate-owned public types and
traits. It must not use wildcard re-exports from internal modules and must not
re-export general external traits such as `serde::Serialize`. This keeps
compiler errors, generated documentation, and future API reviews precise.

The state and analysis portion includes:

- `StateFieldSchema`, `SystemStateSchema`, `SystemState`, and `SimulationTime`;
- `PayloadInsertError` and `StateError`;
- `StateSeries`, `StateSeriesView`, `StateSeriesPushError`, and `StateSeriesError`.

The configuration portion includes:

- `ProjectConfig`, `ParameterSpace`, `TaskParameters`, `TaskParametersIter`,
  `TaskConfig`, `TaskConfigIter`, and `MatchingTaskConfigIter`;
- `ProjectPaths` and `ConfigurationError`.

The reporting portion includes:

- `ProgressReporter`, `ProgressReporterBuilder`, `TaskProgress`, and
  `TaskIdentity`;
- `TaskStatus`, `ProgressSummary`, and `ReportingError`.

The storage portion includes:

- `SystemStateWriter`, `SystemStateWriterBuilder`, `StateStreamConfig`, and `TimeAxisMetadata`;
- `StorageError`;
- `StoredStateSeriesReader`, `JsonPayloadDecoderRegistry`, and `JsonPayloadDecoder`;
- `JsonStringDecoder` and `JsonVecF64Decoder`.

Low-level encoding, queue, chunk-format, and metadata implementation types are
not prelude members. `JsonStateRecordEncoder`, `StateWriterWorker`, `EncodedStateRecord`, and raw
metadata structures remain private implementation details behind `SystemStateWriter`.
Storage and configuration integration tests import only the public prelude, so
an omitted or accidentally private supported type is detected by compilation.

##### Reference

    consumer simulation and analysis modules -> use scientific_workflow::prelude::*
    public API integration tests -> use scientific_workflow::prelude::*
    crate root -> pub mod prelude

## Runtime model integration audit

The crate-level work required for runtime model state ownership, checkpoint
recovery, and the one-state/one-writer resource model is complete. Inspection
of runtime model's hot loop, checkpoint path, and multi-system runner originally
identified four integration gates. Gates 1, 2, and 4 are resolved. Gate 3 is
partially resolved: completion now returns timing and terminal metadata, while
a complete public manifest and aggregate stream statistics remain deferred.

Already compatible:

- named `signal` and `space` streams naturally preserve independent sampling
  intervals and output identities;
- PiP's local `SquareLattice` serializer borrows its dense storage, so
  `SystemStateWriter::observe_state` can encode a due lattice without first cloning it;
- exact encoded-byte chunking is a stricter implementation of runtime model's
  desired maximum-file-size policy than its current estimated record sizing;
- bounded stream queues provide deterministic per-stream backpressure;
- generic Serde decoders can reconstruct `Vec<usize>`, PiP tensors, lattices,
  matrices, vector lists, and `PhysObj`; application decoders can reconstruct
  `ActivityStatus`, and scalar payloads without built-in crate support;
- immutable JSONL chunks, checksums, and complete-run reconstruction satisfy
  analysis once a run has terminated successfully.

### Gate 1: live-state mutation boundary

`EcoSystem` evolves `SquareLattice` and `TaxonTable` through simultaneous
mutable borrows on every event. This boundary is now supported by
`SystemState::borrow_payloads_mut`, which returns distinct heterogeneous payload
references after complete validation. Exporting runtime model's old snapshot is no
longer needed and would remain unacceptable because it clones the full lattice
for every space sample.

The integration contract is now fixed: runtime model must own and mutate one
`SystemState` directly, replacing its dedicated runtime state fields and old IO
snapshot struct. Storage will continue sampling that same state by borrow; an
external keyed-field sampling abstraction is not the chosen architecture.

Gate 1 completion criteria are satisfied: payload types are established during
state assembly and retained independently of payload presence; blank derived
states inherit those contracts; immutable and mutable tuple borrowing supports
arities two through eight; duplicate, unknown, missing, and mismatched requests
fail before references are returned; and safe stack-only slot separation adds
no payload copy, ownership transfer, heap allocation, lock, or unsafe code.
Integrated PiP tensor tests cover the complete public boundary.

#### Runtime model mutation dependency

Runtime model does not literally write the lattice and taxon table simultaneously.
For one event, `Decider` reads source and target taxa from the lattice, may read
the current table to sample an effective target, decides whether replacement
occurs, writes the lattice target, and then adjusts two table counts. Those
instructions can be expressed as a read/decide phase followed by separate
lattice and table writes.

They cannot generally be decoupled at sweep or batch granularity without
changing behavior. In randomized-target mode, event `n + 1` samples from the
table produced by event `n`; delaying count updates would sample from stale
abundances. Every accepted replacement must therefore update both
representations before the next event. The table is a derived cache of lattice
population counts, but it is also live model input and must remain consistent.

A strictly single-field state API could preserve semantics by repeatedly
borrowing the lattice and table in separate scopes for every event. That would
introduce several key lookups and dynamic type checks per lattice event and
create a visible two-step commit in application code. It is viable for
correctness, but it is an inferior hot-loop boundary. Simultaneous mutable
borrowing is not required for mathematical expressiveness; it is required to
retain the current efficient sweep shape, in which both payload references are
resolved once and reused across all events.

Accordingly, coordinated tuple borrowing is required to retain runtime model's
efficient sweep shape. The intended use is one borrow before the inner event
loop, not one lookup per event:

```text
borrow_mut::<(SquareLattice, TaxonTable)>(("space", "population"))
    -> (space, population)
    -> for each event: decide(space, population, source, target)
    -> release both borrows
    -> advance SystemState time and evaluate other fields
```

This keeps `SystemState` authoritative, preserves exact randomized-target
semantics, and adds only one pair of key/type validations per sweep.

#### General scientific workload

Coordinated mutation of multiple state components is a common scientific
computing requirement. Representative cases include position/velocity/force
arrays in particle models, density/momentum/energy fields in fluid solvers,
coupled species in reaction systems, primal/dual or parameter/momentum arrays
in optimization, field values and auxiliary caches, and runtime model's lattice
plus population table. Even when an algorithm stages its numerical writes, it
often needs several mutable component references for the duration of one
kernel, integrator step, or sweep.

“At the same time” means simultaneous exclusive Rust borrows, not simultaneous
machine instructions. Individual writes remain ordered. The capability matters
because resolving typed state components once around a hot kernel avoids
repeated name lookup, dynamic type validation, and artificial take/reinsert
ownership cycles.

The typical borrow arity is small and statically known. A state may contain many
fields, while a particular kernel usually couples two or three. The sealed tuple
contract supports arities two through eight without exposing erased wrappers,
query objects, generated numbered methods, or public macros.

Users may still group fields that form one inseparable domain object into one
payload. Grouping is a modeling choice, not a workaround imposed by the borrow
API: independently sampled, encoded, or analyzed fields should remain separate
state keys.

#### Simplified assembly-bound type model

Users should not define or retain a parallel field-selector structure. Payload
definition occurs while the initial state is assembled:

```text
state = spec.create_empty_state(time)
state.insert_payload("space", space_payload)
state.insert_payload("population", population_payload)
state.insert_payload("activity", activity_payload)
```

The first successful insertion into a slot establishes that field's runtime
concrete type for this state layout. Clearing or taking the payload empties the
slot but retains its type definition; subsequent insertion must use the same
type. An empty state derived from an assembled state retains all field type
definitions while omitting payloads. This is distinct from a fresh unassembled
state created directly from the type-free JSON `SystemStateSchema`.

Coordinated access then names and types fields in one expression:

```text
(space, population) =
    state.borrow_payloads_mut::<(SquareLattice<usize>, TaxonTable)>(
        ("space", "population"),
    )
```

The public vocabulary is only `borrow` and `borrow_mut`. A sealed tuple trait
implemented internally for arities two through eight associates each type
tuple with its equally sized name tuple and returned reference tuple. Users do
not name the trait, create query objects, keep typed handles, or invoke macros.
Existing `payload<T>` and `payload_mut<T>` remain the explicit single-field
operations.

Each multi-borrow resolves the names through the specification's existing hash
map and validates retained field types before producing references. Distinct
slot separation uses fixed const-generic stack arrays and safe `split_at_mut`;
it performs no heap allocation, unsafe pointer construction, payload movement,
or cloning. The intended call surrounds a full kernel or sweep, so lookup cost
occurs once rather than per event. A future cached-handle layer is warranted
only if measurement shows repeated borrow setup itself is material.

#### Idiomaticity assessment

The simplified API is idiomatic Rust despite relying on private generated tuple
implementations. Rust has no variadic generics, so sealed traits implemented for
a documented range of tuple arities are the conventional way to express a
heterogeneous operation whose arity is known at compile time. The public call
uses normal method syntax, turbofish type selection, tuple construction and
destructuring, `Result`-based validation, and borrow lifetimes enforced by the
compiler.

It also follows core ownership conventions:

- the state owns every concrete payload;
- assembly moves payloads into the state;
- access returns ordinary `&T` or `&mut T`, not guards or smart wrappers;
- one `&mut SystemState` is the exclusive authority from which all disjoint
  mutable references originate;
- an error is reported before any reference is returned and cannot partially
  mutate or empty the state; and
- all references expire before time advancement, sampling, or another state
  operation can borrow the state again.

`borrow_mut` is acceptable as an inherent method name: it describes granting
temporary references and does not imply interior mutability because it requires
`&mut self`. Rust also has `BorrowMut`, but an inherent method with tuple and
type arguments is unambiguous. `get_many_mut` or `get_disjoint_mut` would more
closely resemble collection APIs, but expose implementation mechanics and are
less symmetrical with immutable `borrow`. The concise `borrow`/`borrow_mut`
pair is retained unless implementation experience reveals confusing compiler
diagnostics.

The deliberately non-standard part is dynamic, assembly-established field
typing, which follows directly from the type-free JSON template. Retaining a
slot's `TypeId` after `take` or `clear` makes that dynamic boundary predictable
and gives derived empty states a stable program-level schema. This invariant
must be documented prominently because users may otherwise expect an empty
dictionary slot to accept a different type.

#### Concrete payload type and erasure boundaries

Every payload remains its original concrete Rust value `T` for its entire time
inside `SystemState`. Insertion moves that `T` into owned storage; typed access
returns `&T` or `&mut T`; extraction returns the same owned `T`. No conversion
to JSON, `serde_json::Value`, bytes, or a common scientific container occurs.
Runtime `TypeId` and the concrete Rust type name remain available for checked
downcasting and diagnostics.

There are two distinct meanings of type erasure:

1. **Heterogeneous storage erasure is unavoidable internally.** A Rust `Vec`
   cannot directly contain unrelated concrete types. Each slot therefore holds
   a private trait-object owner whose concrete allocation is still `T`. This
   erases the static type only from the vector's element type; it does not erase
   runtime type identity, transform data, clone payloads, or expose erasure to
   the user. A type map based on `Any` would make the same tradeoff under a
   different name.
2. **Serialization erasure is demand-driven.** When storage samples a field,
   the private wrapper temporarily borrows its concrete `T` as
   `&dyn erased_serde::Serialize`. That borrowed serialization view exists only
   for encoding. It neither replaces the stored payload nor persists after the
   call. Capturing the serialization function in the private value vtable at
   insertion is necessary because, after heterogeneous storage erasure, plain
   `Any` alone cannot rediscover an arbitrary type's `Serialize`
   implementation.

The type tuple supplied to `borrow_payloads` or `borrow_payloads_mut` states what the program
expects from each named slot. The method compares those expectations with the
concrete `TypeId` definitions retained during state assembly before returning
typed references. The tuple does not own, wrap, convert, or serialize any
payload.

Avoiding even internal storage erasure would require abandoning at least one
core requirement: use a compile-time fixed generic state struct, generate a
typed struct from a typed schema, or restrict payloads to a closed enum. All
three conflict with the runtime JSON key template and open-ended payload types.
The private erased owner plus demand-driven serialization view is therefore the
appropriate implementation for this general state.

### Gate 2: interrupted-run resume and append (implemented)

Runtime model can continue an incomplete task in its existing directory, load only
the newest full-state checkpoint, and append later chunks.
`create_new_recording` remains strictly new-directory-only; explicit
`continue_existing_recording` and
`continue_recording_from_latest_checkpoint` validate recording identity and
recover append position. Descriptors are committed incrementally, while
ordinary `StoredStateSeriesReader` analysis remains completed-recording-only.

#### Filename-based seal contract

No per-chunk status, journal, checksum sidecar, or recovery marker is added.
Each chunk payload has exactly two possible names:

    chunk-000012.jsonl.tmp    open and recoverable
    chunk-000012.jsonl        sealed and immutable

These are two names for the same payload inode at different lifecycle stages,
not two payload copies. A final `.jsonl` name is the authoritative seal marker.
Append-position recovery does not open, hash, parse, or decode older sealed
chunks merely to find the next ordinal. It may inspect directory names to
enforce consecutive ordinals. Those chunks are unexamined, not newly validated;
any later operation that consumes them must verify their checksums.

To make that rule compatible with the single `metadata.json` inventory, sealing
uses a prepare-then-rename transaction:

1. finish the open chunk, synchronize its bytes, and build its descriptor;
2. atomically commit that descriptor to `metadata.json` while the recording remains
   `running`;
3. rename `.jsonl.tmp` to `.jsonl`; and
4. synchronize the stream directory.

This ordering prevents a crash from producing a sealed chunk whose descriptor
is absent from metadata. A crash between steps 2 and 3 instead leaves the one
open chunk plus its prepared descriptor, which is unambiguous and recoverable.
Readers continue to reject ordinary analysis of a running recording, so the brief
prepared state is never presented as completed output.

At resume, each stream can contain at most one open chunk: its highest ordinal.
The recovery pass opens and examines only that chunk. If metadata already has
its prepared descriptor, recovery verifies the open bytes against that
descriptor and completes the rename. Otherwise it parses complete JSONL
records, truncates an incomplete trailing record if present, rebuilds the
incremental checksum/counters, and continues appending to the same chunk (or
seals it immediately when it already meets the byte target). Older sealed
chunks remain unopened during this positional recovery.

`SystemStateWriterBuilder::continue_existing_recording` is explicit rather
than making `create_new_recording` silently reuse a directory. It accepts only
`running` metadata, compares the caller's expected recording/schema/stream
configuration, acquires exclusive writer ownership, performs the
one-open-chunk recovery above, and seeds every stream sink in the sole worker.
Complete and failed recordings remain terminal.

#### Resume-state reconstruction

The writer facade provides the convenience path needed by simulations:

    let (writer, state) = builder.continue_recording_from_latest_checkpoint("space", decoders)?;

`continue_recording_from_latest_checkpoint` is deliberately not a `SystemState` constructor.
Directory layout, JSONL recovery, stream selection, and decoder orchestrating belong to
storage; keeping them out of `system_state` preserves the core state's format-
and IO-independence. The operation performs one coordinated transaction:

1. acquire exclusive ownership of the existing running recording;
2. validate expected recording, schema, and stream declarations;
3. recover each stream's open chunk;
4. select and decode the newest complete checkpoint record from the requested
   stream;
5. seed and start the append worker; and
6. return both the live writer and reconstructed owned state.

For the selected checkpoint stream, recovery first examines its highest open
chunk. A final non-newline-terminated fragment is ignored and truncated as a
crash remnant. Every earlier newline-terminated record must be structurally
valid; recovery never skips corruption in the middle and calls a later record
"valid." If the open chunk contains no complete record, lookup falls back to
the last record of the highest sealed chunk. Because that payload becomes the
resumed scientific state, the selected sealed chunk must first pass exact
byte-count and checksum verification. Earlier sealed chunks remain unopened.

The returned `SystemState` is complete, not a partially populated full-state
shell. Therefore the selected checkpoint stream must declare every field in
the builder's full `SystemStateSchema`, and the supplied registry must provide one
decoder for every field. A partial diagnostic stream such as `signal` is valid
for analysis but is rejected as a resume source. Successful reconstruction
creates an owned state at the record's stored `SimulationTime`, with every slot
populated by its concrete decoder output.

The lower-level `continue_existing_recording()` remains useful when an application restores
its state elsewhere. `continue_recording_from_latest_checkpoint` is its checkpoint-aware
convenience counterpart, not a second recovery implementation.

#### Resume API naming

The explicit public vocabulary is:

    builder.continue_existing_recording()?;
    builder.continue_recording_from_latest_checkpoint("space", decoders)?;

`continue_existing_recording` means storage-only continuation: validate and recover an
existing running recording, then return `SystemStateWriter` ready for later records.
`continue_recording_from_latest_checkpoint` means workflow continuation: perform the same storage
recovery, reconstruct a full checkpoint `SystemState`, and return
`(SystemStateWriter, SystemState)`. Both names state their result rather than exposing
an internal file-open mode.

The writer provides a per-stream durability barrier. `flush_stream_to_storage(stream)` blocks
until all records accepted before the call are written, prepares and seals a
non-empty open chunk even below its byte target, and commits its descriptor.
Runtime model can call this after a resume-critical space checkpoint; ordinary
signal samples can retain automatic byte-target chunking.

### Gate 3: public manifest and aggregate stream summary (partially deferred)

Runtime model and orchestration layer inspect configuration, end time, activity, sample
counts, and writer statistics without decoding payload chunks. Current run
metadata now accepts structurally separate startup and terminal annotations,
and completion returns `CompletedRecording` with directory and timing. The
remaining public boundary needs:

- a read-only recording manifest/status view, including per-stream aggregate records
  and bytes;
- access to user recording metadata from the reader;
- aggregate stream facts on `CompletedRecording` or an equivalent cheap
  inspection API after finish.

This allows orchestration layer validation to depend on the scientific-workflow format
instead of runtime model-private metadata.

Gate 2 is a sufficient foundation for this work: chunk descriptors now enter
the shared manifest incrementally, lifecycle transitions are serialized, and
resume consumes the same authoritative metadata. Gate 3 can therefore expose
owned read-only snapshots and derived aggregates without changing chunk files,
payload encoding, recovery rules, or writer ownership.

#### Proposed public responsibility

Use one public owned `RecordingSummary`. It is a read-only snapshot of the sole `metadata.json` at a
particular lifecycle point. It contains no payload bytes, decoder registry,
file handles, writer handles, or mutable access to internal metadata.

The snapshot exposes:

- format version and `Running` / `Complete` / `Failed` status;
- time-axis documentation;
- immutable run metadata supplied at startup;
- terminal result metadata supplied when finishing or failing;
- stream names, field declarations, sampling intervals, and configured byte limits; and
- derived per-stream chunk count, record count, encoded bytes, and first/last
  recorded index.

Aggregate facts are derived from already committed chunk descriptors. Creating
or inspecting a manifest never opens, hashes, parses, or decodes a chunk.

The intended entry points are:

    writer.recording_summary()?                  // owned running snapshot
    writer.complete_recording()?                    // complete RecordingSummary
    writer.complete_recording_with(result)?         // complete RecordingSummary plus result data
    writer.mark_recording_failed(message)?               // failed RecordingSummary
    writer.mark_recording_failed_with(message, result)?  // failed RecordingSummary plus result data
    RecordingSummary::open(path)?            // standalone metadata inspection
    reader.recording_summary()                   // borrow completed reader snapshot

`run` metadata and terminal `result` metadata remain separate namespaces.
Startup provenance must not be overwritten by terminal measurements. Empty
result metadata is omitted from JSON. `SystemStateWriter::complete_recording` and `SystemStateWriter::mark_recording_failed` are convenience
forms of their `_with` counterparts using an empty result map.

This API lets a orchestration layer determine whether a task completed, validate run
identity, list its output streams, report sample/byte counts, and inspect final
measurements without knowing payload types. A runtime model can inspect current
committed progress while running and receives the authoritative terminal
manifest directly from its consuming lifecycle call.

The manifest does not reconstruct states, schedule sampling, control chunking,
or expose live queue internals. Checkpoint reconstruction remains
`continue_recording_from_latest_checkpoint`; payload analysis remains `StoredStateSeriesReader`.

### Gate 4: aggregate resource control and failure lifecycle

Backpressure is per stream, while each recording owns exactly one bounded FIFO
and worker thread. A full encoded record is allocated before admission and must
individually fit its configured stream budget. Independent simulations remain
independent resource and failure domains.

Runtime model error propagation currently uses early `?` returns. Dropping
`SystemStateWriter` drains its writer but cannot infer a simulation failure,
leaving a running manifest. Integration must either explicitly call `SystemStateWriter::mark_recording_failed` on
all terminal paths or introduce a run guard/coordinator whose failure policy is
compatible with recovery.

#### Failure-lifecycle decision

An unexpected early return, panic, process loss, or transient IO error should
leave `Running`, not automatically publish `Failed`. `Running` now means
interrupted but recoverable: `SystemStateWriter` drop drains its in-process
queue and seals what it can, while `continue_existing_recording` or
`continue_recording_from_latest_checkpoint` can restart the same recording. An
RAII guard that converts every drop into `Failed` would destroy
this recovery path and is therefore the wrong default.

`Failed` is reserved for an explicit scientifically terminal decision for which
the caller does not intend continuation, such as rejected model conditions or
an intentional abort. Those paths call `SystemStateWriter::mark_recording_failed`. Successful paths call
`SystemStateWriter::complete_recording`. Ordinary `?` propagation needs no special guard and leaves a
recoverable recording. This resolves the lifecycle half of Gate 4 without another
type or automatic policy.

#### Aggregate-resource decision

Each simulation directly owns and evolves one `SystemState` and owns one
corresponding queued state-output writer. That writer coordinates every named
partial-state stream belonging to the simulation. Writers are not shared
between simulations and there is no process-global writer manager, registry,
global queue, worker pool, or aggregate backpressure policy.

    simulation thread
        -> owned evolving SystemState
        -> owned SystemStateWriter
        -> writer-owned bounded queue
        -> writer-owned stream/chunk state

The one-state/one-writer boundary keeps failures, queue pressure, output paths,
and lifecycle transitions local to their scientific recording. Per-stream byte
limits bound only that recording. Applications running many simulations
control aggregate memory and writer count through `WorkflowRuntime`
task/resource limits. Storage does not infer relationships among independent
runs, and the runtime does not merge their writers or queues.

This remains the final storage architecture: no central-writer manager or
process-global storage queue is planned. The separate `WorkflowRuntime`
coordinates execution resources outside storage without changing writer
ownership or the on-disk format.

#### Naming refactor completed

The approved vocabulary is implemented throughout production code, tests, the
prelude, examples, diagnostics, and internal modules. Public concepts are
`SystemStateSchema`, `SystemState`, `StateSeries`, `SystemStateWriter`,
`StateStreamConfig`, `JsonPayloadDecoderRegistry`, and
`StoredStateSeriesReader`. “Recording” means the complete on-disk directory and
its lifecycle; “stream” means one named sampling interval and field selection within that
recording. The former ambiguous coordinator name and method vocabulary no
longer exist.

Method names state their effects explicitly: state methods refer to payloads,
writer methods distinguish recording, queue admission, and durable stream
flushes, and reader methods say whether they open a completed recording or
reconstruct a state series. The refactor intentionally provides no deprecated
aliases or legacy compatibility layer.

The sole `StateWriterWorker` owns one recording-wide FIFO queue. Private
`StateStreamSink` values retain independent chunk rollover and recovered append
positions. Per-stream byte budgets and the recording-wide hard-coded record
limit apply backpressure before queue admission.

### Runtime model migration

Migration can now begin. It consists of adding local scientific-workflow and
PiP dependencies, declaring runtime model keys and streams, using checked
`usize`/`u64` time conversion, registering application decoders, replacing
sampling-interval writes, and updating orchestration layer completion validation. Those are
consumer adaptation tasks rather than missing state or storage features.

## Verification gate

### Audit of `suggestions.md`

The August 2026 audit suggestions were compared with the implemented API,
public documentation, and consolidated integration workflows. The accepted
targeted changes below are now implemented; the rejected broad test and
benchmark proposals remain intentionally out of scope.

- The named state error families, the successful
  `SystemStateSchema::field_schema` path, tuple borrow arities, and
  valid/bounds/type-error forms of `StateSeries::payload_mut_at` are already
  asserted directly by `state_workflow` and `analysis_workflow`. The
  `field_schema` unknown-key `None` branch is not asserted separately, but is a
  trivial accessor outcome rather than a missing behavioral workflow.
- The supported tuple range of two through eight is already stated on both
  public borrow methods and in the crate README. The private macro centralizes
  identical implementations, while `disjoint_slots_mut` uses fixed stack
  arrays, sort-by-index, and safe progressive slice splitting. No simplification
  is justified without a concrete readability or performance improvement.
- The custom decoder extension contract is already public: applications may
  register a named `JsonPayloadDecoder<T>` or any thread-safe closure of the
  documented form. Ordinary PiP payloads now use the generic Serde path, while
  resilience coverage retains the custom decoder path for domain conversions.
  A shorter README example may improve discoverability, but it is not an API
  gap.
- Testing every error variant in isolation would conflict with the adopted
  behavior-oriented integration-test policy. New cases should be added only
  for distinct externally meaningful invariants. Combined malformed-input
  cases that merely exercise validation order should not become contractual
  tests.
- `resume_workflow` now contains several sealed chunks followed by one open
  chunk. It deliberately corrupts an older sealed payload, then proves recovery
  still reconstructs the highest open tail and appends at the correct next
  ordinal. The same workflow now rejects terminal continuation, incompatible
  continuation configuration, and checkpoint reconstruction without any
  complete record.
- Chunk/metadata benchmarks should follow profiling and measure an identified
  operation separately from filesystem latency. The existing encoder benchmark
  proposal remains better scoped; a generic micro-test for metadata persistence
  would not provide a stable performance contract.
- Responsibility separation remains explicit in the crate-level docs and
  README: `SystemState` is live mutable state, `StateSeries` is in-memory
  analysis, and storage owns persistence. The published crate README now also
  demonstrates both named and closure-based custom payload decoders and states
  the reader/decoder responsibility boundary directly.

The suggestion to add unit tests inside source modules is rejected: permanent
tests remain dedicated behavior-oriented integration targets under `tests/`,
in accordance with the project test architecture.

Before beginning the run-level facade:

1. `cargo fmt --all -- --check` passes.
2. `cargo test --all-targets --no-fail-fast --locked` passes.
3. `cargo clippy --all-targets --all-features --locked -- -D warnings` passes.
4. `cargo package --allow-dirty --no-verify` succeeds when dependency registry
   availability permits it.
5. `git diff --check` passes.

The unified storage target must prove both default decoder round trips and
real PiP tensor/`PhysObj` workflows using generic per-key Serde decoders. It prints
bounded logs under `cargo test --test storage_workflow -- --nocapture` and removes all
temporary output afterward.

## Integration-test architecture

The detailed and authoritative test architecture is maintained in `tests.md`.
This section records only its relationship to the crate architecture; when test
scope or file allocation changes, update `tests.md` first and keep this summary
consistent.

The former focused file-mirroring suites were useful during production-file
review and have now been replaced by six behavior-oriented Cargo integration
targets plus their real JSON fixtures:

    tests/
    ├── fixtures/
    │   ├── configuration/
    │   │   ├── cartesian_project/config/{fixed,sweep,paths}.json
    │   │   └── cases_project/config/{fixed,sweep,paths}.json
    │   ├── state.json
    │   └── coupled_state.json
    ├── configuration_workflow.rs
    ├── state_workflow.rs
    ├── analysis_workflow.rs
    ├── storage_workflow.rs
    ├── storage_resilience.rs
    └── resume_workflow.rs

Every target prints a short stable report under `--nocapture`. Logs contain
counts, indices, byte sizes, chunk facts, pointer/clone evidence, and expected
error classes; they never dump full scientific payloads or nondeterministic
thread timing.

### state_workflow.rs

One realistic simulation-state lifecycle using checked-in templates and PiP
tensors. It covers template semantic round trip, shared layouts, assembly-bound
type retention, heterogeneous immutable/mutable tuple access for arities two
through eight, duplicate/preflight errors, time advancement, zero-copy
extraction with allocation identity, explicit deep-clone accounting,
rejected-set payload recovery, and bounded diagnostics.

Key log output:

    [template] fields=3 round_trip=true
    [state] iteration=... physical_time=... loaded=... mutation=verified
    [ownership] set_take_pointer_preserved=true clone_calls=...
    [tuple] immutable=true mutable=true duplicate_rejected=true unknown_rejected=true preflight_atomic=true
    [type-contract] take_retained=true clear_retained=true empty_inherited=true
    [tuple-arities] min=2 max=8 reverse_order_mutation=true
    [result] state_workflow=passed

### analysis_workflow.rs

Builds an ordered `StateSeries` from evolving states, verifies move-based push
and pop, shared-layout and increasing-time rejection with ownership recovery,
borrowed `StateSeriesView` traversal, narrow field mutation, capacity reuse, and the
explicit cost boundary of deep cloning.

Key log output:

    [series] states=... indices=[...]
    [invariants] layout_rejection=true ordering_rejection=true
    [ownership] push_pop_preserved=true clone_calls=...
    [result] analysis_workflow=passed

### storage_workflow.rs

The principal success-path test. It evolves one live state, samples multiple
streams at different sampling intervals, uses borrowed encoding and bounded writers,
commits one metadata file, verifies automatic byte chunking, then reconstructs
complete series. It exercises the generic JSON path for strings, vectors, PiP
tensors, and a heterogeneous PiP `PhysObj`. It explicitly asserts semantic
JSON metadata round trip and typed payload equality.

Key log output:

    [sample] iteration=... physical_time=... signal=true space=...
    [writer] signal_records=... signal_bytes=... space_records=... space_bytes=...
    [chunk] stream=... file=... records=... bytes=... checksum_verified=true
    [readback] signal_states=... space_states=... typed_round_trip=true clone_calls=0
    [result] storage_workflow=passed

### storage_resilience.rs

One failure-oriented target retaining only cross-boundary risks: strict queue
byte rejection, non-increasing writer indices, existing-output refusal,
incomplete metadata, missing decoder coverage, wrong payload type with source
context, missing/size-changed/checksum-corrupt chunks, and worker termination.
Each case asserts the exact `StorageError` class and the most important owned
context without exhaustively snapshotting every display string.

Key log output:

    [expected-error] case=... variant=... context_verified=true
    [integrity] missing=true size=true checksum=true
    [backpressure] oversized_rejected=true ordering_rejected=true
    [result] storage_resilience=passed

### resume_workflow.rs

Reproduces prepared and unprepared crash windows, trusts sealed history,
recovers only the highest open tail, reconstructs a complete typed checkpoint,
continues append ordering, and rejects terminal, mismatched, or empty
continuation attempts.

Key log output:

    [resume-state] iteration=... physical_time=... fields=... complete=true
    [recovery] incomplete_tail_truncated=true continued_open_chunk=true records=... durable_barrier=true
    [multi-chunk] sealed_history_trusted=true open_tail_scanned=true resumed_index=... next_ordinal=...
    [resume-rejections] terminal=true configuration_mismatch=true no_checkpoint=true

### configuration_workflow.rs

Loads real Cartesian and explicit-case project fixtures through the public
prelude, proves deterministic expansion and shared JSON ownership, exercises
the complete dict/path API, exports and reloads byte-identical source files,
and rejects meaningful ambiguity and validation failures.

Key log output:

    [load] fixed=... swept=... parameters=... tasks=... paths=...
    [cartesian] tasks=... last_axis_fastest=true first=(...) last=(...)
    [ownership] fixed_shared=true selected_shared=true task_clone_shared=true merged_map_allocated=false
    [round-trip] fixed_bytes=true sweep_bytes=true paths_bytes=true reload=true overwrite_rejected=true
    [cases] tasks=... correlated=true key_order_normalized=true
    [validation] fixed_only=true nested_duplicate=true overlap=true inconsistent_cases=true invalid_path=true
    [result] configuration_workflow=passed

Trivial getter, formatting, constructor, and one-variant tests are removed when
the same behavior is naturally exercised by these workflows. High-risk
properties remain explicit assertions rather than being considered covered
merely because a method was called. The six targets run independently and
clean up their own precisely owned temporary directories.

### Consolidated coverage rule

The six-file design must cover the complete implemented API surface, but it
does not recreate one test per method:

- every public structure is constructed or obtained in at least one workflow;
- every public method is invoked in its natural scenario;
- ownership, mutation, ordering, serialization, backpressure, integrity, and
  reconstruction methods receive explicit semantic assertions;
- trivial accessors may be checked together in one workflow section;
- `Debug`, `Display`, iterator, and `Error::source` implementations are invoked
  only where their bounded output or source preservation is part of a useful
  diagnostic;
- crate-private and private helpers are not tested directly merely to increase
  coverage. They are covered through public boundary outcomes, such
  as checksum verification proving `ActiveChunk::append/seal` and reader
  corruption tests proving borrowed-record validation;
- not every `StorageError` variant needs an isolated constructor test. Every
  failure family and every externally reachable high-risk branch must be
  represented.

Required method allocation:

| Workflow | Structures and API families exercised |
|---|---|
| `state_workflow` | `StateFieldSchema`, `SystemStateSchema`, `SimulationTime`, `SystemState`, doc-hidden `PayloadTuple`, `StateError`, `PayloadInsertError`; all public spec, time, single/tuple state access, ownership, retained-type, clear, clone, and inspection methods |
| `analysis_workflow` | `StateSeries`, `StateSeriesView`, `StateSeriesPushError`, `StateSeriesError`; all public construction, capacity, lookup, iteration, mutation, append/rejection, extraction, clear, and clone methods |
| `storage_workflow` | `TimeAxisMetadata`, `StateStreamConfig`, `SystemStateWriterBuilder`, `SystemStateWriter`, `JsonPayloadDecoder`, `JsonPayloadDecoderRegistry`, both default decoders, and `StoredStateSeriesReader`; every public success-path method including `read_all`, with private encoding/writing/format behavior verified through files and readback |
| `storage_resilience` | `StorageError` source/context behavior and reachable configuration, lifecycle, queue, decoder, record, metadata, filesystem, and integrity failure families |
| `resume_workflow` | explicit `continue_existing_recording`/`continue_recording_from_latest_checkpoint`, full-state schema enforcement, typed checkpoint reconstruction, prepared and unprepared crash windows, multi-sealed-plus-open recovery without sealed-content inspection, continuation rejection boundaries, append seeding, `flush`, and exclusive root leasing |
| `configuration_workflow` | `ProjectConfig`, `ParameterSpace`, `TaskParameters`, `TaskParametersIter`, `TaskConfig`, `TaskConfigIter`, `MatchingTaskConfigIter`, `ProjectPaths`, and `ConfigurationError`; all public loading, inspection, complete task generation, exact filtering, unique selection, lookup, iteration, decoding, path, exact-export, ownership, and diagnostic methods plus meaningful parser/validation families |
| `reporting_workflow` | `Phase`, `Task`, structured selectors, configuration task generation, `ProgressReporterBuilder`, `PhaseProgressReporterBuilder`, `ProgressReporter`, `TaskProgress`, `ActivityTask`, `TaskIdentity`, `TaskStatus`, `ProgressSummary`, and `ReportingError`; identity validation, generated labels, phase headings, automatic ordering, parallel atomic updates, output modes, exclusive session ownership, lifecycle finalization, and failure-on-drop |

The finished source reads as seven coherent workflows rather than an API census.
The old aggregators and focused subdirectories have been removed.

Current test architecture: seven logged integration files plus production
doctests. Each workflow passes independently and the consolidated all-target
suite passes. Formatting and Clippy across all targets pass with warnings
denied. Archive preparation also succeeds. The manifest declares the published
development dependency `physics_in_parallel = "3.0.4"`, resolved directly from
crates.io without a local path override.

## Example architecture

Full-stack examples are repository-level consumer applications, not Cargo
example targets embedded in the publishable library crate. This keeps the
library package focused while allowing each scientific project to own its
manifest, source tree, input assets, documentation, and output policy. The
first proposed application is a complete two-dimensional Hopf-normal-form
workflow:

```text
examples/
└── attractor_2d/
    ├── Cargo.toml
    ├── Cargo.lock
    ├── README.md
    ├── design.md
    ├── config/
    │   ├── fixed.json
    │   ├── sweep.json
    │   ├── paths.json
    │   └── state.json
    └── src/
        ├── main.rs
        ├── project_setup.rs
        ├── hopf_model.rs
        ├── state_recording.rs
        └── recording_validation.rs
```

From the repository root, Cargo runs it with:

```text
cargo run --manifest-path examples/attractor_2d/Cargo.toml
```

The standalone manifest depends on the local library through
`scientific-workflow = { version = "0.1.0", path = "../../rust" }`. The version
constraint documents compatibility while the path keeps repository development
joint and offline. A root Cargo workspace is not required for the first
example; adding one should be a separate repository-wide decision if multiple
standalone projects later need unified commands.

Because the example is an executable application, its generated `Cargo.lock`
is tracked to make the demonstrated dependency resolution reproducible.

The evolving point follows the supercritical Hopf normal form
`dx/dt = mu*x - omega*y - (x^2+y^2)*x` and
`dy/dt = omega*x + mu*y - (x^2+y^2)*y`. `fixed.json` defines the model name,
initial point, angular frequency, explicit-Euler timestep, iteration count,
sampling intervals, and storage budgets. `sweep.json` varies `mu` across the Hopf
bifurcation so every resolved task is one independent recording. `paths.json`
names the state template and output root. The schema declares the evolving
`point` payload (`Vec<f64>`) and retained scalar `radius` diagnostic. The immutable model settings remain in fixed
configuration and are recorded once in task metadata.

The example demonstrates one coherent full-stack happy path:

1. load the project configuration, resolve paths, inspect declared parameters,
   and enumerate deterministic parameter-sweep tasks;
2. load one shared `SystemStateSchema`, create one owned mutable state per task,
   insert the initial point and radius payloads, and evolve them directly in
   that state;
3. configure a bounded asynchronous `SystemStateWriter` with time-axis and task
   metadata plus independently sampled `trajectory`, `radius`, and `checkpoint`
   streams;
4. borrow and encode the current state at each stream's sampling interval, allowing the
   writer to apply byte-bounded backpressure and whole-record chunking;
5. finish each recording, register direct Serde JSON decoding for `Vec<f64>`
   and `f64`, and read only the complete checkpoint stream's latest state with
   `StoredStateSeriesReader`;
6. verify that the reconstructed checkpoint matches the live final state
   exactly;
   and
7. leave the generated recording directory under the example's ignored
   `target/recordings` area for inspection beneath a timestamp-and-process-based
   execution directory.

The example covers the primary evolution workflow across configuration, system
state, recording storage, and decoding. It should
not mechanically call every accessor or manufacture every error variant;
dedicated integration tests remain responsible for exhaustive API and failure
coverage. Small modules keep simulation, writer construction, and validation
legible while all crate interaction goes through the public prelude.

Normal output is deliberately one validation line:

```text
[workflow] status=completed tasks=3 completed=3 failed=0 pending=0 message=round_trip=true output=...
```

The source project is self-contained and never depends on test fixtures. It is
tracked in the Git repository but deliberately excluded from the library's
crates.io archive. The example writes only beneath its ignored `target`
directory, so generated recordings are neither source assets nor publishable
artifacts. CI and release checks must invoke the standalone manifest explicitly.
This repository-level placement is approved. The configuration and state
template, manifest, modular executable, and typed verification now
live under `examples/attractor_2d`.

The package boundary has been verified with
`cargo package --allow-dirty --no-verify --list` from `rust/`. The resulting
44-file manifest contains the library sources, crate documentation, license,
and integration-test fixtures only. It contains neither the repository-level
`examples/attractor_2d` project nor any generated `target/recordings` data.
Both the repository README and packaged crate README provide the standalone
manifest command and state the full workflow coverage boundary.

### Standard scientific-project procedure

The attractor example establishes a reusable development sequence for projects
built on `scientific-workflow`:

1. **Define the scientific contract.** State the evolution rule, distinguish
   evolving state from fixed and swept parameters, define one independent task,
   and identify the observations needed for analysis or restart.
2. **Define configuration.** Put shared constants in `fixed.json`, parameter
   axes or correlated cases in `sweep.json`, and named filesystem locations in
   `paths.json`. Validate task expansion before running the model.
3. **Define state shape.** Declare stable field names and descriptions in the
   state template. Choose the concrete Rust payload type for each field in the
   state-assembly code; the JSON template does not pretend to encode Rust types.
4. **Implement state assembly.** Convert one complete `TaskConfig` handle into one
   complete, directly owned `SystemState`. Reject invalid parameter types and
   dimensions before starting a recording.
5. **Implement the evolution kernel.** Evolve only the owned state and explicit
   task parameters. First establish deterministic single-task behavior without
   storage or sweep concurrency.
6. **Design sampling streams.** Group fields by scientific purpose and sampling interval,
   such as frequent observations and complete checkpoints. A sampled state is
   never split, even when it exceeds a chunk-size target.
7. **Configure recording before evolution.** Create one writer per independent
   task with schema, time-axis documentation, resolved task metadata, stream
   definitions, chunk targets, and a finite queue-byte budget. Initial metadata
   therefore exists before samples are admitted.
8. **Offer state after evolution.** The model passes one borrow of its current
   state to the writer after every step. The writer evaluates stream sampling intervals;
   encoding is synchronous only for due streams, queued writing is
   asynchronous, and bounded backpressure is accepted as part of execution.
9. **Close the lifecycle explicitly.** Complete a successful recording or mark
   it failed with context. Do not treat dropping a writer as successful
   completion.
10. **Define readback by payload field.** Register a decoder for every field
    selected by the stream, open the completed recording, and reconstruct typed
    `StateSeries` values.
11. **Implement analysis against series views.** Keep analysis separate from
    evolution and operate on `StateSeriesView` when ownership is unnecessary.
    Verify at least one known live value against reconstructed output.
12. **Scale from one task to the sweep.** Only after one task passes its full
    round trip should the program enumerate all tasks. Each concurrent task
    continues to own its state, writer, and output directory.
13. **Add restart as a distinct path.** A resumable project uses a complete
    checkpoint stream and matching decoders, reconstructs the latest valid
    state, continues the existing recording, and then re-enters the same
    evolution loop.

The normal runtime order is consequently:

```text
load project config -> load state schema -> resolve task
-> assemble owned state -> create task writer
-> evolve / sample / advance time -> complete recording
-> register decoders -> reconstruct series -> analyze and verify
```

This ordering keeps scientific computation independent from persistence while
making configuration, state ownership, recording lifecycle, and readback
explicit at their boundaries.

### Evolution-phase capability audit

The existing crate has every primitive required for the attractor's evolution
and recording-submission phase:

- `ProjectConfig` resolves paths and deterministic task dictionaries;
- `TaskParameters::decode_value` produces owned, typed model settings;
- `SystemStateSchema` creates one state that shares immutable field metadata;
- `SystemState::insert_payload` moves the initial allocation into the state;
- `payload_mut` permits direct in-place evolution without payload cloning;
- `advance_simulation_time` transactionally increments iteration and advances physical time;
- `SystemStateWriterBuilder` validates independent observation and checkpoint
  streams before execution;
- `observe_state` checks writer-owned sampling intervals before borrowing live payloads,
  then applies backpressure only to owned encoded bytes; and
- explicit complete and failed lifecycle methods cover both simulation exits.

Cross-parameter validation and collision-free task path
construction remain application responsibilities by design. Resolved task
parameters can be collected into writer user metadata by cloning only their
small JSON configuration values; a dedicated conversion API would be an
ergonomic convenience, not a capability requirement.

The state boundary is finalized: `model` is immutable fixed configuration and
belongs in task metadata, not in the evolving state or every checkpoint
record. The attractor schema contains the evolving `point` plus the retained
scientific diagnostic `radius`; a complete checkpoint records both.

The minimal example covers state evolution, sampled record submission, explicit
recording completion, decoder registration, latest-record reconstruction, and
exact final-state verification. Visualization and numerical analysis remain
separate consumer concerns.

The example's post-run ownership boundary has one authority: its domain-named
`HopfModel` remains the only application structure that owns the live
`SystemState`. Recording may return storage facts, but it must not manufacture
a `FinalState` or another scalar mirror of scientific payloads. Logging and
round-trip verification borrow the completed simulation state directly. This
prevents duplicated scientific truth even when copying the individual scalars
would be computationally cheap.

The example is intentionally a minimal happy path. Checked-in inputs are
decoded directly, crate errors propagate with `?`, and round-trip facts use
plain assertions. Redundant schema/domain validation, expected-count
recalculation, collision retry machinery, and layered failure-error rewriting
are omitted so the core workflow remains visible. The library's integration
suite, rather than this tutorial application, carries exhaustive validation
and failure coverage.

Example source modules use descriptive snake-case nouns consistently:
`project_setup.rs`, `hopf_model.rs`, `state_recording.rs`, and
`recording_validation.rs`. These names identify either the primary domain object
or the module's exact application responsibility and avoid ambiguous generic
labels such as `project`, `simulation`, or `storage`. `main.rs` remains the
conventional orchestrator entry point.

The library baseline was cleaned and verified immediately before example
implementation: formatting passes, all 11 integration tests pass, Clippy passes
across every target and feature with warnings denied, all 6 doctests pass with
rustdoc warnings denied, documentation builds without warnings, and isolated
`cargo package` verification succeeds. The archive contains 44 files and
excludes the repository-level example and generated output.

### Finalized two-dimensional ODE

The example uses the supercritical Hopf normal form. In polar coordinates its
dynamics are `dr/dt = mu*r - r^3` and `dtheta/dt = omega`, so `mu < 0` produces
a stable origin while `mu > 0` produces a stable limit cycle of radius
`sqrt(mu)`. This gives the parameter sweep a clear qualitative transition and
an analytic expectation for later verification.

Evolution uses fixed-step explicit Euler. The planned values are
`omega = 1.0`, `dt = 0.01`, initial point `[0.25, 0.0]`, and
`mu = [-0.25, 0.25, 1.0]`. Every derivative is evaluated from the same old
point before both coordinates are updated. The method is intentionally simple,
allocation-free, and sufficiently stable for these bounded demonstration
parameters; the example does not introduce a generic integrator abstraction.

The example crate root is also the scientific project root; there is no nested
`project/` wrapper. `ScientificProject::load` therefore receives the
`examples/attractor_2d` path, standard configuration lives directly under
`config/`, including the state template.

The first example artifact is finalized in `config/fixed.json`:
`model_name = "supercritical_hopf_normal_form"`, `step_count = 5000`,
trajectory sampling interval of 10 iterations, radius interval of 5 iterations,
checkpoint interval of 1000 iterations, an 8192-byte chunk target, and a
65536-byte writer queue.
Together with `dt = 0.01`, one task evolves to physical time `50.0`. These are
example-local defaults and will be validated by the consumer application
before recording begins.

The configuration set is complete. `config/sweep.json` defines the ordered
Cartesian `mu` axis `[-0.25, 0.25, 1.0]`, producing three tasks across the Hopf
bifurcation. `ScientificProject` loads conventional `config/state.json`
directly. `config/paths.json` resolves only `recording_root` to the ignored
`target/recordings` directory relative to the standalone example root.

The state/recording boundary now deliberately demonstrates heterogeneous
payloads. A complete Hopf state is built-in `SimulationTime`,
`point: Vec<f64>` containing `[x, y]`, and `radius: f64` containing the current
radial amplitude. Although derivable, radius is the model's primary diagnostic:
it tends to zero for negative `mu` and to `sqrt(mu)` for positive `mu`, and it
provides a cheap invariant against the point payload.

The planned partial streams are `trajectory` (`point` at a 10-iteration interval) and
`radius` (`radius` at a 5-iteration interval). The complete `checkpoint` stream records both
payloads at a 1000-iteration interval. Including endpoints yields respectively 501, 1001,
and 6 records per task. The approved configuration now uses
`trajectory_sampling_interval = {"iterations":10}`,
`radius_sampling_interval = {"iterations":5}`, and
`checkpoint_sampling_interval = {"iterations":1000}`.

Both scalar and vector payloads use the registry's generic Serde JSON path:
`with_json_field::<f64>` and `with_json_field::<Vec<f64>>`. Applications need
custom per-field decoders only when a payload uses a specialized wire shape or
requires configured conversion.

The point payload uses `Vec<f64>` in this example. For two coordinates it gives
the leanest JSON representation, avoids an additional numerical dependency,
and matches the existing default vector decoder. `ndarray::Array1` would add
Serde feature configuration without helping this kernel. PiP dense tensors are
already compatible with `SystemState` and are the preferred choice for genuine
ranked or high-dimensional scientific data, but their repeated kind/shape JSON
metadata and later PiP-aware decoder would be ornamental overhead for two
scalars. A later tensor-scale example should demonstrate PiP where its shape
and algorithms carry real meaning.

`config/state.json` is complete with canonical keys `point` and
`radius` plus language-neutral scientific descriptions. Concrete Rust types
remain an assembly-time contract rather than template metadata.

The four JSON inputs pass an end-to-end public-API audit: `ProjectConfig` loads
10 fixed and one swept parameter into three ordered tasks, typed decoding
returns the expected values, both named paths resolve relative to the example
root, and `SystemStateSchema` loads the canonical `point`/`radius` field order.

The example README gives end users one coherent introduction to the ODE, exact
inputs, state ownership, explicit Euler evolution, stream sampling intervals
and counts, minimal output policy, typed verification, and the runnable command.

The standalone example manifest is complete: `attractor-2d` is an unpublished
Rust 2024 binary with a Rust 1.85 floor and an explicit `src/main.rs` target. It
uses the local `scientific-workflow` crate plus Rayon 1.12 for bounded
task-level parallelism. Its application `Cargo.lock` is retained for
reproducibility.

The minimal modular executable and `Cargo.lock` are implemented. `main.rs`
orchestrates dedicated project, simulation, recording, and validation modules.
It feeds the lazy `TaskConfigIter` into Rayon's bounded work-stealing pool with
`par_bridge`, avoiding an eager task list while moving owned handles safely
between workers. The program
loads configuration, moves `Vec<f64>` into each state without cloning,
updates heterogeneous payloads through tuple mutation, samples three streams,
explicitly closes every writer lifecycle, reads the complete checkpoint's
latest state, and verifies final values exactly. `HopfModel` is the sole owner of each live state;
recording and validation borrow the same instance immutably. The successful full
trial completes all three tasks, each with 501 trajectory,
1001 radius, and 6 checkpoint records. Example and library formatting, tests,
doctests, and warnings-denied Clippy pass.

The first full trial initially exposed a one-ULP discrepancy in a vector
coordinate decoded by Serde JSON's default fast float parser. The stored
decimal itself round-tripped through the standard parser to the original bit
pattern. Because exact finite-float reconstruction is important for scientific
payload verification, the crate now enables Serde JSON's `float_roundtrip`
feature alongside `raw_value`. The rerun reconstructs every final time and
payload with exact equality; the example does not hide the issue behind a
numerical tolerance.

The example now exercises only the minimum reporting surface: parameter-based
identity, task start, absolute iteration updates, task completion, and reporter
completion. It omits custom progress phases, checked-arithmetic scaffolding,
and explicit parallel success/failure branching. Readback validates only the
complete checkpoint because repeating the same endpoint assertions for partial
streams teaches no additional storage API. Its README directs readers from
configuration through the model and storage modules to `main.rs`, making the
orchestrator the conclusion rather than the entry point for understanding.
`HopfModel::step` includes a demonstration-only 500-microsecond pause so the
interactive reporter remains visible during this otherwise tiny calculation;
real models must omit that artificial delay.

An independent one-file reference at
`examples/attractor_2d/validation/naive_hopf.rs` now validates the scientific
kernel without using `scientific-workflow` or any external dependency. Direct
`rustc` execution matches the workflow's final iteration and the exact
IEEE-754 bit patterns of accumulated physical time, both coordinates, and
radius for all three swept tasks. Its scope is deliberately numerical; storage
and reconstruction remain validated by the workflow and library tests.

### User-API simplification implementation

The crate and attractor example retain explicit live-state ownership and the
observation loop while simplifying common setup. The completed changes are:

1. Writer-wide shared chunk and queue byte limits, plus
   `add_sampled_state_stream(name, fields, sampling_interval)`. The common path then
   stops repeating identical limits for every stream. Per-stream overrides
   remain available for asymmetric workloads, and byte budgets remain explicit
   rather than silently hard-coded.
2. `SystemStateWriterBuilder::with_task_parameters(&TaskParameters)`
   copies the small resolved JSON values and task ordinal into user metadata
   internally, removing application knowledge of `serde_json::Map` without
   coupling the writer to `fixed.json` loading.
3. Generic JSON decoder registration through
   `with_json_field::<T>(key)`, for any supported `DeserializeOwned` payload.
   Domain-specific or validating decoders continue using
   `register_for_field`. This removes one-off scalar decoders and makes common
   vectors, strings, numbers, booleans, and plain structs uniform.
4. `TimeAxisMetadata::with_physical_axis(name, unit)` replaces the common
   two-call name/unit pair while retaining the existing granular methods.
5. The example decodes `NonZeroU64`
   directly through `TaskParameters::decode_value`, stops pre-collecting a
   redundant task-plan vector, and lets `HopfModel` own its immutable scientific
   coefficients so evolution becomes `model.advance()`.

The following boundaries should remain explicit: the writer must not read
project JSON itself; `observe_state` must not retain the state; byte limits
must not become invisible defaults; recording completion must remain an
explicit lifecycle call; and reader/decoder configuration must remain separate
from scientific analysis.

The input-layout correction is complete: all four human-authored JSON inputs
live in `config/`, and the path dictionary resolves `config/state.json`.
Parsing ownership remains with `SystemStateSchema`, not `ProjectConfig`. The
existing exact configuration exporter currently copies only fixed, sweep, and
paths JSON; it does not include the separately owned state template.

### PiP serialization audit

The published `physics_in_parallel` 3.0.4 crate has the correct primary
integration boundary for ordinary Scientific Workflow payloads: dense tensors,
vector lists, matrices, and square lattices implement Serde `Serialize`, and
the state writer invokes that borrowed implementation directly. Dense tensor,
vector-list, and lattice serializers use borrowed shape/data slices, so they do
not clone the scientific allocation before producing the unavoidable encoded
record bytes. Typed reconstruction can use
`JsonPayloadDecoderRegistry::with_json_field::<T>` because these types also
implement `Deserialize`.

The coordinated PiP serialization refactor is implemented:

1. Sparse tensors and sparse matrices use `kind`, `version`, `scalar`, `shape`,
   strictly increasing row-major `indices`, and matching `values`. Encoding
   sorts and copies only `nnz` entries; decoding validates shape products,
   lengths, bounds, ordering, duplicates, scalar identity, and explicit zeros
   before direct sparse construction. No logical-dense intermediate remains.
2. `AttrsCore` and `PhysObj` directly stream Serde output. Their versioned
   schema persists stable attribute IDs, slot count, labels, sealed PiP scalar
   tags, and typed vector-list payloads. Raw per-attribute JSON is dispatched
   by scalar tag during deserialization without a `serde_json::Value` tree.
   `PhysObj` now satisfies the Scientific Workflow payload contract and has an
   end-to-end writer/reader integration test using `with_json_field::<PhysObj>`.
3. PiP independently enables `float_roundtrip`. Exact-bit regression coverage
   retains the previously sensitive `f64` value without relying on consumer
   Cargo feature unification.
4. Every current PiP payload schema is version 1 and includes a stable scalar
   identifier. Serialization rejects non-finite real or complex scalar values
   because ordinary JSON numbers cannot preserve NaN or infinity.
5. Direct Serde remains the canonical high-performance API; the allocating
   payload, value-tree, and pretty-string helpers remain convenience APIs and
   still allocate by design.
6. Standalone lattice and `PhysObj` file helpers write through `BufWriter`,
   accept path-like inputs, and propagate serialization errors. Scientific
   Workflow remains responsible for durable chunk publication.
7. The Python loader recognizes current sparse matrix/tensor and emitted square
   lattice tags, validates version and sparse invariants, restores scalar
   dtypes including complex values, rejects lossy or non-finite numeric input,
   and uses overflow-safe shape products.
8. Validation errors report received tags, unknown fields are rejected, and
   Rust tests cover exact schemas, compact sparse size, malformed inputs,
   deterministic round trips, mixed composite types, and file helpers.

Encoded-size estimation is not required for Scientific Workflow integration:
its writer chunks and applies backpressure using the exact serialized record
length after encoding. PiP should avoid maintaining a second estimator unless
a separate PiP use case demonstrates a need for it.

#### Scientific Workflow coordination status

No further storage or `SystemState` API change is required for the current PiP
serialization contract. Scientific Workflow already borrows every payload
through `Serialize`, stores the resulting JSON field unchanged, and delegates
typed reconstruction to `with_json_field::<T>`. The integration suite now
proves this boundary for dense PiP tensors and heterogeneous `PhysObj` values,
including checkpoint continuation. PiP's wire-level `version` and `scalar`
metadata remain opaque payload content here; validating them belongs to PiP's
own `Deserialize` implementations.

Scientific Workflow resolves PiP 3.0.4 from crates.io as a development-only
integration dependency. There is no local path override, compatibility adapter,
or PiP-specific decoder.

The dependency boundary is strict: Scientific Workflow may consume PiP values
through their public Serde implementations, but PiP must never depend on
Scientific Workflow. PiP owns scientific container representation and typed
round trips; Scientific Workflow owns sampling intervals, asynchronous writing,
chunk lifecycle, run metadata, checkpoint continuation, and analysis-series
reconstruction.

## Time vocabulary clarification

The former verb-based sampling-interval spelling was grammatically awkward and did not
clearly distinguish a sampling rule from a time coordinate. It was not renamed
to “sweep epoch.” Within this project, *sweep* already means the
parameter-space expansion that produces independent tasks, while *epoch* often
means a full optimization pass, Monte Carlo sweep, or wall-clock era. Combining
them would make the ambiguity worse.

The corrected vocabulary distinguishes an action from every coordinate and
count derived from it:

1. **Step** means exactly one successful model-evolution action. Completing one
   step increments the iteration counter by one. A step is not a timestamp,
   counter, interval, duration, or parameter-sweep task.
2. **Iteration** is the monotonic integer coordinate stored in
   `SimulationTime`. Initial state is iteration 0; after one completed step the
   state is at iteration 1.
3. **Physical time** is the optional modeled continuous coordinate stored in
   `SimulationTime`. A fixed solver may add one physical-time increment when a
   step completes, but adaptive solvers need not use a constant increment.
4. **Task index** identifies one fixed-plus-sweep parameter selection. It is
   configuration provenance, not simulation time.
5. **Record position** is a stream-local storage/analysis ordinal. It must not
   be confused with iteration or physical time.

The implementation now reserves “step” for the evolution action and uses
**sampling interval** for the selection policy. Its coordinate belongs in its
type rather than in an awkward field or method name. The public API is
`SimulationTime::from_iteration`, `SimulationTime::iteration`,
`SamplingInterval::Iterations(NonZeroU64)`, and
`add_sampled_state_stream(name, fields, sampling_interval)`. The noun variant
states that the interval is measured on the iteration coordinate without
calling an iteration a step.

Persisted stream metadata should use a typed value:

```json
{"sampling_interval":{"iterations":10}}
```

This keeps `sampling_interval` as the stable concept and leaves room for a
future explicitly different coordinate such as
`{"sampling_interval":{"physical_time":0.5}}`. Example project keys may stay
purpose-oriented—`trajectory_sampling_interval`, `radius_sampling_interval`,
and `checkpoint_sampling_interval`—with the same typed JSON value. An
iteration interval of 10 selects iterations 0, 10, 20, and so on. The
final-state rule may add a terminal iteration that is not aligned to that
interval.

Solver configuration must call a duration what it is:
`physical_time_increment_per_step`, not a “physical time step.” A configured
`step_count` may count how many evolution actions to perform, but an iteration
number must never be called a step. Persisted records should therefore use
`iteration` and `physical_time`; chunk ranges should use `first_iteration` and
`last_iteration`.

Future physical-time sampling should be a separate noun-based policy such as
`SamplingSchedule::PhysicalTimeInterval`. Domain-specific Monte Carlo sweeps
remain model terminology and must not redefine the generic meanings of step or
iteration.

## Runtime model and orchestration layer migration readiness

The crate is now a stable migration target for both consumer projects. A
runtime model can own one authoritative `SystemState`, mutate disjoint typed
payloads through tuple borrowing, perform one model `step`, advance its
`iteration`, and offer the borrowed state to a `SystemStateWriter`. The writer
owns typed per-stream `SamplingInterval` policies, bounded backpressure,
whole-record chunking, terminal sampling, durability barriers, and checkpoint
continuation.

Orchestration layer can replace its fixed/sweep expansion with
`ProjectConfig::task_configs`, move deterministic complete `TaskConfig` handles
directly into mission queues, configure one recording per task, and inspect
completed results through the storage reader. Exact matching can select a
subset while retaining the Cartesian product of every unconstrained axis.
Scoped execution policy and richer logging remain orchestration layer-level work rather
than prerequisites missing from this crate. Migration should preserve the new
storage-format version 4 contract and should not introduce compatibility
aliases for the former step-based counter or sampling names.

## dependent-model crate migration audit

`the dependent model crate` should be the first consumer refactor, before
runtime model and orchestration layer. Orchestration layer directly imports dependent-model crate solver, task-outcome,
metadata, and output APIs, whereas runtime model does not depend on dependent-model crate. Migrating
dependent-model crate first therefore establishes the new consumer contract and removes one
entire legacy persistence format before orchestration layer is changed. dependent-model crate is also a
smaller real-world proving ground than runtime model: it already owns one evolving
state, uses ordinary Serde-compatible `ndarray` payloads, and produces two
logical streams with different field selections.

The current dependent-model crate boundary duplicates Scientific Workflow in four places:

1. its generic `SystemState<T>` owns mode, integer time, aggregate array,
   optional spatial array, and cached mass as public fields;
2. solvers alternate between two complete dependent-model crate states and directly decide when
   signal and space samples are due;
3. `SignalWriter` and `SpaceWriter` clone arrays into estimated-size JSON
   chunks; and
4. `TaskOutcome` writes a second, dependent-model crate-specific `metadata.json` lifecycle that
   orchestration layer parses directly.

The target dependent-model crate state uses Scientific Workflow's `SystemState` as the sole
authoritative evolving state. Its schema should declare `state`, `mass`, and,
for spatial models, `space`. Concrete payload types remain `Array1<f64>`,
`f64`, and `ArrayD<f64>`; their types are retained in memory and erased only
when storage requests borrowed serialization. `Mode`, model coefficients,
cutoff, carrying capacity, solver settings, and termination settings are model
configuration rather than evolving payloads and should remain outside the
state. Separate non-spatial and spatial schemas avoid pretending that a
missing spatial field is a loaded scientific payload.

Each dependent-model crate model should expose one `step` operation. Solver scratch arrays and a
next-array buffer may remain model-owned implementation details, but a second
complete `SystemState` should not. After a successful numerical update, the
model swaps or writes the resulting arrays into its authoritative state,
refreshes `mass`, and calls `advance_simulation_time(Some(dt))`. Termination
logic borrows the typed fields it needs. A run orchestrator offers the state to
one `SystemStateWriter` after initialization and after every completed step;
the writer alone evaluates `SamplingInterval` for `signal`, `space`, and
checkpoint streams.

dependent-model crate's `io` module, fixed estimated-byte chunking, `SignalRecord`,
`SpaceRecord`, and file loaders should disappear after equivalence is proven.
Scientific Workflow then supplies exact encoded-byte chunking, bounded
backpressure, one authoritative metadata document, typed reconstruction, and
checkpoint continuation. dependent-model crate configuration known before execution—requested
step count, model identity, solver increment, and sampling intervals—can be
recorded as writer user metadata at creation. The termination reason and
completed step count are known only at the end. dependent-model crate may continue returning
those values in an in-memory run outcome, but preserving them in the sole
`metadata.json` requires the deferred terminal-metadata API. That is the one
Scientific Workflow decision to settle before deleting dependent-model crate's metadata
implementation; creating a second dependent-model crate sidecar is not acceptable.

Migration order is:

1. add a local Scientific Workflow dependency and define checked-in spatial
   and non-spatial state schemas;
2. introduce model owners around the authoritative state and adapt
   sanitization, noise, termination, and numerical kernels;
3. replace solver-owned save branches with writer observation and typed stream
   intervals;
4. replace dependent-model crate readers and metadata validation with Scientific Workflow
   readers and metadata;
5. migrate examples and add numerical/output equivalence tests before deleting
   legacy IO; and
6. publish dependent-model crate, then refactor orchestration layer against that new API before migrating
   runtime model's more complex PiP state.

The audited dependent-model crate baseline is green: all ten library tests and all five example
test targets pass before migration. Existing tests currently live inside
production modules; the refactor should move meaningful coverage into the
dedicated `tests/` directory to match the repository-wide testing convention.

### dependent-model crate refactor kickoff

dependent-model crate development proceeds on its dedicated `sw-version` branch.

## Example-pattern absorption audit

The complete `attractor_2d` example was audited to distinguish reusable
workflow infrastructure from application policy. The example should remain
explicit enough to teach the ownership model, but it should not have to invent
generic project and recording lifecycle types that every consumer project
will repeat.

### Patterns that belong in Scientific Workflow

#### Conventional project definition

`project_setup::load_project` always performs the same three operations: load
`fixed.json`, `sweep.json`, and `paths.json`; locate `state.json`; and retain
the shared `SystemStateSchema`. This is now one crate-owned
`ScientificProject` rather than an application-specific `ProjectSetup` wrapper.

The preferred clean-slate convention is a mandatory
`project-root/config/state.json` beside the other three configuration files.
The state template is structural project configuration, not an arbitrary data
path, so `paths.json` no longer needs a `state_template` entry.
`ScientificProject` owns `ProjectConfig` plus `SystemStateSchema` and exposes
the existing parameter and path views without copying either. It describes
immutable project inputs and does not execute tasks.

#### Scoped execution directories

The former `create_execution_directory` used an application-formatted timestamp
and process identifier while each task manually derived `task_{index}`.
`ExecutionScope` now creates generated or caller-named scopes, reopens existing
scopes, and derives deterministic task recording paths. Exclusive directory
creation remains the collision authority, and task paths stay absent until the
recording writer exclusively creates them.

#### Completed-recording result

The example's former path-only `CompletedRecording` wrapper has been replaced
by the crate-owned immutable `CompletedRecording`. Writer completion returns it
with the recording directory, automatic `RecordingTiming`, and terminal user
metadata. Per-stream byte summaries may be added later without changing its
lifecycle role.

The completed handle should be produced only after all writer work is drained
and terminal metadata is durable. It is not a second owner of the active
writer and cannot append records.

#### Terminal metadata at completion

The example knows all metadata before evolution, but dependent-model crate demonstrates that
completed iteration count and termination reason exist only at the end. The
completion operation now accepts an optional terminal user-metadata map that is
committed atomically with completed status in the sole `metadata.json`.
Initial and terminal metadata must remain structurally distinguishable so a
terminal value cannot silently rewrite task parameters. This capability and
the completed-recording result should be one lifecycle refactor.

#### Latest-record reading

`recording_validation::validate_recording` uses
`read_latest_state_from_stream` to reconstruct
the latest state of one completed stream without scanning or retaining the
entire series. It returns that stream's partial state schema; callers decide
whether it is a complete checkpoint. Full-series reconstruction remains the
analysis API.

### Patterns that should remain application code

- `HopfModel`, its coefficients, field-name constants, payload assembly,
  explicit-Euler `step`, and derived-radius maintenance are scientific model
  behavior.
- `TaskSettings` is a useful application boundary. Automatically decoding
  magic parameter names such as `step_count` or
  `trajectory_sampling_interval` would make the generic crate less clear.
- Stream names and field selections are scientific output design. The writer
  builder already expresses them directly and should not assume `trajectory`,
  `radius`, `signal`, `space`, or `checkpoint`.
- Exact final-state equality is example validation policy. Plotting, bounds,
  and scientific interpretation are intentionally absent from the runtime demo.
- The executable's `AppResult`, logging text, and process-exit behavior belong
  to the application.

### Patterns to defer until dependent-model crate supplies a second use case

The short `observe initial -> step and observe -> complete with final state`
loop is attractive but should not yet become a framework runner or an
`EvolvingSystem` trait. dependent-model crate requires early termination, stochastic failure,
progress reporting, separate solver scratch, and continuation; runtime model adds
group execution and activity-based stopping. A trait inferred only from the
Hopf loop would either be too narrow or would prematurely make execution policy
part of `SystemState`.

Likewise, stream declarations should not yet be decoded automatically from
magic `fixed.json` keys. If dependent-model crate repeats the same declaration structure, a
format-independent recording-plan type may be justified. Until then,
`StateStreamConfig` and `SystemStateWriterBuilder` are the correct explicit
boundary.

### Completed absorption order

1. Terminal metadata and a completed-recording handle now share one terminal
   lifecycle transition.
2. Efficient latest-state reading is available for completed streams.
3. `ScientificProject` owns the conventional `config/state.json` schema.
4. `ExecutionScope` owns generated/named/opened execution directories and
   deterministic task recording paths.
5. Revisit a generic run controller only after dependent-model crate and runtime model expose the
   complete set of evolution and termination needs.

## Operational timestamp and duration architecture

Scientific time and operational time are distinct domains and must remain
separate throughout the API and wire format:

1. `SimulationTime::iteration` is the integer scientific coordinate.
2. `SimulationTime::physical_time` is an optional modeled continuous
   coordinate.
3. A UTC timestamp identifies when a workflow event happened on the host
   clock.
4. A monotonic duration measures elapsed execution without being affected by
   wall-clock adjustment.
5. `SamplingInterval` selects scientific states and is neither a timestamp nor
   an elapsed duration.

The workflow layer should manage operational timing automatically. Applications
should not have to call `SystemTime::now`, format directory timestamps, or
calculate elapsed run time for ordinary project execution.

### Recording timing

Every recording's sole `metadata.json` contains one structural timing
section. A newly created writer records its UTC creation/session-start
timestamp automatically. Successful completion and explicit failure record a
UTC terminal timestamp and monotonic active duration atomically with terminal
status. The completed-recording handle exposes this information without
requiring callers to parse JSON.

Recommended conceptual shape:

```json
{
  "timing": {
    "created_at_utc": "2026-08-09T21:15:30.123456789Z",
    "finalized_at_utc": "2026-08-09T21:17:04.987654321Z",
    "active_duration_ns": 94864197532,
    "continuation_count": 0
  }
}
```

UTC timestamps use a canonical RFC 3339 representation with a `Z` suffix and
subsecond precision. Durations use integer nanoseconds rather than floating
seconds, avoiding rounding ambiguity and retaining an exact machine-readable
unit. Human-facing formatters may present a friendlier duration without
changing persisted data.

`created_at_utc` is immutable. `finalized_at_utc` is absent while status is
running. `active_duration_ns` measures time actively owned by writer sessions,
not the wall-clock difference between creation and completion; a paused or
interrupted recording may therefore have a much larger wall span than active
duration. `continuation_count` makes resumed execution explicit without adding
another artifact.

Continuation must preserve the original creation timestamp. Each active writer
uses a process-local monotonic timer and adds its elapsed duration during a
durable lifecycle transition. A process crash can prevent the unfinished
session's monotonic duration from being committed; the format must not invent
precision it cannot recover. If detailed session provenance later proves
necessary, a session list can be added in a new format version, but it is not
required for the first timing implementation.

### Execution-scope timing and names

A generated execution scope receives both an opaque collision-resistant
identifier and an automatic UTC creation timestamp. A readable timestamp may
appear in its directory name, but timestamp text alone is not a uniqueness
mechanism because concurrent processes can observe the same clock value. Named
scope creation remains available when an application or orchestration layer owns the
external identity.

Scope metadata should eventually record scope creation/finalization and total
duration independently of each task recording. That belongs to the future
orchestration layer-oriented execution-scope feature; recording-level timing can be
implemented first.

### What should not be automatic

- State JSONL records should not receive wall-clock timestamps automatically.
  They remain lean and deterministic: only iteration, optional physical time,
  and selected scientific payloads are recorded.
- Chunk descriptors do not initially need timestamps. Checksums, byte counts,
  record counts, and iteration ranges already define their scientific and
  durability contract.
- Model phases, solver substeps, benchmark spans, or domain events should not
  become hard-coded metadata fields. Applications may store scientifically
  relevant values as state payloads; a later structured logging/metrics module
  may record operational events.
- Wall-clock subtraction must not be used as the authoritative active duration;
  host clocks can jump.

Timing, terminal metadata, and the completed-recording handle are implemented
through the same lifecycle transition and produce one atomic terminal metadata
commit.

### Effect on application examples

Automatic operational timing removes all direct wall-clock handling from
`attractor_2d`. In particular, `project_setup::create_execution_directory`,
its `SystemTime`/`UNIX_EPOCH` use, process-identifier suffix, and manual run-name
formatting have disappeared. The completed
recording handle likewise replaces the example's path-only wrapper.

It deliberately does not remove scientific-time handling from the model:

- configuration still supplies `step_count` because it requests a number of
  evolution actions;
- configuration still supplies `physical_time_increment_per_step` because it
  is part of the explicit-Euler model;
- `HopfModel::step` still calls `advance_simulation_time` only after committing
  a successful state transition;
- stream configuration still supplies `SamplingInterval` values; and
- validation still compares iteration and physical time from reconstructed states.

The application may also continue declaring physical-time axis names and units
because those are scientific semantics. Workflow can provide sensible
iteration defaults, but it cannot infer whether physical time is measured in
seconds, days, generations, or a dimensionless coordinate.

Accordingly, the desired result is **no application-owned operational clock**,
not “no time code anywhere in the application.” Scientific time remains an
explicit part of model correctness.
