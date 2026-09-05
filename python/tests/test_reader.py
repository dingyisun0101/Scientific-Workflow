from __future__ import annotations

import hashlib
import json
import shutil
import tempfile
import unittest
from pathlib import Path

from scientific_workflow import (
    DecoderError,
    IntegrityError,
    MetadataError,
    RecordError,
    RecordingError,
    RecordingNotCompleteError,
    UnknownStreamError,
    FORMAT_NAME,
    FORMAT_VERSION,
    open_completed_recording,
)

FIXTURE = Path(__file__).parent / "fixtures" / "complete"
INVALID_METADATA_CASES = (
    Path(__file__).parent / "fixtures" / "invalid_metadata_cases.json"
)
REPOSITORY_ROOT = Path(__file__).parents[2]
COMPATIBILITY = REPOSITORY_ROOT / "protocol" / "compatibility.json"
PROTOCOL_SCHEMA = REPOSITORY_ROOT / "protocol" / "recording-v8.schema.json"
PYPROJECT = Path(__file__).parents[1] / "pyproject.toml"


def replace_json_pointer(document: object, pointer: str, value: object) -> None:
    """Replaces one existing list/object value in the shared mutation corpus."""
    segments = [
        segment.replace("~1", "/").replace("~0", "~")
        for segment in pointer.split("/")[1:]
    ]
    current = document
    for segment in segments[:-1]:
        if isinstance(current, list):
            current = current[int(segment)]
        elif isinstance(current, dict):
            current = current[segment]
        else:
            raise AssertionError(f"pointer {pointer!r} traverses a scalar")
    final = segments[-1]
    if isinstance(current, list):
        current[int(final)] = value
    elif isinstance(current, dict):
        current[final] = value
    else:
        raise AssertionError(f"pointer {pointer!r} ends at a scalar parent")


