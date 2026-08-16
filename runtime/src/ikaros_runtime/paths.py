"""Canonical paths owned by one Ikaros Runtime home."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True, slots=True)
class RuntimePaths:
    home: Path
    config: Path
    state_db: Path
    memory_db: Path
    skills: Path

    @classmethod
    def from_home(cls, runtime_home: Path) -> RuntimePaths:
        home = runtime_home.expanduser().resolve()
        return cls(
            home=home,
            config=home / "config.yaml",
            state_db=home / "state.db",
            memory_db=home / "memory.db",
            skills=home / "skills",
        )


__all__ = ["RuntimePaths"]
