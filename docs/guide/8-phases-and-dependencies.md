# 8. Phases and dependencies

**Outcome:** connect an upstream recording to a downstream scientific task.
You should understand the model lifecycle and parameter expansion first.

## Declare ordering and data requirements

A phase's `after` list declares prerequisite phases. Dependencies must exist,
be unique, and form an acyclic graph. A downstream task uses selectors to choose
the intended upstream result; ordering alone does not choose a member.

The bundled dependency pipeline is a complete example:

```text
initialize → simulate → $npy → analysis
    7       7,8,9,10,11,12       summary.json
```

Its [Rust implementation](../../examples/dependency_pipeline/src/main.rs),
[parameters](../../examples/dependency_pipeline/wf_configs/parameters.json),
and [state schema](../../examples/dependency_pipeline/wf_configs/states/value.json)
work with this complete `study.json`:

```json
{
  "active_phases": [
    0,
    1,
    2,
    3
  ],
  "workflow_schema": 1,
  "threads": 2,
  "paths": {
    "states": {
      "value": "wf_configs/states/value.json"
    }
  },
  "phases": {
    "initialize": {
      "tasks": [
        {
          "execution_unit": "initialize",
          "state": "value",
          "resources": {
            "threads": 1
          }
        }
      ]
    },
    "simulate": {
      "after": [
        "initialize"
      ],
      "tasks": [
        {
          "execution_unit": "simulation",
          "state": "value",
          "resources": {
            "threads": 1
          }
        }
      ]
    },
    "$npy": {
      "after": [
        "simulate"
      ]
    },
    "analysis": {
      "after": [
        "$npy"
      ],
      "tasks": [
        {
          "python": {
            "script": "scripts/analyze.py",
            "environment": {
              "manager": "system"
            }
          }
        }
      ]
    }
  },
  "compute": {
    "mode": "isolated"
  }
}
```

The initialize unit begins already complete and records one boundary checkpoint.
The simulation starts a new state from that checkpoint, then advances five steps.
The isolated resource policy assigns one thread per execution unit.

## Acquire a typed prerequisite

Inside the simulation's `initialize`, use the supplied context:

```rust,ignore
let recording = context
    .dependencies()
    .recordings()
    .execution_unit("initialize")
    .member("initialization")
    .one()?;
let decoders = JsonPayloadDecoderRegistry::new().with_json_field::<u64>("value")?;
let mut checkpoint =
    StoredStateSeriesReader::open_completed_recording(recording.directory(), decoders)?
        .read_latest_state_from_stream("checkpoint")?;
let mut state = schema.create_empty_state(StateTime::from_iteration(0));
state.initialize_payload("value", checkpoint.take_payload::<u64>("value")?)?;
```

Import `JsonPayloadDecoderRegistry` and `StoredStateSeriesReader` from
`scientific_workflow::persistence`; the model types come from the prelude.
The complete linked source supplies all surrounding methods and constants.
The verified reader owns reconstruction; the consumer moves the decoded value
into its own schema-backed state. It does not append to the producer's recording.

## Select deliberately

| Intent | Selector behavior |
| --- | --- |
| Exactly one source | `.one()` rejects zero matches and ambiguity. |
| Zero or one | `.optional()` permits absence but still rejects ambiguity. |
| All matching sources | `.iter()` makes multiplicity explicit. |
| Disambiguate repeated producers | Add `.in_phase("initialize")`, task identity, or member filters. |

A sweep can turn a formerly unique producer into many candidates. Do not fix an
ambiguity by silently taking the first file. Decide whether the science needs
one checkpoint, matched configurations, or aggregation of all results.
Dependencies correlate work by global configuration; local sweep multiplicity
still requires deliberate selection.

## Run and inspect the complete example

From a repository checkout with the companion installed, in a terminal:

```sh
cargo run -p workflow-dependency-pipeline
```

After completion, type `exit`. Expect the analysis task's `artifacts/summary.json`
to contain values `[7, 8, 9, 10, 11, 12]` at iterations `[0, 1, 2, 3, 4, 5]`.
Chapter 9 explains the Python side of this handoff.

See [typed dependency APIs](../../rust/src/task/dependencies/api.md) for filters,
errors, acquisition, and compatibility contracts.

---

**Previous:** [7. Parameter sweeps and replicates](7-parameter-sweeps-and-replicates.md) · **Guide index:** [Documentation](../README.md)

**Next:** [9. Python analysis and visualization](9-python-analysis-and-visualization.md)
