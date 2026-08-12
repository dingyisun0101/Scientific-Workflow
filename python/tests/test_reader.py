from __future__ import annotations

import hashlib
import json
import shutil
import tempfile
import unittest
from pathlib import Path

from scientific_workflow_reader import (
    DecoderError,
    IntegrityError,
    MetadataError,
    RecordError,
    RecordingNotCompleteError,
    UnknownStreamError,
    open_completed_recording,
)

FIXTURE = Path(__file__).parent / "fixtures" / "complete"


class ReaderTests(unittest.TestCase):
    def copied_fixture(self) -> tuple[tempfile.TemporaryDirectory[str], Path]:
        temporary = tempfile.TemporaryDirectory()
        destination = Path(temporary.name) / "recording"
        shutil.copytree(FIXTURE, destination)
        return temporary, destination

    def test_completed_fixture_reconstructs_exact_series_and_latest_state(self) -> None:
        reader = open_completed_recording(FIXTURE)
        self.assertEqual(reader.format_version, 4)
        self.assertEqual(reader.stream_names, ("signal",))
        self.assertEqual(reader.user_metadata["study"], "python-reader-conformance")
        self.assertEqual(
            reader.terminal_metadata["termination_reason"], "fixture_complete"
        )
        self.assertEqual(reader.stream_record_count("signal"), 2)
        self.assertEqual(reader.stream_encoded_bytes("signal"), 172)

        series = reader.read_stream("signal")
        self.assertEqual(series.iterations, (0, 2))
        self.assertEqual(series[0].physical_time, 0.0)
        self.assertEqual(series[0].values["population"], [2.0, 1.0])
        self.assertEqual(series[-1], reader.read_latest("signal"))
        self.assertEqual(reader.read_all_streams(), (("signal", series),))

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


if __name__ == "__main__":
    unittest.main()
