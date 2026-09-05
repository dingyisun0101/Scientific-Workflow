"""Test-only Python half of the bidirectional Rust/Python round trip.

This is deliberately not a package writer API. It produces one closed
format-v7 conformance recording so Workflow's Rust reader can verify bytes
reconstructed by the official Python reader and re-encoded by Python.
"""

from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path
from typing import Any

from scientific_workflow import open_completed_recording


def _compact_json(value: Any) -> bytes:
    return json.dumps(
        value,
        ensure_ascii=False,
        allow_nan=False,
        separators=(",", ":"),
    ).encode("utf-8")


def _write_python_recording(destination: Path, source_reader: Any) -> None:
    series = source_reader.read_stream("signal")
    destination.mkdir()
    stream_directory = destination / "streams" / "signal"
    stream_directory.mkdir(parents=True)

    chunks: list[dict[str, Any]] = []
    for ordinal, state in enumerate(series):
        record = {
            "iteration": state.iteration,
            "physical_time": state.physical_time,
            "values": [state.values[field.name] for field in series.fields],
        }
        payload = _compact_json(record) + b"\n"
        filename = f"chunk-{ordinal:06}.jsonl"
        (stream_directory / filename).write_bytes(payload)
        chunks.append(
            {
                "ordinal": ordinal,
                "file": filename,
                "records": 1,
                "bytes": len(payload),
                "checksum": "sha256:" + hashlib.sha256(payload).hexdigest(),
                "first_iteration": state.iteration,
                "last_iteration": state.iteration,
            }
        )

    metadata = {
        "format": "scientific-workflow-jsonl",
        "version": 7,
        "status": {"state": "complete"},
        "timing": {
            "created_at_utc": "2026-08-12T00:00:00Z",
            "finalized_at_utc": "2026-08-12T00:00:01Z",
            "active_duration_ns": 1_000_000_000,
            "continuation_count": 0,
        },
        "records": {"encoding": "json", "framing": "json_lines"},
        "time": {
            "iteration_name": "iteration",
            "physical_time_name": "physical_time",
            "physical_time_unit": "s",
        },
        "user_metadata": {
            "producer": "python-roundtrip-bridge",
            "rust_origin": source_reader.user_metadata["producer"],
        },
        "terminal_metadata": {
            "termination_reason": "python_roundtrip_complete"
        },
        "streams": [
            {
                "name": "signal",
                "directory": "streams/signal",
                "sampling_interval": {"iterations": 1},
                "fields": [
                    {"name": "population", "description": "Exact float payload"},
                    {"name": "label", "description": "Unicode round-trip label"},
                ],
                "storage": {
                    "layout": {"kind": "chunked", "target_bytes": 256},
                    "storage_queue_bytes": 4096,
                },
                "chunks": chunks,
            }
        ],
    }
    (destination / "metadata.json").write_bytes(_compact_json(metadata) + b"\n")


def main() -> None:
    if len(sys.argv) != 3:
        raise SystemExit("usage: roundtrip_bridge.py RUST_RECORDING PYTHON_RECORDING")
    rust_recording = Path(sys.argv[1])
    python_recording = Path(sys.argv[2])

    reader = open_completed_recording(
        rust_recording,
        decoders={"population": tuple, "label": str},
    )
    assert reader.user_metadata["producer"] == "rust-public-writer"
    assert reader.terminal_metadata["termination_reason"] == "rust_roundtrip_ready"
    assert reader.stream_names == ("signal",)
    assert reader.stream_record_count("signal") == 2
    series = reader.read_stream("signal")
    assert series.iterations == (0, 1)
    assert series[0].physical_time == 0.0
    assert series[1].physical_time == 0.25
    assert series[0].values["label"] == "rust → python 世界"
    assert reader.read_latest("signal") == series[-1]

    _write_python_recording(python_recording, reader)


if __name__ == "__main__":
    main()
