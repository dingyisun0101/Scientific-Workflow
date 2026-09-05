"""Analyze the standard batch using generic Workflow accessors."""
import json
from scientific_workflow.dependencies import Dependencies
from scientific_workflow.npy import open_npy_batch
from scientific_workflow.project import output_directory, parameters
from scientific_workflow.reporting import log

settings = parameters("analysis")
batch = open_npy_batch(Dependencies.from_env().npy_batches().one().directory)
results = []
for conversion in batch.members:
    if conversion.execution_unit == "simulation":
        series = conversion.series("state", "value")
        results.append({"iterations": series.iterations.tolist(), "values": [int(series.record(n).item()) for n in range(len(series))]})
(output_directory() / settings["filename"]).write_text(json.dumps(results, indent=2) + "\n", encoding="utf-8")
log(f"analyzed {len(results)} simulation(s)", level="success")
