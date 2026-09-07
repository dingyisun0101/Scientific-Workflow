from __future__ import annotations

from dataclasses import dataclass
import hashlib
import json
import math
import os
from pathlib import Path
import re
from collections.abc import Iterable, Mapping
from typing import Any

import numpy as np

from .. import _control

NPY_FORMAT = "scientific-workflow-npy.v2"
NPY_BATCH_FORMAT = "scientific-workflow-npy-batch.v2"
MANIFEST_FILE = "manifest.json"
JsonPath = tuple[str, ...]

class NpyConversionError(ValueError):
    """A recording or converted dataset violates the NPY contract."""


def _normalize_exclusions(names: Iterable[str]) -> tuple[str, ...]:
    if isinstance(names, (str, bytes)) or not isinstance(names, Iterable):
        raise NpyConversionError("exclude_streams must be an iterable of stream names")
    result: set[str] = set()
    for name in names:
        if not isinstance(name, str) or not name or name.strip() != name:
            raise NpyConversionError("excluded stream names must be nonempty strings without surrounding whitespace")
        if name in result:
            raise NpyConversionError(f"duplicate excluded stream name: {name!r}")
        result.add(name)
    return tuple(sorted(result))


def _manifest_exclusions(document: Mapping[str, object]) -> tuple[str, ...]:
    names = document.get("exclude_streams", [])
    if not isinstance(names, list):
        raise NpyConversionError("manifest exclude_streams must be an array")
    return _normalize_exclusions(names)


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            _control.checkpoint()
            digest.update(block)
    return "sha256:" + digest.hexdigest()


def _safe_name(value: str) -> str:
    name = re.sub(r"[^A-Za-z0-9_.-]+", "_", value).strip("._")
    return name or "unnamed"


def _pointer(path: JsonPath) -> str:
    if not path:
        return ""
    return "/" + "/".join(part.replace("~", "~0").replace("/", "~1") for part in path)


def _write_json(path: Path, document: Mapping[str, object]) -> None:
    with path.open("x", encoding="utf-8") as handle:
        json.dump(document, handle, indent=2, sort_keys=True)
        handle.write("\n")
        handle.flush()
        os.fsync(handle.fileno())


def _read_json(path: Path) -> dict[str, Any]:
    try:
        document = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise NpyConversionError(f"cannot read conversion manifest {path}") from error
    if not isinstance(document, dict):
        raise NpyConversionError(f"conversion manifest must be an object: {path}")
    return document


def _component_path(root: Path, value: object) -> Path:
    if not isinstance(value, str):
        raise NpyConversionError("array descriptor path must be a string")
    path = Path(value)
    if path.is_absolute() or ".." in path.parts:
        raise NpyConversionError(f"array path is not safe and relative: {value!r}")
    try:
        resolved = (root / path).resolve(strict=True)
    except OSError as error:
        raise NpyConversionError(f"cannot resolve converted array: {value!r}") from error
    if not resolved.is_relative_to(root.resolve()):
        raise NpyConversionError(f"array path escapes converted directory: {value!r}")
    return resolved


def _validate_arrays(root: Path, document: Mapping[str, object]) -> None:
    arrays = document.get("arrays")
    if not isinstance(arrays, list):
        raise NpyConversionError("conversion manifest arrays must be an array")
    seen: set[str] = set()
    for raw in arrays:
        if not isinstance(raw, dict):
            raise NpyConversionError("conversion array descriptor must be an object")
        relative = raw.get("path")
        if not isinstance(relative, str) or relative in seen:
            raise NpyConversionError("conversion array paths must be unique strings")
        seen.add(relative)
        path = _component_path(root, relative)
        try:
            array = np.load(path, mmap_mode="r", allow_pickle=False)
        except (OSError, ValueError) as error:
            raise NpyConversionError(f"cannot open converted array: {path}") from error
        if (
            array.dtype.str != raw.get("dtype")
            or list(array.shape) != raw.get("shape")
            or not array.flags.c_contiguous
            or raw.get("c_contiguous") is not True
            or _sha256(path) != raw.get("checksum")
        ):
            raise NpyConversionError(f"converted array does not match manifest: {path}")


def _declared_component(
    root: Path, declared: set[str], dataset: Mapping[str, object], key: str
) -> np.ndarray[Any, Any]:
    relative = dataset.get(key)
    if not isinstance(relative, str) or relative not in declared:
        raise NpyConversionError(f"dataset {key!r} must name a declared array")
    return np.load(
        _component_path(root, relative), mmap_mode="r", allow_pickle=False
    )


