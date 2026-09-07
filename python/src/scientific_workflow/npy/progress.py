from __future__ import annotations

import multiprocessing
import os
import queue
import sys
import time

from threadpoolctl import threadpool_limits

from .. import _control
from ..reporting import progress

_PROGRESS_QUEUE = None
_LAST_PROGRESS = 0.0
_THREAD_LIMIT = None


def _worker_context() -> multiprocessing.context.BaseContext:
    if os.name == "posix" and sys.platform.startswith("linux"):
        default = "fork"
    else:
        default = "spawn"
    requested = os.environ.get("WORKFLOW_NPY_CONTEXT", default).strip().lower()
    if requested in {"fork", "spawn", "forkserver"}:
        return multiprocessing.get_context(requested)
    return multiprocessing.get_context(default)


def _worker_setup(updates):
    global _PROGRESS_QUEUE, _THREAD_LIMIT
    _PROGRESS_QUEUE = updates
    _THREAD_LIMIT = threadpool_limits(limits=1)


def _stage(stage: str, completed: int = 0, total: int | None = None, *, force: bool = False):
    global _LAST_PROGRESS
    now = time.monotonic()
    if not force and now - _LAST_PROGRESS < 0.1:
        return
    _LAST_PROGRESS = now
    if _PROGRESS_QUEUE is None:
        progress(stage, completed, total)
    else:
        try:
            _PROGRESS_QUEUE.put_nowait((_control._TOKEN, stage, completed, total))
        except queue.Full:
            pass  # Display updates are bounded; verified data and logs are unaffected.


def _drain_updates(updates):
    while True:
        try:
            token, stage, completed, total = updates.get_nowait()
        except queue.Empty:
            return
        progress(f"member {token}: {stage}", completed, total)
