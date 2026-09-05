"""Verified conversion of completed Workflow recordings to NumPy datasets."""

from __future__ import annotations

import argparse
from dataclasses import dataclass
import hashlib
import json
import math
import os
from pathlib import Path
import re
import shutil
from collections.abc import Mapping
from typing import Any

import numpy as np

from .reader import FORMAT_NAME, open_completed_recording

NPY_FORMAT = "scientific-workflow-npy.v2"
NPY_BATCH_FORMAT = "scientific-workflow-npy-batch.v2"
MANIFEST_FILE = "manifest.json"

_SCALAR_DTYPES = {
    "bool": np.dtype("?"),
    "f32": np.dtype("<f4"),
    "f64": np.dtype("<f8"),
    "i8": np.dtype("i1"),
    "i16": np.dtype("<i2"),
    "i32": np.dtype("<i4"),
    "i64": np.dtype("<i8"),
    "isize": np.dtype("<i8"),
    "u8": np.dtype("u1"),
    "u16": np.dtype("<u2"),
    "u32": np.dtype("<u4"),
    "u64": np.dtype("<u8"),
    "usize": np.dtype("<u8"),
}

JsonPath = tuple[str, ...]


class NpyConversionError(ValueError):
    """A recording or converted dataset violates the NPY contract."""


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return "sha256:" + digest.hexdigest()


def _safe_name(value: str) -> str:
    name = re.sub(r"[^A-Za-z0-9_.-]+", "_", value).strip("._")
    return name or "unnamed"


def _pointer(path: JsonPath) -> str:
    if not path:
        return ""
    return "/" + "/".join(part.replace("~", "~0").replace("/", "~1") for part in path)


def _numeric(value: object) -> np.ndarray[Any, Any] | None:
    try:
        array = np.asarray(value)
    except (TypeError, ValueError, OverflowError):
        return None
    if array.dtype.kind not in {"b", "i", "u", "f"}:
        return None
    return np.ascontiguousarray(array)


def _numeric_envelope(
    value: object,
) -> tuple[bool, np.ndarray[Any, Any] | None]:
    if not isinstance(value, Mapping) or not {"scalar", "shape", "data"}.issubset(value):
        return False, None
    scalar = value["scalar"]
    shape = value["shape"]
    data = value["data"]
    if not isinstance(scalar, str):
        raise NpyConversionError("numeric envelope scalar must be a string")
    if not isinstance(shape, list) or any(
        isinstance(extent, bool) or not isinstance(extent, int) or extent < 0
        for extent in shape
    ):
        raise NpyConversionError("numeric envelope shape must contain unsigned extents")
    if not isinstance(data, list):
        raise NpyConversionError("numeric envelope data must be an array")
    expected = math.prod(shape)
    if len(data) != expected:
        raise NpyConversionError(
            f"numeric envelope has {len(data)} values but shape requires {expected}"
        )
    dtype = _SCALAR_DTYPES.get(scalar)
    if dtype is None:
        return True, None
    try:
        array = np.asarray(data, dtype=dtype).reshape(tuple(shape), order="C")
    except (TypeError, ValueError, OverflowError) as error:
        raise NpyConversionError(
            f"numeric envelope data cannot be represented as {scalar}"
        ) from error
    return True, np.ascontiguousarray(array)


def _whole_numeric(value: object) -> np.ndarray[Any, Any] | None:
    envelope, array = _numeric_envelope(value)
    return array if envelope else _numeric(value)


