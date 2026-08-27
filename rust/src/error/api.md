# Error API

The `error` module owns the single error returned by the crate-level complete
workflow facade. It composes failures from Study and Runtime without taking
ownership of either subsystem's detailed error vocabulary.

## Basic API

### `error::basic::WorkflowError`

`WorkflowError` is a public non-exhaustive enum and the error type returned by
`scientific_workflow::run(&Path)`. Its canonical module path is
`scientific_workflow::error::basic::WorkflowError`; the same type is
re-exported as `scientific_workflow::WorkflowError` and through
`prelude::basic`.

It has two variants:

- `Study(StudyError)` means project loading, parsing, expansion, model
  discovery, constants decoding, observation binding, or another preflight
  operation failed before Runtime received a Study and before output creation;
- `Runtime(RuntimeError)` means active execution failed after a valid immutable
  Study was available. The unique execution directory and diagnostic recording
  evidence may already exist.

Both variants own their subsystem error, preserve its source chain, and are
constructed automatically by `From<StudyError>` and `From<RuntimeError>`.
Applications normally propagate the value with `?`. Because the enum is
non-exhaustive, downstream matching requires a wildcard arm; applications
that need detailed handling can inspect the source or match the owning
subsystem error through an advanced split workflow.

The error performs no IO, starts no work, and has no cancellation behavior.
Formatting it is side-effect free. It borrows nothing from the project or
Runtime, so it can outlive the failed `run` call. Its thread-safety follows
the owned Study or Runtime error source rather than an independent wrapper
mechanism.

## Advanced API

`error::advanced` is a strict superset of Basic and currently adds no symbols.
It deliberately does not re-export `StudyError` or `RuntimeError`: those remain
canonically owned by `study::advanced` and `runtime::advanced` and are already
aggregated by `prelude::advanced`.

Advanced callers that need phase separation load `Study` and call
`runtime::advanced::execute`. Those calls return their precise owning error
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
use scientific_workflow::error::basic::WorkflowError;

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
own project parsing, execution, recovery, logging, exit-code selection, or
diagnostic rendering. New error variants require compatibility review because
the enum is public, but its non-exhaustive contract permits Workflow to add a
new complete-workflow stage without breaking exhaustive downstream matches.
