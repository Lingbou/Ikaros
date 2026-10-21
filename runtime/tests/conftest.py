"""Shared test isolation for the Runtime suite."""

from __future__ import annotations

from pathlib import Path

import pytest


@pytest.fixture(autouse=True)
def _isolated_managed_workspaces(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """Keep automatically provisioned Thread workspaces inside the test tmp dir."""

    monkeypatch.setenv("IKAROS_WORKSPACES_ROOT", str(tmp_path / "managed-workspaces"))