def _discover_numeric(
    value: object, path: JsonPath = ()
) -> tuple[dict[JsonPath, np.ndarray[Any, Any]], set[JsonPath]]:
    envelope, array = _numeric_envelope(value)
    if envelope:
        return ({path: array} if array is not None else {}), set()

    if isinstance(value, list) and not value:
        return {}, {(*path, "*")}

    array = _numeric(value)
    if array is not None:
        return {path: array}, set()

    if isinstance(value, Mapping):
        leaves: dict[JsonPath, np.ndarray[Any, Any]] = {}
        empty: set[JsonPath] = set()
        for key in sorted(value):
            if not isinstance(key, str):
                continue
            child_leaves, child_empty = _discover_numeric(value[key], (*path, key))
            leaves.update(child_leaves)
            empty.update(child_empty)
        return leaves, empty

    if isinstance(value, list):
        wildcard = (*path, "*")
        items = [_discover_numeric(item) for item in value]
        key_sets = [set(leaves) for leaves, _ in items]
        if key_sets and all(keys == key_sets[0] for keys in key_sets[1:]):
            compatible = True
            for key in key_sets[0]:
                arrays = [leaves[key] for leaves, _ in items]
                first = arrays[0]
                compatible &= all(
                    array.dtype.str == first.dtype.str and array.shape == first.shape
                    for array in arrays[1:]
                )
            if compatible:
                leaves = {
                    (*wildcard, *key): np.ascontiguousarray(
                        np.stack([item_leaves[key] for item_leaves, _ in items])
                    )
                    for key in sorted(key_sets[0])
                }
                empty = {
                    (*wildcard, *child)
                    for _, item_empty in items
                    for child in item_empty
                }
                return leaves, empty

        leaves = {}
        empty = set()
        for index, (item_leaves, item_empty) in enumerate(items):
            prefix = (*path, str(index))
            leaves.update({(*prefix, *key): array for key, array in item_leaves.items()})
            empty.update({(*prefix, *key) for key in item_empty})
        return leaves, empty

    return {}, set()


def _canonical_json(value: object) -> bytes:
    try:
        return json.dumps(
            value,
            allow_nan=False,
            ensure_ascii=False,
            separators=(",", ":"),
            sort_keys=True,
        ).encode("utf-8")
    except (TypeError, ValueError, UnicodeError) as error:
        raise NpyConversionError("field cannot be represented as canonical JSON") from error


@dataclass(frozen=True, slots=True)
class _NumericMeta:
    dtype: str
    shape: tuple[int, ...]

    @classmethod
    def from_array(cls, array: np.ndarray[Any, Any]) -> _NumericMeta:
        return cls(array.dtype.str, tuple(array.shape))


@dataclass(frozen=True, slots=True)
class _NumericPlan:
    logical_path: JsonPath
    dtype: str
    shapes: tuple[tuple[int, ...], ...]
    storage: str

    @property
    def rank(self) -> int:
        return len(self.shapes[0])

    @property
    def elements(self) -> int:
        return sum(math.prod(shape) for shape in self.shapes)


class _FieldScan:
    def __init__(self, name: str) -> None:
        self.name = name
        self.direct: list[_NumericMeta | None] = []
        self.projections: dict[JsonPath, list[_NumericMeta | None]] = {}
        self.empty_sequences: list[set[JsonPath]] = []
        self.json_lengths: list[int] = []

    def observe(self, value: object) -> None:
        whole = _whole_numeric(value)
        self.direct.append(_NumericMeta.from_array(whole) if whole is not None else None)
        leaves, empty = _discover_numeric(value)
        record = len(self.json_lengths)
        for entries in self.projections.values():
            entries.append(None)
        for path, array in leaves.items():
            if path not in self.projections:
                self.projections[path] = [None] * (record + 1)
            self.projections[path][-1] = _NumericMeta.from_array(array)
        self.empty_sequences.append(empty)
        self.json_lengths.append(len(_canonical_json(value)))

    def finish(self) -> _FieldPlan:
        direct = _plan_numeric((), self.direct, self.empty_sequences)
        if direct is not None:
            return _FieldPlan(self.name, direct, (), tuple(self.json_lengths))
        projections = tuple(
            plan
            for path in sorted(self.projections)
            if (
                plan := _plan_numeric(
                    path, self.projections[path], self.empty_sequences
                )
            )
            is not None
        )
        return _FieldPlan(self.name, None, projections, tuple(self.json_lengths))


