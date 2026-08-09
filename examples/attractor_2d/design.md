# `attractor_2d` example design

## Status

The approved repository-level home is `examples/attractor_2d`. Configuration,
state assembly, evolution, sampling, recording completion, typed readback,
analysis, and round-trip verification are implemented.

The local library baseline is ready for this work: formatting, 11 integration
tests, 6 doctests, warnings-denied Clippy and rustdoc, documentation generation,
and isolated package verification all pass. Example implementation can
therefore treat a later failure as a change-specific regression rather than an
unknown pre-existing crate failure.

## Scientific model

The finalized model is the supercritical Hopf normal form, also called the
Stuart–Landau oscillator in its equivalent complex representation:

```text
dx/dt = mu * x - omega * y - (x^2 + y^2) * x
dy/dt = omega * x + mu * y - (x^2 + y^2) * y
```

In polar coordinates its behavior is transparent:

```text
dr/dt     = mu * r - r^3
dtheta/dt = omega
```

For `mu < 0`, trajectories converge to the stable origin. For `mu > 0`, the
origin is unstable and trajectories converge to a stable circular limit cycle
of radius `sqrt(mu)`. Sweeping `mu` across zero therefore demonstrates a real
bifurcation with dynamics that remain easy to understand and verify.

The evolution method is fixed-step explicit Euler. Each step copies the old
`x` and `y` scalars from the state-owned point, evaluates both derivatives from
that same old time point, writes `x + dt * dx` and `y + dt * dy` back in place,
and advances the state's integer and physical time. No numerical dependency or
intermediate state allocation is required.

The planned demonstration values are `omega = 1.0`, `dt = 0.01`, initial point
`[0.25, 0.0]`, and a `mu` sweep of `[-0.25, 0.25, 1.0]`. These values keep
explicit Euler well behaved while producing one decaying trajectory and two
stable limit cycles with different radii.

## Purpose

`attractor_2d` will be the first full-stack example for `scientific-workflow`.
It must read like a small scientific project rather than an API catalogue. One
execution will demonstrate configuration loading, parameter-sweep expansion,
directly owned mutable system state, asynchronous chunked recording, typed
readback, and time-series analysis.

The example will use only the crate's public prelude and the Rust standard
library. Test fixtures and crate-private implementation details are outside its
boundary.

## Directory layout

```text
<repository>/examples/attractor_2d/
├── Cargo.toml
├── Cargo.lock
├── README.md
├── design.md
├── todo.md
├── config/
│   ├── fixed.json
│   ├── sweep.json
│   ├── paths.json
│   └── state.json
└── src/
    └── main.rs
```

The example is a standalone downstream crate. From the repository root it will
run with:

```bash
cargo run --manifest-path examples/attractor_2d/Cargo.toml
```

Its manifest will declare
`scientific-workflow = { version = "0.1.0", path = "../../dev" }`, preserving a
meaningful compatibility constraint while using the local library during joint
development. It will not be a member of a new root workspace unless a later
repository-wide design explicitly adopts one.
As an executable application, the example will commit its generated
`Cargo.lock` for reproducible builds.

The standalone manifest is now implemented with package name `attractor-2d`,
Rust 2024, minimum Rust 1.85, publication disabled, automatic binary discovery
disabled, and one explicit `src/main.rs` binary target. The only application
dependency is the versioned local path to `scientific-workflow`; neither
`ndarray` nor PiP is required by this two-scalar model.

## Configuration contract

`config/fixed.json` contains common scientific and operational
values:

- model name;
- initial two-coordinate point;
- iteration count;
- explicit-Euler timestep;
- fixed angular frequency `omega`;
- trajectory sampling cadence;
- radius sampling cadence;
- checkpoint cadence;
- maximum chunk bytes; and
- bounded writer queue bytes.

The finalized fixed values are:

