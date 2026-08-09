# Building a scientific project with `scientific-workflow`

This guide presents a reliable order for building a scientific application
with `scientific-workflow`. The `attractor_2d` project follows the same order,
but the procedure applies equally to simulations, numerical experiments,
parameter studies, and other state-evolving calculations.

The sequence is intentional. First prove that one scientific task evolves
correctly. Then prove that it can be recorded and reconstructed. Only then
scale it across a parameter sweep or add restart behavior. This keeps errors in
the numerical model, configuration, storage, and orchestration distinguishable.

## The project model

A project has three different kinds of information:

- **Configuration** describes values that are fixed across tasks, values that
  vary between tasks, and named filesystem paths.
- **System state** contains the typed, mutable values that evolve during one
  task.
- **Recorded series** contain sampled states reconstructed for analysis after
  execution.

One resolved parameter combination is one independent **task**. Each running
task owns its own `SystemState`, `SystemStateWriter`, and recording directory.
The model offers its current state after every evolution step; the writer owns
the configured stream sampling intervals and decides whether to encode, queue, chunk, or
ignore each observation.

The normal runtime flow is:

```text
project configuration
        |
        v
resolved task + state schema
        |
        v
owned mutable SystemState
        |
        v
evolve -> sample -> writer -> recording chunks
                              |
                              v
                    reader + payload decoders
                              |
                              v
                         StateSeries
                              |
                              v
                           analysis
```

## Step 1: Define the scientific contract

Before writing application code, identify:

- the equations or transition rule;
- values that evolve during execution;
- values shared by every task;
- values varied by the parameter study;
- what constitutes one independent task;
- which observations are needed for analysis; and
- which complete values are required to restart a task.

Do not put immutable parameters into the evolving state merely because the
evolution function needs them. Conversely, do not treat an evolving value as a
parameter merely because it has an initial value.

**Ready when:** every value is classified as state, fixed configuration, swept
configuration, path configuration, or derived local data.

## Step 2: Define project configuration

Create a project `config` directory containing:

```text
config/
├── fixed.json
├── sweep.json
├── paths.json
└── state.json
```

Use `fixed.json` for constants shared by all tasks. Use `sweep.json` for either
Cartesian axes or explicitly correlated cases. Use `paths.json` for named paths
instead of scattering path literals through the program.

Load the directory through `ProjectConfig`. Inspect the task count and several
resolved `TaskParameters` values before implementing the numerical model. This
is where incorrect sweep dimensions, parameter names, and JSON types should be
found.

Keep operational values such as sampling intervals and storage budgets in
configuration when users are expected to tune them between runs.

**Ready when:** the program can load the project, enumerate tasks in the
expected deterministic order, decode every required parameter, and resolve all
named paths.

## Step 3: Define the state schema

Create `config/state.json` containing stable field names and useful natural
language descriptions. The template describes the shape of a state, not its
Rust payload types. Concrete types are established when application code
inserts values into a newly created state.

Choose fields according to scientific meaning and sampling needs. A field
should be independently addressable when it has a distinct payload type,
mutation pattern, sampling interval, or analysis role.

Load the template once as a `SystemStateSchema`. Its clones are lightweight
shared schema handles, so all states and storage components for a project can
refer to the same schema instance.

**Ready when:** every evolving or recordable value has exactly one stable key,
and every field needed for a complete restart has been identified.

## Step 4: Assemble one complete state

Write one state-assembly function whose inputs are the shared schema and one
resolved task. It should:

1. decode and validate task parameters;
2. create an empty state at the initial simulation time;
3. construct each typed payload;
4. transfer each payload into its declared field; and
5. return a complete owned state or a descriptive error.

Large payloads should be constructed once and moved into the state. During
evolution, retrieve payloads by their concrete Rust type. Use coordinated tuple
borrows when an operation genuinely needs mutable access to multiple distinct
fields; otherwise mutate one payload at a time for clarity.

**Ready when:** one resolved task consistently produces a populated state with
the expected payload types, dimensions, and initial time.

## Step 5: Implement the evolution kernel

Implement the smallest function that advances the scientific state by one
step. It should depend on the owned state and explicit task parameters, not on
the storage writer or parameter-space iterator.

When several next values depend on the same old state, read the required old
values first, calculate the transition, and then write the new values. Advance
the iteration and, when applicable, physical time only after a
successful transition.

Run this kernel for one task without recording. Check known values,
conservation laws, bounds, or other model-specific invariants.

**Ready when:** a single task evolves deterministically and scientific
correctness can be evaluated without involving I/O.

## Step 6: Design sample streams

Group recorded fields by purpose and sampling interval. Common stream roles include:

- frequent, partial observations for analysis;
- infrequent, complete checkpoints for restart; and
- specialized diagnostics sampled on their own interval.

Every stream selects exact schema keys. A checkpoint intended for restart must
contain every payload required to reconstruct a complete runnable state.

Choose a target maximum chunk size rather than a number of states per chunk.
The writer estimates chunk rollover from encoded record bytes and never splits
one sampled state across chunks. Choose a finite queue-byte budget that bounds
memory use; reaching that budget deliberately blocks simulation submission
until storage catches up.

