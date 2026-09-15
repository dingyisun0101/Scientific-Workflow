# 6. State and recording

**Outcome:** choose what to record and read back verified scientific values.
Use the population model from chapter 3 and the lifecycle explained in chapter 5.

## Declare fields, then initialize typed payloads

A schema declares field names and optional descriptions:

```json
{
  "fields": [
    {"name": "population", "description": "Current population count"},
    {"name": "cumulative_births", "description": "Births since initialization"}
  ]
}
```

Save this as `wf_configs/states/population.json` and retain its named mapping in
`study.json.paths.states`. Rust initialization assigns payload types, such as
`u64`; the schema is not a place to add guessed `dtype` or shape properties.
Initialize payloads through the supplied schema, borrow them through typed state
accessors, and advance `StateTime` after each scientific step.

## Default recording

Without a custom `preflight`, Workflow records all fields into the `state`
stream at initialization and every iteration. The first study produces six
states. Your model does not open files, write chunks, or finalize recordings.
Persistence owns these operations for each exposed member.

## Record a sampled series and a checkpoint

Add this method inside the chapter 3 `ExecutionUnit` implementation:

```rust,ignore
fn preflight(_: &Constants, _: &SystemStateSchema) -> UnitResult<ObservationPlan> {
    Ok(ObservationPlan::streams([
        ObservationStream::fields("population_series", ["population"])?
            .every_iterations(2)?,
        ObservationStream::all_fields("checkpoint")?.initial_and_final(),
    ])?)
}
```

With five steps, `population_series` records iterations 0, 2, 4, and the final
iteration 5. The checkpoint records iterations 0 and 5. An initially complete
member has one boundary record, not two duplicates. Boundary streams require
recording format 8; ordinary periodic recordings use format 7.

Field names must exist and be unique within a selection. Stream names must be
unique, cadence must be positive, and failed members do not gain a successful
final checkpoint. Selecting all fields enables reconstruction of the whole state.

## Read the changed streams

Use the recording path found in chapter 3 with the Python core reader:

```python
from scientific_workflow import open_completed_recording

# Replace this path with the completed member recording from your run.
reader = open_completed_recording("output/EXECUTION/replicate-000000/task-000000")
for record in reader.read_stream("population_series"):
    print(record.iteration, record.values["population"])
checkpoint = reader.read_latest("checkpoint")
print(checkpoint.values["cumulative_births"])
```

Expect population values 10, 12, 14, 15 and final cumulative births 5. The reader
verifies selected chunks before returning the complete stream. Missing files,
invalid metadata, or checksum mismatches are errors, not partial success.

Rust consumers use `StoredStateSeriesReader` with explicit payload decoders;
chapter 8 demonstrates checkpoint handoff. Chapter 9 introduces NumPy output.

## Output and memory

A single-member task is itself the recording directory. Multi-member tasks have
separate `members/member-NNNNNN/` directories. Use dependency selectors in a
pipeline to obtain these paths. See [output layout](../reference/output-layout.md).

`persistence.chunk_target_mb` controls approximate chunk size, and
`queue_capacity_mb` bounds each stream's queued encoded data. A full queue applies
backpressure; these values do not cap total application memory. The model's
payloads, conversion, and analysis also consume memory.

See [State](../../rust/src/state/api.md), [Observation](../../rust/src/observation/api.md),
and [Persistence](../../rust/src/persistence/api.md) for detailed contracts.

---

**Previous:** [5. Scientific models](5-scientific-models.md) · **Guide index:** [Documentation](../README.md)

**Next:** [7. Parameter sweeps and replicates](7-parameter-sweeps-and-replicates.md)
