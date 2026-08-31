# UI API

This guide documents the `scientific-workflow` 0.12.1 subsystem contract.

The `ui` subsystem is the sole presentation interface for execution facts
already known by Runtime. It does not inspect execution units, scientific payloads,
project JSON, or persistence files. Execution units never define display fields, format
messages, increment counters, or receive a UI handle.

Runtime owns the planned-task, lifecycle, progress, path, and outcome fact
vocabulary plus the observer port. Crate-level composition attaches UI's
automatic adapter after Runtime allocates the execution scope. UI alone owns
terminal detection, its inferred refresh policy, the Ratatui dashboard state,
command editing, message history, and the exit request observed through the
Runtime port. Study has no UI dependency.

## Basic API

The default `terminal-ui` Cargo feature compiles the private UI and starts it
automatically through `scientific_workflow::run(&Path)` or
`runtime::execute(Study)`. This feature is enabled by default, so existing
dependency declarations and every visible interactive/noninteractive behavior
remain unchanged.

When both standard input and standard error are terminals, UI enters a
Crossterm alternate screen and renders a Ratatui dashboard containing:

- a task panel containing only the current replicate/phase's tasks in declared
  order; each `PhaseStarted` event replaces the previous phase's rows;
- replicate and phase shown once in the task-panel title rather than repeated
  in every task row;
- pending, running, completed, failed, cancelled, and skipped counts;
- aggregate execution-unit iteration gauges when every `MemberView` target is known;
- execution-unit spinners when the target is unknown;
- one-shot spinners while generic programs and Python tasks are running;
- elapsed time and inferred ETA where enough progress exists;
- the execution output path;
- a bounded 100-line lifecycle/error message panel; and
- the command editor.

The task table itself contains the task label with a concise kind tag, status,
progress, and an `elapsed / ETA` timing column. The internal `execution_unit`
kind is presented as `unit`; configuration and API vocabulary are unchanged.
Inferred task identities, full subjects, and phase prefixes remain available to
lifecycle messages and durable summaries but are not repeated in each row.
Runtime lifecycle lines are appended to the message panel instead of scrolling
the interactive terminal. Scientific payloads are never rendered. When either
standard stream is not interactive, UI deliberately selects its stable
line-oriented standard-error renderer, so redirected runs and CI retain
diagnostics without terminal control sequences. This is a complete UI mode,
not recovery from a broken interactive renderer.

The command editor supports character insertion, Left/Right, Home/End,
Backspace/Delete, Escape to clear, and Enter to submit. Exact lowercase `exit`
(surrounding whitespace allowed) is the only normal way to close the interactive
dashboard. If submitted while work is active, it also requests cooperative
cancellation: Runtime stops admission, asks active execution units to stop between
steps, terminates active external programs, and waits for cleanup before closing.
Ctrl+C requests the same cooperative cancellation while work is active, but does
not close the dashboard; after cleanup the user must still type `exit`. Unknown
commands appear in the message panel.

After Runtime publishes successful, failed, or cancelled execution completion,
the interactive dashboard remains on screen with its command editor active.
`UiSession::finish` waits for an explicit `exit` submission before restoring the
terminal and allowing `runtime::execute` to return. Noninteractive plain rendering
does not wait for input and returns immediately after its terminal lifecycle line.

There is no `ui` object in `wf_configs/study.json`: no refresh rate, theme,
field list, message callback, progress counter, renderer, or cancellation
handle is user-defined. With `terminal-ui` enabled, UI is the sole presentation
interface, so failure to
start its renderer thread, initialize the selected terminal, poll interactive
input, draw the dashboard, or write plain output is fatal and returns
`RuntimeError::Presentation`. Such failures are not reclassified as
cooperative cancellation and are transparently wrapped by `WorkflowError` from
the ordinary crate facade.

## Advanced API

Runtime and UI meet through crate-visible boundaries owned by Runtime:

- `RuntimeEvent` is the borrowed synchronous fact vocabulary;
- `RuntimeObserver` is the downstream publication, cancellation, and final
  join port;
- `RuntimePresentation` is Runtime's clone-cheap adapter handle; and
- `TaskPresentation` publishes iteration/target facts for one inferred task.

UI owns `UiPlan`, `UiSession`, and `UiFailure`. `UiSession` implements the
Runtime-owned observer port and converts no execution outcome itself.

An explicit `default-features = false` build omits the complete `ui` module and
both terminal dependencies. Crate composition then attaches a private silent
observer to the same Runtime port. Execution, persistence, summaries, and
errors remain available, but no lifecycle lines, dashboard, terminal input, or
UI cancellation source exists. This is intended for reader-only and headless
embedding builds; it is not an alternate supported visual interface.

These are peer-subsystem contracts, not downstream API. Event strings and
paths are copied into small UI-owned presentation snapshots as required; UI
never retains an execution unit, `SystemState`, payload, `Study`, recording writer, or
runtime summary. The reducer retains all planned-task status internally but
publishes only the most recently started replicate/phase to the task panel.
Concurrent Runtime workers share one clone-cheap session. A
mutex protects only dashboard presentation state, and a single bounded-refresh
thread owns interactive terminal input and drawing.

Interactive initialization uses a renderer-thread handshake, so setup failure
returns before a usable session is published. Later terminal IO failures are
retained in shared render health and returned from the next Runtime-facing
publication, scheduler cancellation check, or final join. Unexpected renderer
thread panics become the same presentation failure. The terminal lease still
restores raw mode, alternate screen, cursor, and mouse state during ordinary
error return and unwinding.

When fail-fast, timeout, or cancellation prevents admission, phase, replicate,
and execution terminal events close affected `pending` rows as `skipped`.
Task cancellation detail is deliberately source-neutral because the request
may originate from the user, a sibling failure, a deadline, or replicate
policy.

`UiSession::finish` is called internally after the terminal execution event.
For an interactive dashboard it marks execution finished, waits for the renderer
to receive `exit`, and then joins it before Runtime returns. Alternate-screen,
raw-mode, cursor, and mouse state are restored on success, workflow failure,
cancellation, presentation error, or unexpected panic. With plain rendering
there is no renderer to join and no interactive wait.

## Example

The complete user interaction remains one call:

```rust,no_run
use std::path::Path;

fn main() -> Result<(), scientific_workflow::WorkflowError> {
    scientific_workflow::run(Path::new("."))
}
```

Running it in a terminal shows the dashboard. Typing `exit` and pressing Enter
cancels active work or closes an already finished dashboard. Successful completion
otherwise remains visible until that command is submitted. Redirecting standard
error selects plain lifecycle lines automatically and does not wait for input. No
execution unit or JSON change is involved.

## Not API

Ratatui/Crossterm types, the headless silent observer, event variants,
dashboard snapshots, task statuses,
command parser/editor, renderer thread, alternate-screen lease, message
capacity, layout, colors, glyphs, refresh interval, ETA formula, plain-line
format, and cancellation atomics are private. Applications must not parse the
human display as a machine protocol; durable facts belong to Runtime summaries
and persistence metadata.

A replacement UI must implement Runtime's observer port, remain downstream of
Runtime facts, require no Study/execution-unit/config participation, tolerate
concurrent publishers, preserve plain noninteractive diagnostics, support
cooperative `exit`, restore terminal state on return and while unwinding, and
return failure of its selected presentation mode as a fatal presentation error
rather than cancellation or silent degradation.
