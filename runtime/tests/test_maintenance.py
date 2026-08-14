from __future__ import annotations

import json
import os
import sqlite3
import subprocess
import sys
from pathlib import Path

import pytest

from ikaros_runtime.domain import JOURNAL_EVENT_SCHEMA_VERSION, ModelUsage, utc_now
from ikaros_runtime.errors import RuntimeHomeLockError
from ikaros_runtime.maintenance import check_runtime_state
from ikaros_runtime.server.host import RuntimeHomeLock
from ikaros_runtime.storage import SqliteRuntimeStore
from ikaros_runtime.storage import maintenance as storage_maintenance


def test_backup_captures_committed_wal_and_publishes_a_standalone_database(
    tmp_path: Path,
) -> None:
    state_path = tmp_path / "state.db"
    backup_path = tmp_path / "snapshots" / "state.db"
    store = SqliteRuntimeStore(state_path)
    try:
        expected, event = store.create_thread("WAL snapshot")
        wal_path = Path(f"{state_path}-wal")
        assert wal_path.is_file()
        assert wal_path.stat().st_size > 0

        report = store.create_state_backup(backup_path)

        assert report.backup_path == backup_path.resolve()
        assert report.latest_seq == event.seq
        assert report.event_count == 1
        assert report.size_bytes == backup_path.stat().st_size
    finally:
        store.close()

    connection = sqlite3.connect(backup_path)
    try:
        assert str(connection.execute("PRAGMA journal_mode").fetchone()[0]) == "delete"
        assert connection.execute("PRAGMA quick_check").fetchone()[0] == "ok"
    finally:
        connection.close()

    backup = SqliteRuntimeStore(backup_path)
    try:
        assert backup.list_thread_page(cursor=None, limit=50).threads == (expected,)
        assert backup.latest_sequence() == event.seq
    finally:
        backup.close()