def _validate_offsets(offsets: np.ndarray[Any, Any], count: int, size: int) -> None:
    if offsets.dtype != np.dtype("uint64") or offsets.shape != (count + 1,):
        raise NpyConversionError("offsets must be a uint64 array of record_count + 1")
    if int(offsets[0]) != 0 or int(offsets[-1]) != size:
        raise NpyConversionError("offsets do not span their complete data array")
    if np.any(offsets[1:] < offsets[:-1]):
        raise NpyConversionError("offsets must be nondecreasing")


def _validate_numeric_dataset(
    root: Path,
    declared: set[str],
    dataset: Mapping[str, object],
    count: int,
) -> None:
    dtype = dataset.get("dtype")
    rank = dataset.get("rank")
    storage = dataset.get("storage")
    if not isinstance(dtype, str):
        raise NpyConversionError("numeric dataset dtype must be a string")
    if isinstance(rank, bool) or not isinstance(rank, int) or rank < 0:
        raise NpyConversionError("numeric dataset rank must be unsigned")
    data = _declared_component(root, declared, dataset, "data")
    if data.dtype.str != dtype:
        raise NpyConversionError("numeric dataset dtype differs from its data array")
    if storage == "fixed":
        if data.ndim != rank + 1 or data.shape[0] != count:
            raise NpyConversionError("fixed dataset shape does not match rank and records")
        return
    if storage != "ragged" or data.ndim != 1:
        raise NpyConversionError(f"unknown or invalid numeric storage mode: {storage!r}")
    offsets = _declared_component(root, declared, dataset, "offsets")
    shapes = _declared_component(root, declared, dataset, "shapes")
    _validate_offsets(offsets, count, len(data))
    if shapes.dtype != np.dtype("uint64") or shapes.shape != (count, rank):
        raise NpyConversionError("ragged shapes must be uint64 with record/rank shape")
    for index in range(count):
        elements = math.prod(int(extent) for extent in shapes[index])
        if int(offsets[index + 1]) - int(offsets[index]) != elements:
            raise NpyConversionError(
                f"ragged record {index} offsets disagree with its shape"
            )


def _validate_json_fallback(
    root: Path,
    declared: set[str],
    fallback: Mapping[str, object],
    count: int,
) -> None:
    if fallback.get("storage") != "json_bytes" or fallback.get("encoding") != "utf-8-json":
        raise NpyConversionError("structured fallback must use UTF-8 JSON bytes")
    data = _declared_component(root, declared, fallback, "data")
    offsets = _declared_component(root, declared, fallback, "offsets")
    if data.dtype != np.dtype("uint8") or data.ndim != 1:
        raise NpyConversionError("structured fallback data must be flat uint8")
    _validate_offsets(offsets, count, len(data))
    for index in range(count):
        start, stop = int(offsets[index]), int(offsets[index + 1])
        try:
            json.loads(bytes(data[start:stop]).decode("utf-8"))
        except (UnicodeError, json.JSONDecodeError) as error:
            raise NpyConversionError(
                f"structured fallback record {index} is not valid JSON"
            ) from error


