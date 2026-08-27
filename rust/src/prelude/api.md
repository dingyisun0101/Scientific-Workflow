# Prelude API

`scientific_workflow::prelude` is an import index. It creates no values, owns
no behavior, changes no canonical symbol, and performs no validation, I/O,
blocking, persistence, threading, or cancellation work. Importing a prelude
only places re-exported names in scope.

The canonical behavioral contract for each symbol remains in its owning
module. During the staged architecture migration, the prelude also aggregates
the existing configuration, storage, execution, artifact, RNG, and study
surfaces. As each becomes a first-level tiered subsystem, its module-owned
`basic` or `advanced` scope replaces that transitional direct inventory.

## Basic API

### `prelude::basic`

`prelude::basic` is the ordinary application import scope. It re-exports the
complete `state::basic` scope:

- `StateTime`, `SystemStateSchema`, `SystemState`, and `StateSeries`;
- `StateError`, `PayloadInsertError`, `StateSeriesError`, and
  `StateSeriesPushError`.

Their methods, ownership, validation, errors, and replacement constraints are
documented in `state/api.md`.

It re-exports the complete `writer::basic` definition boundary:

- `Writer` for an application-owned scientific output definition;
- `Stream` for an optional named field selection and positive iteration
  cadence; and
- `WriterError` for definition, schema binding, observation, and encoding
  failures.

The writer defaults to one `state` stream containing every field at every
iteration. Its complete contract and inference rules are documented in
`writer/api.md`.

It re-exports the complete `task::basic` definition boundary:

- `Task` for opaque reusable stateful or one-shot definitions;
- `ScientificModel` for an application model that directly owns its canonical
  `SystemState`; and
- `TaskResult<T = ()>` for thread-safe application failures.

Task definitions accept no user-supplied identity, path, lifecycle, progress,
message, or recording administration. Their complete execution and ownership
contract is documented in `task/api.md`.

It also re-exports the complete `config::basic` scope, which is intentionally
empty. Ordinary applications
write a study manifest, state schema, and task input documents; they do not
load or query them through Rust. Consequently, it contributes no names to
`prelude::basic`.

It also re-exports these existing application APIs while their owning modules
await the same tier refactor:

- artifact: `ArtifactDescriptor`, `ArtifactDisposition`, `ArtifactError`,
  `ArtifactLoadError`, `PersistedArtifact`, `VerifiedArtifact`,
  `load_verified_artifact`, and `persist_artifact`;
- configuration: `ConfigurationError`, `ConfigurationIter`, `ProjectPaths`,
  `ReplicateFailurePolicy`, `ReplicateScheduling`, `ReplicateSettings`,
  `ResolvedConfiguration`, `StudyConfiguration`, `StudySettings`, and
  `WorkloadConfiguration`;
- execution: `ExecutionScope`, `ExecutionScopeError`, `ReplicateContext`,
  `ReplicateExecutionError`, and `ReplicateExecutor`;
- RNG provenance: `DerivedSeed`, `RNG_RECORDS_METADATA_KEY`,
  `ReplicateSeedDeriver`, `RngRecord`, and `RngRecordError`;
- storage: `CompletedRecording`, `CompletedStreamSummary`,
  `JsonPayloadDecoder`, `JsonPayloadDecoderRegistry`, `JsonStringDecoder`,
  `JsonVecF64Decoder`, `RecordingTiming`, `StateStreamLayout`,
  `StateStreamStorage`, `StorageError`, `StoredStateSeriesReader`,
  `SystemStateWriter`, and `SystemStateWriterBuilder`.

The transitional storage builder accepts `Writer` through `with_writer`; it
infers stream metadata, cadence metadata, axis names, stream directories, and
a default persistence policy. `StateStreamStorage` remains temporarily public
for explicit record-layer tuning and will move behind the validated effective plan
when `record` is refactored.

These are identity-preserving `pub use` declarations. A value imported through
the prelude is exactly the owning module's type, with the same construction,
lifetimes, thread-safety, validation, effects, errors, and failure atomicity.
The prelude never wraps errors or adds compatibility promises beyond the
owning module.

Glob imports are intended for application composition modules and examples.
Libraries that expose Workflow types in their own public signatures should
prefer narrow canonical imports such as `state::basic::SystemState` so
ownership remains obvious.

## Advanced API

### `prelude::advanced`

`prelude::advanced` first re-exports every name in `prelude::basic`; it is a
strict superset. It then re-exports the complete additional supported state
integration boundary:

- `StateFieldSchema` for immutable validated field metadata;
- `StateSchemaAccess` for schema provenance, traversal, lookup, size, and JSON
  representation, including immutable schema-allocation identity;
- `StateMaintenance` for explicit structural cloning, time replacement,
  payload counting/type inspection, and clearing;
- `StateSchemaSource` for deriving a borrowed schema from a state or schema;
  and
- the documentation-hidden sealed `PayloadTuple` bound required by the public
  tuple-borrow method signatures.

It also re-exports the complete additional `writer::advanced` integration
boundary:

- `WriterDescriptor` and `StreamDescriptor` for schema-bound, canonical output
  metadata;
- `Observation` for a checked borrowed state handoff;
- `EncodedObservation` for an owned backend handoff; and
- `ObservationSink` and `SessionOutcome` for replaceable persistence
  integration.

It also re-exports the complete additional `task::advanced` runtime boundary:

- `TaskKind` and `TaskDescriptor` for read-only execution-shape inspection;
- `TaskDefinition` for executing opaque compiled work from one config-owned
  `ResolvedTaskInput`; and
- `TaskExecutionHost` for canonical schema access, cooperative cancellation,
  and automatic initial/step/final observation boundaries.

It additionally re-exports the complete `config::advanced` integration
boundary:

- `ProjectSpecification`, `StudyManifest`, `ReplicatePolicy`,
  `ReplicateScheduling`, `FailurePolicy`, and `PhaseSpecification` for the
  immutable effective project declaration;
- `ResolvedTaskInput` for config-owned complete typed-input supply;
- `ProjectDocument` and `StateSchemaDocument` for exact source provenance and
  the already-parsed state integration seam; and
- `ConfigError` for contextual loading, strict parsing, path-containment,
  expansion, dependency, and typed-decode failures.

These target config names coexist temporarily with the old
`configuration::*` inventory in `prelude::basic` while legacy study and
execution code is migrated. The old types are not aliases for the new config
boundary.

The advanced scope grants supported integration responsibility, not access to
private implementation files. Peer subsystems should import the narrow owning
scope such as `crate::state::advanced` or `crate::writer::advanced` rather than
this aggregate, which avoids accidental dependencies on unrelated modules.
External integration binaries may use the central advanced prelude when they
intentionally compose several advanced module boundaries.

Name collisions are resolved in the owning modules, not with prelude aliases.
Two owners must not publish different meanings under one centrally aggregated
name. If a future collision appears, the central prelude must omit the
ambiguous glob-style aggregation until one owner chooses a distinct public
name; it must not silently rename types and create a second vocabulary.

The existing `prelude::study` remains a separate transitional orchestration
scope. It is neither included in basic nor advanced because task/runtime/ui
ownership has not yet been refactored into the target tiers.

## Example

Ordinary application code can use the central basic scope while retaining
typed path semantics:

```rust,no_run
use std::path::Path;

use scientific_workflow::prelude::basic::*;

fn load_state() -> Result<SystemState, StateError> {
    let schema = SystemStateSchema::load_json_template(Path::new("config/state.json"))?;
    let mut state = schema.create_empty_state(StateTime::from_iteration(0));
    state.initialize_payload("population", vec![1_u64, 2, 3])?;
    Ok(state)
}
```

An integration imports the advanced aggregate only when it uses an advanced
contract:

```rust,no_run
use scientific_workflow::prelude::advanced::*;

fn declared_fields(schema: &SystemStateSchema) -> usize {
    schema.len()
}
```

The narrow equivalent is
`use scientific_workflow::state::advanced::{StateSchemaAccess,
SystemStateSchema};`.

## Not API

The prelude contains no constructors, wrapper types, adapter traits, macros,
extension behavior, or private fallback exports. Implementation files beneath
an owning module remain private even when a public type is re-exported here.
In particular, writer sampling internals, encoding helpers, the concrete
writer session, task type-erasure adapters, task contract errors, and runtime
progress/message machinery are not reachable through either prelude.

The removed `prelude::basics` spelling is not a compatibility alias.
`prelude::study` is transitional and does not establish a third long-term API
tier. It currently exports the legacy `study::Task`, while `prelude::basic`
exports the new context-free `task::Task`; code importing both glob scopes must
name the legacy type explicitly (for example,
`use scientific_workflow::study::Task as StudyTask`) until runtime migration
removes the overlap. Import order, compiler glob-resolution details, and undocumented names
reachable only through an owning implementation path are not supported API.
