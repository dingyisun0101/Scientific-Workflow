# Changelog

This repository coordinates the independently versioned Rust workflow crate
and Python recording reader. Recording-format versions remain independent of
both package versions; see the [compatibility matrix](protocol/compatibility.md).

## Unreleased

## Rust 0.13.6 — 2026-09-05

- Resolve example Workflow dependencies and runtime macros from crates.io instead
  of local workspace paths.
- Update the PiP integration-test dependency to 4.1.0-alpha.
- Keep runtime APIs, recording formats, macros 0.2.1, and Python 0.4.3 unchanged.

## Rust 0.13.5 and Python companion 0.4.3 — 2026-09-05

**Breaking changes despite the requested patch increments:** typed initialization
dependencies replace raw JSON access; the Python distribution becomes
`scientific-workflow`, imported as `scientific_workflow`, with no old-name aliases.
Python tools require 3.14+ on Linux and a manually installed, activated environment.
Follow the [migration guide](docs/migration-0.13.5.md) before upgrading.

- Adds typed dependency selection, standard-layout project accessors, cached
  whole-series NPY views, and opt-in Python logging/progress reporting.
- Adds true initial/final observations and recording format 8 for that policy;
  periodic-only recordings remain v7, both readers support v7/v8, and NPY stays v2.
- Runs bounded converter processes under the shared study resource budget with
  single-thread native numeric libraries, resumable publication, and pause points.
- Supervises owned Linux process trees, preserves raw logs, validates bounded
  live event frames, and treats logging/renderer failures as execution failures.
- Freezes execution budgets during pause, adds a continuously advancing total
  wall clock, and shows active groups with bounded, scrollable outcome messages.
- Adds a complete dependency pipeline example, synchronized subsystem references,
  installation/migration guides, cross-language tests, and Linux qualification.
- Leaves `scientific-workflow-macros` at 0.2.1 and project schema at 1.

## Rust 0.13.3 and Python reader 0.4.2 — 2026-09-03

- Converts every recorded top-level field into manifest-described NumPy data:
  fixed-shape numeric fields use C-contiguous arrays, changing numeric shapes
  use flat data with offsets and per-record shapes, and all other fields retain
  a lossless canonical-JSON fallback.
- Discovers generic numeric projections inside structured fields, including
  typed tensor envelopes, without adding dependencies on downstream scientific
  crates or relying on NumPy object arrays and pickle.
- Adds verified single-recording and batch reader APIs that reconstruct fields
  exclusively from `manifest.json`, checking member manifests, array headers,
  shapes, offsets, C-contiguity, and SHA-256 checksums before exposing data.
- Publishes the normative manifest-directed NPY v2 dataset contract and updates
  the end-to-end attractor example to consume it through the standard reader.

## Rust 0.13.2 and Python reader 0.4.1 — 2026-09-01

- Makes `$npy` one aggregate task per replicate, covering transitive member
  recordings across every inferred global configuration.
- Publishes converted data at the standard discoverable path
  `<execution>/processed/replicate-NNNNNN` and exposes that path to downstream
  tasks as `processed_directory`.
- Simplifies the attractor plotter to read the batch manifest and C-contiguous
  arrays directly from the standard processed directory.

## Rust 0.13.1 and Python reader 0.4.0 — 2026-09-01

- Adds the standard single-recording `scientific-workflow-to-npy` converter.
  It verifies completed metadata and chunks, publishes fixed-shape numeric
  fields and scientific coordinates as C-contiguous `.npy` arrays, records
  non-convertible fields, and never modifies the raw recording.
- Keeps the core Python reader dependency-free while offering NumPy conversion
  through the `npy` installation extra.
- Adds the reserved task-free `$npy` phase. Workflow synthesizes one converter
  per global configuration and supplies all transitive prerequisite member
  recordings from that configuration.
- Extends the attractor example with `$npy` and makes its plotter consume only
  processed manifests and arrays.

## Rust 0.13.0 — 2026-09-01

This is a breaking parameter-expansion release. It keeps project configuration
at `workflow_schema: 1` and recording format v7, but does not provide a
compatibility alias for the former interpretation of non-unit top-level
`$sweep` values.

- Infers global parameter sweeps from every top-level `parameters.json` value
  not selected as an execution-unit section, then clones the complete phase
  graph for each resolved configuration.
- Keeps execution-unit-section sweeps local, correlates dependency summaries
  by resolved global configuration, exposes them through
  `InitializationContext`, and supplies each program with its resolved config.
- Records resolved project parameters in execution-unit provenance without
  adding user-authored scope, reference, or ordinal syntax.

## Rust 0.12.1 and Python reader 0.3.1 — 2026-08-31

This is a breaking Rust workflow-generation release and the coordinated Python
reader release for recording format v7.

### Rust workflow

- Requires `workflow_schema: 1` in every `wf_configs/study.json`; projects from
  the unversioned 0.11.x grammar have no compatibility alias for an omitted
  schema declaration.
- Exposes immutable, output-free study-plan inspection while keeping runtime
  execution responsible for effects.
- Makes Runtime the owner of lifecycle and progress facts and keeps UI as the
  downstream presentation adapter. The default interactive dashboard and
  redirected plain-output behavior are unchanged for users.
- Adds an explicit opt-out `default-features = false` mode that excludes the
  terminal dependencies and attaches a silent observer.
- Enforces one required study-wide compute budget. External program and Python
  tasks may reserve `resources.threads`; omitted reservations consume one slot.
- Uses one repository Cargo workspace, lockfile, and CI qualification path.

### Recording compatibility and Python reader

- Keeps `scientific-workflow-jsonl` recording format v7 unchanged and publishes
  its normative protocol, strict structural schema, and package compatibility
  manifest together.
- Qualifies Rust writing/reading and Python reading against the same golden
  fixture plus bidirectional cross-language conformance tests.
- Publishes `scientific-workflow-reader` 0.3.1 as the verified, dependency-free
  Python 3.10+ reader. It has no supported writer API.

### Upgrade notes

- Add `"workflow_schema": 1` and a positive top-level `"threads"` value to
  existing study manifests before upgrading from 0.11.x.
- Keep the default Rust feature set to preserve all existing dashboard and
  plain lifecycle output. Disable default features only when intentionally
  selecting silent/headless execution.
- Recording consumers should continue accepting only the format versions
  listed in `protocol/compatibility.json`; package versions do not replace the
  recording-format marker.
