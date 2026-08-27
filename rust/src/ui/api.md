# UI API

The `ui` subsystem owns automatic presentation of execution facts already
known by Runtime. It does not inspect models, scientific payloads, project JSON,
or persistence files. Models never define display fields, format messages,
increment counters, or receive a UI handle.

Study owns a private inferred UI policy. Runtime publishes planned-task,
lifecycle, progress, path, and outcome facts. UI alone owns terminal detection,
the Ratatui dashboard state, command editing, message history, refresh timing,
and the internal exit request observed by Runtime.

## Basic API

`scientific_workflow::ui::basic` intentionally exports no Rust symbols. The UI
starts automatically through `scientific_workflow::run(&Path)` or
`runtime::advanced::execute(Study)`.

When both standard input and standard error are terminals, UI enters a
Crossterm alternate screen and renders a Ratatui dashboard containing:

- inferred replicate, phase, task, label, kind, and subject rows in declared
  study order;
- pending, running, completed, failed, cancelled, and skipped counts;
- model iteration gauges when `ScientificModel::target_iteration` is known;
- model spinners when the target is unknown;
- one-shot spinners while generic programs and Python tasks are running;
- elapsed time and inferred ETA where enough progress exists;
- the execution output path;
- a bounded 100-line lifecycle/error message panel; and
- the command editor.

Runtime lifecycle lines are appended to the message panel instead of scrolling
the interactive terminal. Scientific payloads are never rendered. When either
standard stream is not interactive, the same lifecycle messages use the former
stable line-oriented standard-error fallback, so redirected runs and CI retain
diagnostics without terminal control sequences.

The command editor supports character insertion, Left/Right, Home/End,
Backspace/Delete, Escape to clear, and Enter to submit. Exact lowercase `exit`
(surrounding whitespace allowed) and Ctrl+C request cooperative cancellation.
Unknown commands appear in the message panel. Once exit is requested, Runtime
stops admission, asks active models to stop between steps, terminates active
external programs, waits for cleanup, publishes cancellation, and then UI
restores the terminal.

There is no `ui` object in `study.json`: no refresh rate, theme, field list,
message callback, progress counter, renderer, or cancellation handle is
user-defined. Terminal setup/drawing failure is best-effort and cannot turn
valid scientific work into failure.

## Advanced API

`scientific_workflow::ui::advanced` is the strict public superset of Basic and
adds no public symbols. Runtime and Study use crate-visible boundaries:

- `UiPlan` is the immutable inferred refresh policy;
- `UiEvent` is the borrowed synchronous fact vocabulary;
- `UiSession` owns shared reduced state, terminal selection, the renderer
  thread, and cancellation request; and
- `TaskUi` publishes iteration/target facts for one inferred task.

These are peer-subsystem contracts, not downstream API. Event strings and
paths are copied into small UI-owned presentation snapshots as required; UI
never retains a model, `SystemState`, payload, `Study`, recording writer, or
runtime summary. Concurrent Runtime workers share one clone-cheap session. A
mutex protects only dashboard presentation state, and a single bounded-refresh
thread owns interactive terminal input and drawing.

`UiSession::finish` is called internally after the terminal execution event.
It joins the renderer before Runtime returns, so alternate-screen, raw-mode,
cursor, and mouse state are restored on success, failure, or cancellation.

## Example

The complete user interaction remains one call:

```rust,no_run
use std::path::Path;

fn main() -> Result<(), scientific_workflow::WorkflowError> {
    scientific_workflow::run(Path::new("."))
}
```

Running it in a terminal shows the dashboard. Typing `exit` and pressing Enter
requests cancellation. Redirecting standard error selects plain lifecycle
lines automatically. No model or JSON change is involved.

## Not API

Ratatui/Crossterm types, event variants, dashboard snapshots, task statuses,
command parser/editor, renderer thread, alternate-screen lease, message
capacity, layout, colors, glyphs, refresh interval, ETA formula, plain-line
format, and cancellation atomics are private. Applications must not parse the
human display as a machine protocol; durable facts belong to Runtime summaries
and persistence metadata.

A replacement UI must remain downstream of Runtime facts, require no model
participation, tolerate concurrent publishers, preserve plain noninteractive
diagnostics, support cooperative `exit`, restore terminal state, and never make
presentation failure fail scientific execution.
