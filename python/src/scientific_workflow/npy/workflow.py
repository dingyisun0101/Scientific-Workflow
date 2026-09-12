from __future__ import annotations

from contextlib import contextmanager
from concurrent.futures import FIRST_COMPLETED, ProcessPoolExecutor, wait
import fcntl
import json
import os
from pathlib import Path
import shutil
import tempfile
import time
from collections.abc import Iterable

from .. import _control
from ..dependencies import Dependencies
from ..reader import FORMAT_NAME, open_completed_recording
from ..reporting import log, progress
from .format import (
    MANIFEST_FILE, NPY_BATCH_FORMAT, NPY_FORMAT, NpyConversionError,
    _existing, _manifest_exclusions, _normalize_exclusions, _read_json,
    _sha256, _write_json, open_npy_conversion, open_npy_batch,
)
from .progress import _drain_updates, _stage, _worker_context, _worker_setup
from .writing import _convert_stream

@contextmanager
def _publisher(directory: Path):
    directory.mkdir(parents=True, exist_ok=True)
    descriptor = os.open(directory, os.O_RDONLY | os.O_DIRECTORY)
    try:
        while True:
            try:
                fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                _control.checkpoint(force=True)
                time.sleep(0.01)
        yield
    finally:
        fcntl.flock(descriptor, fcntl.LOCK_UN)
        os.close(descriptor)


def convert_recording(
    recording_directory: str | Path,
    output_directory: str | Path | None = None,
    *,
    exclude_streams: Iterable[str] = (),
) -> dict[str, object]:
    """Publish selected streams atomically; exclusions use exact stream names.

    Missing names are ignored. Excluded chunks are not read or verified. The
    recording metadata and every included stream are still verified. Reuse
    requires the same exclusions; excluding every stream publishes metadata only.
    """
    excluded = _normalize_exclusions(exclude_streams)
    recording = Path(recording_directory).expanduser().resolve(strict=True)
    output = Path(output_directory).expanduser().resolve() if output_directory is not None else recording.with_name(recording.name + "-npy")
    with _publisher(output.parent):
        _control.checkpoint(force=True)
        return _convert_recording(recording, output, excluded)


