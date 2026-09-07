from __future__ import annotations

from dataclasses import dataclass
import json
import math
from collections.abc import Mapping
from typing import Any

import numpy as np

from .format import JsonPath, NpyConversionError

_SCALAR_DTYPES = {
    "bool": np.dtype("?"), "f32": np.dtype("<f4"), "f64": np.dtype("<f8"),
    "i8": np.dtype("i1"), "i16": np.dtype("<i2"), "i32": np.dtype("<i4"),
    "i64": np.dtype("<i8"), "isize": np.dtype("<i8"), "u8": np.dtype("u1"),
    "u16": np.dtype("<u2"), "u32": np.dtype("<u4"), "u64": np.dtype("<u8"),
    "usize": np.dtype("<u8"),
}

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
