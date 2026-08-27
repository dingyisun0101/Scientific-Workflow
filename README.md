# Scientific Workflow

Scientific Workflow turns registered Rust scientific models, arbitrary
executable programs, and declarative JSON into validated, recorded studies.

The user workflow is intentionally limited to:

1. implement/register each stateful Rust model and provide any standalone task
   programs or Python scripts;
2. write `study.json` and all project JSON beneath `config/`; and
3. call `scientific_workflow::run(project_root)`.

```rust
fn main() -> Result<(), scientific_workflow::WorkflowError> {
    scientific_workflow::run(std::path::Path::new("."))
}
```

Users do not construct tasks, phases, studies, runtimes, output paths,
recording sessions, progress counters, or messages. A task is either one
registered model plus one resolved model-input combination, one executable
declared directly in `study.json`, or one `.py` script with its environment
declared inside the task's `python` object. Neither program form needs a Rust
wrapper.

```text
registered models and/or programs + project JSON
    → crate-level run(&Path) facade
    → one central immutable parse of all JSON requested by Study
    → immutable Study binding and preflight
    → Study-owned effective persistence plan
    → runtime::execute(Study)
    → automatic scheduling, persistence, and terminal UI
```

The Rust crate lives in [`rust/`](rust/). Start with its
[user guide](rust/README.md), then use the complete
[architecture](docs/architecture.md), [test map](docs/tests.md), and each
subsystem's `src/<module>/api.md` for exhaustive contracts.

The [attractor example](examples/attractor_2d) demonstrates the final workflow:
its Rust executable is one `run(&Path)` call, while its model owns state and a
custom observation plan and its JSON owns study organization and constants sweeps.
Persistence construction, submission, finalization, and shutdown are entirely
internal. Interactive progress is inferred from Runtime facts and requires no
model callbacks or JSON settings.

> This crate is pre-1.0 test software. Public API behavior may change through
> coordinated refactor releases.
