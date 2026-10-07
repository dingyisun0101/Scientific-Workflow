from __future__ import annotations

import hashlib
import json
from pathlib import Path
import shutil
import tempfile
import unittest

from snapshot_fixture import seal_inputs

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
            dependencies = root / "workflow-dependencies.json"
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
            seal_inputs(dependencies.parent)
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
            deps = root / "workflow-dependencies.json"
            deps.write_text(json.dumps([{"phase":"run", "tasks":[{"identity":"t", "output_directory":str(root), "workload":{"kind":"execution_unit", "execution_unit":"fixture", "members":[{"identity":str(n), "final_iteration":1, "output_directory":str(source)} for n,source in enumerate(sources)]}}]}]))
            seal_inputs(deps.parent)
            with patch.dict(os.environ, {"WORKFLOW_THREADS":"1"}):
                serial = convert_workflow_dependencies(deps, root / "serial")
            with patch.dict(os.environ, {"WORKFLOW_THREADS":"2"}):
                parallel = convert_workflow_dependencies(deps, root / "parallel")
                self.assertEqual(serial, parallel)
                automatic = convert_workflow_dependencies(deps, root / "automatic", worker_mode="auto")
                self.assertEqual(automatic, serial)
                with self.assertRaises(NpyConversionError):
                    convert_workflow_dependencies(deps, root / "invalid-mode", worker_mode="invalid")
                self.assertFalse((root / "invalid-mode").exists())
                member = root / "parallel/member-000000/manifest.json"
                before = member.stat().st_mtime_ns
                batch_path = root / "parallel/manifest.json"
                batch_before = (batch_path.stat().st_mtime_ns, batch_path.read_bytes())
                self.assertEqual(convert_workflow_dependencies(deps, root / "parallel"), parallel)
                self.assertEqual(member.stat().st_mtime_ns, before)
                self.assertEqual((batch_path.stat().st_mtime_ns, batch_path.read_bytes()), batch_before)
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
            deps = root / "workflow-dependencies.json"
            deps.write_text(json.dumps([{"phase":"run", "tasks":[{"identity":"t", "output_directory":str(root), "workload":{"kind":"execution_unit","execution_unit":"fixture","members":[{"identity":"one","final_iteration":1,"output_directory":str(source)}]}}]}]))
            seal_inputs(deps.parent)
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
    def test_active_workers_acknowledge_pause_and_cancel_without_success_batch(self):
        import os
        import subprocess
        import sys
        import time
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = recording_with_fields(root, ["values"], [(n, float(n), [[float(n)] * 32]) for n in range(20000)])
            second = root / "second"
            shutil.copytree(source, second)
            deps = root / "workflow-dependencies.json"
            deps.write_text(json.dumps([{"phase":"run","tasks":[{"identity":"t","output_directory":str(root),"workload":{"kind":"execution_unit","execution_unit":"fixture","members":[{"identity":str(n),"final_iteration":19999,"output_directory":str(path)} for n,path in enumerate((source, second))]}}]}]))
            seal_inputs(deps.parent)
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


