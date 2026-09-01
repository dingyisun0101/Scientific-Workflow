from __future__ import annotations

import json
from pathlib import Path
import shutil
import tempfile
import unittest

import numpy as np

from scientific_workflow_reader import IntegrityError
from scientific_workflow_reader.npy import (
    NPY_BATCH_FORMAT,
    NPY_FORMAT,
    NpyConversionError,
    convert_recording,
    convert_workflow_dependencies,
)

FIXTURE = Path(__file__).parent / "fixtures" / "complete"


class NpyConversionTests(unittest.TestCase):
    def test_numeric_fields_become_verified_c_contiguous_arrays(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            recording = root / "recording"
            shutil.copytree(FIXTURE, recording)
            output = root / "processed"

            manifest = convert_recording(recording, output)
            resumed = convert_recording(recording, output)

            self.assertEqual(manifest, resumed)
            self.assertEqual(manifest["format"], NPY_FORMAT)
            stream = manifest["streams"][0]
            self.assertEqual(stream["converted_fields"], ["population"])
            self.assertEqual(
                stream["omitted_fields"],
                [{"field": "label", "reason": "not fixed-shape numeric JSON"}],
            )
            descriptors = {
                (item["role"], item.get("field")): item
                for item in manifest["arrays"]
            }
            points = np.load(output / descriptors[("field", "population")]["path"])
            iterations = np.load(output / descriptors[("iterations", None)]["path"])
            physical = np.load(output / descriptors[("physical_times", None)]["path"])
            self.assertEqual(points.dtype, np.dtype(np.float64))
            self.assertEqual(points.shape, (2, 2))
            self.assertTrue(points.flags.c_contiguous)
            np.testing.assert_array_equal(iterations, np.array([0, 2], dtype=np.uint64))
            np.testing.assert_allclose(physical, np.array([0.0, 0.5]))

    def test_default_output_is_a_sibling_of_the_immutable_recording(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            recording = Path(temporary) / "member-000000"
            shutil.copytree(FIXTURE, recording)
            convert_recording(recording)
            self.assertTrue(recording.with_name("member-000000-npy").is_dir())
            self.assertFalse((recording / "manifest.json").exists())

    def test_integrity_failure_publishes_no_partial_output(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            recording = root / "recording"
            shutil.copytree(FIXTURE, recording)
            chunk = recording / "streams" / "signal" / "chunk-000000.jsonl"
            chunk.write_bytes(chunk.read_bytes() + b" ")
            output = root / "processed"
            with self.assertRaises(IntegrityError):
                convert_recording(recording, output)
            self.assertFalse(output.exists())

    def test_output_inside_recording_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            recording = Path(temporary) / "recording"
            shutil.copytree(FIXTURE, recording)
            with self.assertRaises(NpyConversionError):
                convert_recording(recording, recording / "processed")

    def test_workflow_dependencies_convert_all_unique_execution_unit_members(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            first = root / "first"
            second = root / "second"
            shutil.copytree(FIXTURE, first)
            shutil.copytree(FIXTURE, second)
            dependencies = root / "dependencies.json"
            dependencies.write_text(
                json.dumps(
                    [
                        {
                            "phase": "simulate",
                            "tasks": [
                                {
                                    "workload": {
                                        "kind": "execution_unit",
                                        "members": [
                                            {"output_directory": str(first)},
                                            {"output_directory": str(second)},
                                        ],
                                    }
                                }
                            ],
                        },
                        {
                            "phase": "export",
                            "tasks": [
                                {
                                    "workload": {
                                        "kind": "execution_unit",
                                        "members": [{"output_directory": str(first)}],
                                    }
                                }
                            ],
                        },
                    ]
                ),
                encoding="utf-8",
            )
            output = root / "processed"

            manifest = convert_workflow_dependencies(dependencies, output)

            self.assertEqual(manifest["format"], NPY_BATCH_FORMAT)
            self.assertEqual(len(manifest["members"]), 2)
            self.assertTrue((output / "member-000000" / "manifest.json").is_file())
            self.assertTrue((output / "member-000001" / "manifest.json").is_file())


if __name__ == "__main__":
    unittest.main()
