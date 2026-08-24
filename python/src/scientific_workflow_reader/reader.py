"""Verified eager reader for Scientific Workflow JSONL format version 7."""

from __future__ import annotations

import hashlib
import json
import math
import re
from datetime import datetime
from pathlib import Path
from types import MappingProxyType
from collections.abc import Iterator
from typing import Any, Callable, Mapping

from .errors import (
    DecoderError,
    IntegrityError,
    MetadataError,
    RecordError,
    RecordingNotCompleteError,
    UnknownStreamError,
)
from .model import StateField, StateRecord, StateSeries

FORMAT_NAME = "scientific-workflow-jsonl"
FORMAT_VERSION = 7
METADATA_FILE = "metadata.json"

Decoder = Callable[[Any], Any]
_CHECKSUM = re.compile(r"^([a-z0-9]+):([0-9a-f]+)$")
_UTC_RFC3339 = re.compile(
    r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z$"
)
_TOP_LEVEL_KEYS = {
    "format",
    "version",
    "status",
    "timing",
    "records",
    "time",
    "user_metadata",
    "terminal_metadata",
    "streams",
}


def _strict_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    output: dict[str, Any] = {}
    for key, value in pairs:
        if key in output:
            raise ValueError(f"duplicate object key {key!r}")
        output[key] = value
    return output


def _reject_constant(value: str) -> None:
    raise ValueError(f"nonstandard JSON number {value}")


def _load_json(data: bytes, location: Path) -> Any:
    try:
        return json.loads(
            data,
            object_pairs_hook=_strict_object,
            parse_constant=_reject_constant,
        )
    except (json.JSONDecodeError, UnicodeDecodeError, ValueError) as error:
        raise MetadataError(f"invalid JSON at {location}: {error}") from error


def _exact_keys(
    value: Mapping[str, Any],
    required: set[str],
    optional: set[str],
    context: str,
) -> None:
    keys = set(value)
    missing = required - keys
    extra = keys - required - optional
    if missing or extra:
        raise MetadataError(
            f"{context} has missing keys {sorted(missing)} and unknown keys {sorted(extra)}"
        )


