"""Verified conversion of one completed Workflow recording to NumPy arrays."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
from collections.abc import Mapping
from typing import Any

import numpy as np

from .reader import FORMAT_NAME, FORMAT_VERSION, open_completed_recording

NPY_FORMAT = "scientific-workflow-npy.v1"
NPY_BATCH_FORMAT = "scientific-workflow-npy-batch.v1"
MANIFEST_FILE = "manifest.json"


class NpyConversionError(ValueError):
    """A recording cannot be published as a verified NumPy artifact."""


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for block in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(block)
    return "sha256:" + digest.hexdigest()


def _safe_name(value: str) -> str:
    name = re.sub(r"[^A-Za-z0-9_.-]+", "_", value).strip("._")
    return name or "unnamed"


def _numeric(value: object) -> np.ndarray[Any, Any] | None:
    try:
        array = np.asarray(value)
    except (TypeError, ValueError, OverflowError):
        return None
    if array.dtype.kind not in {"b", "i", "u", "f"}:
        return None
    return array


def _descriptor(
    root: Path,
    path: Path,
    *,
    stream: str,
    field: str | None,
    role: str,
) -> dict[str, object]:
    array = np.load(path, mmap_mode="r", allow_pickle=False)
    if not array.flags.c_contiguous:
        raise NpyConversionError(f"array is not C-contiguous: {path}")
    return {
        "role": role,
        "stream": stream,
        **({"field": field} if field is not None else {}),
        "path": str(path.relative_to(root)),
        "dtype": array.dtype.str,
        "shape": list(array.shape),
        "c_contiguous": True,
    }


def _flush(array: np.ndarray[Any, Any]) -> None:
    if isinstance(array, np.memmap):
        array.flush()


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


def _existing(
    output: Path, recording: Path, metadata_checksum: str
) -> dict[str, object] | None:
    manifest_path = output / MANIFEST_FILE
    if not manifest_path.is_file():
        return None
    document = _read_json(manifest_path)
    if (
        document.get("format") != NPY_FORMAT
        or document.get("source_recording") != str(recording)
        or document.get("source_metadata_checksum") != metadata_checksum
    ):
        return None
    arrays = document.get("arrays")
    if not isinstance(arrays, list):
        return None
    for raw in arrays:
        if not isinstance(raw, dict) or not isinstance(raw.get("path"), str):
            return None
        path = output / raw["path"]
        try:
            array = np.load(path, mmap_mode="r", allow_pickle=False)
        except (OSError, ValueError):
            return None
        if (
            array.dtype.str != raw.get("dtype")
            or list(array.shape) != raw.get("shape")
            or not array.flags.c_contiguous
        ):
            return None
    return document


def _stream_plan(reader: Any, stream: str) -> tuple[dict[str, tuple[tuple[int, ...], str]], list[dict[str, str]], bool]:
    records = iter(reader.iter_verified_records(stream))
    try:
        first = next(records)
    except StopIteration as error:
        raise NpyConversionError(f"stream {stream!r} is empty") from error
    candidates: dict[str, tuple[tuple[int, ...], str]] = {}
    omitted: list[dict[str, str]] = []
    for field, value in first.values.items():
        array = _numeric(value)
        if array is None:
            omitted.append({"field": field, "reason": "not fixed-shape numeric JSON"})
        else:
            candidates[field] = (tuple(array.shape), array.dtype.str)
    physical_time = first.physical_time is not None
    for record in records:
        if (record.physical_time is not None) != physical_time:
            raise NpyConversionError(
                f"physical-time presence changes within stream {stream!r}"
            )
        for field in tuple(candidates):
            array = _numeric(record.values[field])
            shape, dtype = candidates[field]
            if array is None or tuple(array.shape) != shape or array.dtype.str != dtype:
                candidates.pop(field)
                omitted.append(
                    {"field": field, "reason": "numeric dtype or shape changes across records"}
                )
    return candidates, omitted, physical_time


def _convert_stream(
    reader: Any,
    stream: str,
    stream_index: int,
    temporary: Path,
) -> tuple[list[dict[str, object]], dict[str, object]]:
    fields, omitted, has_physical_time = _stream_plan(reader, stream)
    count = reader.stream_record_count(stream)
    prefix = f"{stream_index:04d}-{_safe_name(stream)}"
    arrays: dict[str, np.memmap[Any, Any]] = {}
    paths: dict[str, Path] = {}
    for field_index, (field, (shape, dtype)) in enumerate(fields.items()):
        path = temporary / f"{prefix}_{field_index:04d}-{_safe_name(field)}.npy"
        paths[field] = path
        arrays[field] = np.lib.format.open_memmap(
            path, mode="w+", dtype=np.dtype(dtype), shape=(count, *shape)
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
    for observed, record in enumerate(reader.iter_verified_records(stream), start=1):
        index = observed - 1
        iterations[index] = record.iteration
        if physical_times is not None:
            if record.physical_time is None:
                raise NpyConversionError(f"stream {stream!r} lost physical time")
            physical_times[index] = record.physical_time
        for field, target in arrays.items():
            target[index] = np.asarray(record.values[field], dtype=target.dtype)
    if observed != count:
        raise NpyConversionError(
            f"stream {stream!r} yielded {observed} records; expected {count}"
        )
    for array in (*arrays.values(), iterations):
        _flush(array)
    if physical_times is not None:
        _flush(physical_times)
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
        descriptors.append(
            _descriptor(
                temporary,
                physical_path,
                stream=stream,
                field=None,
                role="physical_times",
            )
        )
    descriptors.extend(
        _descriptor(
            temporary,
            path,
            stream=stream,
            field=field,
            role="field",
        )
        for field, path in paths.items()
    )
    return descriptors, {
        "name": stream,
        "records": count,
        "converted_fields": list(fields),
        "omitted_fields": omitted,
    }


def convert_recording(
    recording_directory: str | Path,
    output_directory: str | Path | None = None,
) -> dict[str, object]:
    """Verify and convert one completed recording into C-contiguous NPY arrays."""
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
            "source_version": FORMAT_VERSION,
            "source_recording": str(recording),
            "source_metadata_checksum": metadata_checksum,
            "user_metadata": dict(reader.user_metadata),
            "terminal_metadata": dict(reader.terminal_metadata),
            "streams": streams,
            "arrays": arrays,
        }
        _write_json(temporary / MANIFEST_FILE, manifest)
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
        description="Convert completed Scientific Workflow recordings to NPY arrays."
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
            output_directory = os.environ["WORKFLOW_TASK_OUTPUT"]
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