class ReaderTests(unittest.TestCase):
    def copied_fixture(self) -> tuple[tempfile.TemporaryDirectory[str], Path]:
        temporary = tempfile.TemporaryDirectory()
        destination = Path(temporary.name) / "recording"
        shutil.copytree(FIXTURE, destination)
        return temporary, destination

    def split_fixture(self) -> tuple[tempfile.TemporaryDirectory[str], Path]:
        temporary, recording = self.copied_fixture()
        stream_directory = recording / "streams" / "signal"
        source = stream_directory / "chunk-000000.jsonl"
        lines = source.read_bytes().splitlines(keepends=True)
        chunks = []
        for ordinal, line in enumerate(lines):
            path = stream_directory / f"chunk-{ordinal:06}.jsonl"
            path.write_bytes(line)
            record = json.loads(line)
            chunks.append(
                {
                    "ordinal": ordinal,
                    "file": path.name,
                    "records": 1,
                    "bytes": len(line),
                    "checksum": "sha256:" + hashlib.sha256(line).hexdigest(),
                    "first_iteration": record["iteration"],
                    "last_iteration": record["iteration"],
                }
            )
        metadata_path = recording / "metadata.json"
        metadata = json.loads(metadata_path.read_text())
        metadata["streams"][0]["chunks"] = chunks
        metadata_path.write_text(json.dumps(metadata))
        return temporary, recording

    def test_protocol_manifests_match_the_python_package_and_reader(self) -> None:
        compatibility = json.loads(COMPATIBILITY.read_text())
        schema = json.loads(PROTOCOL_SCHEMA.read_text())
        package_version = next(
            line.split('"')[1]
            for line in PYPROJECT.read_text().splitlines()
            if line.startswith("version = ")
        )

        self.assertEqual(compatibility["recording"], {
            "format": FORMAT_NAME,
            "version": FORMAT_VERSION,
        })
        self.assertEqual(schema["properties"]["format"]["const"], FORMAT_NAME)
        self.assertEqual(schema["properties"]["version"]["const"], FORMAT_VERSION)
        implementation = compatibility["implementations"]["python"]
        self.assertEqual(implementation["package"], "scientific-workflow")
        self.assertEqual(implementation["version"], package_version)
        self.assertEqual(implementation["writes"], [])
        self.assertEqual(implementation["reads"], [7, FORMAT_VERSION])

    def test_completed_fixture_reconstructs_exact_series_and_latest_state(self) -> None:
        reader = open_completed_recording(FIXTURE)
        self.assertEqual(reader.format_version, 7)
        self.assertEqual(reader.stream_names, ("signal",))
        self.assertEqual(reader.user_metadata["study"], "python-reader-conformance")
        self.assertEqual(
            reader.terminal_metadata["termination_reason"], "fixture_complete"
        )
        self.assertEqual(reader.stream_record_count("signal"), 2)
        self.assertEqual(reader.stream_encoded_bytes("signal"), 130)
        series = reader.read_stream("signal")
        self.assertEqual(series.iterations, (0, 2))
        self.assertEqual(series[0].physical_time, 0.0)
        self.assertEqual(series[0].values["population"], [2.0, 1.0])
        self.assertEqual(series[-1], reader.read_latest("signal"))
        self.assertEqual(reader.read_all_streams(), (("signal", series),))

    def test_object_valued_v4_record_is_rejected(self) -> None:
        temporary, recording = self.copied_fixture()
        self.addCleanup(temporary.cleanup)
        chunk = recording / "streams" / "signal" / "chunk-000000.jsonl"
        records = [json.loads(line) for line in chunk.read_text().splitlines()]
        records[0]["values"] = {"population": [2.0, 1.0], "label": "start"}
        data = "".join(
            json.dumps(record, separators=(",", ":")) + "\n" for record in records
        ).encode()
        chunk.write_bytes(data)
        metadata_path = recording / "metadata.json"
        metadata = json.loads(metadata_path.read_text())
        descriptor = metadata["streams"][0]["chunks"][0]
        descriptor["bytes"] = len(data)
        descriptor["checksum"] = "sha256:" + hashlib.sha256(data).hexdigest()
        metadata_path.write_text(json.dumps(metadata))
        with self.assertRaises(RecordError):
            open_completed_recording(recording).read_stream("signal")

    def test_field_decoders_are_explicit_and_fail_closed(self) -> None:
        reader = open_completed_recording(
            FIXTURE,
            decoders={"population": tuple, "label": str},
        )
        self.assertEqual(
            reader.read_stream("signal")[0].values["population"], (2.0, 1.0)
        )

        missing = open_completed_recording(FIXTURE, decoders={"population": tuple})
        with self.assertRaises(DecoderError):
            missing.read_stream("signal")

    def test_incremental_reader_yields_only_fully_verified_chunks(self) -> None:
        temporary, recording = self.split_fixture()
        self.addCleanup(temporary.cleanup)
        reader = open_completed_recording(recording)
        records = reader.iter_verified_records("signal")
        first = next(records)
        self.assertEqual(first.iteration, 0)

        final_chunk = recording / "streams" / "signal" / "chunk-000001.jsonl"
        final_chunk.write_bytes(final_chunk.read_bytes() + b" ")
        with self.assertRaises(IntegrityError):
            next(records)

    def test_unknown_stream_is_typed(self) -> None:
        with self.assertRaises(UnknownStreamError):
            open_completed_recording(FIXTURE).read_stream("absent")

    def test_chunk_integrity_is_mandatory(self) -> None:
        for mutation in ("missing", "size", "checksum"):
            with self.subTest(mutation=mutation):
                temporary, recording = self.copied_fixture()
                self.addCleanup(temporary.cleanup)
                chunk = recording / "streams" / "signal" / "chunk-000000.jsonl"
                if mutation == "missing":
                    chunk.unlink()
                elif mutation == "size":
                    chunk.write_bytes(chunk.read_bytes() + b" ")
                else:
                    data = bytearray(chunk.read_bytes())
                    data[data.index(ord("2"))] = ord("3")
                    chunk.write_bytes(data)
                with self.assertRaises(IntegrityError):
                    open_completed_recording(recording).read_stream("signal")

    def test_record_order_is_strict_after_integrity_update(self) -> None:
        temporary, recording = self.copied_fixture()
        self.addCleanup(temporary.cleanup)
        chunk = recording / "streams" / "signal" / "chunk-000000.jsonl"
        records = [json.loads(line) for line in chunk.read_text().splitlines()]
        records[1]["iteration"] = 0
        data = "".join(
            json.dumps(record, separators=(",", ":")) + "\n" for record in records
        )
        chunk.write_text(data)
        metadata_path = recording / "metadata.json"
        metadata = json.loads(metadata_path.read_text())
        descriptor = metadata["streams"][0]["chunks"][0]
        descriptor["bytes"] = len(data.encode())
        descriptor["checksum"] = "sha256:" + hashlib.sha256(data.encode()).hexdigest()
        descriptor["last_iteration"] = 0
        metadata_path.write_text(json.dumps(metadata))
        with self.assertRaises((MetadataError, RecordError)):
            open_completed_recording(recording).read_stream("signal")

    def test_noncomplete_lifecycle_is_rejected_at_open(self) -> None:
        temporary, recording = self.copied_fixture()
        self.addCleanup(temporary.cleanup)
        metadata_path = recording / "metadata.json"
        metadata = json.loads(metadata_path.read_text())
        metadata["status"] = {"state": "running"}
        metadata["timing"].pop("finalized_at_utc")
        metadata["terminal_metadata"] = {}
        metadata_path.write_text(json.dumps(metadata))
        with self.assertRaises(RecordingNotCompleteError):
            open_completed_recording(recording)

    def test_null_optional_fields_match_rust_option_semantics(self) -> None:
        temporary, recording = self.copied_fixture()
        self.addCleanup(temporary.cleanup)
        metadata_path = recording / "metadata.json"
        metadata = json.loads(metadata_path.read_text())
        metadata["time"]["iteration_unit"] = None
        metadata["time"]["physical_time_unit"] = None
        metadata["streams"][0]["fields"][0]["description"] = None
        metadata_path.write_text(json.dumps(metadata))

        reader = open_completed_recording(recording)
        self.assertIsNone(reader.read_stream("signal").fields[0].description)

    def test_unknown_metadata_fields_are_rejected(self) -> None:
        temporary, recording = self.copied_fixture()
        self.addCleanup(temporary.cleanup)
        metadata_path = recording / "metadata.json"
        metadata = json.loads(metadata_path.read_text())
        metadata["unsupported"] = True
        metadata_path.write_text(json.dumps(metadata))
        with self.assertRaises(MetadataError):
            open_completed_recording(recording)

    def test_shared_invalid_metadata_corpus_is_rejected(self) -> None:
        cases = json.loads(INVALID_METADATA_CASES.read_text())
        for case in cases:
            with self.subTest(case=case["name"]):
                temporary, recording = self.copied_fixture()
                self.addCleanup(temporary.cleanup)
                metadata_path = recording / "metadata.json"
                metadata = json.loads(metadata_path.read_text())
                replace_json_pointer(metadata, case["pointer"], case["value"])
                metadata_path.write_text(json.dumps(metadata))
                with self.assertRaises(RecordingError):
                    reader = open_completed_recording(recording)
                    reader.read_all_streams()


if __name__ == "__main__":
    unittest.main()