@dataclass(frozen=True, slots=True)
class _FieldPlan:
    name: str
    direct: _NumericPlan | None
    projections: tuple[_NumericPlan, ...]
    json_lengths: tuple[int, ...]


def _plan_numeric(
    path: JsonPath,
    entries: list[_NumericMeta | None],
    empty_sequences: list[set[JsonPath]],
) -> _NumericPlan | None:
    prototype = next((entry for entry in entries if entry is not None), None)
    if prototype is None:
        return None
    completed: list[_NumericMeta] = []
    for entry, empty in zip(entries, empty_sequences, strict=True):
        if entry is not None:
            completed.append(entry)
            continue
        prefix = next(
            (
                candidate
                for candidate in sorted(empty, key=len)
                if len(candidate) <= len(path) and path[: len(candidate)] == candidate
            ),
            None,
        )
        if prefix is None:
            return None
        axis = sum(part == "*" for part in prefix) - 1
        if axis < 0 or axis >= len(prototype.shape):
            return None
        shape = list(prototype.shape)
        shape[axis] = 0
        completed.append(_NumericMeta(prototype.dtype, tuple(shape)))
    if any(
        entry.dtype != prototype.dtype or len(entry.shape) != len(prototype.shape)
        for entry in completed
    ):
        return None
    shapes = tuple(entry.shape for entry in completed)
    storage = "fixed" if all(shape == shapes[0] for shape in shapes[1:]) else "ragged"
    return _NumericPlan(path, prototype.dtype, shapes, storage)


def _descriptor(
    root: Path,
    path: Path,
    *,
    stream: str,
    field: str | None,
    role: str,
    logical_path: JsonPath | None = None,
) -> dict[str, object]:
    array = np.load(path, mmap_mode="r", allow_pickle=False)
    if not array.flags.c_contiguous:
        raise NpyConversionError(f"array is not C-contiguous: {path}")
    return {
        "role": role,
        "stream": stream,
        **({"field": field} if field is not None else {}),
        **({"logical_path": _pointer(logical_path)} if logical_path is not None else {}),
        "path": str(path.relative_to(root)),
        "dtype": array.dtype.str,
        "shape": list(array.shape),
        "c_contiguous": True,
        "checksum": _sha256(path),
    }


def _flush(array: np.ndarray[Any, Any]) -> None:
    if isinstance(array, np.memmap):
        array.flush()