def _mapping(value: Any, context: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise MetadataError(f"{context} must be an object")
    return value


def _list(value: Any, context: str) -> list[Any]:
    if not isinstance(value, list):
        raise MetadataError(f"{context} must be an array")
    return value


def _string(value: Any, context: str, *, nonempty: bool = True) -> str:
    if not isinstance(value, str) or (nonempty and not value.strip()):
        raise MetadataError(f"{context} must be a nonempty string")
    return value


def _uint(value: Any, context: str, *, positive: bool = False) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise MetadataError(f"{context} must be a nonnegative integer")
    if positive and value == 0:
        raise MetadataError(f"{context} must be positive")
    if value > 2**64 - 1:
        raise MetadataError(f"{context} exceeds u64")
    return value


def _safe_relative(value: Any, context: str) -> str:
    path_text = _string(value, context)
    path = Path(path_text)
    textual_parts = re.split(r"[/\\]", path_text)
    if (
        path.is_absolute()
        or any(part in {"", ".", ".."} for part in textual_parts)
        or path_text.startswith(("/", "\\"))
    ):
        raise MetadataError(f"{context} must be a safe relative path")
    return path_text


def _utc_timestamp(value: Any, context: str) -> str:
    text = _string(value, context)
    if _UTC_RFC3339.fullmatch(text) is None:
        raise MetadataError(f"{context} must be a UTC RFC 3339 timestamp")
    try:
        datetime.fromisoformat(text[:-1] + "+00:00")
    except ValueError as error:
        raise MetadataError(f"{context} must be a UTC RFC 3339 timestamp") from error
    return text


def _validate_metadata(document: Any, path: Path) -> dict[str, Any]:
    metadata = _mapping(document, "metadata")
    _exact_keys(
        metadata,
        _TOP_LEVEL_KEYS - {"user_metadata", "terminal_metadata"},
        {"user_metadata", "terminal_metadata"},
        "metadata",
    )
    if metadata["format"] != FORMAT_NAME:
        raise MetadataError(f"metadata format must be {FORMAT_NAME!r}")
    version = _uint(metadata["version"], "metadata.version")
    if version != FORMAT_VERSION:
        raise MetadataError(
            f"unsupported metadata version {metadata['version']!r}; supported version is 7"
        )

    status = _mapping(metadata["status"], "status")
    state = status.get("state")
    if state == "complete":
        _exact_keys(status, {"state"}, set(), "complete status")
    elif state == "running":
        _exact_keys(status, {"state"}, set(), "running status")
    elif state == "failed":
        _exact_keys(status, {"state", "message"}, set(), "failed status")
        _string(status["message"], "failed status message")
    else:
        raise MetadataError(f"unsupported recording status {state!r}")

    records = _mapping(metadata["records"], "records")
    _exact_keys(records, {"encoding", "framing"}, set(), "records")
    if records != {"encoding": "json", "framing": "json_lines"}:
        raise MetadataError("records must use json encoding and json_lines framing")

    time = _mapping(metadata["time"], "time")
    _exact_keys(
        time,
        {"iteration_name"},
        {"iteration_unit", "physical_time_name", "physical_time_unit"},
        "time",
    )
    _string(time["iteration_name"], "time.iteration_name")
    for key in ("iteration_unit", "physical_time_name", "physical_time_unit"):
        if time.get(key) is not None:
            _string(time[key], f"time.{key}")
    if (
        time.get("physical_time_unit") is not None
        and time.get("physical_time_name") is None
    ):
        raise MetadataError("time.physical_time_unit requires physical_time_name")

    timing = _mapping(metadata["timing"], "timing")
    required_timing = {
        "created_at_utc",
        "active_duration_ns",
        "continuation_count",
    }
    _exact_keys(timing, required_timing, {"finalized_at_utc"}, "timing")
    _utc_timestamp(timing["created_at_utc"], "timing.created_at_utc")
    finalized_at = timing.get("finalized_at_utc")
    if finalized_at is not None:
        _utc_timestamp(finalized_at, "timing.finalized_at_utc")
    if state == "running" and finalized_at is not None:
        raise MetadataError("running recording must not have finalized_at_utc")
    if state != "running" and finalized_at is None:
        raise MetadataError("terminal recording requires finalized_at_utc")
    _uint(timing["active_duration_ns"], "timing.active_duration_ns")
    _uint(timing["continuation_count"], "timing.continuation_count")

    for key in ("user_metadata", "terminal_metadata"):
        if key in metadata:
            _mapping(metadata[key], key)
        else:
            metadata[key] = {}
    if state == "running" and metadata["terminal_metadata"]:
        raise MetadataError("running recording must not contain terminal_metadata")

    streams = _list(metadata["streams"], "streams")
    if not streams:
        raise MetadataError("at least one stream must be declared")
    names: set[str] = set()
    directories: set[str] = set()
    for index, raw_stream in enumerate(streams):
        stream = _mapping(raw_stream, f"streams[{index}]")
        _exact_keys(
            stream,
            {
                "name",
                "directory",
                "sampling_interval",
                "fields",
                "storage",
            },
            {"chunks"},
            f"streams[{index}]",
        )
        name = _string(stream["name"], f"streams[{index}].name")
        directory = _safe_relative(
            stream["directory"], f"streams[{index}].directory"
        )
        if name in names:
            raise MetadataError(f"duplicate stream name {name!r}")
        if directory in directories:
            raise MetadataError(f"duplicate stream directory {directory!r}")
        names.add(name)
        directories.add(directory)
        interval = _mapping(stream["sampling_interval"], f"stream {name} interval")
        _exact_keys(interval, {"iterations"}, set(), f"stream {name} interval")
        _uint(interval["iterations"], f"stream {name} interval", positive=True)
        storage = _mapping(stream["storage"], f"stream {name} storage")
        _exact_keys(
            storage,
            {"layout", "storage_queue_bytes"},
            set(),
            f"stream {name} storage",
        )
        _uint(
            storage["storage_queue_bytes"],
            f"stream {name} storage_queue_bytes",
            positive=True,
        )
        layout = _mapping(storage["layout"], f"stream {name} layout")
        if layout.get("kind") == "chunked":
            _exact_keys(layout, {"kind", "target_bytes"}, set(), f"stream {name} layout")
            _uint(layout["target_bytes"], f"stream {name} target_bytes", positive=True)
        elif layout.get("kind") == "individual_files":
            _exact_keys(layout, {"kind"}, set(), f"stream {name} layout")
        else:
            raise MetadataError(f"stream {name!r} has unsupported storage layout")

        fields = _list(stream["fields"], f"stream {name} fields")
        field_names: set[str] = set()
        for position, raw_field in enumerate(fields):
            field = _mapping(raw_field, f"stream {name} field {position}")
            _exact_keys(field, {"name"}, {"description"}, f"stream {name} field")
            field_name = _string(field["name"], f"stream {name} field name")
            if field_name in field_names:
                raise MetadataError(f"stream {name!r} repeats field {field_name!r}")
            field_names.add(field_name)
            if field.get("description") is not None:
                _string(field["description"], f"stream {name} field description")

        chunks = _list(stream.get("chunks", []), f"stream {name} chunks")
        stream["chunks"] = chunks
        previous_last: int | None = None
        for ordinal, raw_chunk in enumerate(chunks):
            chunk = _mapping(raw_chunk, f"stream {name} chunk {ordinal}")
            _exact_keys(
                chunk,
                {
                    "ordinal",
                    "file",
                    "records",
                    "bytes",
                    "checksum",
                    "first_iteration",
                    "last_iteration",
                },
                set(),
                f"stream {name} chunk {ordinal}",
            )
            if _uint(chunk["ordinal"], "chunk ordinal") != ordinal:
                raise MetadataError(f"stream {name!r} has non-contiguous chunk ordinals")
            expected_file = f"chunk-{ordinal:06}.jsonl"
            if _safe_relative(chunk["file"], "chunk file") != expected_file:
                raise MetadataError(f"chunk {ordinal} filename must be {expected_file!r}")
            records = _uint(chunk["records"], "chunk records", positive=True)
            if layout["kind"] == "individual_files" and records != 1:
                raise MetadataError(
                    f"individual-files stream {name!r} chunk {ordinal} must contain one record"
                )
            _uint(chunk["bytes"], "chunk bytes", positive=True)
            first = _uint(chunk["first_iteration"], "chunk first_iteration")
            last = _uint(chunk["last_iteration"], "chunk last_iteration")
            if first > last:
                raise MetadataError("chunk iteration range is reversed")
            if previous_last is not None and first <= previous_last:
                raise MetadataError("chunk iteration ranges are not strictly ordered")
            previous_last = last
            checksum = _string(chunk["checksum"], "chunk checksum")
            if _CHECKSUM.fullmatch(checksum) is None:
                raise MetadataError("chunk checksum has invalid syntax")
    if state == "running":
        raise RecordingNotCompleteError(f"recording at {path.parent} is still running")
    if state == "failed":
        raise RecordingNotCompleteError(f"recording at {path.parent} failed")
    return metadata


class RecordingReader:
    """Validated authority for one successfully completed recording."""

    def __init__(
        self,
        directory: str | Path,
        decoders: Mapping[str, Decoder] | None = None,
    ) -> None:
        self._root = Path(directory)
        self._metadata_path = self._root / METADATA_FILE
        try:
            data = self._metadata_path.read_bytes()
        except OSError as error:
            raise MetadataError(f"cannot read {self._metadata_path}: {error}") from error
        self._metadata = _validate_metadata(
            _load_json(data, self._metadata_path), self._metadata_path
        )
        self._decoders = dict(decoders) if decoders is not None else None
        self._streams = {stream["name"]: stream for stream in self._metadata["streams"]}

    @property
    def directory(self) -> Path:
        return self._root

    @property
    def format_version(self) -> int:
        return FORMAT_VERSION

    @property
    def stream_names(self) -> tuple[str, ...]:
        return tuple(self._streams)

    @property
    def user_metadata(self) -> Mapping[str, Any]:
        return MappingProxyType(self._metadata["user_metadata"])

    @property
    def terminal_metadata(self) -> Mapping[str, Any]:
        return MappingProxyType(self._metadata["terminal_metadata"])

    @property
    def timing(self) -> Mapping[str, Any]:
        return MappingProxyType(self._metadata["timing"])

    def stream_record_count(self, stream: str) -> int:
        declaration = self._stream(stream)
        return self._sum_u64(declaration, "records")

    def stream_encoded_bytes(self, stream: str) -> int:
        declaration = self._stream(stream)
        return self._sum_u64(declaration, "bytes")

    @staticmethod
    def _sum_u64(declaration: dict[str, Any], key: str) -> int:
        total = 0
        for chunk in declaration["chunks"]:
            total += chunk[key]
            if total > 2**64 - 1:
                raise IntegrityError(
                    f"stream {declaration['name']!r} {key} total exceeds u64"
                )
        return total

    def read_stream(self, stream: str) -> StateSeries:
        """Eagerly verifies and reconstructs an entire stream transactionally."""
        declaration = self._stream(stream)
        fields = self._fields(declaration)
        return StateSeries(stream, fields, tuple(self.iter_verified_records(stream)))

    def iter_verified_records(self, stream: str) -> Iterator[StateRecord]:
        """Incrementally yields records from fully verified chunks.

        Each chunk's size, checksum, framing, record count, descriptor facts,
        and ordering are validated before its first record is yielded. A later
        chunk may still fail after records from earlier chunks were consumed;
        callers requiring an all-or-nothing result must use :meth:`read_stream`.
        """
        declaration = self._stream(stream)
        fields = self._fields(declaration)
        self._require_decoders(fields)
        previous: int | None = None
        for chunk in declaration["chunks"]:
            records, previous = self._read_chunk(declaration, chunk, previous)
            yield from records

    def read_all_streams(self) -> tuple[tuple[str, StateSeries], ...]:
        return tuple(
            (name, self.read_stream(name)) for name in self.stream_names
        )

    def read_latest(self, stream: str) -> StateRecord:
        """Verifies the newest chunk and reconstructs only its final state."""
        declaration = self._stream(stream)
        fields = self._fields(declaration)
        self._require_decoders(fields)
        if not declaration["chunks"]:
            raise RecordError(f"stream {stream!r} contains no recorded state")
        chunk = declaration["chunks"][-1]
        data = self._verified_chunk_bytes(declaration, chunk)
        if not data.endswith(b"\n"):
            raise RecordError("latest chunk is not newline terminated")
        line = data[:-1].rsplit(b"\n", 1)[-1]
        if not line:
            raise RecordError("latest record is empty")
        record = self._decode_record(line, declaration, chunk["records"])
        if record.iteration != chunk["last_iteration"]:
            raise RecordError("latest record differs from chunk descriptor")
        return record

    def _stream(self, name: str) -> dict[str, Any]:
        try:
            return self._streams[name]
        except KeyError as error:
            raise UnknownStreamError(name) from error

    @staticmethod
    def _fields(declaration: dict[str, Any]) -> tuple[StateField, ...]:
        return tuple(
            StateField(field["name"], field.get("description"))
            for field in declaration["fields"]
        )

    def _require_decoders(self, fields: tuple[StateField, ...]) -> None:
        if self._decoders is None:
            return
        missing = [field.name for field in fields if field.name not in self._decoders]
        if missing:
            raise DecoderError(f"missing field decoders: {missing}")

    def _verified_chunk_bytes(
        self, declaration: dict[str, Any], chunk: dict[str, Any]
    ) -> bytes:
        path = self._root / declaration["directory"] / chunk["file"]
        try:
            stat = path.stat()
        except FileNotFoundError as error:
            raise IntegrityError(f"missing chunk {path}") from error
        except OSError as error:
            raise IntegrityError(f"cannot inspect chunk {path}: {error}") from error
        if stat.st_size != chunk["bytes"]:
            raise IntegrityError(
                f"chunk {path} has {stat.st_size} bytes; expected {chunk['bytes']}"
            )
        try:
            data = path.read_bytes()
        except OSError as error:
            raise IntegrityError(f"cannot read chunk {path}: {error}") from error
        algorithm, expected = chunk["checksum"].split(":", 1)
        if algorithm != "sha256":
            raise IntegrityError(f"unsupported checksum algorithm {algorithm!r}")
        actual = hashlib.sha256(data).hexdigest()
        if actual != expected:
            raise IntegrityError(
                f"checksum mismatch for {path}: expected {expected}, got {actual}"
            )
        return data

    def _read_chunk(
        self,
        declaration: dict[str, Any],
        chunk: dict[str, Any],
        previous: int | None,
    ) -> tuple[list[StateRecord], int | None]:
        data = self._verified_chunk_bytes(declaration, chunk)
        if not data.endswith(b"\n"):
            raise RecordError(f"chunk {chunk['file']!r} is not newline terminated")
        raw_lines = data[:-1].split(b"\n")
        if any(not line for line in raw_lines):
            raise RecordError(f"chunk {chunk['file']!r} contains an empty record")
        records: list[StateRecord] = []
        for line_number, line in enumerate(raw_lines, 1):
            record = self._decode_record(line, declaration, line_number)
            if previous is not None and record.iteration <= previous:
                raise RecordError(
                    f"iteration {record.iteration} is not greater than {previous}"
                )
            previous = record.iteration
            records.append(record)
        if len(records) != chunk["records"]:
            raise IntegrityError("chunk record count differs from descriptor")
        if not records:
            raise IntegrityError("committed chunk is empty")
        if records[0].iteration != chunk["first_iteration"]:
            raise IntegrityError("chunk first iteration differs from descriptor")
        if records[-1].iteration != chunk["last_iteration"]:
            raise IntegrityError("chunk last iteration differs from descriptor")
        return records, previous

    def _decode_record(
        self, line: bytes, declaration: dict[str, Any], line_number: int
    ) -> StateRecord:
        try:
            document = json.loads(
                line,
                object_pairs_hook=_strict_object,
                parse_constant=_reject_constant,
            )
        except (json.JSONDecodeError, UnicodeDecodeError, ValueError) as error:
            raise RecordError(f"invalid record at line {line_number}: {error}") from error
        if not isinstance(document, dict):
            raise RecordError(f"record at line {line_number} must be an object")
        allowed = {"iteration", "physical_time", "values"}
        if set(document) - allowed or not {"iteration", "values"} <= set(document):
            raise RecordError(f"record at line {line_number} has invalid keys")
        iteration = document["iteration"]
        if (
            isinstance(iteration, bool)
            or not isinstance(iteration, int)
            or not 0 <= iteration <= 2**64 - 1
        ):
            raise RecordError(f"record at line {line_number} has invalid iteration")
        physical = document.get("physical_time")
        if physical is not None:
            if isinstance(physical, bool) or not isinstance(physical, (int, float)):
                raise RecordError(f"record at line {line_number} has invalid physical time")
            physical = float(physical)
            if not math.isfinite(physical):
                raise RecordError(f"record at line {line_number} has nonfinite physical time")
        raw_values = document["values"]
        if not isinstance(raw_values, list):
            raise RecordError(f"record at line {line_number} values must be an array")
        expected = [field["name"] for field in declaration["fields"]]
        if len(raw_values) != len(expected):
            raise RecordError(
                f"record at line {line_number} has {len(raw_values)} values "
                f"but stream declares {len(expected)} fields"
            )
        decoded: dict[str, Any] = {}
        for name, value in zip(expected, raw_values, strict=True):
            if self._decoders is not None:
                try:
                    value = self._decoders[name](value)
                except Exception as error:
                    raise DecoderError(
                        f"decoder for field {name!r} failed at iteration {iteration}"
                    ) from error
            decoded[name] = value
        return StateRecord.create(iteration, physical, decoded)


def open_completed_recording(
    directory: str | Path,
    decoders: Mapping[str, Decoder] | None = None,
) -> RecordingReader:
    """Opens and validates one successfully completed Workflow recording."""
    return RecordingReader(directory, decoders)
