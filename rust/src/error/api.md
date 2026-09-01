# Error API

This guide documents the `scientific-workflow` 0.13.1 subsystem contract.

The `error` module owns the single error returned by the crate-level complete
workflow facade. It composes failures from Study and Runtime without taking
ownership of either subsystem's detailed error vocabulary.

## Basic API

### `WorkflowError`

`WorkflowError` is a public non-exhaustive enum and the error type returned by
`scientific_workflow::run(&Path)`. Its canonical module path is
`scientific_workflow::WorkflowError`; the same type is
also re-exported through `scientific_workflow::prelude`.

It has two variants:

- `Study(StudyError)` means project loading, parsing, expansion, execution unit
  discovery, constants decoding, observation binding, or another preflight
  operation failed before Runtime received a Study and before output creation;
- `Runtime(RuntimeError)` means active execution failed after a valid immutable
  Study was available. The unique execution directory and diagnostic recording
  evidence may already exist.

Both variants own their subsystem error and are constructed automatically by
`From<StudyError>` and `From<RuntimeError>`. Their transparent error behavior
forwards the subsystem error's display text and source chain without inserting
an additional facade message. Matching the public variant retrieves the owned
subsystem error. Applications normally propagate the value with `?`. Because
the enum is non-exhaustive, downstream matching requires a wildcard arm;
applications that want the precise stage-specific result type can instead use
the split `Study::load` and `runtime::execute` workflow.

The error performs no IO, starts no work, and has no cancellation behavior.
Formatting it is side-effect free. It borrows nothing from the project or
Runtime, so it can outlive the failed `run` call. `WorkflowError` is `Send +
Sync` because every retained subsystem source has that contract.

Fatal UI renderer failures are active-runtime failures. They return
`RuntimeError::Presentation` and therefore appear through
`WorkflowError::Runtime` from the complete facade; they are never reported as
cooperative cancellation or hidden behind a fallback renderer.

## Advanced API

Callers that need phase separation load `Study` and call
`runtime::execute`. Those calls return their precise owning error
types directly; `WorkflowError` is reserved for the complete crate facade.

## Example

Ordinary applications normally propagate the complete error:

```rust,no_run
use std::path::Path;

fn main() -> Result<(), scientific_workflow::WorkflowError> {
    scientific_workflow::run(Path::new("."))
}
```

A caller can distinguish the broad failure stage without depending on every
subsystem error variant:

```rust,no_run
use std::path::Path;
use scientific_workflow::WorkflowError;

match scientific_workflow::run(Path::new(".")) {
    Ok(()) => {}
    Err(WorkflowError::Study(error)) => eprintln!("preflight failed: {error}"),
    Err(WorkflowError::Runtime(error)) => eprintln!("execution failed: {error}"),
    Err(error) => eprintln!("workflow failed: {error}"),
}
```

## Not API

The private storage file for `WorkflowError`, crate-facade sequencing, and
`thiserror` derive expansion are implementation details. The module does not
own project parsing, execution, recovery, logging, exit-code selection,
diagnostic rendering, or panic recovery. New error variants require
compatibility review because the enum is public, but its non-exhaustive
contract permits Workflow to add a new complete-workflow stage without
breaking exhaustive downstream matches.
