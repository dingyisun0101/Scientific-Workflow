# Scientific Workflow documentation

Follow the numbered guide from a first run to complete scientific pipelines.
Every chapter ends with a link to the next.

**All AI agents must first read [the Workflow agent instructions](AGENTS.md).**

## Learning path

1. [Overview](guide/1-overview.md)
2. [Installation](guide/2-installation.md)
3. [Your first study](guide/3-first-study.md)
4. [Project configuration](guide/4-project-configuration.md)
5. [Scientific models](guide/5-scientific-models.md)
6. [State and recording](guide/6-state-and-recording.md)
7. [Parameter sweeps and replicates](guide/7-parameter-sweeps-and-replicates.md)
8. [Phases and dependencies](guide/8-phases-and-dependencies.md)
9. [Python analysis and visualization](guide/9-python-analysis-and-visualization.md)
10. [Running and monitoring](guide/10-running-and-monitoring.md)
11. [Reusing results](guide/11-reusing-results.md)
12. [AI-assisted studies](guide/12-ai-assisted-studies.md)

Chapters 1–3 get a first study running. Chapters 4–6 explain configuration and
scientific integration. Chapters 7–9 assemble experiments and figures. Chapters
10–12 cover operation, reuse, and AI-assisted iteration.

**Already have an integrated project?** Start with [sweeps](guide/7-parameter-sweeps-and-replicates.md),
then dependencies, analysis, and [AI-assisted studies](guide/12-ai-assisted-studies.md).
Use [operations](guide/10-running-and-monitoring.md) when launching a study.

## Reference

- [Complete `study.json` settings and explanations](guide/4-project-configuration.md#studyjson-system-settings)
  and the [crate README's compact tables](../rust/README.md#studyjson-system-settings).
- [Rust API overview](reference/rust-api.md), with links to exhaustive subsystem contracts.
- [Parameter grammar and expansion](../rust/src/config/api.md#wf_configsparametersjson).
- [State API and schemas](../rust/src/state/api.md).
- [Python reader, project helpers, dependencies, and NumPy API](../python/src/scientific_workflow/api.md).
- [Output layout](reference/output-layout.md).
- Recording protocols [v7](../protocol/recording-v7.md), [v8](../protocol/recording-v8.md),
  [NPY v2](../protocol/npy-v2.md), and [compatibility](../protocol/compatibility.md).

## Examples and support

- [Worked examples](examples.md).
- [Troubleshooting](troubleshooting.md).
- [Migration to Rust 0.15.0 / Python 0.5.0](migration-0.15.0.md),
  [0.14.0](migration-0.14.0.md), and [0.13.5](migration-0.13.5.md).
- [Changelog](../CHANGELOG.md).

## Development

- [Architecture and subsystem ownership](architecture.md).
- [Tests and validation commands](tests.md).
- [Required instructions for AI agents](AGENTS.md).
- [Repository contribution instructions](../AGENTS.md).

The current guides describe Rust 0.15.2 and Python companion 0.5.0. This Rust
patch updates documentation; scientific APIs and recording formats are unchanged.
