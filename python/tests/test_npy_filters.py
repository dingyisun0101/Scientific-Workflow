from __future__ import annotations

import copy
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

from scientific_workflow import IntegrityError
from scientific_workflow.npy import (
    NpyConversionError,
    convert_recording,
    convert_workflow_dependencies,
    open_npy_batch,
    open_npy_conversion,
)

FIXTURE = Path(__file__).parent / "fixtures" / "complete"


def mixed_recording(root: Path) -> Path:
    recording = root / "recording"
    shutil.copytree(FIXTURE, recording)
    shutil.copytree(recording / "streams/signal", recording / "streams/checkpoint")
    metadata_path = recording / "metadata.json"
    metadata = json.loads(metadata_path.read_text())
    checkpoint = copy.deepcopy(metadata["streams"][0])
    checkpoint["name"] = "checkpoint"
    checkpoint["directory"] = "streams/checkpoint"
    metadata["streams"].append(checkpoint)
    metadata_path.write_text(json.dumps(metadata))
    return recording


def dependencies(root: Path, recordings: list[Path]) -> Path:
    path = root / "dependencies.json"
    path.write_text(json.dumps([{"phase": "evolve", "tasks": [{
        "identity": "ensemble", "output_directory": str(root),
        "workload": {"kind": "execution_unit", "execution_unit": "fixture",
                     "members": [{"identity": str(n), "final_iteration": 2,
                                  "output_directory": str(source)}
                                 for n, source in enumerate(recordings)]},
    }]}]))
    return path


class StreamExclusionTests(unittest.TestCase):
    def test_excluded_chunks_are_not_read_and_included_streams_are_verified(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            recording = mixed_recording(root)
            excluded_chunk = recording / "streams/checkpoint/chunk-000000.jsonl"
            excluded_chunk.write_bytes(b"corrupt excluded data")
            output = root / "filtered"
            manifest = convert_recording(recording, output, exclude_streams=["checkpoint"])
            self.assertEqual(manifest["exclude_streams"], ["checkpoint"])
            self.assertEqual(open_npy_conversion(output).stream_names, ("signal",))
            self.assertTrue(all(array["stream"] == "signal" for array in manifest["arrays"]))
            self.assertEqual(excluded_chunk.read_bytes(), b"corrupt excluded data")
            self.assertEqual(convert_recording(recording, output, exclude_streams=["checkpoint"]), manifest)
            with self.assertRaises(NpyConversionError):
                convert_recording(recording, output)
            with self.assertRaises(IntegrityError):
                convert_recording(recording, root / "all")
            self.assertFalse((root / "all").exists())

    def test_empty_unknown_and_all_excluded_selections(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            recording = mixed_recording(root)
            for name, excluded, expected in [
                ("all", [], ("signal", "checkpoint")),
                ("unknown", ["not-present"], ("signal", "checkpoint")),
                ("none", ["signal", "checkpoint"], ()),
            ]:
                output = root / name
                manifest = convert_recording(recording, output, exclude_streams=excluded)
                self.assertEqual(open_npy_conversion(output).stream_names, expected)
                self.assertEqual(manifest["exclude_streams"], sorted(excluded))
            self.assertEqual(convert_recording(recording, root / "none",
                             exclude_streams=["checkpoint", "signal"])["arrays"], [])

    def test_invalid_filters_fail_before_output_creation(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            recording = mixed_recording(root)
            for excluded in ["checkpoint", None, [""], [" checkpoint"], [1], ["signal", "signal"]]:
                with self.assertRaises(NpyConversionError):
                    convert_recording(recording, root / "invalid", exclude_streams=excluded)
                self.assertFalse((root / "invalid").exists())

    def test_serial_parallel_filtering_and_batch_retry_identity(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            first = mixed_recording(root)
            second = root / "second"
            shutil.copytree(first, second)
            deps = dependencies(root, [first, second])
            for source in [first, second]:
                (source / "streams/checkpoint/chunk-000000.jsonl").write_bytes(b"excluded corruption")
            manifests = []
            for workers in [1, 2]:
                output = root / f"workers-{workers}"
                with patch.dict(os.environ, {"WORKFLOW_THREADS": str(workers)}):
                    manifest = convert_workflow_dependencies(deps, output, exclude_streams=["checkpoint"])
                    self.assertEqual(convert_workflow_dependencies(deps, output,
                                     exclude_streams=["checkpoint"]), manifest)
                    before = (output / "manifest.json").read_bytes()
                    with self.assertRaises(NpyConversionError):
                        convert_workflow_dependencies(deps, output)
                    self.assertEqual((output / "manifest.json").read_bytes(), before)
                self.assertTrue(all(member.stream_names == ("signal",)
                                    for member in open_npy_batch(output).members))
                manifests.append(manifest)
            self.assertEqual(*manifests)

    def test_cli_forwards_filters_in_direct_and_batch_modes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            recording = mixed_recording(root)
            direct = root / "direct"
            subprocess.run([sys.executable, "-m", "scientific_workflow.npy", str(recording),
                            "--output", str(direct), "--exclude-stream", "checkpoint"],
                           check=True, capture_output=True, text=True)
            self.assertEqual(open_npy_conversion(direct).stream_names, ("signal",))
            deps = dependencies(root, [recording])
            batch = root / "batch"
            env = {**os.environ, "WORKFLOW_THREADS": "1", "WORKFLOW_DEPENDENCIES_PATH": str(deps),
                   "WORKFLOW_NPY_OUTPUT": str(batch)}
            subprocess.run([sys.executable, "-m", "scientific_workflow.npy", "--workflow-dependencies",
                            "--exclude-stream=checkpoint"], env=env,
                           check=True, capture_output=True, text=True)
            self.assertEqual(open_npy_batch(batch).members[0].stream_names, ("signal",))


if __name__ == "__main__":
    unittest.main()