def _validate_layout(root: Path, document: Mapping[str, object]) -> None:
    excluded = _manifest_exclusions(document)
    arrays = document.get("arrays")
    streams = document.get("streams")
    if not isinstance(arrays, list) or not isinstance(streams, list):
        raise NpyConversionError("conversion manifest arrays and streams must be arrays")
    declared = {
        descriptor["path"]
        for descriptor in arrays
        if isinstance(descriptor, dict) and isinstance(descriptor.get("path"), str)
    }
    stream_names: set[str] = set()
    for stream in streams:
        if not isinstance(stream, Mapping):
            raise NpyConversionError("converted stream metadata must be an object")
        name = stream.get("name")
        count = stream.get("records")
        fields = stream.get("fields")
        if not isinstance(name, str) or not name or name in stream_names:
            raise NpyConversionError("converted stream names must be unique nonempty strings")
        if name in excluded:
            raise NpyConversionError(f"excluded stream is present in converted output: {name!r}")
        stream_names.add(name)
        if isinstance(count, bool) or not isinstance(count, int) or count <= 0:
            raise NpyConversionError("converted stream record count must be positive")
        if not isinstance(fields, list) or not fields:
            raise NpyConversionError("converted stream fields must be a nonempty array")
        field_names: set[str] = set()
        for field in fields:
            if not isinstance(field, Mapping):
                raise NpyConversionError("converted field metadata must be an object")
            field_name = field.get("name")
            if (
                not isinstance(field_name, str)
                or not field_name
                or field_name in field_names
            ):
                raise NpyConversionError(
                    "converted field names must be unique nonempty strings"
                )
            field_names.add(field_name)
            representation = field.get("representation")
            if representation == "numeric":
                dataset = field.get("dataset")
                if not isinstance(dataset, Mapping):
                    raise NpyConversionError("numeric field must contain dataset metadata")
                _validate_numeric_dataset(root, declared, dataset, count)
                continue
            if representation != "structured":
                raise NpyConversionError(
                    f"unknown field representation: {representation!r}"
                )
            fallback = field.get("fallback")
            projections = field.get("projections")
            if not isinstance(fallback, Mapping) or not isinstance(projections, list):
                raise NpyConversionError(
                    "structured field must contain fallback and projection metadata"
                )
            _validate_json_fallback(root, declared, fallback, count)
            paths: set[str] = set()
            for projection in projections:
                if not isinstance(projection, Mapping):
                    raise NpyConversionError("numeric projection metadata must be an object")
                path = projection.get("logical_path")
                if not isinstance(path, str) or path in paths:
                    raise NpyConversionError("numeric projection paths must be unique strings")
                paths.add(path)
                _validate_numeric_dataset(root, declared, projection, count)


def _existing(
    output: Path,
    recording: Path,
    metadata_checksum: str,
    exclude_streams: tuple[str, ...],
) -> dict[str, object] | None:
    manifest_path = output / MANIFEST_FILE
    if not manifest_path.is_file():
        return None
    try:
        document = _read_json(manifest_path)
        if (
            document.get("format") != NPY_FORMAT
            or document.get("source_recording") != str(recording)
            or document.get("source_metadata_checksum") != metadata_checksum
            or _manifest_exclusions(document) != exclude_streams
        ):
            return None
        _validate_arrays(output, document)
        _validate_layout(output, document)
    except NpyConversionError:
        return None
    return document


@dataclass(frozen=True, slots=True)
class FixedSeries:
    """Read-only memory-mapped fixed-shape values and stream coordinates."""
    values: np.ndarray[Any, Any]
    iterations: np.ndarray[Any, Any]
    physical_times: np.ndarray[Any, Any] | None

    def __len__(self) -> int:
        return len(self.values)

    def record(self, index: int) -> np.ndarray[Any, Any]:
        _check_record(index, len(self))
        return self.values[index]


@dataclass(frozen=True, slots=True)
class RaggedSeries:
    """Read-only flattened values with offsets and per-record logical shapes."""
    data: np.ndarray[Any, Any]
    offsets: np.ndarray[Any, Any]
    shapes: np.ndarray[Any, Any]
    iterations: np.ndarray[Any, Any]
    physical_times: np.ndarray[Any, Any] | None

    def __len__(self) -> int:
        return len(self.shapes)

    def record(self, index: int) -> np.ndarray[Any, Any]:
        _check_record(index, len(self))
        start, stop = int(self.offsets[index]), int(self.offsets[index + 1])
        return self.data[start:stop].reshape(tuple(int(n) for n in self.shapes[index]), order="C")


NumericSeries = FixedSeries | RaggedSeries


