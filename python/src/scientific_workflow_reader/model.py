"""Structurally read-only representations reconstructed from recordings."""

from __future__ import annotations

from dataclasses import dataclass
from types import MappingProxyType
from typing import Any, Iterator, Mapping, Sequence


@dataclass(frozen=True, slots=True)
class StateField:
    """One exact field in a partial stream schema."""

    name: str
    description: str | None = None


@dataclass(frozen=True, slots=True)
class StateRecord:
    """One reconstructed partial state at an exact scientific coordinate."""

    iteration: int
    physical_time: float | None
    values: Mapping[str, Any]

    @classmethod
    def create(
        cls,
        iteration: int,
        physical_time: float | None,
        values: dict[str, Any],
    ) -> StateRecord:
        return cls(iteration, physical_time, MappingProxyType(values))


@dataclass(frozen=True, slots=True)
class StateSeries(Sequence[StateRecord]):
    """Complete eager result for one verified logical stream."""

    stream: str
    fields: tuple[StateField, ...]
    records: tuple[StateRecord, ...]

    def __len__(self) -> int:
        return len(self.records)

    def __getitem__(self, index):  # type: ignore[no-untyped-def]
        return self.records[index]

    def __iter__(self) -> Iterator[StateRecord]:
        return iter(self.records)

    @property
    def iterations(self) -> tuple[int, ...]:
        return tuple(record.iteration for record in self.records)
