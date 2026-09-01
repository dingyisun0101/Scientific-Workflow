# Changelog

This repository coordinates the independently versioned Rust workflow crate
and Python recording reader. Recording-format versions remain independent of
both package versions; see the [compatibility matrix](protocol/compatibility.md).

## Unreleased

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