class NpyConversion:
    """Verified view of one converted member directory and its manifest."""

    def __init__(self, directory: Path, manifest: dict[str, Any]) -> None:
        self.directory = directory
        self.manifest = manifest
        self._arrays: dict[str, np.ndarray[Any, Any]] = {}
        self._series: dict[tuple[str, str, str | None], NumericSeries] = {}
        self._declared_paths = {
            descriptor["path"]
            for descriptor in manifest["arrays"]
            if isinstance(descriptor, dict) and isinstance(descriptor.get("path"), str)
        }

    def array(self, relative_path: str) -> np.ndarray[Any, Any]:
        """Memory-map one component declared by this conversion manifest."""
        if relative_path not in self._declared_paths:
            raise NpyConversionError(f"array is not declared by manifest: {relative_path!r}")
        if relative_path not in self._arrays:
            self._arrays[relative_path] = np.load(
                _component_path(self.directory, relative_path), mmap_mode="r", allow_pickle=False,
            )
        return self._arrays[relative_path]

    @property
    def stream_names(self) -> tuple[str, ...]:
        """Declared streams in manifest order."""
        return tuple(stream["name"] for stream in self.manifest["streams"])

    @property
    def execution_unit(self) -> str | None:
        """Workflow execution-unit provenance, when present."""
        metadata = self.manifest.get("user_metadata", {})
        workflow = metadata.get("workflow", {}) if isinstance(metadata, Mapping) else {}
        value = workflow.get("execution_unit") if isinstance(workflow, Mapping) else None
        return value if isinstance(value, str) else None

    def coordinates(self, stream: str) -> tuple[np.ndarray[Any, Any], np.ndarray[Any, Any] | None]:
        """Return (iterations, physical_times); absent physical time is None."""
        if stream not in self.stream_names:
            raise NpyConversionError(f"unknown stream {stream!r}")
        result = []
        for role in ("iterations", "physical_times"):
            matches = [a for a in self.manifest["arrays"] if a.get("stream") == stream and a.get("role") == role]
            if role == "physical_times" and not matches:
                result.append(None)
                continue
            if len(matches) != 1:
                raise NpyConversionError(f"stream {stream!r} requires exactly one {role} array")
            result.append(self.array(matches[0]["path"]))
        return result[0], result[1]

    def series(self, stream: str, field: str, logical_path: str | None = None) -> NumericSeries:
        """Return a cached complete numeric series without reconstructing JSON.

        Omit logical_path for a wholly numeric field. Structured fields require
        an exact projection path. Array components remain read-only memory maps.
        """
        key = (stream, field, logical_path)
        if key in self._series:
            return self._series[key]
        metadata = self.field(stream, field)
        if logical_path is None:
            if metadata.get("representation") != "numeric":
                raise NpyConversionError(f"structured field {field!r} requires logical_path")
            dataset = metadata["dataset"]
        else:
            matches = [d for d in metadata.get("projections", []) if d.get("logical_path") == logical_path]
            if len(matches) != 1:
                raise NpyConversionError(f"expected exactly one projection {logical_path!r} in {stream!r}/{field!r}")
            dataset = matches[0]
        iterations, physical_times = self.coordinates(stream)
        data = self.array(dataset["data"])
        if dataset["storage"] == "fixed":
            result = FixedSeries(data, iterations, physical_times)
        else:
            result = RaggedSeries(data, self.array(dataset["offsets"]), self.array(dataset["shapes"]), iterations, physical_times)
        self._series[key] = result
        return result

    def field(self, stream: str, field: str) -> Mapping[str, object]:
        """Return one field's required representation metadata."""
        for stream_entry in self.manifest["streams"]:
            if stream_entry.get("name") != stream:
                continue
            for field_entry in stream_entry.get("fields", []):
                if field_entry.get("name") == field:
                    return field_entry
            raise NpyConversionError(f"unknown converted field {field!r} in {stream!r}")
        raise NpyConversionError(f"unknown converted stream {stream!r}")

    def reconstruct(self, stream: str, field: str, record: int) -> object:
        """Reconstruct one field record from its manifest-directed NPY data."""
        metadata = self.field(stream, field)
        if metadata.get("representation") == "numeric":
            dataset = metadata.get("dataset")
            if not isinstance(dataset, Mapping):
                raise NpyConversionError("numeric field dataset metadata is malformed")
            return self._numeric_record(dataset, record)
        fallback = metadata.get("fallback")
        if not isinstance(fallback, Mapping):
            raise NpyConversionError("structured field fallback metadata is malformed")
        data_path = fallback.get("data")
        offsets_path = fallback.get("offsets")
        if not isinstance(data_path, str) or not isinstance(offsets_path, str):
            raise NpyConversionError("structured field fallback paths are malformed")
        data = self.array(data_path)
        offsets = self.array(offsets_path)
        _check_record(record, len(offsets) - 1)
        start, stop = int(offsets[record]), int(offsets[record + 1])
        try:
            return json.loads(bytes(data[start:stop]).decode("utf-8"))
        except (UnicodeError, json.JSONDecodeError) as error:
            raise NpyConversionError("structured field fallback is not valid JSON") from error

    def projection(
        self, stream: str, field: str, logical_path: str, record: int
    ) -> object:
        """Return one record from a structured field's numeric projection."""
        metadata = self.field(stream, field)
        projections = metadata.get("projections")
        if not isinstance(projections, list):
            raise NpyConversionError(f"field {field!r} has no numeric projections")
        for dataset in projections:
            if isinstance(dataset, Mapping) and dataset.get("logical_path") == logical_path:
                return self._numeric_record(dataset, record)
        raise NpyConversionError(
            f"unknown numeric projection {logical_path!r} for field {field!r}"
        )

    def _numeric_record(self, dataset: Mapping[str, object], record: int) -> object:
        data_path = dataset.get("data")
        if not isinstance(data_path, str):
            raise NpyConversionError("numeric dataset data path is malformed")
        data = self.array(data_path)
        storage = dataset.get("storage")
        if storage == "fixed":
            _check_record(record, len(data))
            return data[record]
        if storage != "ragged":
            raise NpyConversionError(f"unknown numeric storage mode: {storage!r}")
        offsets_path = dataset.get("offsets")
        shapes_path = dataset.get("shapes")
        if not isinstance(offsets_path, str) or not isinstance(shapes_path, str):
            raise NpyConversionError("ragged dataset component paths are malformed")
        offsets = self.array(offsets_path)
        shapes = self.array(shapes_path)
        _check_record(record, len(shapes))
        start, stop = int(offsets[record]), int(offsets[record + 1])
        shape = tuple(int(extent) for extent in shapes[record])
        return data[start:stop].reshape(shape, order="C")


