# Scientific Workflow

> Warning: this crate is test software. API shape and guarantees are unstable.
> Treat all releases as potentially breaking until a stable line is announced and
> update downstream integrations together with each bump.

The Rust crate lives in [`rust/`](rust/). Its complete public overview is in
[`rust/README.md`](rust/README.md), and the target architecture is in
[`docs/architecture.md`](docs/architecture.md).

The current registry release is consumed with:

```toml
[dependencies]
scientific-workflow = "0.10.0"
```

See the [0.9 migration checklist](rust/README.md#migrating-from-09) before
updating an existing application.

## Vocabulary

The orchestration hierarchy is:

```text
Study → Phase → Task → workload
```

- `Study` is the largest scope and owns scheduling, display, cancellation,
  `StudyPlan`, `StudyRecord`, and `StudySummary`.
- `Phase` owns tasks plus concurrency, delay, timeout, dependency, failure, and
  optional whole-phase completion-examination policies.
- `Task` is every registerable workload. Progress and one-shot work are modes
  of the same task type.
- `TaskContext` is the sole task-to-study communication boundary.

Configuration is independent:

```text
study.json
→ StudySettings
├── replicate_settings → ReplicateExecutor → output_root/replicate_<index>
└── application → application-owned typed settings

parameters.json
→ StudyConfiguration
→ selected WorkloadConfiguration
→ ResolvedConfiguration combinations
→ application-defined Tasks
```

Workflow owns the one-subprocess-per-replicate boundary. Applications own study
paths, schemas, model inputs, recordings, artifacts, networking, and any
domain-specific subprocesses started by tasks.

Workflow completion examination is intentionally whole-phase only. A verified
complete phase is reused; an incomplete phase is invoked normally with an
explicit warning that validation and continuation within the phase remain
application-owned.

## Terminal display

The study renderer owns all terminal writes and preserves the established task
progress bar. The display separates study, phase, task, message, and command
sections. The command input module currently accepts one command:

```text
exit
```

It requests cooperative study cancellation.

## Attractor example

[`examples/attractor_2d`](examples/attractor_2d) demonstrates the complete
boundary. The application loads its single-replicate `StudySettings`, enters
the isolated output scope, loads `StudyConfiguration`, selects the dynamics
and validation workload configurations, maps each
`ResolvedConfiguration` into simulation and validation tasks, builds phases,
and runs one study. A final one-shot task uses Python to render the verified
trajectories. Each task owns its scientific state, recording I/O, or derived
visualization output.

```bash
mamba run -n DSES cargo run --manifest-path examples/attractor_2d/Cargo.toml
```

Use the maintained `DSES` Mamba environment: the example's final phase invokes
Matplotlib through `mamba run -n DSES`.

## Storage integrity

Every sealed recording chunk carries a SHA-256 checksum. Completed-recording
read paths always validate lifecycle, framing, schema, byte count, and digest
before treating a selected chunk as scientific output.

The official Python reader in [`python`](python) follows the same format and
integrity rules as Rust's `StoredStateSeriesReader`.

## Tests

Run the Rust suite with:

```bash
cargo test --all-targets --manifest-path rust/Cargo.toml
```

The suite covers study scheduling and rendering, configuration expansion,
state ownership, writer inference and encoding, analysis series, storage resilience and continuation,
artifact integrity, RNG records, and Rust/Python format conformance. See
[`docs/tests.md`](docs/tests.md) for the responsibility-oriented test map.