class _NumericWriter:
    def __init__(
        self,
        directory: Path,
        stem: str,
        plan: _NumericPlan,
        count: int,
    ) -> None:
        self.plan = plan
        self.data_path = directory / (
            f"{stem}.npy" if plan.storage == "fixed" else f"{stem}_data.npy"
        )
        shape = (count, *plan.shapes[0]) if plan.storage == "fixed" else (plan.elements,)
        self.data = np.lib.format.open_memmap(
            self.data_path, mode="w+", dtype=np.dtype(plan.dtype), shape=shape
        )
        self.offsets_path: Path | None = None
        self.offsets: np.memmap[Any, Any] | None = None
        self.shapes_path: Path | None = None
        self.shapes: np.memmap[Any, Any] | None = None
        if plan.storage == "ragged":
            self.offsets_path = directory / f"{stem}_offsets.npy"
            self.offsets = np.lib.format.open_memmap(
                self.offsets_path, mode="w+", dtype=np.uint64, shape=(count + 1,)
            )
            self.offsets[0] = 0
            self.shapes_path = directory / f"{stem}_shapes.npy"
            self.shapes = np.lib.format.open_memmap(
                self.shapes_path,
                mode="w+",
                dtype=np.uint64,
                shape=(count, plan.rank),
            )

    def write(self, index: int, array: np.ndarray[Any, Any]) -> None:
        expected = self.plan.shapes[index]
        if array.dtype.str != self.plan.dtype or tuple(array.shape) != expected:
            raise NpyConversionError(
                f"numeric value changed after planning at record {index}"
            )
        if self.plan.storage == "fixed":
            self.data[index] = array
            return
        if self.offsets is None or self.shapes is None:
            raise AssertionError("ragged writer has no layout arrays")
        start = int(self.offsets[index])
        stop = start + array.size
        self.data[start:stop] = array.reshape(-1, order="C")
        self.offsets[index + 1] = stop
        self.shapes[index] = expected

    def finish(
        self,
        root: Path,
        *,
        stream: str,
        field: str,
        role: str,
    ) -> tuple[dict[str, object], list[dict[str, object]]]:
        _flush(self.data)
        descriptors = [
            _descriptor(
                root,
                self.data_path,
                stream=stream,
                field=field,
                role=role,
                logical_path=self.plan.logical_path,
            )
        ]
        dataset: dict[str, object] = {
            "logical_path": _pointer(self.plan.logical_path),
            "storage": self.plan.storage,
            "dtype": self.plan.dtype,
            "rank": self.plan.rank,
            "data": str(self.data_path.relative_to(root)),
        }
        if self.offsets is not None and self.offsets_path is not None:
            _flush(self.offsets)
            descriptors.append(
                _descriptor(
                    root,
                    self.offsets_path,
                    stream=stream,
                    field=field,
                    role=f"{role}_offsets",
                    logical_path=self.plan.logical_path,
                )
            )
            dataset["offsets"] = str(self.offsets_path.relative_to(root))
        if self.shapes is not None and self.shapes_path is not None:
            _flush(self.shapes)
            descriptors.append(
                _descriptor(
                    root,
                    self.shapes_path,
                    stream=stream,
                    field=field,
                    role=f"{role}_shapes",
                    logical_path=self.plan.logical_path,
                )
            )
            dataset["shapes"] = str(self.shapes_path.relative_to(root))
        return dataset, descriptors


class _JsonWriter:
    def __init__(self, directory: Path, stem: str, lengths: tuple[int, ...]) -> None:
        self.data_path = directory / f"{stem}_json_data.npy"
        self.data = np.lib.format.open_memmap(
            self.data_path, mode="w+", dtype=np.uint8, shape=(sum(lengths),)
        )
        self.offsets_path = directory / f"{stem}_json_offsets.npy"
        self.offsets = np.lib.format.open_memmap(
            self.offsets_path, mode="w+", dtype=np.uint64, shape=(len(lengths) + 1,)
        )
        self.offsets[0] = 0

    def write(self, index: int, value: object) -> None:
        encoded = _canonical_json(value)
        start = int(self.offsets[index])
        stop = start + len(encoded)
        self.data[start:stop] = np.frombuffer(encoded, dtype=np.uint8)
        self.offsets[index + 1] = stop

    def finish(
        self, root: Path, *, stream: str, field: str
    ) -> tuple[dict[str, object], list[dict[str, object]]]:
        _flush(self.data)
        _flush(self.offsets)
        descriptors = [
            _descriptor(
                root,
                self.data_path,
                stream=stream,
                field=field,
                role="json_data",
            ),
            _descriptor(
                root,
                self.offsets_path,
                stream=stream,
                field=field,
                role="json_offsets",
            ),
        ]
        return (
            {
                "storage": "json_bytes",
                "encoding": "utf-8-json",
                "data": str(self.data_path.relative_to(root)),
                "offsets": str(self.offsets_path.relative_to(root)),
            },
            descriptors,
        )


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
    output: Path, recording: Path, metadata_checksum: str
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
        ):
            return None
        _validate_arrays(output, document)
        _validate_layout(output, document)
    except NpyConversionError:
        return None
    return document


