from __future__ import annotations

import hashlib
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
    open_npy_batch,
    open_npy_conversion,
)

FIXTURE = Path(__file__).parent / "fixtures" / "complete"


def recording_with_fields(
    root: Path,
    fields: list[str],
    records: list[tuple[int, float, list[object]]],
) -> Path:
    recording = root / "recording"
    shutil.copytree(FIXTURE, recording)
    chunk = recording / "streams" / "signal" / "chunk-000000.jsonl"
    encoded = b"".join(
        json.dumps(
            {"iteration": iteration, "physical_time": physical, "values": values},
            allow_nan=False,
            separators=(",", ":"),
        ).encode("utf-8")
        + b"\n"
        for iteration, physical, values in records
    )
    chunk.write_bytes(encoded)
    metadata_path = recording / "metadata.json"
    metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    stream = metadata["streams"][0]
    stream["fields"] = [{"name": field} for field in fields]
    descriptor = stream["chunks"][0]
    descriptor.update(
        {
            "records": len(records),
            "bytes": len(encoded),
            "checksum": "sha256:" + hashlib.sha256(encoded).hexdigest(),
            "first_iteration": records[0][0],
            "last_iteration": records[-1][0],
        }
    )
    metadata_path.write_text(
        json.dumps(metadata, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    return recording


def field_metadata(
    manifest: dict[str, object], field: str
) -> dict[str, object]:
    streams = manifest["streams"]
    assert isinstance(streams, list)
    fields = streams[0]["fields"]
    return next(entry for entry in fields if entry["name"] == field)


class NpyConversionTests(unittest.TestCase):
    def test_every_field_becomes_verified_c_contiguous_npy_data(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            recording = root / "recording"
            shutil.copytree(FIXTURE, recording)
            output = root / "processed"

            manifest = convert_recording(recording, output)
            resumed = convert_recording(recording, output)

            self.assertEqual(manifest, resumed)
            self.assertEqual(manifest["format"], NPY_FORMAT)
            population = field_metadata(manifest, "population")
            label = field_metadata(manifest, "label")
            self.assertEqual(population["representation"], "numeric")
            self.assertEqual(label["representation"], "structured")
            self.assertEqual(label["projections"], [])
            for descriptor in manifest["arrays"]:
                array = np.load(output / descriptor["path"], mmap_mode="r", allow_pickle=False)
                self.assertTrue(array.flags.c_contiguous)
                self.assertEqual(
                    descriptor["checksum"],
                    "sha256:" + hashlib.sha256((output / descriptor["path"]).read_bytes()).hexdigest(),
                )

            converted = open_npy_conversion(output)
            np.testing.assert_allclose(
                converted.reconstruct("signal", "population", 1), [1.0, 2.0]
            )
            self.assertEqual(converted.reconstruct("signal", "label", 1), "later")

    def test_structured_field_has_typed_numeric_projections_and_json_fallback(self) -> None:
        values = [
            {
                "stats": {"count": 2, "energy": 1.5},
                "tensor": {
                    "kind": "vector_list",
                    "version": 2,
                    "scalar": "f64",
                    "shape": [2, 2],
                    "data": [1.0, 2.0, 3.0, 4.0],
                },
            },
            {
                "stats": {"count": 3, "energy": 2.5},
                "tensor": {
                    "kind": "vector_list",
                    "version": 2,
                    "scalar": "f64",
                    "shape": [2, 2],
                    "data": [5.0, 6.0, 7.0, 8.0],
                },
            },
        ]
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            recording = recording_with_fields(
                root,
                ["state"],
                [(0, 0.0, [values[0]]), (1, 0.1, [values[1]])],
            )
            output = root / "processed"

            manifest = convert_recording(recording, output)
            metadata = field_metadata(manifest, "state")

            self.assertEqual(metadata["representation"], "structured")
            self.assertEqual(
                [projection["logical_path"] for projection in metadata["projections"]],
                ["/stats/count", "/stats/energy", "/tensor"],
            )
            converted = open_npy_conversion(output)
            self.assertEqual(converted.reconstruct("signal", "state", 1), values[1])
            np.testing.assert_allclose(
                converted.projection("signal", "state", "/tensor", 1),
                [[5.0, 6.0], [7.0, 8.0]],
            )
            self.assertEqual(
                converted.projection("signal", "state", "/stats/count", 0), 2
            )

    def test_variable_numeric_field_uses_ragged_data_offsets_and_shapes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            recording = recording_with_fields(
                root,
                ["values"],
                [(0, 0.0, [[1, 2]]), (1, 0.1, [[3, 4, 5]])],
            )
            output = root / "processed"

            manifest = convert_recording(recording, output)
            dataset = field_metadata(manifest, "values")["dataset"]

            self.assertEqual(dataset["storage"], "ragged")
            converted = open_npy_conversion(output)
            np.testing.assert_array_equal(
                converted.reconstruct("signal", "values", 0), [1, 2]
            )
            np.testing.assert_array_equal(
                converted.reconstruct("signal", "values", 1), [3, 4, 5]
            )

    def test_dynamic_structured_sequence_preserves_empty_ragged_record(self) -> None:
        springs = {
            "springs": [
                {"pair": [0, 1], "law": {"k": 1.0, "l_0": 0.5}},
                {"pair": [1, 2], "law": {"k": 2.0, "l_0": 0.75}},
            ]
        }
        empty = {"springs": []}
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            recording = recording_with_fields(
                root,
                ["network"],
                [(0, 0.0, [springs]), (1, 0.1, [empty])],
            )
            output = root / "processed"

            manifest = convert_recording(recording, output)
            metadata = field_metadata(manifest, "network")
            paths = {
                projection["logical_path"]: projection
                for projection in metadata["projections"]
            }

            self.assertEqual(paths["/springs/*/pair"]["storage"], "ragged")
            converted = open_npy_conversion(output)
            np.testing.assert_array_equal(
                converted.projection("signal", "network", "/springs/*/pair", 0),
                [[0, 1], [1, 2]],
            )
            self.assertEqual(
                converted.projection(
                    "signal", "network", "/springs/*/pair", 1
                ).shape,
                (0, 2),
            )
            self.assertEqual(converted.reconstruct("signal", "network", 1), empty)

    def test_malformed_numeric_envelope_publishes_no_partial_output(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            recording = recording_with_fields(
                root,
                ["tensor"],
                [
                    (
                        0,
                        0.0,
                        [{"scalar": "f64", "shape": [2, 2], "data": [1.0]}],
                    )
                ],
            )
            output = root / "processed"

            with self.assertRaisesRegex(NpyConversionError, "shape requires 4"):
                convert_recording(recording, output)
            self.assertFalse(output.exists())

    def test_converted_reader_requires_manifest_and_component_integrity(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            recording = root / "recording"
            shutil.copytree(FIXTURE, recording)
            output = root / "processed"
            manifest = convert_recording(recording, output)
            component = output / manifest["arrays"][0]["path"]
            component.write_bytes(component.read_bytes() + b"corrupt")

            with self.assertRaisesRegex(NpyConversionError, "does not match manifest"):
                open_npy_conversion(output)
            (output / "manifest.json").unlink()
            with self.assertRaises(NpyConversionError):
                open_npy_conversion(output)

    def test_default_output_is_a_sibling_of_the_immutable_recording(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            recording = Path(temporary) / "member-000000"
            shutil.copytree(FIXTURE, recording)
            convert_recording(recording)
            self.assertTrue(recording.with_name("member-000000-npy").is_dir())
            self.assertFalse((recording / "manifest.json").exists())

    def test_recording_integrity_failure_publishes_no_partial_output(self) -> None:
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
            batch = open_npy_batch(output)
            self.assertEqual(len(batch.members), 2)
            self.assertEqual(
                batch.members[0].reconstruct("signal", "label", 0), "start"
            )


if __name__ == "__main__":
    unittest.main()
