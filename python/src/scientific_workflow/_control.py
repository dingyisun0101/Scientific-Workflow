"""Private cooperative control for the coordinated standard converter."""
import json
import os
from pathlib import Path
import time

_TOKEN = "parent"
_LAST_CHECK = 0.0
_PAUSE_STARTED = None
_PAUSE_TOTAL = 0.0


def state() -> tuple[Path | None, bool, bool]:
    value = os.environ.get("WORKFLOW_CONTROL_PATH")
    if not value:
        return None, False, False
    path = Path(value)
    document = json.loads(path.read_text(encoding="utf-8"))
    global _PAUSE_STARTED, _PAUSE_TOTAL
    now = time.monotonic()
    if document["paused"] and _PAUSE_STARTED is None:
        _PAUSE_STARTED = now
    elif not document["paused"] and _PAUSE_STARTED is not None:
        _PAUSE_TOTAL += now - _PAUSE_STARTED
        _PAUSE_STARTED = None
    return path, document["paused"], document["cancelled"]


def acknowledgement(path: Path, token: str) -> Path:
    return path.with_name(path.name + f".{token}.paused")


def checkpoint(*, force: bool = False) -> None:
    global _LAST_CHECK
    now = time.monotonic()
    if not force and now - _LAST_CHECK < 0.02:
        return
    _LAST_CHECK = now
    path, paused, cancelled = state()
    if cancelled:
        raise InterruptedError("Workflow conversion cancelled")
    if not paused or path is None:
        return
    ack = acknowledgement(path, _TOKEN)
    ack.touch()
    try:
        while paused:
            time.sleep(0.01)
            _, paused, cancelled = state()
            if cancelled:
                raise InterruptedError("Workflow conversion cancelled")
    finally:
        ack.unlink(missing_ok=True)


def active_time() -> float:
    """Converter diagnostic clock; Runtime remains authoritative for task budgets."""
    state()
    return (_PAUSE_STARTED if _PAUSE_STARTED is not None else time.monotonic()) - _PAUSE_TOTAL
