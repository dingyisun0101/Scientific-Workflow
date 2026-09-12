# Workflow update pass

The workspace `update.md` defines seven phases. The user authorized execution
on `main`, with a commit and successful push between phases, on 2026-09-12.
This instruction replaces the temporary-branch/merge workflow for this pass.

1. Contracts and upstream review: `upstream.md` records the proposed dependency
   list. Await agreement before changing dependency choices or versions.
2. Establish and validate the approved dependency baseline.
3. Separate work identity from operational settings and tighten JSON immutability.
4. Timestamp execution IDs, implement `--clean`, and timestamp every log line.
5. Implement the configurable 95% disk guard and NPY concurrency controls.
6. Expose per-unit/global thread allocations and implement real task paging.
7. Complete integrated checks, documentation, push/tag/publication, and update
   downstream consumers to the published release.

## Proposed behavioral contracts

- UTC execution IDs use a sortable filesystem-safe timestamp and atomic
  directory creation with collision protection.
- `--clean` clears the configured output directory before execution creation,
  after preflight and cleanup-target validation. It must not delete a selected
  reuse source, project inputs, or another active execution.
- Work identity retains parameters, schemas, seeds, execution-unit/program
  selection, arguments, and the dependency graph. Scheduling, resource limits,
  disk policy, and host-specific launcher settings are operational metadata.
- JSON configuration is captured once. Task input JSON is immutable and checked
  for modification; finalized output JSON cannot be rewritten. Runtime control
  documents and active recording metadata need explicit lifecycle exceptions.
- Proposed disk recovery: automatically resume at two percentage points below
  the pause threshold, preserving independently requested manual pause.
- Proposed NPY auto target: gradually admit single-thread workers to the phase
  limit within the global thread budget; this is not utilization-based tuning.
- Dashboard counts represent allocated compute threads, not all OS threads.
- Every physical line in `log.txt` receives an absolute UTC timestamp.

The disk recovery, NPY target, and JSON boundary questions were sent to the user.
The implementation adopted the documented snapshot, automatic recovery, and
worker-count defaults while the preference questions remained unanswered.

## Validation and release

### Progress

- Phase 1 review pushed as `c1daaea`; package agreement remains pending.
- Phase 2 dependency changes are deferred pending that agreement. Independent
  implementation proceeds with the existing published dependency versions.
- Phase 3 implemented using the proposed snapshot contract. Rust all-target
  tests and Clippy passed; all 35 Python tests passed under Python 3.14.
- Phase 4 implemented timestamp execution names, guarded `--clean`, and UTC
  timestamps on every physical log line. Rust all-target tests passed.
- Phase 5 implemented the default 95% disk guard, independent pause reasons,
  process-group disk suspension, and NPY phase limits/fixed/auto admission.
  All-feature and headless Rust suites, Clippy, and 35 Python tests passed.
- Phase 6 implemented current per-task allocations, global Usage totals,
  full-page navigation, and resize clamping. Allocation/rebalance and rendered
  paging regressions are included; all-target Rust tests and Clippy passed.

The existing all-feature Rust workspace suite is the starting baseline.
Use Python 3.14 for companion validation; the shell's default Python is 3.12.
Keep subsystem API guides and architecture synchronized in each phase.
Follow `docs/tests.md` for final validation. Publish only after successfully
pushing release changes and tags. The macro crate is a public upstream and
requires separate permission for source or manifest changes.


### Release candidate checkpoint

Rust 0.15.0 and Python 0.5.0 are prepared, with migration guidance and breaking
notices. Phase 7 validation covers all-feature/headless Rust tests, explicit
coordinated NPY handoff, Clippy, rustdoc/doctests, Rust package verification,
installed Python wheel tests, package metadata, real terminal behavior, and both
published-version examples. The final validation result is recorded in the
commit handoff. The output lease explicitly unlocks before descriptor teardown
to avoid transient inherited-lock retention during concurrent child launches.

Phase 2 and the publication/downstream portion of Phase 7 remain outstanding:
no dependency-list agreement has arrived. Registry dependency versions are
unchanged, the macro crate has not been edited, no release tag has been created,
and neither candidate package has been published. Once the package list is
agreed, upgrade and validate dependencies, push that phase, remove candidate
notices, push release changes and tags, publish, then update consumers to the
online versions. Do not treat this checkpoint as a completed release.