def _convert_recording(
    recording_directory: str | Path,
    output_directory: str | Path | None = None,
    exclude_streams: tuple[str, ...] = (),
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
        existing = _existing(output, recording, metadata_checksum, exclude_streams)
        if existing is not None:
            return existing
        raise NpyConversionError(f"conflicting conversion output exists: {output}")
    output.parent.mkdir(parents=True, exist_ok=True)
    temporary = Path(tempfile.mkdtemp(prefix=f".{output.name}.tmp-", dir=output.parent))
    try:
        reader = open_completed_recording(recording)
        arrays: list[dict[str, object]] = []
        streams: list[dict[str, object]] = []
        for index, stream in enumerate(reader.stream_names):
            if stream in exclude_streams:
                continue
            converted, summary = _convert_stream(reader, stream, index, temporary)
            arrays.extend(converted)
            streams.append(summary)
        manifest: dict[str, object] = {
            "format": NPY_FORMAT,
            "source_format": FORMAT_NAME,
            "source_version": reader.format_version,
            "source_recording": str(recording),
            "source_metadata_checksum": metadata_checksum,
            "exclude_streams": list(exclude_streams),
            "user_metadata": dict(reader.user_metadata),
            "terminal_metadata": dict(reader.terminal_metadata),
            "streams": streams,
            "arrays": arrays,
        }
        _write_json(temporary / MANIFEST_FILE, manifest)
        _stage("verification", force=True)
        open_npy_conversion(temporary)
        os.replace(temporary, output)
        return manifest
    except BaseException:
        shutil.rmtree(temporary, ignore_errors=True)
        raise


def _dependency_recordings(dependencies_path: Path) -> list[Path]:
    recordings = list(dict.fromkeys(entry.directory.resolve(strict=True)
        for entry in Dependencies.load(dependencies_path).recordings()))
    if not recordings:
        raise NpyConversionError("$npy received no completed execution-unit recordings")
    return recordings


def _convert_job(ordinal: int, recording: Path, member_output: Path, exclude_streams: tuple[str, ...]) -> dict[str, object]:
    _control._TOKEN = str(ordinal)
    _control.checkpoint(force=True)
    reused = member_output.exists()
    manifest = _convert_recording(recording, member_output, exclude_streams)
    return {
        "ordinal": ordinal,
        "source_recording": manifest["source_recording"],
        "manifest": f"{member_output.name}/{MANIFEST_FILE}",
        "manifest_checksum": _sha256(member_output / MANIFEST_FILE),
        "reused": reused,
    }


def convert_workflow_dependencies(
    dependencies_path: str | Path,
    output_directory: str | Path,
    *,
    exclude_streams: Iterable[str] = (),
) -> dict[str, object]:
    """Convert prerequisites within WORKFLOW_THREADS; publish in stable order.

    Outside Workflow the default is one worker. Parallelism is by recording.
    Linux directory locks serialize publishers; completed members survive failure
    and are verified/reused on retry. No partial success batch is published.
    Exact-name exclusions apply uniformly to every member and are part of reuse
    identity. Missing names are ignored; excluded chunks are never converted.
    """
    excluded = _normalize_exclusions(exclude_streams)
    dependencies = Path(dependencies_path).expanduser().resolve(strict=True)
    output = Path(output_directory).expanduser().resolve()
    recordings = _dependency_recordings(dependencies)
    try:
        allowance = int(os.environ.get("WORKFLOW_THREADS", "1"))
    except ValueError as error:
        raise NpyConversionError("WORKFLOW_THREADS must be a positive integer") from error
    if allowance < 1:
        raise NpyConversionError("WORKFLOW_THREADS must be positive")
    workers = min(allowance, len(recordings))
    context = _worker_context()
    started = _control.active_time()
    log(f"conversion started: {len(recordings)} members, {workers} worker(s)")
    results = {}
    with _publisher(output):
        batch_path = output / MANIFEST_FILE
        if batch_path.exists():
            batch = open_npy_batch(output)
            if _manifest_exclusions(batch.manifest) != excluded:
                raise NpyConversionError("conflicting stream exclusions for existing NPY batch")
            if len(batch.members) != len(recordings) or any(
                member.manifest.get("source_recording") != str(recording)
                or member.manifest.get("source_metadata_checksum") != _sha256(recording / "metadata.json")
                for member, recording in zip(batch.members, recordings)
            ):
                raise NpyConversionError("conflicting inputs for immutable NPY batch")
            log(f"conversion reused: {len(recordings)} members", level="success")
            return dict(batch.manifest)
        pool = None
        updates = None
        control_path = None
        parent_ack = None
        try:
            if workers == 1:
                for ordinal, recording in enumerate(recordings):
                    _control._TOKEN = "parent"
                    _control.checkpoint(force=True)
                    log(f"member {ordinal} started: {recording}")
                    # Keep the parent control token in the serial path.
                    member_output = output / f"member-{ordinal:06d}"
                    reused = member_output.exists()
                    manifest = _convert_recording(recording, member_output, excluded)
                    results[ordinal] = {"ordinal": ordinal, "source_recording": manifest["source_recording"], "manifest": f"{member_output.name}/{MANIFEST_FILE}", "manifest_checksum": _sha256(member_output / MANIFEST_FILE)}
                    log(f"member {ordinal} {'reused' if reused else 'completed'}: {recording}")
                    progress("conversion", len(results), len(recordings), unit="members")
            else:
                updates = context.Queue(maxsize=128)
                pool = ProcessPoolExecutor(max_workers=workers, mp_context=context, initializer=_worker_setup, initargs=(updates,))
                pending = {}
                next_ordinal = 0
                while next_ordinal < len(recordings) or pending:
                    _drain_updates(updates)
                    control_path, paused, cancelled = _control.state()
                    if cancelled:
                        raise InterruptedError("Workflow conversion cancelled")
                    while not paused and next_ordinal < len(recordings) and len(pending) < workers:
                        ordinal = next_ordinal
                        recording = recordings[ordinal]
                        log(f"member {ordinal} started: {recording}")
                        future = pool.submit(_convert_job, ordinal, recording, output / f"member-{ordinal:06d}", excluded)
                        pending[future] = ordinal
                        next_ordinal += 1
                    done, _ = wait(pending, timeout=0.02, return_when=FIRST_COMPLETED) if pending else ((), ())
                    for future in done:
                        ordinal = pending.pop(future)
                        result = future.result()
                        reused = result.pop("reused")
                        results[ordinal] = result
                        log(f"member {ordinal} {'reused' if reused else 'completed'}: {recordings[ordinal]}")
                        progress("conversion", len(results), len(recordings), unit="members")
                    if control_path is not None:
                        parent_ack = _control.acknowledgement(control_path, "parent")
                        if paused and all(_control.acknowledgement(control_path, str(n)).exists() for n in pending.values()):
                            parent_ack.touch()
                        else:
                            parent_ack.unlink(missing_ok=True)
                    if paused and not pending:
                        time.sleep(0.02)
                pool.shutdown(wait=True)
                _drain_updates(updates)
                pool = None
            _control._TOKEN = "parent"
            _control.checkpoint(force=True)
            batch = {"format": NPY_BATCH_FORMAT, "exclude_streams": list(excluded), "members": [results[n] for n in range(len(recordings))]}
            handle, temporary_name = tempfile.mkstemp(prefix=".manifest.tmp-", dir=output)
            temporary = Path(temporary_name)
            try:
                with os.fdopen(handle, "w", encoding="utf-8") as stream:
                    json.dump(batch, stream, indent=2, sort_keys=True)
                    stream.write("\n")
                    stream.flush()
                    os.fsync(stream.fileno())
                os.link(temporary, output / MANIFEST_FILE)
            finally:
                temporary.unlink(missing_ok=True)
            log(f"conversion complete: {len(results)} members in {_control.active_time() - started:.3f} active seconds", level="success")
            return batch
        except BaseException as error:
            if pool is not None:
                pool.kill_workers()
                pool = None
            log(f"conversion failed: {error}", level="error")
            raise
        finally:
            if pool is not None:
                pool.shutdown(wait=True, cancel_futures=True)
            if updates is not None:
                updates.close()
                updates.join_thread()
            if parent_ack is not None:
                parent_ack.unlink(missing_ok=True)