| Key | Value | Role |
|---|---:|---|
| `model_name` | `"supercritical_hopf_normal_form"` | Stable model identity for logs and recording metadata |
| `initial_point` | `[0.25, 0.0]` | Nonzero initial condition shared by every task |
| `angular_frequency` | `1.0` | Fixed angular velocity `omega` |
| `physical_time_step` | `0.01` | Explicit-Euler step size |
| `total_steps` | `5000` | Evolves each task through physical time `50.0` |
| `trajectory_sample_every_steps` | `10` | Samples the point every `0.1` physical-time units |
| `radius_sample_every_steps` | `5` | Samples the scalar diagnostic every `0.05` physical-time units |
| `checkpoint_every_steps` | `1000` | Samples restart state every `10.0` physical-time units |
| `maximum_chunk_bytes` | `8192` | Small rollover target that visibly exercises trajectory chunking |
| `writer_queue_bytes` | `65536` | Finite asynchronous queue budget comfortably larger than one record |

These values are operational defaults for the example, not universal
recommendations. Model-specific validation in the executable will require a
two-element finite initial point, finite positive timestep, positive step and
cadence counts, and nonzero storage byte limits.

`config/sweep.json` uses Cartesian mode with one `mu` axis containing
`[-0.25, 0.25, 1.0]` in that order. It therefore resolves exactly three tasks:
a stable-origin regime followed by limit cycles with expected radii `0.5` and
`1.0`. Task order is the deterministic source order produced by
`ParameterSpace`.

`config/paths.json` names `state_template` as `config/state.json` and
`recording_root` as `target/recordings`. The standalone crate root is also the
project root supplied to
`ProjectConfig::load`; there is no redundant nested `project/` directory.
Configured paths are resolved relative to this root. The
generated root will be beneath the standalone project's ignored
`target/recordings` area so running the example does not dirty tracked source.

The example will load the three files through `ProjectConfig`, retrieve values
through `TaskParameters`, and resolve named paths through `ProjectPaths`.

## System-state contract

`config/state.json` declares two fields in canonical order:

1. `point`: the evolving two-coordinate phase-space value;
2. `radius`: the synchronized radial-amplitude diagnostic.

The template remains language-neutral and records no Rust type names. State
assembly binds `point` to `Vec<f64>` and `radius` to `f64` on first insertion.

Its location alongside the other declarative project inputs does not merge
module responsibilities:
`ProjectPaths` resolves its name, and `SystemStateSchema` validates its content.
One current limitation remains explicit: `ProjectConfig::write_source_config`
round-trips only the three standard configuration files and does not copy the
state template. Extending that export contract, if desired, is separate crate
work and is not required for loading or running this example.

Every sweep task will create a new empty state from one shared
`SystemStateSchema`, move the initial point payload into it, and retain direct
ownership of that state for the simulation lifetime. The immutable model name
remains in fixed configuration and recording user metadata. The simulation
will mutate `point` in place and advance both the integer step and physical
time after each Euler step.

### State-content decision

The complete runnable state contains exactly:

- built-in `SimulationTime`, holding the authoritative integer step and
  physical time; and
- `point: Vec<f64>`, containing the evolving coordinates `[x, y]`; and
- `radius: f64`, containing the current radial amplitude
  `sqrt(x * x + y * y)`.

`radius` is intentionally retained even though it is derivable from `point`.
For the Hopf normal form it is the primary scientific diagnostic: it decays to
zero for `mu < 0` and approaches `sqrt(mu)` for `mu > 0`. Retaining one scalar
allows an inexpensive high-cadence diagnostic stream, demonstrates
heterogeneous state payloads, and provides an invariant that can be checked
against `point` after every update and during later readback.

No other value belongs in the state. `mu`, `omega`, and `physical_time_step`
are immutable task parameters; model identity, cadence, and storage budgets are
configuration; task identity belongs in recording metadata; derivatives are
temporary locals; and explicit Euler has no hidden integrator history. A
restart combines the recorded payloads and time with the task parameters
identified by metadata.

Keeping `[x, y]` in one payload is deliberate. The coordinates form one coupled
numerical value, are always mutated together, and are both required to
interpret the phase trajectory. `point` and `radius` will be borrowed together
through the tuple-mutation API for one coordinated Euler update: derivatives
are evaluated from the old point, the new point is assigned, and radius is
recomputed from that new point before simulation time advances.

### Recording-content decision

Three streams now have distinct scientific and operational roles:

- `trajectory` records only `point` every 10 steps for phase-space output;
- `radius` records only the scalar `radius` every 5 steps for a lean,
  higher-cadence convergence diagnostic; and
- `checkpoint` records both `point` and `radius` every 1000 steps, forming the
  complete state required for direct restart.

