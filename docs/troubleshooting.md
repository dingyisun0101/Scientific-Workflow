# Troubleshooting

| Symptom | Check and remedy |
| --- | --- |
| Missing or renamed configuration | Restore `wf_configs/study.json` and `wf_configs/parameters.json`; pass the project root to `run`. |
| Unknown JSON field | Consult [configuration](guide/4-project-configuration.md); scientific parameters belong in `parameters.json`. |
| Unknown execution unit | Match the task key to a linked `#[execution_unit]` registration. |
| Missing or wrong constants | Match the unit's registered key and `Constants` fields; retain `deny_unknown_fields`. |
| Missing Python/NumPy companion | Activate the intended Python 3.14+ environment and repeat the [installation checks](guide/2-installation.md). |
| Dashboard rejected | Run with terminal stdin and stderr, inside `screen` or `tmux` for persistent sessions. |
| Successful run appears to remain open | Type `exit` and Enter to dismiss the final dashboard. |
| Disk remains paused after freeing space | Reach the recovery threshold, then explicitly type `resume`; early commands are not queued. |
| Fewer working tasks than expected | Check global threads, task resources, phase concurrency, dependencies, and replicate scheduling. |
| Threads differ from CPU utilization | Thread counts show allocations, not measured execution activity. |
| Missing or ambiguous dependency | Check producer completion and add explicit phase/task/member filters; `.optional()` does not resolve ambiguity. |
| Plot settings ignore the sweep | Read resolved settings with project helpers rather than parsing source JSON. |
| Reuse is rejected | Compare captured scientific inputs and source completion; follow [reuse rules](guide/11-reusing-results.md). |
| Interrupted conversion | Retry the same destination; verified completed members can be reused. A failed batch has no success manifest. |
| Reader rejects a recording | Check completion, integrity, and recording-format support. Boundary streams require format 8 support. |

Read execution `log.txt` and program stdout/stderr for the specific failure.
Do not edit Workflow-owned manifests to mark incomplete work successful. Remove
orphan conversion staging directories only after confirming no converter owns
the destination.

[Guide index](README.md) · [Operation guide](guide/10-running-and-monitoring.md)
