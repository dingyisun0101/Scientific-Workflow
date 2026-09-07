from __future__ import annotations

from pathlib import Path
from typing import Any

import numpy as np

from .format import NpyConversionError, JsonPath, _pointer, _safe_name, _sha256
from .planning import (
    _FieldPlan,
    _FieldScan,
    _NumericPlan,
    _canonical_json,
    _discover_numeric,
    _whole_numeric,
)
from .progress import _stage

def _descriptor(
    root: Path,
    path: Path,
    *,
    stream: str,
    field: str | None,
    role: str,
    array: np.ndarray[Any, Any] | None = None,
    logical_path: JsonPath | None = None,
) -> dict[str, object]:
    if array is None:
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
                array=self.data,
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
                    array=self.offsets,
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
                    array=self.shapes,
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
                array=self.data,
            ),
            _descriptor(
                root,
                self.offsets_path,
                stream=stream,
                field=field,
                role="json_offsets",
                array=self.offsets,
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


def _stream_plan(reader: Any, stream: str) -> tuple[list[_FieldPlan], bool, int]:
    count = reader.stream_record_count(stream)
    _stage(f"planning {stream}", total=count, force=True)
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
    return [scan.finish() for scan in scans.values()], physical_time, count


def _empty_array(plan: _NumericPlan, index: int) -> np.ndarray[Any, Any]:
    return np.empty(plan.shapes[index], dtype=np.dtype(plan.dtype), order="C")


def _convert_stream(
    reader: Any,
    stream: str,
    stream_index: int,
    temporary: Path,
) -> tuple[list[dict[str, object]], dict[str, object]]:
    plans, has_physical_time, count = _stream_plan(reader, stream)
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
        _stage(f"writing {stream}", observed, count)
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
            array=iterations,
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
                array=physical_times,
            )
        )

    fields: list[dict[str, object]] = []
    for plan in plans:
        if plan.direct is not None:
            dataset, field_descriptors = numeric_writers[plan.name].finish(
                temporary,
                stream=stream,
                field=plan.name,
                role="field_data",
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
