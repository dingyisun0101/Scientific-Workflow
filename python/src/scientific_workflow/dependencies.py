"""Typed access to immutable results from Workflow's declared dependencies.

Snapshot selection performs no scientific I/O and never broadens runtime scope.
The core module has no NumPy dependency. See api.md for the complete contract.
"""
from __future__ import annotations

import copy
import json
import os
from dataclasses import dataclass
from pathlib import Path
from typing import Generic, Iterator, TypeVar


class DependencyError(ValueError):
    """Invalid dependency snapshot or selection."""


class MissingDependencyError(DependencyError):
    """No result satisfies a selection requiring exactly one."""


class AmbiguousDependencyError(DependencyError):
    """Multiple results satisfy a selection requiring at most one."""

    def __init__(self, selection: str, matches: tuple[str, ...]):
        self.selection, self.matches = selection, matches
        super().__init__(f"ambiguous dependency {selection}: {', '.join(matches)}; select a phase or task")


@dataclass(frozen=True, slots=True)
class RecordingDependency:
    phase: str
    task: str
    execution_unit: str
    member: str
    final_iteration: int
    directory: Path


@dataclass(frozen=True, slots=True)
class ProgramDependency:
    phase: str
    task: str
    directory: Path
    executable: Path
    python_script: Path | None


@dataclass(frozen=True, slots=True)
class NpyDependency:
    phase: str
    task: str
    directory: Path


T = TypeVar("T", RecordingDependency, ProgramDependency, NpyDependency)


@dataclass(frozen=True, slots=True)
class Selection(Generic[T]):
    """Immutable intersection of exact selectors, in snapshot order."""
    _entries: tuple[T, ...]
    _filters: tuple[str, ...] = ()

    def _filter(self, field: str, value: str) -> Selection[T]:
        return Selection(tuple(e for e in self._entries if getattr(e, field, None) == value),
                         (*self._filters, f"{field}={value!r}"))

    def in_phase(self, phase: str) -> Selection[T]:
        return self._filter("phase", phase)

    def task(self, identity: str) -> Selection[T]:
        return self._filter("task", identity)

    def execution_unit(self, key: str) -> Selection[T]:
        """Restrict recording results to an execution-unit key."""
        return self._filter("execution_unit", key)

    def member(self, identity: str) -> Selection[T]:
        """Restrict recording results to a member identity."""
        return self._filter("member", identity)

    def one(self) -> T:
        result = self.optional()
        if result is None:
            raise MissingDependencyError(f"no dependency matches {self._filters}")
        return result

    def optional(self) -> T | None:
        if len(self._entries) > 1:
            raise AmbiguousDependencyError(str(self._filters), tuple(
                f"{e.phase}/{e.task}" + (f"/{e.member}" if isinstance(e, RecordingDependency) else "")
                for e in self._entries))
        return self._entries[0] if self._entries else None

    def __iter__(self) -> Iterator[T]:
        return iter(self._entries)

    def iter(self) -> Iterator[T]:
        return iter(self)


def _name(value: object) -> str:
    if not isinstance(value, str) or not value or value.strip() != value:
        raise DependencyError("expected a nonempty identifier without surrounding whitespace")
    return value


def _path(value: object) -> Path:
    if not isinstance(value, str) or not Path(value).is_absolute():
        raise DependencyError(f"expected absolute path, got {value!r}")
    return Path(value)


def _list(value: object) -> list:
    if not isinstance(value, list):
        raise DependencyError("expected an array in dependency snapshot")
    return value


def _unique(value: str, seen: set[str]) -> None:
    if value in seen:
        raise DependencyError(f"duplicate dependency identity {value!r}")
    seen.add(value)


class Dependencies:
    """Validated dependency snapshot, including preserved unknown workload kinds."""

    def __init__(self, snapshot: object):
        self._raw = copy.deepcopy(snapshot)
        recordings, programs, batches = [], [], []
        try:
            phases_seen = set()
            for phase in _list(self._raw):
                phase_name = _name(phase["phase"])
                _unique(phase_name, phases_seen)
                tasks_seen = set()
                for task in _list(phase["tasks"]):
                    identity = _name(task["identity"])
                    _unique(identity, tasks_seen)
                    directory = _path(task["output_directory"])
                    workload = task["workload"]
                    kind = _name(workload["kind"])
                    if kind == "execution_unit":
                        key = _name(workload["execution_unit"])
                        members = _list(workload["members"])
                        if not members:
                            raise DependencyError("execution unit has no members")
                        members_seen = set()
                        for member in members:
                            name = _name(member["identity"])
                            _unique(name, members_seen)
                            iteration = member["final_iteration"]
                            if type(iteration) is not int or not 0 <= iteration <= 2**64 - 1:
                                raise DependencyError("final_iteration must be a u64")
                            recordings.append(RecordingDependency(phase_name, identity, key, name, iteration, _path(member["output_directory"])))
                    elif kind in ("program", "python"):
                        script = workload.get("python_script")
                        if kind == "python" and script is None:
                            raise DependencyError("python workload requires python_script")
                        programs.append(ProgramDependency(phase_name, identity, directory / "artifacts", _path(workload["executable"]), _path(script) if script is not None else None))
                    elif kind == "npy":
                        batches.append(NpyDependency(phase_name, identity, _path(workload["processed_directory"])))
        except (KeyError, TypeError, AttributeError) as error:
            raise DependencyError(f"malformed dependency snapshot: {error}") from error
        self._recordings, self._programs, self._batches = tuple(recordings), tuple(programs), tuple(batches)

    @classmethod
    def load(cls, path: str | Path) -> Dependencies:
        """Load an explicit snapshot; failures identify its expected path."""
        try:
            return cls(json.loads(Path(path).read_text(encoding="utf-8")))
        except (OSError, ValueError) as error:
            raise DependencyError(f"cannot load dependency snapshot {path}: {error}") from error

    @classmethod
    def from_env(cls) -> Dependencies:
        """Load WORKFLOW_DEPENDENCIES_PATH from a standard Workflow launch."""
        path = os.environ.get("WORKFLOW_DEPENDENCIES_PATH")
        if not path:
            raise DependencyError("missing WORKFLOW_DEPENDENCIES_PATH; run through Workflow's standard study layout or use Dependencies.load(path)")
        return cls.load(path)

    def raw_json(self) -> object:
        """Return an independent JSON copy, including unknown extensions."""
        return copy.deepcopy(self._raw)

    def recordings(self) -> Selection[RecordingDependency]:
        return Selection(self._recordings)

    def programs(self) -> Selection[ProgramDependency]:
        return Selection(self._programs)

    def npy_batches(self) -> Selection[NpyDependency]:
        return Selection(self._batches)
