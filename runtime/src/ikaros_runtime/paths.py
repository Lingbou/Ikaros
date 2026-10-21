"""Canonical paths owned by one Ikaros Runtime home."""

from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

MANAGED_WORKSPACES_DIRECTORY = "Ikaros"
MANAGED_WORKSPACES_ROOT_ENV = "IKAROS_WORKSPACES_ROOT"


def managed_workspaces_root() -> Path:
    """Root that owns the per-thread workspaces provisioned outside the home."""

    configured = os.environ.get(MANAGED_WORKSPACES_ROOT_ENV, "").strip()
    if configured:
        return Path(configured).expanduser().resolve()
    return (Path.home() / "Documents" / MANAGED_WORKSPACES_DIRECTORY).resolve()


@dataclass(frozen=True, slots=True)
class RuntimePaths:
    home: Path
    config: Path
    state_db: Path
    memory_db: Path
    skills: Path
    artifacts: Path

    @classmethod
    def from_home(cls, runtime_home: Path) -> RuntimePaths:
        home = runtime_home.expanduser().resolve()
        return cls(
            home=home,
            config=home / "config.yaml",
            state_db=home / "state.db",
            memory_db=home / "memory.db",
            skills=home / "skills",
            artifacts=home / "artifacts",
        )


__all__ = [
    "MANAGED_WORKSPACES_DIRECTORY",
    "MANAGED_WORKSPACES_ROOT_ENV",
    "RuntimePaths",
    "managed_workspaces_root",
]