@dataclass(frozen=True, slots=True)
class NpyBatch:
    """Verified view of a replicate-level NPY batch directory."""

    directory: Path
    manifest: Mapping[str, object]
    members: tuple[NpyConversion, ...]


def _check_record(record: int, count: int) -> None:
    if isinstance(record, bool) or not isinstance(record, int) or not 0 <= record < count:
        raise IndexError(f"record index {record!r} is outside 0..{count}")


def open_npy_conversion(directory: str | Path) -> NpyConversion:
    """Open and fully verify one converted member directory."""
    root = Path(directory).expanduser().resolve(strict=True)
    if not root.is_dir():
        raise NpyConversionError(f"converted dataset is not a directory: {root}")
    manifest = _read_json(root / MANIFEST_FILE)
    if manifest.get("format") != NPY_FORMAT:
        raise NpyConversionError(
            f"unsupported NPY conversion format: {manifest.get('format')!r}"
        )
    if not isinstance(manifest.get("streams"), list):
        raise NpyConversionError("conversion manifest streams must be an array")
    _validate_arrays(root, manifest)
    _validate_layout(root, manifest)
    return NpyConversion(root, manifest)


def open_npy_batch(directory: str | Path) -> NpyBatch:
    """Open a batch manifest and fully verify every referenced member dataset."""
    root = Path(directory).expanduser().resolve(strict=True)
    if not root.is_dir():
        raise NpyConversionError(f"converted batch is not a directory: {root}")
    manifest = _read_json(root / MANIFEST_FILE)
    if manifest.get("format") != NPY_BATCH_FORMAT:
        raise NpyConversionError(
            f"unsupported NPY batch format: {manifest.get('format')!r}"
        )
    excluded = _manifest_exclusions(manifest)
    entries = manifest.get("members")
    if not isinstance(entries, list) or not entries:
        raise NpyConversionError("NPY batch members must be a nonempty array")
    members: list[NpyConversion] = []
    seen: set[str] = set()
    for ordinal, entry in enumerate(entries):
        if not isinstance(entry, Mapping) or entry.get("ordinal") != ordinal:
            raise NpyConversionError("NPY batch member ordinals must be contiguous")
        relative = entry.get("manifest")
        checksum = entry.get("manifest_checksum")
        if not isinstance(relative, str) or relative in seen:
            raise NpyConversionError("NPY batch member manifests must be unique paths")
        seen.add(relative)
        path = _component_path(root, relative)
        if path.name != MANIFEST_FILE or _sha256(path) != checksum:
            raise NpyConversionError("NPY batch member manifest checksum mismatch")
        member = open_npy_conversion(path.parent)
        if member.manifest.get("source_recording") != entry.get("source_recording"):
            raise NpyConversionError("NPY batch member source recording mismatch")
        if _manifest_exclusions(member.manifest) != excluded:
            raise NpyConversionError("NPY batch member stream exclusions mismatch")
        members.append(member)
    return NpyBatch(root, manifest, tuple(members))
