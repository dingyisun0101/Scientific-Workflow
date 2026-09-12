# Workflow update pass

The workspace `update.md` defines seven phases. The user authorized execution
on `main`, with a commit and successful push between phases, on 2026-09-12.
This instruction replaces the temporary-branch/merge workflow for this pass.

1. Contracts and upstream review: `upstream.md` records the proposed dependency
   list. The user approved it during the final decision review.
2. Establish and validate the approved dependency baseline.
3. Separate work identity from operational settings and tighten JSON immutability.
4. Timestamp execution IDs, implement `--clean`, and timestamp every log line.
5. Implement the configurable 95% disk guard and NPY concurrency controls.
6. Expose per-unit/global thread allocations and implement real task paging.
7. Complete integrated checks, documentation, push/tag/publication, and update
   downstream consumers to the published release.

## Agreed behavioral contracts

The user settled these decisions individually before final release validation:

- Refresh the reviewed dependencies, including `fs2` → synchronous `fs4`.
  Keep the separately published macro crate unchanged.
- Disk usage pauses work at 95% by default. Recovery to two percentage points
  below the threshold only enables manual resumption: the user must type
  `resume` and Enter. Remind users when paused and when space is sufficient.
- Disk monitor startup/sampling failures stop the run with a clear error;
  active work is cancelled and existing output retained.
- NPY defaults to `auto`: start one single-thread worker, add one every 250 ms
  of active time, and retain the full reserved allowance throughout the ramp.
  Advise users to inherit the study thread budget unless reserving resources
  for other tasks requires a lower conversion limit.
- Source JSON remains editable for future runs. Active runs retain immutable
  snapshots, captured inputs, and finalized output JSON; active runtime control
  and recording metadata have lifecycle exceptions.
- `--clean` removes all previous output after preflight and protection checks.
- Operational settings, including Python environment selection, do not
  invalidate reuse; scientific inputs still must match.
- Dashboard thread counts show allocated compute threads and used/budget totals.
- Execution names and every physical log line use absolute UTC timestamps.
- The dashboard is required. There is no headless execution mode. Reject
  noninteractive launches before output mutation and instruct users to run
  inside `screen` or `tmux`. Typed `resume` uses the dashboard command input.
- Preserve page-capacity navigation, resize clamping, and stable task identity.

## Validation and release

### Progress

- Phase 1 review pushed as `c1daaea`; package agreement is now complete.
- Phase 2 dependency refresh completed: fs4 1.1.0 plus compatible lockfile
  updates. Workspace tests, Clippy, and 35 Python tests passed.
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
notices. The earlier Phase 7 checkpoint covered all-feature/headless Rust tests, explicit
coordinated NPY handoff, Clippy, rustdoc/doctests, Rust package verification,
installed Python wheel tests, package metadata, real terminal behavior, and both
published-version examples. The final validation result is recorded in the
commit handoff. The output lease explicitly unlocks before descriptor teardown
to avoid transient inherited-lock retention during concurrent child launches.

At the earlier candidate checkpoint, dependency agreement and publication were
pending. Agreement is now complete and Phase 2 is validated. The subsequent
behavioral corrections listed above must pass final validation before release.
That checkpoint had no release tag or package publication. The release sequence
is to push changes and tags before publication, then update consumers online.

### Final decision implementation

The dependency refresh is pushed as `c6ccce5`. Subsequent changes implement
latched disk pauses with typed resume and dashboard reminders, default NPY auto
admission, and mandatory dashboard execution. Headless/plain execution and the
terminal-ui feature switch are removed. Existing Runtime tests use test-only
observers; public execution is qualified with a real PTY.

Validation passed: 156 Rust unit tests plus 20 integration tests, three
doctests, warnings-denied Clippy/rustdoc, explicit fixed/auto NPY handoff,
35 Python source tests and 35 installed-wheel tests, and wheel/sdist checks.
The PTY integration covers the required dashboard and disk-resume rejection.

The behavior phase is pushed as `57164a5`. Cargo publication dry-run verified
the 110-file crate against registry dependencies. Final release preparation
removes candidate notices, stabilizes the pause test deadline, and runs examples
in a real terminal. Publication follows a green release-commit CI run.
