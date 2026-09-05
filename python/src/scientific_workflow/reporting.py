"""Opt-in standard logging and progress events for Workflow program launches."""
from __future__ import annotations
import json
import logging
import sys
import threading

_LOCK = threading.Lock()
_LEVELS = {"debug", "info", "warning", "error", "success"}


def _emit(event: dict) -> None:
    line = "@workflow " + json.dumps({"version": 1, **event}, ensure_ascii=True, allow_nan=False)
    if len(line.encode()) > 16383:
        raise ValueError("Workflow event exceeds 16 KiB")
    with _LOCK:
        sys.stderr.write(line + "\n")
        sys.stderr.flush()


def log(message: str, *, level: str = "info") -> None:
    """Emit one versioned log event; stderr errors propagate to the caller."""
    if level not in _LEVELS or not isinstance(message, str):
        raise ValueError("log requires a supported level and string message")
    _emit({"kind": "log", "level": level, "message": message})


def progress(stage: str, completed: int, total: int | None = None, *, unit: str = "records") -> None:
    """Emit task-local stage progress; totals are counts, not duration estimates."""
    if not isinstance(stage, str) or not stage or not isinstance(unit, str) or not unit:
        raise ValueError("stage and unit must be nonempty strings")
    if type(completed) is not int or not 0 <= completed < 2**64:
        raise ValueError("completed must be a u64")
    if total is not None and (type(total) is not int or not completed <= total < 2**64):
        raise ValueError("total must be a u64 at least completed")
    _emit({"kind": "progress", "stage": stage, "completed": completed, "total": total, "unit": unit})


class WorkflowHandler(logging.Handler):
    """Explicit logging adapter; importing this module never changes root logging."""
    def emit(self, record: logging.LogRecord) -> None:
        level = "error" if record.levelno >= logging.ERROR else "warning" if record.levelno >= logging.WARNING else "info" if record.levelno >= logging.INFO else "debug"
        log(self.format(record), level=level)


def install_logging(logger: logging.Logger | None = None, *, level: int = logging.INFO) -> WorkflowHandler:
    """Attach one adapter, idempotently per logger; caller controls logger level.

    Outside Workflow frames remain ordinary stderr lines. Attach explicitly in
    each process that needs reporting; standard converter workers report through
    their parent instead. Remove with logger.removeHandler(handler) and close().
    """
    logger = logger if logger is not None else logging.getLogger()
    with _LOCK:
        for handler in logger.handlers:
            if isinstance(handler, WorkflowHandler):
                handler.setLevel(level)
                return handler
        handler = WorkflowHandler(level)
        logger.addHandler(handler)
        return handler