All streams include step 0 and step 5000. The resulting expected counts are 501
trajectory records, 1001 radius records, and 6 checkpoint records per task.
Neither partial stream alone is a complete `SystemState`; the checkpoint is.

The approved configuration retains `checkpoint_every_steps`, uses
`trajectory_sample_every_steps = 10`, and defines
`radius_sample_every_steps = 5`.

## Recording contract

Every parameter task owns one independent `SystemStateWriter` and output
directory. The directory name will include the task index and a per-execution
identifier, preventing accidental overwrite on repeated runs.

The writer exposes the three streams defined above: partial `trajectory` and
`radius` streams at independent cadences, plus the complete `checkpoint`
stream.

Each stream stores a typed nonzero step cadence decoded from `fixed.json`. The
model loop offers its state after every step; the writer checks time and returns
without payload access for non-due streams. Due streams serialize borrowed
payloads, block when the finite byte queue is full, and keep records whole
across chunk boundaries. Completion receives the final state and records it
only for streams that did not already accept that step. Time-axis metadata
identifies the iteration index and physical time, while `every_steps` persists
the machine-readable cadence once per stream.

## Readback and analysis contract

After a task recording is complete, the example creates a
`JsonPayloadDecoderRegistry` and registers:

- `JsonVecF64Decoder` for `point`; and
- a scalar `f64` decoder for `radius`.

The scalar decoder is example-local and demonstrates the public extension
contract without enlarging the library's small default-decoder set.

`StoredStateSeriesReader` reconstructs all three streams. Analysis uses owned
`StateSeries` values to report:

- sample count;
- first and last simulation coordinates;
- minimum and maximum `x` and `y` values;
- final sampled point; and
- a compact terminal ASCII scatter plot of the reconstructed trajectory.

The example explicitly compares each reconstructed final sample with the
corresponding sample observed by the live simulation. Final times and payloads
must be bitwise equal or the application returns an error. The library enables
Serde JSON's `float_roundtrip` feature because the first trial demonstrated
that the default fast float parser could differ by one ULP even when the
stored decimal uniquely represented the original `f64`.

## Console-output contract

Output will be concise, deterministic apart from the generated recording path,
and grouped by stable labels:

```text
[project] model=... tasks=... iterations=...
[task] index=... mu=... omega=... dt=...
[simulation] task=... samples=... final_point=...
[storage] task=... recording=... streams=... complete=true
[analysis] task=... samples=... bounds=... final_point=...
[plot] task=... legend=S:start,E:end,*:sample
[verify] task=... round_trip=true
[result] attractor_2d=complete output_root=...
```

## Coverage boundary

The example covers the primary successful workflow of every major public
submodule: configuration, system state, storage, and time series. It will not
manufacture every error variant or call accessors solely to claim method-level
coverage. Exhaustive behavior and failure coverage remain the responsibility
of integration tests.

The example will not add plotting dependencies, Python code, a simulator
framework, custom payload decoders, or compatibility layers. Its ASCII plot is
derived from the reconstructed Rust time series. The project is retained in
the Git repository but is not included in the library crate's published
archive; its build, lint, test, and run checks must target its own manifest.

## Reusable implementation sequence

This example is also the reference procedure for adopting the crate in a
general scientific project:

1. specify the evolution equation, independent-task boundary, evolving values,
   parameters, observations, and restart state;
2. encode fixed values, sweep dimensions, and named paths in the three project
   configuration files and inspect the resolved task set;
3. declare stable state-field names in the JSON template and select their Rust
   payload types in state-assembly code;
4. implement one function that validates a resolved task and returns a complete
   owned `SystemState`;
5. implement and verify the deterministic evolution kernel against one state,
   before connecting storage or executing multiple tasks;
6. define observation and checkpoint streams by selected fields and cadence;
7. create the task writer with complete metadata and bounded storage settings
   before the first evolution step;
8. evolve the state, submit borrowed samples when due, and advance simulation
   and physical time;
9. explicitly complete successful recordings or mark failures;
10. register field-specific payload decoders, reconstruct `StateSeries` values,
    and analyze them through borrowed views;
11. verify a live sampled value against its reconstructed counterpart;
12. expand the proven single-task path to all sweep tasks, retaining one state,
    writer, and output directory per task; and