def _stream_plan(reader: Any, stream: str) -> tuple[list[_FieldPlan], bool]:
    records = iter(reader.iter_verified_records(stream))
    try:
        first = next(records)
    except StopIteration as error:
        raise NpyConversionError(f"stream {stream!r} is empty") from error
    scans = {field: _FieldScan(field) for field in first.values}
    physical_time = first.physical_time is not None
    for field, value in first.values.items():
        scans[field].observe(value)
    for record in records:
        if (record.physical_time is not None) != physical_time:
            raise NpyConversionError(
                f"physical-time presence changes within stream {stream!r}"
            )
        for field, value in record.values.items():
            scans[field].observe(value)
    return [scan.finish() for scan in scans.values()], physical_time


def _empty_array(plan: _NumericPlan, index: int) -> np.ndarray[Any, Any]:
    return np.empty(plan.shapes[index], dtype=np.dtype(plan.dtype), order="C")


def _convert_stream(
    reader: Any,
    stream: str,
    stream_index: int,
    temporary: Path,
) -> tuple[list[dict[str, object]], dict[str, object]]:
    plans, has_physical_time = _stream_plan(reader, stream)
    count = reader.stream_record_count(stream)
    prefix = f"{stream_index:04d}-{_safe_name(stream)}"

    numeric_writers: dict[str, _NumericWriter] = {}
    projection_writers: dict[tuple[str, JsonPath], _NumericWriter] = {}
    json_writers: dict[str, _JsonWriter] = {}
    for field_index, plan in enumerate(plans):
        stem = f"{prefix}_{field_index:04d}-{_safe_name(plan.name)}"
        if plan.direct is not None:
            numeric_writers[plan.name] = _NumericWriter(
                temporary, stem, plan.direct, count
            )
            continue
        json_writers[plan.name] = _JsonWriter(temporary, stem, plan.json_lengths)
        for projection_index, projection in enumerate(plan.projections):
            projection_stem = f"{stem}_projection-{projection_index:04d}"
            projection_writers[(plan.name, projection.logical_path)] = _NumericWriter(
                temporary, projection_stem, projection, count
            )

    iteration_path = temporary / f"{prefix}_iterations.npy"
    iterations = np.lib.format.open_memmap(
        iteration_path, mode="w+", dtype=np.uint64, shape=(count,)
    )
    physical_path = temporary / f"{prefix}_physical_times.npy"
    physical_times = (
        np.lib.format.open_memmap(
            physical_path, mode="w+", dtype=np.float64, shape=(count,)
        )
        if has_physical_time
        else None
    )

    observed = 0
    plans_by_name = {plan.name: plan for plan in plans}
    for observed, record in enumerate(reader.iter_verified_records(stream), start=1):
        index = observed - 1
        iterations[index] = record.iteration
        if physical_times is not None:
            if record.physical_time is None:
                raise NpyConversionError(f"stream {stream!r} lost physical time")
            physical_times[index] = record.physical_time
        for field, value in record.values.items():
            plan = plans_by_name[field]
            if plan.direct is not None:
                array = _whole_numeric(value)
                if array is None:
                    raise NpyConversionError(
                        f"field {field!r} stopped being numeric after planning"
                    )
                numeric_writers[field].write(index, array)
                continue
            json_writers[field].write(index, value)
            leaves, _ = _discover_numeric(value)
            for projection in plan.projections:
                array = leaves.get(projection.logical_path)
                projection_writers[(field, projection.logical_path)].write(
                    index, array if array is not None else _empty_array(projection, index)
                )
    if observed != count:
        raise NpyConversionError(
            f"stream {stream!r} yielded {observed} records; expected {count}"
        )

    _flush(iterations)
    descriptors = [
        _descriptor(
            temporary,
            iteration_path,
            stream=stream,
            field=None,
            role="iterations",
        )
    ]
    if physical_times is not None:
        _flush(physical_times)
        descriptors.append(
            _descriptor(
                temporary,
                physical_path,
                stream=stream,
                field=None,
                role="physical_times",
            )
        )

    fields: list[dict[str, object]] = []
    for plan in plans:
        if plan.direct is not None:
            dataset, field_descriptors = numeric_writers[plan.name].finish(
                temporary, stream=stream, field=plan.name, role="field_data"
            )
            descriptors.extend(field_descriptors)
            fields.append(
                {
                    "name": plan.name,
                    "representation": "numeric",
                    "dataset": dataset,
                }
            )
            continue
        fallback, fallback_descriptors = json_writers[plan.name].finish(
            temporary, stream=stream, field=plan.name
        )
        descriptors.extend(fallback_descriptors)
        projections = []
        for projection in plan.projections:
            dataset, projection_descriptors = projection_writers[
                (plan.name, projection.logical_path)
            ].finish(
                temporary,
                stream=stream,
                field=plan.name,
                role="projection_data",
            )
            descriptors.extend(projection_descriptors)
            projections.append(dataset)
        fields.append(
            {
                "name": plan.name,
                "representation": "structured",
                "fallback": fallback,
                "projections": projections,
            }
        )

    return descriptors, {
        "name": stream,
        "records": count,
        "fields": fields,
    }


