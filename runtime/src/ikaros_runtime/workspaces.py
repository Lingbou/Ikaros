"""Managed per-thread workspaces kept outside the Runtime home.

Every Thread created without an explicit project workspace owns one directory
under ``~/Documents/Ikaros/<YYYY-MM-DD>/<HHMMSS>-<thread suffix>`` so the agent
starts from an empty, user-visible location instead of the Runtime process
working directory.
"""

from __future__ import annotations

from datetime import datetime
from pathlib import Path

from .domain import WorkspaceSummary
from .paths import managed_workspaces_root

MANAGED_WORKSPACE_ID_PREFIX = "workspace_"
_THREAD_ID_PREFIX = "thread_"
_SUFFIX_LENGTH = 8


def provision_managed_workspace(
    thread_id: str,
    created_at: str,
    *,
    root: Path | None = None,
) -> WorkspaceSummary:
    """Create (or reuse) the dated workspace directory owned by one Thread."""

    moment = datetime.fromisoformat(created_at).astimezone()
    suffix = thread_id.removeprefix(_THREAD_ID_PREFIX)[:_SUFFIX_LENGTH]
    day = moment.strftime("%Y-%m-%d")
    base = root if root is not None else managed_workspaces_root()
    workspace_root = base / day / f"{moment:%H%M%S}-{suffix}"
    workspace_root.mkdir(parents=True, exist_ok=True)
    return WorkspaceSummary(
        id=f"{MANAGED_WORKSPACE_ID_PREFIX}{thread_id.removeprefix(_THREAD_ID_PREFIX)}",
        name=f"{day} {moment:%H:%M}",
        kind="managed",
        root_uri=str(workspace_root),
    )


__all__ = ["MANAGED_WORKSPACE_ID_PREFIX", "provision_managed_workspace"]