13. add restart as a separate entry path through a complete checkpoint stream,
    then reuse the same evolution loop.

The example itself will be implemented in that order. This prevents storage
and task orchestration from obscuring errors in the scientific kernel.

The end-user tutorial for this procedure is maintained in `steps.md`. This
design document remains the example's architectural contract, while `steps.md`
explains the sequence, rationale, expected artifacts, and readiness criteria in
a form intended for users learning the crate.

## Evolution-phase capability audit

No crate capability blocks implementation through state evolution and sample
submission. The current public API supports the complete required path:

```text
load ProjectConfig -> decode one TaskParameters selection
-> load shared SystemStateSchema -> move initial payload into SystemState
-> mutate payload in place -> advance step and physical time
-> borrow state for stream encoding -> queue owned bytes with backpressure
-> explicitly complete or fail the recording
```

The checked-in example inputs already satisfy model-specific relationships.
The minimal application therefore demonstrates typed decoding and lets the
crate APIs enforce their own contracts instead of adding a second validation
framework. Production projects may add domain validation and richer output
allocation policies without changing the core ownership flow.

Writer user metadata accepts a JSON map rather than `TaskParameters` directly.
The example can collect the task's iterator into that map by cloning only small
configuration `Value` objects. A future convenience conversion may reduce
boilerplate, but it is not needed for correct or efficient evolution.

### Finalized state boundary

The state has two payloads: `point: Vec<f64>` and `radius: f64`. The model name
remains fixed configuration and recording metadata. The checkpoint stream is
complete only when it records both payloads. `JsonStringDecoder` is deliberately
absent because no scientifically meaningful string payload evolves.

### Payload representation

`point` uses the standard `Vec<f64>` representation for this example:

- two coordinates do not benefit from tensor rank or backend machinery;
- JSON remains the lean array `[x, y]` rather than repeating a tensor kind and
  shape in every record;
- the crate already supplies `JsonVecF64Decoder` for deferred readback; and
- a new user needs no numerical-container dependency to understand the first
  workflow.

An `ndarray::Array1<f64>` would add a dependency and require Serde feature
configuration without improving this two-element kernel. A PiP dense tensor is
fully compatible and is the preferred representation when shape, rank,
high-dimensional operations, or other PiP algorithms are scientifically
meaningful. PiP is deliberately reserved for a later tensor-scale example
rather than used ornamentally here.

The payload decision is final for this example. The complete input set has also
been exercised through the crate's public loaders: 10 fixed parameters, one
swept parameter, three ordered tasks, two resolved paths, and two canonical
state fields all load and decode consistently.

The root `README.md` is the end-user entry point. It documents the model,
configuration, payload ownership, Euler update, three streams, run command
and output, typed analysis, and verification boundary.

## Implemented full workflow

The application uses only the public crate prelude and standard library. It
decodes all three tasks, moves each initial point into an owned state, updates
`point` and `radius` through one tuple borrow, advances physical time, and
samples endpoint-inclusive cadences. One writer per task owns all three streams
and is explicitly completed. After completion, all streams are reconstructed
and analyzed through typed decoders.

Two real executions pass. Every task completes with exactly 501 trajectory,
1001 radius, and 6 checkpoint records; authoritative metadata confirms all
counts and terminal status. The second execution creates a distinct run
directory, proving non-overwrite behavior. Formatting, warnings-denied Clippy,
and the standalone Cargo test target pass.

Repository/package separation is also verified. The library's crates.io
package listing contains 44 crate-owned files and excludes this standalone
project, its executable lockfile, and all generated recordings. The repository
and crate READMEs now direct users to the standalone manifest without implying
that it is a Cargo example embedded in the published library.

## Executable source boundaries

`main.rs` is a process boundary, not the implementation container. Its only
responsibilities are to declare the application's modules, sequence their
top-level operations, print bounded phase results, report one terminal error,
and select the process exit status. Scientific rules, configuration decoding,
state mutation, persistence, and analysis implementations do not belong in
`main.rs`.

The normalized source layout is:

```text
src/
├── main.rs               # end-to-end orchestration and terminal errors
├── project_setup.rs      # configuration loading and typed task setup
├── hopf_model.rs         # sole state owner and Hopf evolution kernel
├── state_recording.rs    # sample streams, writer setup, and recording
└── recording_analysis.rs # typed readback, metrics, plot, and verification
```

