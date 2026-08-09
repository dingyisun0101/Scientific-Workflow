# `attractor_2d` implementation plan

The complete first-trial workflow is implemented and verified. The checklist
retains the development sequence as a concise record of completed scope.

## Placement

- [x] After approval, move this planning skeleton from
  `dev/examples/attractor_2d` to the repository-level
  `examples/attractor_2d` directory.
- [x] Create the `src` directory for the application entry point.

## Input files

- [x] Create `config/fixed.json` with shared model, sampling-interval, and storage
  settings.
- [x] Create `config/sweep.json` with the Cartesian `mu` axis.
- [x] Create `config/paths.json` with state-template and generated-output
  paths.
- [x] Create `config/state.json` with the evolving `point` and retained
  `radius` diagnostic fields.
- [x] Review the four JSON files together through the crate's public loaders for
  naming, type, task-order, path-resolution, and schema consistency.

## Documentation

- [x] Create `README.md` with the scientific equations, directory explanation,
  run command, output behavior, and expected terminal sections.

## Executable

- [x] Create the standalone `Cargo.toml` with a versioned local path dependency
  on `../../dev`.
- [x] Create `src/main.rs` using only `scientific_workflow::prelude::*` and the
  standard library.
- [x] Split project loading, live simulation, recording, and analysis into
  dedicated modules while retaining `main.rs` as the orchestrator.
- [x] Implement concise configuration loading and typed task extraction.
- [x] Implement schema loading and per-task state assembly.
- [x] Implement the Hopf-normal-form explicit-Euler loop with direct state
  mutation and simulation-time advancement.
- [x] Configure the `trajectory`, `radius`, and `checkpoint` streams and record
  them at separate sampling intervals.
- [x] Explicitly complete successful recordings; leave production-grade failed
  lifecycle policy to the library tests and future applications.
- [x] Generate and retain `Cargo.lock`, as is conventional for an executable
  application.

## Readback and analysis

- [x] Reconstruct all three streams with vector and scalar decoders after each
  writer reaches completed status.
- [x] Analyze the reconstructed series and render the terminal ASCII plot.
- [x] Explicitly verify the storage round trip and return an error on mismatch.
- [x] Enable exact finite-float round trips in the library's JSON dependency
  after the first trial exposed a one-ULP decode discrepancy.
- [x] Remove the duplicate `FinalState`, retain `HopfModel` as the sole live
  state owner, and reduce validation/error scaffolding to demo essentials.
- [x] Normalize module names to `project_setup`, `hopf_model`,
  `state_recording`, and `recording_analysis`.
- [x] Add a one-file standard-library Hopf reference and confirm bit-identical
  final iteration, physical time, point, and radius for all three tasks.
- [x] Move periodic and final-state sampling decisions into
  `SystemStateWriter`; the model loop now offers every state without sampling-interval
  branches.
- [x] Adopt shared stream limits, concise periodic stream declarations,
  automatic task metadata, paired physical-axis setup, and generic Serde JSON
  decoder registration.
- [x] Generate tasks lazily, decode `NonZeroU64` settings directly, and make
  `HopfModel::advance()` self-contained by retaining its immutable ODE
  coefficients.

## Packaging and verification

- [x] Keep the complete example in the Git repository and confirm it is absent
  from the library crate's crates.io archive.
- [x] Confirm generated data stays beneath the standalone project's ignored
  `target/recordings` directory.
- [x] Run formatting through the standalone manifest.
- [x] Run Clippy through the standalone manifest with warnings denied.
- [x] Run tests through the standalone manifest.
- [x] Run
  `cargo run --manifest-path examples/attractor_2d/Cargo.toml` and inspect every
  labeled result.
- [x] Run the example a second time and confirm it does not overwrite the first
  recording.
- [x] Run the library package listing and confirm the repository example and all
  generated data are absent.
- [x] Update the crate README and root design documentation after the example is
  verified.
