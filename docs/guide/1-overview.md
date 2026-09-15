# 1. Overview

**Outcome:** understand what Workflow handles and where your scientific code belongs.
No installation is needed for this chapter.

## Focus on science

A scientific project needs more than a model: parameter combinations, repeated
runs, data recording, dependency handoff, analysis, and progress monitoring.
Workflow supplies that experiment infrastructure. You provide the scientific
model and any domain-specific analysis; after integration, ordinary study
changes are JSON edits rather than new orchestration programs.

```text
Scientific model + JSON study
             ↓
Initialization → simulation sweep → NumPy conversion → Python analysis
             ↓
Recorded states, analysis artifacts, and a live dashboard
```

Workflow turns project configuration into scheduled work and recorded results:

```text
Study
`-- Phase
    `-- Task
        `-- ExecutionUnit
            `-- Member -> SystemState -> recording
```

The words have precise meanings:

| Term | Meaning |
| --- | --- |
| **Study** | The complete experiment described by the project configuration: its phases, tasks, dependencies, parameters, and execution settings. Ordinary applications edit `wf_configs/study.json` and call `run`; they do not construct Rust's `Study` type. |
| **Phase** | A named stage of a study, such as `simulate` or `plot`. A phase contains tasks and may depend on earlier phases. |
| **Task** | One schedulable piece of work inside a phase. A task runs either a registered Rust execution unit, an executable program, or a Python script. A parameter sweep can expand one task declaration into several concrete tasks. |
| **Execution unit** | A Rust scientific model adapted to Workflow's lifecycle. Workflow initializes it, repeatedly asks it to step, and observes the members it exposes. |
| **Member** | One independently tracked state and result inside an execution unit. Each member has its own identity, `SystemState`, completion status, progress target, recording, and final result. Most models expose one member; an ensemble can expose several. |
| **State** | The current typed scientific values and time owned by a member. A JSON state schema names the fields, while the execution unit supplies their Rust payload values. |

For a single model, the common case is one task, one execution unit, and one
member. The separate names matter when a task runs an ensemble: the task still
has one coordinated execution-unit lifecycle, but each member is recorded and
completed independently.

## What you author

| Component | Your responsibility | Workflow's responsibility |
| --- | --- | --- |
| Scientific model | Implement and register an execution unit, or supply an executable. | Invoke tasks and coordinate their lifecycle. |
| Study configuration | Declare phases, dependencies, resources, and task kinds. | Validate and assemble the study, then schedule work. |
| Parameters | Define constants, sweeps, cases, and replicate policy. | Expand combinations and retain resolved inputs. |
| Scientific state | Define fields, payloads, and observation meaning. | Record exposed member state automatically. |
| Analysis | Supply a script or reuse an integrated analysis component. | Hand off dependencies, run conversion and analysis, and capture outputs. |

Automatic recording applies to registered Rust member state. Arbitrary external
programs own their scientific artifacts. Workflow does not invent a model or a
plotting algorithm; it removes the need to rewrite their surrounding pipeline.

## Choose your route

New projects should follow this guide in order. The first runnable study is in
chapter 3; chapter 5 explains its model implementation. Existing integrated
projects can start with [sweeps](7-parameter-sweeps-and-replicates.md) and
[AI-assisted studies](12-ai-assisted-studies.md).

The runtime supports Linux and requires an interactive terminal dashboard.
Platform and interpreter requirements are covered next.

---

**Guide index:** [Documentation](../README.md)

**Next:** [2. Installation](2-installation.md)
