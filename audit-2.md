# Workflow Audit 2

## Scope

This review focuses only on simplistic style: places where existing behavior
could be expressed more concisely without sacrificing clarity. Correctness,
architecture, performance, and documentation concerns are excluded.

## Findings

- **Low:** `read_all_streams` uses `output.clear()` inside an `except` before re-raising in [reader.py](python/src/scientific_workflow_reader/reader.py#L413). Because `output` is local and is discarded when the exception propagates, the cleanup has no effect. The method can directly return a tuple comprehension or append in a simple loop without the `try`/`except`.

- **Low:** `TaskHandle` still repeats the same two-variant dispatch for `is_cancelled`, `set_detail`, `report`, `complete`, `fail`, and `cancel` in [task.rs](rust/src/study/task.rs#L22). A private handle trait, or a narrower helper for the shared operations, could reduce the repeated `match` blocks. This is a readability tradeoff: the current explicit matches are also easy to audit.

- **Low:** `TaskIdentity::iter` wraps a straightforward map iterator in `Box<dyn Iterator>` in [renderer.rs](rust/src/study/renderer.rs#L107). Returning `impl Iterator<Item = (&str, &Value)> + '_` would remove the allocation and dynamic-dispatch wrapper while keeping the public behavior unchanged.

- **Low:** `SamplingInterval::iterations` expands `NonZeroU64::new` into a `match` in [storage.rs](rust/src/storage.rs#L363). The same behavior can be expressed as `NonZeroU64::new(interval).map(Self::Iterations)`, which is shorter and idiomatic.

- **Low:** Temporary-directory setup is repeated between [configuration_workflow.rs](rust/tests/configuration_workflow.rs#L10) and [artifact_workflow.rs](rust/tests/artifact_workflow.rs#L10), including process-based naming, a sequence counter, directory creation, and cleanup. A shared test utility would make the test files smaller, though it may be less worthwhile if the suites are intentionally standalone.

## Refactor Notes

The previous attractor task-construction duplication appears to have been
addressed by the current `attractor_run.rs` extraction and is not listed here.

No correctness or behavioral findings are included in this audit.
