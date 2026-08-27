# UI API

The `ui` subsystem owns automatic presentation of execution facts already
known by Runtime. It does not inspect models, scientific payloads, project JSON,
or persistence files. Models never define display fields, format messages,
increment progress counters, or receive a UI handle.

Study owns a private immutable effective UI plan. The current plan is inferred
completely: progress rendering is checked at runtime and progress updates are
throttled to at most once per task every 100 milliseconds. Runtime supplies
live lifecycle facts; UI owns activation, throttling, formatting, and output.

## Basic API

`scientific_workflow::ui::basic` intentionally exports no Rust symbols.
Interactive progress is automatic whenever the application calls
`scientific_workflow::run(&Path)` or `runtime::advanced::execute(Study)`.

The activation rule is inferred without configuration:

- when standard error is attached to an interactive terminal, UI prints
  progress;
- when standard error is redirected, captured by tests, or used in ordinary
  noninteractive CI, UI remains silent.

There is no `ui` object in `study.json`. Consequently there are no user-defined
refresh rates, enable flags, themes, field lists, templates, callbacks, or
message channels. If genuine customization is added later, Config must remain
its sole JSON parser and Study must own the fully defaulted effective settings.

The automatic terminal output includes:

- execution start, inferred replicate count, task count, and output directory;
- replicate and phase start/completion/failure;
- task identity, inferred label, workload kind, model key or executable, and
  phase;
- current scientific iteration and optional target for model tasks;
- percentage only when the model supplies a target;
- task failure reasons; and
- successful optional final iteration and generic task output directory.

Program tasks—including Python scripts, presented by their script filename—
publish lifecycle start/failure/completion but no fabricated iteration
progress. Model tasks continue to publish observations as progress.

UI displays structural and operational facts only. It never serializes or
formats scientific payload values. Rendering uses standard error so standard
output remains available for application results and pipelines.

## Advanced API

`scientific_workflow::ui::advanced` is the strict public superset of the empty
Basic scope and currently adds no public symbols. There is no public event,
session, renderer, snapshot, sink, command, or configuration type.

Runtime uses crate-visible exports from this scope:

- `UiPlan` is the immutable Study-owned inferred policy;
- `UiSession` is a clone-cheap, thread-safe runtime session; and
- `TaskUi` is the task-scoped progress publisher held by Runtime's host;
- `UiEvent` is the borrowed synchronous fact vocabulary.

These are peer-subsystem boundaries, not downstream API. An event borrows its
strings and paths only for the synchronous `publish` call. The session retains
only task identities and monotonic timestamps needed for throttling; it never
retains a state, payload, model, Study, runtime summary, or recording handle.

Concurrent replicate and task workers share one `UiSession`. A mutex protects
only the small per-task throttle map, and terminal writes are serialized by the
standard-error lock. UI starts no thread, queue, async runtime, or terminal raw
mode. Rendering failures are deliberately best-effort and cannot turn a valid
scientific execution into a Runtime failure.

## Example

The complete user interaction is unchanged:

```rust,no_run
use std::path::Path;

fn main() -> Result<(), scientific_workflow::WorkflowError> {
    scientific_workflow::run(Path::new("."))
}
```

Running this executable directly in a terminal shows progress. Redirecting
standard error disables UI automatically:

```text
scientific-program 2>workflow.log
```

No model or JSON change is required in either case.

## Not API

Event variants, inferred refresh timing, terminal-detection mechanics, line
format, prefixes, percentage precision, throttle-map structure, stderr
locking, and best-effort write handling are private implementation details.
Applications must not parse rendered lines as a machine-readable protocol;
durable facts belong to Runtime summaries and persistence metadata.

A replacement UI must remain downstream of Runtime facts, require no model
participation, avoid retaining scientific payloads, preserve noninteractive
silence, tolerate concurrent publishers, and never make presentation failure
fail scientific work. Commands, cancellation input, full-screen rendering, or
remote presentation require separate justification and must remain outside the
model/task contracts.