**Ready when:** every stream has a name, field set, sampling interval, chunk-byte target,
queue-byte budget, and documented scientific purpose.

## Step 7: Create the task writer

Create the writer before the first evolution step. Configure it with:

- the task's unique recording directory;
- the shared state schema;
- time-axis names and units;
- the resolved task index and parameters as user metadata; and
- all stream definitions.

Starting the writer publishes the recording metadata before sample data is
accepted. Never let two tasks share one recording directory.

The application should refuse accidental overwrite. Generate task directories
from a collision-free execution identifier and stable task index rather than
deleting an existing recording automatically.

**Ready when:** a new recording starts with complete metadata and every stream
is validated against the state schema.

## Step 8: Offer each evolved state to the writer

The writer owns the typed sampling interval configured for every stream. A typical loop
has this shape:

```text
for each iteration:
    evolve the owned state
    advance its simulation time
    writer observes the current state
```

Observation first inspects simulation time. Non-due streams do not look up or
borrow payloads. Due streams borrow selected payloads only for encoding; the
live state remains owned by the model and continues evolving after observation
returns. Encoded records move into the writer's bounded queue. Submission can
block under backpressure; this protects memory when storage is slower than the
simulation.

Observe the initial state before evolution. Complete the recording with a
borrow of the final state; the writer adds it only when that stream did not
already record the same iteration through normal sampling.

**Ready when:** expected sample counts and sample times can be calculated before
the run and match the produced recording.

## Step 9: Close the recording lifecycle

On success, explicitly complete the recording. Completion drains queued data,
seals remaining chunks, and records a completed lifecycle in metadata.

If the task fails after recording starts, mark the recording failed with useful
context. Dropping a writer protects memory and file safety, but it does not mean
that the scientific task completed successfully.

**Ready when:** every exit path classifies the recording as complete, failed, or
intentionally still running for later recovery.

## Step 10: Register payload decoders

Stored JSON identifies fields by schema key, while concrete Rust payload types
remain application knowledge. Create a `JsonPayloadDecoderRegistry` and
register one decoder for every field selected by the stream being read.

Use the built-in decoders for supported common types. Implement a custom
`JsonPayloadDecoder` when a project owns a domain-specific serializable type.
Registration is per field, so two fields may use different decoders even when
their serialized JSON shapes look similar.

**Ready when:** decoder coverage exactly matches the selected fields and each
decoder returns the type expected by downstream analysis.

## Step 11: Reconstruct and analyze series

Open a completed recording with `StoredStateSeriesReader` and reconstruct the
desired stream as a `StateSeries`. The reader validates recording structure and
recreates typed states using the registered decoders.

Use an owned `StateSeries` when states must be retained or consumed. Use
`StateSeriesView` for borrowed traversal and analysis. Keep analysis code
separate from the simulation loop so it can also operate on recordings produced
by earlier executions.

Compare at least one reconstructed sample with a value retained or logged by
the live simulation. This explicit round-trip check catches incorrect field
selection, sampling-interval assumptions, and decoder registration.

**Ready when:** recorded states reconstruct with the correct types, times, and
values, and the analysis consumes only the data it actually needs.

## Step 12: Expand to the parameter sweep

Only after one task completes the entire evolution-to-analysis round trip
should the application execute every resolved task.

For each task, create a separate:

- resolved parameter handle;
- owned state;
- writer and recording directory; and
- success or failure result.

Begin with sequential execution. Add task-level parallelism only when it is
useful and measured. Parallel tasks still retain independent writers so their
queues, storage rates, and failures remain isolated.

**Ready when:** task identity maps unambiguously to parameters, metadata,
recording directory, and final result.

## Step 13: Add restart as a separate entry path

Restart is not part of the initial happy path. Add it after normal recording and
readback are reliable.

A restart-capable project must have a complete checkpoint stream and decoders
for every checkpoint field. The restart path should:

1. load the same project configuration and schema;
2. open the existing running recording for continuation;
3. reconstruct the latest valid complete checkpoint state;
4. verify its task identity and simulation time;
5. continue with the same evolution kernel and sampling-interval rules; and
6. explicitly complete or fail the continued recording.

Sealed chunks are treated as committed. Recovery examines the latest unsealed
chunk and resumes only from its last valid complete record.

**Ready when:** interrupting and resuming a task produces the same scientific
result and valid recording structure as uninterrupted execution.

## Final project checklist

Before treating a scientific project as ready:

- configuration round trips exactly and expands to the expected tasks;
- state assembly validates all types and dimensions;
- the evolution kernel is tested independently of storage;
- sample counts and endpoints are explicit;
- checkpoint streams contain a complete restart state;
- chunk and queue byte budgets are intentional;
- every writer lifecycle is closed explicitly;
- decoder coverage is complete;
- at least one live-to-recorded round trip is verified;
- generated output cannot overwrite prior runs accidentally; and
- one-task correctness is established before parallel sweep execution.
