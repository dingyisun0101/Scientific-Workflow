"""Reproducible Linux conversion timing and aggregate-RSS smoke benchmark.

Run with the installed companion: python python/benchmarks/conversion.py.
Aggregate RSS sums processes and therefore includes shared pages more than once.
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import tempfile
import threading
import time
from scientific_workflow.npy import convert_workflow_dependencies


def rss_tree(pid):
    try:
        fields = Path(f"/proc/{pid}/statm").read_text().split()
        total = int(fields[1]) * os.sysconf("SC_PAGE_SIZE")
        children = Path(f"/proc/{pid}/task/{pid}/children").read_text().split()
        return total + sum(rss_tree(int(child)) for child in children)
    except (OSError, ValueError, IndexError):
        return 0


def main():
    fixture = Path(__file__).resolve().parents[1] / "tests/fixtures/complete"
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        members = []
        for ordinal, count in enumerate([2000, 4000, 6000, 8000]):
            recording = root / f"recording-{ordinal}"
            shutil.copytree(fixture, recording)
            data = b"".join((json.dumps({"iteration": n, "physical_time": n * 0.01, "values": [[float(n % 7)] * 32, "sample"]}, separators=(",", ":")) + "\n").encode() for n in range(count))
            chunk = recording / "streams/signal/chunk-000000.jsonl"
            chunk.write_bytes(data)
            metadata_path = recording / "metadata.json"
            metadata = json.loads(metadata_path.read_text())
            metadata["streams"][0]["chunks"][0].update(records=count, bytes=len(data), checksum="sha256:"+hashlib.sha256(data).hexdigest(), first_iteration=0, last_iteration=count-1)
            metadata_path.write_text(json.dumps(metadata))
            members.append({"identity": str(ordinal), "final_iteration": count-1, "output_directory": str(recording)})
        deps = root / "dependencies.json"
        deps.write_text(json.dumps([{"phase":"simulate","tasks":[{"identity":"simulation","output_directory":str(root),"workload":{"kind":"execution_unit","execution_unit":"benchmark","members":members}}]}]))
        measurements = []
        for workers in (1, 4):
            stopped = threading.Event()
            samples = []
            def sample():
                while not stopped.wait(0.01): samples.append(rss_tree(os.getpid()))
            monitor = threading.Thread(target=sample)
            monitor.start()
            os.environ["WORKFLOW_THREADS"] = str(workers)
            started = time.perf_counter()
            try:
                convert_workflow_dependencies(deps, root / f"out-{workers}")
            finally:
                stopped.set(); monitor.join()
            measurements.append({"workers":workers, "seconds":round(time.perf_counter()-started,3), "peak_aggregate_rss_mib":round(max(samples, default=0)/1024**2,1)})
        print(json.dumps({"records":20000,"members":4,"vector_width":32,"measurements":measurements}, indent=2))


if __name__ == "__main__": main()