@dataclass(frozen=True, slots=True)
class FixedSeries:
    """Read-only memory-mapped fixed-shape values and stream coordinates."""
    values: np.ndarray[Any, Any]
    iterations: np.ndarray[Any, Any]
    physical_times: np.ndarray[Any, Any]

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
    physical_times: np.ndarray[Any, Any]

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

    def coordinates(self, stream: str) -> tuple[np.ndarray[Any, Any], np.ndarray[Any, Any]]:
        """Return (iterations, physical_times); absent physical time is NaN."""
        if stream not in self.stream_names:
            raise NpyConversionError(f"unknown stream {stream!r}")
        result = []
        for role in ("iterations", "physical_times"):
            matches = [a for a in self.manifest["arrays"] if a.get("stream") == stream and a.get("role") == role]
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
        members.append(member)
    return NpyBatch(root, manifest, tuple(members))


def convert_recording(
    recording_directory: str | Path,
    output_directory: str | Path | None = None,
) -> dict[str, object]:
    """Verify and convert one completed recording into C-contiguous NPY data."""
    recording = Path(recording_directory).expanduser().resolve(strict=True)
    if not recording.is_dir():
        raise NpyConversionError(f"recording is not a directory: {recording}")
    output = (
        Path(output_directory).expanduser().resolve()
        if output_directory is not None
        else recording.with_name(recording.name + "-npy")
    )
    if output == recording or output.is_relative_to(recording):
        raise NpyConversionError("conversion output must be outside the raw recording")
    metadata_checksum = _sha256(recording / "metadata.json")
    if output.exists():
        existing = _existing(output, recording, metadata_checksum)
        if existing is not None:
            return existing
        raise NpyConversionError(f"conflicting conversion output exists: {output}")
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = output.with_name(f".{output.name}.tmp-{os.getpid()}")
    if temporary.exists():
        shutil.rmtree(temporary)
    temporary.mkdir()
    try:
        reader = open_completed_recording(recording)
        arrays: list[dict[str, object]] = []
        streams: list[dict[str, object]] = []
        for index, stream in enumerate(reader.stream_names):
            converted, summary = _convert_stream(reader, stream, index, temporary)
            arrays.extend(converted)
            streams.append(summary)
        manifest: dict[str, object] = {
            "format": NPY_FORMAT,
            "source_format": FORMAT_NAME,
            "source_version": reader.format_version,
            "source_recording": str(recording),
            "source_metadata_checksum": metadata_checksum,
            "user_metadata": dict(reader.user_metadata),
            "terminal_metadata": dict(reader.terminal_metadata),
            "streams": streams,
            "arrays": arrays,
        }
        _write_json(temporary / MANIFEST_FILE, manifest)
        open_npy_conversion(temporary)
        os.replace(temporary, output)
        return manifest
    except Exception:
        shutil.rmtree(temporary, ignore_errors=True)
        raise


