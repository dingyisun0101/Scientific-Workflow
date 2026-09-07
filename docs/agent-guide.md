# Guide for coding agents

Treat Workflow primarily as a declarative dependency. For ordinary scientific
projects, author or update the project JSON beneath `wf_configs/`; do not edit
Workflow itself unless the requested behavior cannot be expressed through its
documented contracts.

## Author strict JSON

- Write standards-compliant JSON: double-quoted keys and strings, no comments,
  no trailing commas, and no shell interpolation.
- Put orchestration in `wf_configs/study.json`, execution-unit constants and
  project parameters in `wf_configs/parameters.json`, and project-owned state
  schemas in JSON files registered by `study.json.paths.states`.
- Declare `workflow_schema`, `threads`, `compute`, and `phases`. Omit
  `active_phases` to run every phase, or provide dependency-order indices to
  select a subset. An execution-unit task may set `"active": false` without
  changing its compiled identity or output ordinal.
- Prefer existing execution-unit, program, Python, and reserved `$npy` task
  declarations over application-side orchestration code.
- Validate edited JSON with a JSON parser and run the project's normal dry
  loading or tests before launching a long study.

## Find the owning API

Start at the repository [README](../README.md), then use the subsystem guide
that owns the requested behavior:

- [`config`](../rust/src/config/api.md) for `study.json`, parameters, paths,
  compute policy, phase selection, and task declarations;
- [`study`](../rust/src/study/api.md) for compiled-plan inspection and
  preflight;
- [`task`](../rust/src/task/api.md) for execution-unit implementations,
  members, initialization, and dependencies;
- [`observation`](../rust/src/observation/api.md) and
  [`state`](../rust/src/state/api.md) for scientific state and sampling;
- [`runtime`](../rust/src/runtime/api.md) for scheduling and completed run
  summaries;
- [`persistence`](../rust/src/persistence/api.md) and the
  [protocol directory](../protocol/) for durable recordings; and
- [`ui`](../rust/src/ui/api.md) for terminal behavior and `log.txt`.

Use repository search to find the public symbol or JSON field named by the
relevant guide. Follow links from that owning API instead of copying an
internal type or guessing a private module boundary.

## Keep changes downstream

Prefer, in order: a project JSON edit, application execution-unit/program code,
documented public APIs, and only then a Workflow implementation change. Never
patch Workflow merely to encode one study's constants, phase choices, paths,
or launch policy. Do not reach into private modules, copy recording internals,
or manually write Workflow-owned output files.

If a genuine framework change is required, read `AGENTS.md`, the architecture
guide, and every affected subsystem's `api.md` first. Keep ownership in the
lowest appropriate subsystem, update tests and contracts together, and do not
modify or publish upstream crates without the user's explicit permission.
