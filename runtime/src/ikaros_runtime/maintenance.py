"""Application boundary for exclusive offline Runtime storage maintenance."""

from __future__ import annotations

from collections.abc import Callable
from pathlib import Path

from .paths import RuntimePaths
from .server.host import RuntimeHomeLock
from .storage import SqliteRuntimeStore
from .storage.maintenance import (
    ProjectionRepairReport,
    StateBackupReport,
    StateCheckReport,
)


async def check_runtime_state(runtime_home: Path) -> StateCheckReport:
    return await _with_offline_store(
        runtime_home,
        lambda store: store.check_state(),
        read_only=True,
    )


async def backup_runtime_state(
    runtime_home: Path,
    destination: Path | None = None,
) -> StateBackupReport:
    return await _with_offline_store(
        runtime_home,
        lambda store: store.create_state_backup(destination),
        read_only=True,
    )


async def repair_runtime_projections(
    runtime_home: Path,
    backup_destination: Path | None = None,
) -> ProjectionRepairReport:
    return await _with_offline_store(
        runtime_home,
        lambda store: store.repair_state_projections(backup_destination),
        read_only=False,
    )


async def _with_offline_store[ReportT: (
    StateCheckReport,
    StateBackupReport,
    ProjectionRepairReport,
)](
    runtime_home: Path,
    operation: Callable[[SqliteRuntimeStore], ReportT],
    *,
    read_only: bool,
) -> ReportT:
    paths = RuntimePaths.from_home(runtime_home)
    lock = await RuntimeHomeLock.acquire(paths.home, timeout_seconds=0)
    store: SqliteRuntimeStore | None = None
    try:
        state_path = _existing_state_path(paths)
        store = SqliteRuntimeStore.open_existing(state_path, read_only=read_only)
        return operation(store)
    finally:
        if store is not None:
            store.close()
        lock.release()


def _existing_state_path(paths: RuntimePaths) -> Path:
    state_path = paths.state_db
    if not state_path.is_file():
        raise FileNotFoundError(f"Runtime state database does not exist: {state_path}")
    return state_path


__all__ = [
    "backup_runtime_state",
    "check_runtime_state",
    "repair_runtime_projections",
]