Application modules follow one naming convention: a descriptive snake-case
noun or compound noun naming the responsibility or primary domain object.
Generic process labels and ambiguous repository terms are avoided. Compound
names are preferred when they prevent confusion with similarly named library
modules; for example, `state_recording` is the application sampling layer,
while the library owns the general `storage` module.

`main.rs` retains Rust's conventional binary entry-point name. File names do
not repeat `attractor_2d`, because the crate already supplies that namespace.

`main.rs` is the application orchestrator. It composes the other modules
without absorbing their implementation details: load the project, allocate a
fresh timestamped execution directory, simulate and record
each task, analyze the completed recording, verify the round trip, and print
the bounded result summary. It contains no scientific kernel, decoder,
configuration parser, writer-construction details, or reusable data model.

The modules exchange small application-level values with explicit ownership.
`project_setup.rs` produces decoded task plans; `hopf_model.rs` owns and mutates
each live `SystemState`; `state_recording.rs` borrows that state only long
enough to encode due samples; and `recording_analysis.rs` receives the
completed recording path plus an immutable borrow of the same completed model.
None of these boundaries clones scientific payload allocations or creates a
second authoritative representation merely to cross a module boundary.

`HopfModel` is the only application structure that owns the live
`SystemState`. A separate `FinalState` snapshot is forbidden even when it
contains only copied scalars: it duplicates scientific truth, can drift from
the state, and makes verification appear to compare against a second model
rather than the simulation's actual final state. Recording returns only
storage facts such as admitted sample counts and the recording directory.
After completion, logging and analysis borrow the unchanged state directly
from `HopfModel`; they never extract, clone, or mirror its payloads into a
second result structure.

The name deliberately omits `Task`: task identity and swept constants belong
to `TaskPlan` and `TaskSettings`, while `HopfModel` represents the scientific
system itself. It also avoids the generic `AttractorModel`, which would hide
the exact equation family demonstrated by this project. In another scientific
project, the corresponding state-owning type should use that project's domain
name rather than inherit a framework-level `TaskSimulation` abstraction.

This is deliberately a minimal happy-path example. It retains natural `?`
propagation from crate operations and explicit assertions for the demonstrated
round trip, but omits redundant schema checks, finite-value checks, expected
sample-count arithmetic, collision retry loops, and layered error rewriting.
Those concerns belong in production applications or the library's dedicated
failure-oriented integration tests, not in the shortest path that teaches
configuration, ownership, evolution, recording, and readback.

The modular refactor and typed analysis are complete. Formatting,
warnings-denied Clippy, tests, and a real three-task execution all pass. Every
task reconstructs 501 trajectory, 1001 radius, and 6 checkpoint states and
reports exact round-trip verification.

The first complete trial initially stopped at task 1 because the built-in
vector decoder recovered one negative coordinate one ULP away from its live
value. Inspection proved the emitted decimal represented the original bits;
the discrepancy came from Serde JSON's default fast float parser. Enabling the
library dependency's `float_roundtrip` feature corrected the decoding path.
The rerun completed tasks `mu = -0.25`, `0.25`, and `1.0`, reported final radii
approximately `0.0000010521`, `0.50497537`, and `1.00249695`, rendered all
three phase portraits, and returned `round_trip=true` for every task. The
library integration suite now retains the sensitive coordinate as an exact-bit
regression case.

## Naive scientific reference

`validation/naive_hopf.rs` is the independent numerical reference. It is a
single standard-library Rust file with hard-coded constants, one `[f64; 2]`, a
plain Euler loop, and final-value printing. It does not load configuration or
use any `scientific-workflow` type, serialization, storage, decoder, or
analysis interface. Compiling it directly with `rustc` keeps the comparison
outside the example application's dependency graph.

Validation compares the workflow and reference by task index. The final step
is compared as an integer; physical time, both coordinates, and radius are
parsed from each program's shortest round-trippable decimal output and compared
as IEEE-754 bit patterns. Tasks `mu = -0.25`, `0.25`, and `1.0` all match in
every compared field. This validates that the workflow abstractions preserve
the naive kernel's final numerical result; it does not attempt to duplicate or
validate the workflow's storage behavior.