def _dependency_recordings(dependencies_path: Path) -> list[Path]:
    try:
        dependencies = json.loads(dependencies_path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise NpyConversionError(
            f"cannot read Workflow dependencies {dependencies_path}"
        ) from error
    if not isinstance(dependencies, list):
        raise NpyConversionError("Workflow dependencies must be an array")
    recordings: list[Path] = []
    seen: set[Path] = set()
    for phase in dependencies:
        if not isinstance(phase, dict) or not isinstance(phase.get("tasks"), list):
            raise NpyConversionError("Workflow dependency phase is malformed")
        for task in phase["tasks"]:
            if not isinstance(task, dict):
                raise NpyConversionError("Workflow dependency task is malformed")
            workload = task.get("workload")
            if not isinstance(workload, dict) or workload.get("kind") != "execution_unit":
                continue
            members = workload.get("members")
            if not isinstance(members, list):
                raise NpyConversionError("execution-unit dependency members must be an array")
            for member in members:
                if not isinstance(member, dict) or not isinstance(
                    member.get("output_directory"), str
                ):
                    raise NpyConversionError("execution-unit dependency member is malformed")
                recording = Path(member["output_directory"]).resolve(strict=True)
                if recording not in seen:
                    seen.add(recording)
                    recordings.append(recording)
    if not recordings:
        raise NpyConversionError("$npy received no completed execution-unit recordings")
    return recordings


def convert_workflow_dependencies(
    dependencies_path: str | Path,
    output_directory: str | Path,
) -> dict[str, object]:
    """Convert every recording visible to Workflow's reserved ``$npy`` phase."""
    dependencies = Path(dependencies_path).expanduser().resolve(strict=True)
    output = Path(output_directory).expanduser().resolve()
    output.mkdir(parents=True, exist_ok=True)
    recordings = _dependency_recordings(dependencies)
    members: list[dict[str, object]] = []
    for ordinal, recording in enumerate(recordings):
        member_output = output / f"member-{ordinal:06d}"
        manifest = convert_recording(recording, member_output)
        members.append(
            {
                "ordinal": ordinal,
                "source_recording": manifest["source_recording"],
                "manifest": str((member_output / MANIFEST_FILE).relative_to(output)),
                "manifest_checksum": _sha256(member_output / MANIFEST_FILE),
            }
        )
        print(f"converted {ordinal + 1}/{len(recordings)}: {recording}", flush=True)
    batch: dict[str, object] = {
        "format": NPY_BATCH_FORMAT,
        "members": members,
    }
    manifest_path = output / MANIFEST_FILE
    temporary_manifest = output / f".{MANIFEST_FILE}.tmp-{os.getpid()}"
    if temporary_manifest.exists():
        temporary_manifest.unlink()
    try:
        _write_json(temporary_manifest, batch)
        os.replace(temporary_manifest, manifest_path)
    finally:
        temporary_manifest.unlink(missing_ok=True)
    return batch


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Convert completed Scientific Workflow recordings to NPY datasets."
    )
    parser.add_argument("recording", type=Path, nargs="?")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--workflow-dependencies", action="store_true", help=argparse.SUPPRESS)
    arguments = parser.parse_args()
    if arguments.workflow_dependencies:
        if arguments.recording is not None or arguments.output is not None:
            parser.error("--workflow-dependencies does not accept recording or --output")
        try:
            dependencies_path = os.environ["WORKFLOW_DEPENDENCIES_PATH"]
            output_directory = os.environ["WORKFLOW_NPY_OUTPUT"]
        except KeyError as error:
            parser.error(f"missing required Workflow environment variable {error.args[0]}")
        convert_workflow_dependencies(dependencies_path, output_directory)
        return
    if arguments.recording is None:
        parser.error("recording is required")
    manifest = convert_recording(arguments.recording, arguments.output)
    print(
        f"converted {manifest['source_recording']} into "
        f"{arguments.output or Path(arguments.recording).with_name(Path(arguments.recording).name + '-npy')}",
        flush=True,
    )


if __name__ == "__main__":
    main()
