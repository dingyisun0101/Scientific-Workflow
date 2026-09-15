# 11. Reusing results

**Outcome:** rerun selected work while retaining compatible completed scientific
results. Start with a successful multi-phase execution such as chapter 9.

## Rerun plotting from completed conversion

For the chapter 9 order (`simulate`, `$npy`, `plot`), edit these root fields in
`study.json`, replacing the example path with your actual completed execution:

```json
"active_phases": [2],
"reuse_from": "output/execution-YYYYMMDDTHHMMSS.nnnnnnnnnZ"
```

Keep scientific inputs and captured plot parameters unchanged for this first
reuse exercise. Run the application normally. Workflow checks completed
prerequisites before output creation, imports their paths, and executes plotting
in a new execution directory. The prior recordings remain immutable.

Changing analysis parameters is not automatically compatible: they are captured
project inputs. If you need to reuse results after a change, inspect the input
compatibility rules rather than assuming any downstream change is allowed.
Operational budgets may change without invalidating completed scientific work.

## Optional phase and execution-unit selection

Omitting `active_phases` selects every phase. To select a subset, set
`"active_phases": [0, 1, ...]`. Indices are zero-based in deterministic dependency
order: visit phases in JSON declaration order, recursively visit each `after`
list in its declared order, then assign each phase its index once. Selecting a
subset never renumbers phases, expanded tasks, output ordinals, or seed identities.
The order of indices in the selection does not change execution order. Duplicate,
negative, non-integer, and out-of-range indices are rejected. An empty list
explicitly selects no work.

Within a selected phase, an execution-unit task may set `"active": false`.
Inactive units retain their compiled identities and output ordinals but are not
run or reused. `active` defaults to `true` and is invalid on other task kinds.

For a six-phase preparation, reference, targets, lattice, export, and conversion
study, `"active_phases": [3, 4, 5]` starts at lattice. Supply
`"reuse_from": "output/execution-1234-0"` to reuse completed prerequisites.
The path names one execution directory and is resolved against the project root;
absolute paths are also accepted. Each replicate imports the matching source
replicate. Unselected phases that are not prerequisites are omitted entirely.

Workflow validates the complete graph and constants during Study loading.
Before creating new output, Runtime requires every imported task to have
successfully completed with matching captured inputs. `active_phases` and
`reuse_from` may differ between the captured and current study snapshots.
`$npy.exclude_streams` may also differ when the imported phase is neither
`$npy` nor a direct or transitive consumer of its output. NPY and its consumers
retain exact filter matching. Work-relevant inputs still must match: parameters,
schemas, seeds, programs/scripts, arguments, replicate count, and phase dependencies.
Compute/thread budgets, scheduling, timeouts, failure policy, persistence buffering,
disk policy, and Python environment-manager settings are operational provenance
and do not invalidate completed work. If a resource setting changes scientific
meaning, express that choice in scientific parameters.
A reused phase cannot depend on a phase selected to execute again. Missing,
failed, incompatible, or ambiguous legacy inputs fail without launching work.
Programs and `$npy` receive the original completed recording/artifact paths.
Recordings are never appended to or rewritten.

New executions commit private `workflow-result.json` receipts after each
successful phase, including references for reused tasks, so reuse can be chained.
Pre-0.13.9 program outputs may be imported from their successful `program.json`
and captured config. Legacy execution-unit imports additionally require an
authoritative matching summary in a dependent program's captured dependency
file; Workflow never guesses a final iteration from sampling cadence.

## Check compatibility before retrying

| Change | Expected reuse rule |
| --- | --- |
| Threads, scheduling, timeouts, buffering, disk policy | Operational provenance; does not invalidate scientific work. |
| Selected phase indices and source execution | May change while importing compatible prerequisites. |
| Scientific parameters, schema, seeds, replicate count | Must match the captured work inputs. |
| Program/script identity and arguments | Work-relevant inputs must match. |
| NPY stream exclusions | May differ only for imports that are neither NPY nor consumers of NPY. |

A phase selected for reuse cannot depend on a phase you are selecting to execute
again. Missing or failed source tasks are not recoverable by pretending they
completed. Read the error, restore compatible inputs, or run the necessary
prerequisites again without reuse.

## Cleanup is a separate operation

`cargo run -- --clean` removes the complete output directory's contents after
guarded preflight. It is not a command to delete just the last run, and it
rejects cleanup that would delete a selected reuse source. See
[cleanup and locks](10-running-and-monitoring.md#run-cleanup-and-timestamps)
before combining cleanup with study selection.

---

**Previous:** [10. Running and monitoring](10-running-and-monitoring.md) · **Guide index:** [Documentation](../README.md)

**Next:** [12. AI-assisted studies](12-ai-assisted-studies.md)
