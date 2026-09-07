# Migrating to Rust 0.14.0 compute allocation

Rust 0.14.0 replaces the single shared Rayon pool from 0.13.x. There is no
compatibility default or alias. Python 0.4.5 and recording formats 7 and 8 are
unchanged.

Every `wf_configs/study.json` must add one global mode:

```json
"compute": {"mode": "auto"}
```

Automatic mode divides the top-level `threads` budget equally among execution
unit tasks that are currently working. Pending tasks receive no share. A start
or finish triggers rebalancing after active initialization or step calls return.
Each working task runs in a private Rayon pool, preventing another long-lived
task from starving it.

Every execution unit selected by an automatic study must explicitly promise
that initialization and steps are correct under changing positive pool sizes:

```rust,ignore
impl scientific_workflow::ExecutionUnit for Model {
    const THREAD_COUNT_INVARIANT: bool = true;
    // ...
}
```

Leave the default `false` when scientific behavior depends on a fixed thread
count. Such tasks must use isolated mode instead:

```json
{
  "threads": 16,
  "compute": {"mode": "isolated"},
  "phases": {
    "simulate": {
      "tasks": [
        {"execution_unit": "small", "resources": {"threads": 4}},
        {"execution_unit": "large", "resources": {"threads": 12}}
      ]
    }
  }
}
```

In isolated mode every execution-unit task requires a positive
`resources.threads` no greater than the global budget. Runtime keeps that pool
size fixed and delays admission until the sum of working fixed allocations
fits. Program and Python resource declarations retain their previous behavior.

Plan inspection now reports `PlanSummary::compute_mode()` as
`PlanComputeMode`, and `PlannedTaskKind::ExecutionUnit` includes optional
`threads` (`None` in auto, `Some` in isolated). Completed and failed member
metadata records compute mode and allocation history.

The same release makes phase selection optional: omitting `active_phases` now
selects every phase. An execution-unit task may set `"active": false` to stay
in the compiled plan while being excluded from execution and reuse. The task
flag defaults to `true` and is invalid on program, Python, and `$npy` tasks.
