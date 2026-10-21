from __future__ import annotations

from datetime import datetime
from pathlib import Path

from ikaros_runtime.workspaces import provision_managed_workspace


def test_provision_managed_workspace_buckets_by_local_day(tmp_path: Path) -> None:
    created_at = "2026-10-21T07:30:45.123Z"
    workspace = provision_managed_workspace(
        "thread_abc12345def", created_at, root=tmp_path
    )

    moment = datetime.fromisoformat(created_at).astimezone()
    expected_root = tmp_path / moment.strftime("%Y-%m-%d") / f"{moment:%H%M%S}-abc12345"
    assert workspace.kind == "managed"
    assert workspace.root_uri == str(expected_root)
    assert workspace.id == "workspace_abc12345def"
    assert expected_root.is_dir()


def test_provision_managed_workspace_reuses_the_same_directory(tmp_path: Path) -> None:
    created_at = "2026-10-21T07:30:45.123Z"
    first = provision_managed_workspace("thread_abc12345def", created_at, root=tmp_path)
    second = provision_managed_workspace("thread_abc12345def", created_at, root=tmp_path)
    other = provision_managed_workspace("thread_ffffffff000", created_at, root=tmp_path)

    assert second == first
    assert other.root_uri != first.root_uri
    assert other.id != first.id