def test_backup_refuses_overwrite_and_cleans_temporary_files_after_publish_failure(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    state_path = tmp_path / "state.db"
    backup_path = tmp_path / "backup.db"
    store = SqliteRuntimeStore(state_path)
    try:
        store.create_thread("Atomic backup")
        backup_path.write_bytes(b"do-not-overwrite")

        with pytest.raises(FileExistsError, match="already exists"):
            store.create_state_backup(backup_path)
        assert backup_path.read_bytes() == b"do-not-overwrite"

        backup_path.unlink()

        def fail_publish(source: Path, destination: Path) -> None:
            raise OSError("simulated publish failure")

        monkeypatch.setattr(storage_maintenance, "_publish_new_file", fail_publish)
        with pytest.raises(OSError, match="simulated publish failure"):
            store.create_state_backup(backup_path)

        assert not backup_path.exists()
        assert list(tmp_path.glob(".backup.db.*.tmp*")) == []
    finally:
        store.close()


def test_backup_removes_the_published_file_if_directory_sync_fails(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    backup_path = tmp_path / "post-publish-failure.db"
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        store.create_thread("Post-publish failure")

        def fail_directory_sync(path: Path) -> None:
            raise OSError("simulated directory sync failure")

        monkeypatch.setattr(storage_maintenance, "_fsync_directory", fail_directory_sync)
        with pytest.raises(OSError, match="simulated directory sync failure"):
            store.create_state_backup(backup_path)

        assert not backup_path.exists()
        assert list(tmp_path.glob(".post-publish-failure.db.*.tmp*")) == []
    finally:
        store.close()


def test_check_detects_projection_drift_and_repair_restores_it_without_touching_config(
    tmp_path: Path,
) -> None:
    state_path = tmp_path / "state.db"
    config_path = tmp_path / "config.yaml"
    config_bytes = b"providers:\r\n  deepseek:\r\n    apiKey: preserve-me\r\n"
    config_path.write_bytes(config_bytes)
    backup_path = tmp_path / "before-repair.db"
    store = SqliteRuntimeStore(state_path)
    try:
        expected, _event = store.create_thread("Canonical title")
        journal_before = _journal_rows(store._connection)
        with store._connection:
            store._connection.execute(
                "UPDATE threads SET title = 'projection drift' WHERE id = ?",
                (expected.id,),
            )

        with pytest.raises(RuntimeError, match="projections do not match"):
            store.check_state()

        report = store.repair_state_projections(backup_path)

        assert report.backup.backup_path == backup_path.resolve()
        assert report.state.thread_count == 1
        assert store.list_thread_page(cursor=None, limit=50).threads == (expected,)
        assert _journal_rows(store._connection) == journal_before
        assert config_path.read_bytes() == config_bytes
    finally:
        store.close()

    original = sqlite3.connect(backup_path)
    try:
        assert original.execute("SELECT title FROM threads").fetchone()[0] == "projection drift"
    finally:
        original.close()


def test_repair_restores_model_usage_projection_from_the_journal(tmp_path: Path) -> None:
    state_path = tmp_path / "state.db"
    backup_path = tmp_path / "usage-drift.db"
    store = SqliteRuntimeStore(state_path)
    try:
        thread, _event = store.create_thread("Usage repair")
        prepared = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="measure",
            provider_id="deepseek",
            model_id="deepseek-chat",
        )
        store.mark_run_running(prepared.run_id)
        store.record_model_usage(
            prepared.run_id,
            step_ordinal=1,
            usage=ModelUsage(input_tokens=7, output_tokens=3, total_tokens=10),
        )
        with store._connection:
            store._connection.execute(
                "UPDATE model_usages SET total_tokens = 999 WHERE run_id = ?",
                (prepared.run_id,),
            )

        with pytest.raises(RuntimeError, match="projections do not match"):
            store.check_state()

        store.repair_state_projections(backup_path)

        assert store.read_usage().summary.lifetime_tokens == 10
        assert store._connection.execute(
            "SELECT total_tokens FROM model_usages WHERE run_id = ?",
            (prepared.run_id,),
        ).fetchone()[0] == 10
    finally:
        store.close()


def test_repair_rolls_back_on_an_unknown_event_and_preserves_its_automatic_backup(
    tmp_path: Path,
) -> None:
    state_path = tmp_path / "state.db"
    backup_path = tmp_path / "failed-repair.db"
    store = SqliteRuntimeStore(state_path)
    try:
        thread, _event = store.create_thread("Keep this projection")
        with store._connection:
            store._connection.execute(
                "UPDATE threads SET title = 'pre-repair projection' WHERE id = ?",
                (thread.id,),
            )
            store._connection.execute(
                """
                INSERT INTO events(
                    schema_version, event_type, thread_id, branch_id,
                    created_at, payload_json
                ) VALUES (?, 'future.event', ?, ?, ?, '{}')
                """,
                (
                    JOURNAL_EVENT_SCHEMA_VERSION,
                    thread.id,
                    thread.default_branch_id,
                    utc_now(),
                ),
            )
        journal_before = _journal_rows(store._connection)
        projection_before = _projection_rows(store._connection)

        with pytest.raises(RuntimeError, match="backup preserved") as failure:
            store.repair_state_projections(backup_path)

        assert isinstance(failure.value.__cause__, RuntimeError)
        assert "unsupported" in str(failure.value.__cause__)
        assert backup_path.is_file()
        assert _journal_rows(store._connection) == journal_before
        assert _projection_rows(store._connection) == projection_before
    finally:
        store.close()


def test_check_rejects_a_noncontiguous_journal_sequence(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        store.create_thread("First")
        store.create_thread("Second")
        with store._connection:
            store._connection.execute("DELETE FROM events WHERE seq = 1")

        with pytest.raises(RuntimeError, match="sequence is not contiguous"):
            store.check_state()
        backup_path = tmp_path / "gap-backup.db"
        with pytest.raises(RuntimeError, match="sequence is not contiguous"):
            store.create_state_backup(backup_path)
        with pytest.raises(RuntimeError, match="sequence is not contiguous"):
            store.repair_state_projections(backup_path)
        assert not backup_path.exists()
    finally:
        store.close()


def test_check_rejects_a_deleted_tail_event_before_the_next_append_can_skip_a_sequence(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        store.create_thread("Deleted tail")
        with store._connection:
            store._connection.execute("DELETE FROM branches")
            store._connection.execute("DELETE FROM threads")
            store._connection.execute("DELETE FROM events")

        with pytest.raises(RuntimeError, match="high-water mark is not contiguous"):
            store.check_state()
    finally:
        store.close()


def test_repair_can_replace_foreign_key_broken_projections_from_a_valid_journal(
    tmp_path: Path,
) -> None:
    state_path = tmp_path / "state.db"
    backup_path = tmp_path / "orphaned-projections.db"
    store = SqliteRuntimeStore(state_path)
    try:
        expected, _event = store.create_thread("Rebuild orphaned projections")
        journal_before = _journal_rows(store._connection)
    finally:
        store.close()

    connection = sqlite3.connect(state_path)
    try:
        connection.execute("PRAGMA foreign_keys = OFF")
        connection.execute("DELETE FROM threads")
        connection.commit()
        assert connection.execute("PRAGMA foreign_key_check").fetchone() is not None
    finally:
        connection.close()

    maintenance_store = SqliteRuntimeStore.open_existing(state_path, read_only=False)
    try:
        with pytest.raises(RuntimeError, match="foreign_key_check failed"):
            maintenance_store.create_state_backup(tmp_path / "verified-backup.db")

        report = maintenance_store.repair_state_projections(backup_path)

        assert report.state.thread_count == 1
        assert maintenance_store.list_thread_page(cursor=None, limit=50).threads == (expected,)
        assert _journal_rows(maintenance_store._connection) == journal_before
        assert backup_path.is_file()
    finally:
        maintenance_store.close()


def test_check_does_not_initialize_an_existing_empty_database(tmp_path: Path) -> None:
    runtime_home = tmp_path / "runtime"
    runtime_home.mkdir()
    state_path = runtime_home / "state.db"
    state_path.write_bytes(b"")

    checked = _run_storage_cli(runtime_home, "check")

    assert checked.returncode != 0
    assert "reset required" in checked.stderr
    assert state_path.read_bytes() == b""
    assert not Path(f"{state_path}-wal").exists()
    assert not Path(f"{state_path}-shm").exists()


@pytest.mark.asyncio
async def test_offline_maintenance_requires_existing_state_and_exclusive_home_lock(
    tmp_path: Path,
) -> None:
    missing_home = tmp_path / "missing"
    with pytest.raises(FileNotFoundError, match="does not exist"):
        await check_runtime_state(missing_home)
    assert not (missing_home / "state.db").exists()

    runtime_home = tmp_path / "runtime"
    store = SqliteRuntimeStore(runtime_home / "state.db")
    store.create_thread("Locked")
    store.close()
    lock = await RuntimeHomeLock.acquire(runtime_home, timeout_seconds=0.5)
    try:
        with pytest.raises(RuntimeHomeLockError, match="already in use"):
            await check_runtime_state(runtime_home)
        completed = _run_storage_cli(runtime_home, "check")
        assert completed.returncode != 0
        assert "already in use" in completed.stderr
    finally:
        lock.release()


@pytest.mark.parametrize(
    "arguments",
    [
        ("check",),
        ("backup", "--output", "missing-backup.db"),
        ("repair-projections", "--backup-output", "missing-repair.db"),
    ],
)
def test_storage_cli_never_creates_missing_state(
    tmp_path: Path,
    arguments: tuple[str, ...],
) -> None:
    runtime_home = tmp_path / "missing-runtime"
    config_path = runtime_home / "config.yaml"
    runtime_home.mkdir()
    config_bytes = b"not: [valid"
    config_path.write_bytes(config_bytes)

    completed = _run_storage_cli(runtime_home, *arguments)

    assert completed.returncode != 0
    assert completed.stdout == ""
    assert not (runtime_home / "state.db").exists()
    assert config_path.read_bytes() == config_bytes


def test_storage_cli_check_backup_and_repair_are_real_offline_commands(tmp_path: Path) -> None:
    runtime_home = tmp_path / "runtime home"
    state_path = runtime_home / "state.db"
    config_path = runtime_home / "config.yaml"
    store = SqliteRuntimeStore(state_path)
    try:
        expected, _event = store.create_thread("CLI title")
    finally:
        store.close()
    config_bytes = b"\xffinvalid yaml that maintenance must never parse"
    config_path.write_bytes(config_bytes)

    checked = _run_storage_cli(runtime_home, "check")
    assert checked.returncode == 0, checked.stderr
    assert checked.stderr == ""
    assert checked.stdout.count("\n") == 1
    check_result = json.loads(checked.stdout)
    assert check_result["operation"] == "storage.check"
    assert check_result["threadCount"] == 1

    backup_path = tmp_path / "cli-backup.db"
    backed_up = _run_storage_cli(runtime_home, "backup", "--output", str(backup_path))
    assert backed_up.returncode == 0, backed_up.stderr
    assert backed_up.stderr == ""
    assert backed_up.stdout.count("\n") == 1
    backup_result = json.loads(backed_up.stdout)
    assert backup_result["operation"] == "storage.backup"
    assert Path(backup_result["backupPath"]) == backup_path.resolve()
    assert backup_path.is_file()

    connection = sqlite3.connect(state_path)
    try:
        connection.execute(
            "UPDATE threads SET title = 'CLI projection drift' WHERE id = ?",
            (expected.id,),
        )
        connection.commit()
    finally:
        connection.close()

    failed_check = _run_storage_cli(runtime_home, "check")
    assert failed_check.returncode != 0
    assert "repair required" in failed_check.stderr

    repair_backup = tmp_path / "cli-before-repair.db"
    repaired = _run_storage_cli(
        runtime_home,
        "repair-projections",
        "--backup-output",
        str(repair_backup),
    )
    assert repaired.returncode == 0, repaired.stderr
    assert repaired.stderr == ""
    assert repaired.stdout.count("\n") == 1
    repair_result = json.loads(repaired.stdout)
    assert repair_result["operation"] == "storage.repair-projections"
    assert Path(repair_result["backup"]["backupPath"]) == repair_backup.resolve()

    reloaded = SqliteRuntimeStore(state_path)
    try:
        assert reloaded.list_thread_page(cursor=None, limit=50).threads == (expected,)
    finally:
        reloaded.close()
    assert config_path.read_bytes() == config_bytes


def _journal_rows(connection: sqlite3.Connection) -> list[tuple[object, ...]]:
    return [
        tuple(row)
        for row in connection.execute(
            """
            SELECT seq, schema_version, event_type, thread_id, branch_id, turn_id,
                   run_id, item_id, created_at, payload_json
            FROM events ORDER BY seq
            """
        )
    ]


def _projection_rows(connection: sqlite3.Connection) -> dict[str, list[tuple[object, ...]]]:
    return {
        table: [tuple(row) for row in connection.execute(f"SELECT * FROM {table} ORDER BY 1")]
        for table in ("threads", "branches", "turns", "runs", "model_usages", "items")
    }


def _run_storage_cli(runtime_home: Path, *arguments: str) -> subprocess.CompletedProcess[str]:
    environment = os.environ.copy()
    environment["IKAROS_HOME"] = str(runtime_home)
    return subprocess.run(
        [sys.executable, "-m", "ikaros_runtime", "storage", *arguments],
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        timeout=30,
        check=False,
        env=environment,
    )
