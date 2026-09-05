from __future__ import annotations

import hashlib
import json
from pathlib import Path
import shutil
import tempfile
import unittest

import numpy as np

from scientific_workflow import IntegrityError
from scientific_workflow.npy import (
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
                                    "identity": "task",
                                    "output_directory": str(root),
                                    "workload": {
                                        "kind": "execution_unit",
                                        "execution_unit": "fixture",
                                        "members": [
                                            {"identity":"first", "final_iteration":1, "output_directory": str(first)},
                                            {"identity":"second", "final_iteration":1, "output_directory": str(second)},
                                        ],
                                    }
                                }
                            ],
                        },
                        {
                            "phase": "export",
                            "tasks": [
                                {
                                    "identity": "task",
                                    "output_directory": str(root),
                                    "workload": {
                                        "kind": "execution_unit",
                                        "execution_unit": "fixture",
                                        "members": [{"identity":"first", "final_iteration":1, "output_directory": str(first)}],
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

class SeriesTests(unittest.TestCase):
    def test_fixed_and_ragged_views_reuse_maps_and_validate_indices(self):
        from scientific_workflow.npy import FixedSeries, RaggedSeries
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            recording = recording_with_fields(root, ["fixed", "ragged"], [
                (0, 0.0, [[1., 2.], [3.]]), (2, 0.5, [[4., 5.], [6., 7.]])])
            convert_recording(recording, root / "processed")
            conversion = open_npy_conversion(root / "processed")
            fixed = conversion.series("signal", "fixed")
            ragged = conversion.series("signal", "ragged")
            self.assertIsInstance(fixed, FixedSeries)
            self.assertIsInstance(ragged, RaggedSeries)
            self.assertIs(conversion.series("signal", "fixed"), fixed)
            self.assertIs(fixed.iterations, ragged.iterations)
            np.testing.assert_array_equal(fixed.iterations, [0, 2])
            np.testing.assert_array_equal(ragged.record(1), [6., 7.])
            self.assertFalse(fixed.values.flags.writeable)
            for index in (-1, True, 2):
                with self.assertRaises(IndexError): ragged.record(index)
            with self.assertRaises(NpyConversionError): conversion.series("missing", "fixed")

class ParallelBatchTests(unittest.TestCase):
    def test_serial_parallel_equivalence_failure_and_retry_reuse(self):
        import os
        from unittest.mock import patch
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            sources = []
            for n in range(3):
                source = root / f"source-{n}"
                shutil.copytree(FIXTURE, source)
                sources.append(source)
            deps = root / "deps.json"
            deps.write_text(json.dumps([{"phase":"run", "tasks":[{"identity":"t", "output_directory":str(root), "workload":{"kind":"execution_unit", "execution_unit":"fixture", "members":[{"identity":str(n), "final_iteration":1, "output_directory":str(source)} for n,source in enumerate(sources)]}}]}]))
            with patch.dict(os.environ, {"WORKFLOW_THREADS":"1"}):
                serial = convert_workflow_dependencies(deps, root / "serial")
            with patch.dict(os.environ, {"WORKFLOW_THREADS":"2"}):
                parallel = convert_workflow_dependencies(deps, root / "parallel")
                self.assertEqual(serial, parallel)
                member = root / "parallel/member-000000/manifest.json"
                before = member.stat().st_mtime_ns
                self.assertEqual(convert_workflow_dependencies(deps, root / "parallel"), parallel)
                self.assertEqual(member.stat().st_mtime_ns, before)
                bad = sources[1] / "streams/signal/chunk-000000.jsonl"
                original = bad.read_bytes()
                bad.write_bytes(original + b"corrupt")
                with self.assertRaises(Exception): convert_workflow_dependencies(deps, root / "failed")
                self.assertFalse((root / "failed/manifest.json").exists())
                bad.write_bytes(original)
                result = convert_workflow_dependencies(deps, root / "failed")
                self.assertEqual(result, serial)
                self.assertEqual(len(open_npy_batch(root / "failed").members), 3)

class ConversionControlTests(unittest.TestCase):
    def test_pause_acknowledgement_resume_and_cancellation(self):
        import os
        import subprocess
        import sys
        import time
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "recording"
            shutil.copytree(FIXTURE, source)
            deps = root / "deps.json"
            deps.write_text(json.dumps([{"phase":"run", "tasks":[{"identity":"t", "output_directory":str(root), "workload":{"kind":"execution_unit","execution_unit":"fixture","members":[{"identity":"one","final_iteration":1,"output_directory":str(source)}]}}]}]))
            control = root / "control.json"
            def write_control(paused, cancelled):
                staged = control.with_suffix(".tmp")
                staged.write_text(json.dumps({"paused":paused,"cancelled":cancelled}))
                staged.replace(control)
            for cancelled in (False, True):
                output = root / f"output-{cancelled}"
                write_control(True, False)
                env = {**os.environ, "WORKFLOW_THREADS":"1", "WORKFLOW_CONTROL_PATH":str(control), "WORKFLOW_DEPENDENCIES_PATH":str(deps), "WORKFLOW_NPY_OUTPUT":str(output)}
                with (root / "stderr.log").open("w") as log:
                    child = subprocess.Popen([sys.executable,"-m","scientific_workflow.npy","--workflow-dependencies"], env=env, stdout=subprocess.DEVNULL, stderr=log)
                    try:
                        ack = control.with_name(control.name + ".parent.paused")
                        deadline = time.monotonic() + 5
                        while not ack.exists() and time.monotonic() < deadline and child.poll() is None:
                            time.sleep(0.01)
                        self.assertTrue(ack.exists())
                        self.assertFalse((output / "manifest.json").exists())
                        write_control(False, cancelled)
                        code = child.wait(timeout=5)
                        if cancelled:
                            self.assertNotEqual(code, 0)
                            self.assertFalse((output / "manifest.json").exists())
                        else:
                            self.assertEqual(code, 0)
                            self.assertEqual(len(open_npy_batch(output).members), 1)
                    finally:
                        if child.poll() is None: child.kill(); child.wait()

    def test_concurrent_same_destination_calls_do_not_conflict(self):
        from concurrent.futures import ThreadPoolExecutor
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "source"
            shutil.copytree(FIXTURE, source)
            with ThreadPoolExecutor(max_workers=2) as pool:
                futures = [pool.submit(convert_recording, source, root / "same") for _ in range(2)]
                self.assertEqual(futures[0].result(), futures[1].result())
            self.assertEqual(open_npy_conversion(root / "same").stream_names, ("signal",))

class ParallelPauseTests(unittest.TestCase):
    def test_active_spawn_workers_acknowledge_pause_and_cancel_without_success_batch(self):
        import os
        import subprocess
        import sys
        import time
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = recording_with_fields(root, ["values"], [(n, float(n), [[float(n)] * 32]) for n in range(20000)])
            second = root / "second"
            shutil.copytree(source, second)
            deps = root / "deps.json"
            deps.write_text(json.dumps([{"phase":"run","tasks":[{"identity":"t","output_directory":str(root),"workload":{"kind":"execution_unit","execution_unit":"fixture","members":[{"identity":str(n),"final_iteration":19999,"output_directory":str(path)} for n,path in enumerate((source, second))]}}]}]))
            control = root / "control.json"
            output = root / "output"
            def write(paused, cancelled):
                temporary = control.with_suffix(".tmp")
                temporary.write_text(json.dumps({"paused":paused,"cancelled":cancelled}))
                temporary.replace(control)
            write(False, False)
            env = {**os.environ,"WORKFLOW_THREADS":"2","WORKFLOW_CONTROL_PATH":str(control),"WORKFLOW_DEPENDENCIES_PATH":str(deps),"WORKFLOW_NPY_OUTPUT":str(output)}
            with (root / "stderr.log").open("w") as log:
                child = subprocess.Popen([sys.executable,"-m","scientific_workflow.npy","--workflow-dependencies"], env=env, stdout=subprocess.DEVNULL, stderr=log)
                try:
                    deadline = time.monotonic() + 8
                    while not list(output.glob(".member-*.tmp-*")) and time.monotonic() < deadline and child.poll() is None:
                        time.sleep(0.005)
                    self.assertIsNone(child.poll())
                    write(True, False)
                    ack = control.with_name(control.name + ".parent.paused")
                    while not ack.exists() and time.monotonic() < deadline and child.poll() is None:
                        time.sleep(0.01)
                    self.assertTrue(ack.exists())
                    self.assertFalse((output / "manifest.json").exists())
                    write(False, True)
                    self.assertNotEqual(child.wait(timeout=8), 0)
                    self.assertFalse((output / "manifest.json").exists())
                finally:
                    if child.poll() is None: child.kill(); child.wait()