class NumericIntegrityTests(unittest.TestCase):
    def test_unrepresentable_integer_mixtures_retain_exact_json_and_projections(self):
        values = [-1, 9223372036854775809]
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = recording_with_fields(root, ["whole", "nested"], [
                (0, 0.0, [values, {"numbers": values}]),
            ])
            from scientific_workflow import open_completed_recording
            self.assertEqual(open_completed_recording(source).read_stream("signal")[0].values["whole"], values)
            convert_recording(source, root / "processed")
            conversion = open_npy_conversion(root / "processed")
            self.assertEqual(conversion.reconstruct("signal", "whole", 0), values)
            self.assertEqual(conversion.reconstruct("signal", "nested", 0), {"numbers": values})
            with self.assertRaises(NpyConversionError):
                conversion.series("signal", "whole")
            with self.assertRaises(NpyConversionError):
                conversion.projection("signal", "nested", "/numbers", 0)
            self.assertEqual(int(conversion.projection("signal", "nested", "/numbers/1", 0)), values[1])

    def test_large_integer_float_and_boolean_mixtures_are_lossless_fallbacks(self):
        for value in ([9007199254740993, 1.0], [True, 1], [2**80, 1]):
            with self.subTest(value=value), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                source = recording_with_fields(root, ["values"], [(0, 0.0, [value])])
                convert_recording(source, root / "processed")
                conversion = open_npy_conversion(root / "processed")
                self.assertEqual(conversion.field("signal", "values")["representation"], "structured")
                restored = conversion.reconstruct("signal", "values", 0)
                self.assertEqual(restored, value)
                self.assertEqual([type(item) for item in restored], [type(item) for item in value])

    def test_numeric_scalars_and_scalar_envelopes_preserve_rank(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = recording_with_fields(root, ["scalar", "envelope", "nested"], [
                (n, float(n), [value, {"scalar": "f64", "shape": [], "data": [value]}, {"energy": value, "label": "a"}])
                for n, value in enumerate((3.25, 4.5))
            ])
            convert_recording(source, root / "processed")
            conversion = open_npy_conversion(root / "processed")
            for field in ("scalar", "envelope"):
                self.assertEqual(conversion.field("signal", field)["dataset"]["rank"], 0)
                self.assertEqual(conversion.series("signal", field).values.shape, (2,))
                self.assertEqual(conversion.reconstruct("signal", field, 0).shape, ())
            self.assertEqual(conversion.series("signal", "nested", "/energy").values.shape, (2,))

    def test_declared_numeric_types_reject_coercion_and_overflow(self):
        for scalar, data in (("i8", [1.9]), ("i8", [128]), ("u8", [-1]),
                             ("bool", ["false"]), ("bool", [0]),
                             ("f32", [1e100]), ("f64", [True])):
            with self.subTest(scalar=scalar, data=data), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                source = recording_with_fields(root, ["typed"], [(0, 0.0, [{"scalar": scalar, "shape": [1], "data": data}])])
                with self.assertRaises(NpyConversionError):
                    convert_recording(source, root / "processed")
                self.assertFalse((root / "processed").exists())
                self.assertFalse(list(root.glob(".processed.tmp-*")))

    def test_declared_float32_allows_its_normal_rounding(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = recording_with_fields(root, ["typed"], [(0, 0.0, [{"scalar": "f32", "shape": [1], "data": [0.1]}])])
            convert_recording(source, root / "processed")
            result = open_npy_conversion(root / "processed").reconstruct("signal", "typed", 0)
            self.assertEqual(result.dtype, np.dtype("float32"))
            self.assertEqual(result[0], np.float32(0.1))


class LayoutIntegrityTests(unittest.TestCase):
    def test_role_ownership_missing_coordinates_and_legacy_formats_fail_at_open(self):
        import copy
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = recording_with_fields(root, ["values"], [(0, 0.0, [[1., 2.]]), (1, 1.0, [[3., 4.]])])
            output = root / "processed"
            original = convert_recording(source, output)
            mutations = (
                lambda m: m.update(format="scientific-workflow-npy.v2"),
                lambda m: m["arrays"][0].update(role="unused"),
                lambda m: m["arrays"][-1].update(role="iterations"),
                lambda m: m["arrays"][-1].update(stream="different"),
                lambda m: m["arrays"][-1].update(logical_path="/wrong"),
                lambda m: m["streams"][0].update(records=True),
            )
            for mutate in mutations:
                manifest = copy.deepcopy(original)
                mutate(manifest)
                (output / "manifest.json").write_text(json.dumps(manifest))
                with self.subTest(manifest=manifest), self.assertRaises(NpyConversionError):
                    open_npy_conversion(output)

    def test_coordinate_dtype_shape_order_and_finiteness_are_checked_independently_of_hashes(self):
        mutations = (
            ("iterations", np.array([0., 1.], dtype=np.float64)),
            ("iterations", np.array([[0], [1]], dtype=np.uint64)),
            ("iterations", np.array([0], dtype=np.uint64)),
            ("iterations", np.array([1, 0], dtype=np.uint64)),
            ("iterations", np.array([0, 0], dtype=np.uint64)),
            ("physical_times", np.array([0., np.inf], dtype=np.float64)),
        )
        for role, value in mutations:
            with self.subTest(role=role, value=value), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                source = recording_with_fields(root, ["values"], [(0, 0.0, [[1.]]), (1, 1.0, [[2.]])])
                output = root / "processed"
                manifest = convert_recording(source, output)
                descriptor = next(entry for entry in manifest["arrays"] if entry["role"] == role)
                path = output / descriptor["path"]
                np.save(path, value, allow_pickle=False)
                descriptor.update(dtype=value.dtype.str, shape=list(value.shape), checksum="sha256:" + hashlib.sha256(path.read_bytes()).hexdigest())
                (output / "manifest.json").write_text(json.dumps(manifest))
                with self.assertRaises(NpyConversionError):
                    open_npy_conversion(output)


class RepeatedBatchControlTests(unittest.TestCase):
    def test_later_batches_receive_current_control_configuration(self):
        import os
        import subprocess
        import sys
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / "source"
            shutil.copytree(FIXTURE, source)
            second = root / "second"
            shutil.copytree(source, second)
            deps = root / "workflow-dependencies.json"
            deps.write_text(json.dumps([{"phase": "run", "tasks": [{"identity": "t", "output_directory": str(root), "workload": {
                "kind": "execution_unit", "execution_unit": "fixture", "members": [
                    {"identity": str(n), "final_iteration": 2, "output_directory": str(path)} for n, path in enumerate((source, second))
                ]}}]}]))
            seal_inputs(root)
            script = root / "batch_runner.py"
            script.write_text('''import json, os, sys
from pathlib import Path
from scientific_workflow.npy import convert_workflow_dependencies, open_npy_batch
def main():
    root = Path(sys.argv[1])
    os.environ["WORKFLOW_THREADS"] = "2"
    first = root / "first-control.json"
    second = root / "second-control.json"
    for path in (first, second):
        path.write_text(json.dumps({"paused": False, "cancelled": False}))
    os.environ["WORKFLOW_CONTROL_PATH"] = str(first)
    convert_workflow_dependencies(root / "workflow-dependencies.json", root / "first-output")
    first.unlink()
    os.environ["WORKFLOW_CONTROL_PATH"] = str(second)
    convert_workflow_dependencies(root / "workflow-dependencies.json", root / "second-output")
    assert len(open_npy_batch(root / "second-output").members) == 2
if __name__ == "__main__":
    main()
''')
            result = subprocess.run([sys.executable, str(script), str(root)], env=os.environ.copy(),
                                    capture_output=True, text=True, timeout=20)
            self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
