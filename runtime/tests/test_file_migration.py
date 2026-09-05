from __future__ import annotations

import json
import sqlite3
from pathlib import Path

import pytest

from ikaros_runtime.errors import UnsupportedJournalEventVersionError
from ikaros_runtime.storage import SqliteRuntimeStore, maintenance, schema
from ikaros_runtime.storage.journal import projection_events, replay_events

_SCHEMA_8 = Path(__file__).parent / "fixtures" / "schema_v8.sql"
_TIMESTAMP = "2026-09-05T00:00:00.000Z"


def _database8(path: Path) -> sqlite3.Connection:
    connection = sqlite3.connect(path)
    connection.row_factory = sqlite3.Row
    connection.execute("PRAGMA foreign_keys = ON")
    connection.execute("PRAGMA journal_mode = WAL")
    connection.execute("PRAGMA wal_autocheckpoint = 0")
    connection.executescript(_SCHEMA_8.read_text())
    return connection


def _thread(connection: sqlite3.Connection, suffix: str, version: int = 5) -> None:
    thread = {
        "id": f"thread-{suffix}", "title": suffix, "defaultBranchId": f"branch-{suffix}",
        "workspace": None, "createdAt": _TIMESTAMP, "updatedAt": _TIMESTAMP,
        "archivedAt": None,
    }
    branch = {
        "id": f"branch-{suffix}", "threadId": f"thread-{suffix}",
        "createdAt": _TIMESTAMP, "isDefault": True,
    }
    with connection:
        connection.execute(
            "INSERT INTO threads(id,title,default_branch_id,created_at,updated_at) "
            "VALUES (?,?,?,?,?)",
            (thread["id"], suffix, branch["id"], _TIMESTAMP, _TIMESTAMP),
        )
        connection.execute(
            "INSERT INTO branches(id,thread_id,created_at,is_default) VALUES (?,?,?,1)",
            (branch["id"], thread["id"], _TIMESTAMP),
        )
        connection.execute(
            "INSERT INTO events(schema_version,event_type,thread_id,branch_id,created_at,"
            "payload_json) VALUES (?,'thread.created',?,?,?,?)",
            (version, thread["id"], branch["id"], _TIMESTAMP,
             json.dumps({"thread": thread, "branch": branch})),
        )


def _journal(connection: sqlite3.Connection) -> list[tuple[object, ...]]:
    return [tuple(row) for row in connection.execute("SELECT * FROM events ORDER BY seq")]


def _has_changes(connection: sqlite3.Connection) -> bool:
    return connection.execute(
        "SELECT 1 FROM sqlite_master WHERE name='file_changes'"
    ).fetchone() is not None


def test_schema8_migration_backs_up_wal_and_preserves_state_and_other_files(tmp_path: Path) -> None:
    database = tmp_path / "state.db"
    connection = _database8(database)
    config = tmp_path / "config.yaml"
    memory = tmp_path / "memory.db"
    config.write_bytes(b"version: 1\nproviders: {}\n")
    with sqlite3.connect(memory) as memory_connection:
        memory_connection.execute("CREATE TABLE retained(value TEXT)")
        memory_connection.execute("INSERT INTO retained VALUES ('memory remains')")
    config_before, memory_before = config.read_bytes(), memory.read_bytes()
    try:
        _thread(connection, "persisted")
        before = _journal(connection)
        assert Path(f"{database}-wal").stat().st_size > 0
        assert not _has_changes(connection)

        migrated = SqliteRuntimeStore(database)
        migrated.close()

        assert connection.execute("PRAGMA user_version").fetchone()[0] == 9
        assert _has_changes(connection)
        assert _journal(connection) == before
        assert connection.execute("SELECT title FROM threads").fetchone()[0] == "persisted"
        assert connection.execute("PRAGMA foreign_key_check").fetchall() == []
        backups = list((tmp_path / "backups").glob("*.db"))
        assert len(backups) == 1
        with sqlite3.connect(backups[0]) as backup:
            assert backup.execute("PRAGMA user_version").fetchone()[0] == 8
            assert backup.execute("PRAGMA journal_mode").fetchone()[0] == "delete"
            assert backup.execute("PRAGMA quick_check").fetchone()[0] == "ok"
            assert _journal(backup) == before
            assert not _has_changes(backup)
        assert config.read_bytes() == config_before
        assert memory.read_bytes() == memory_before

        schema.initialize_schema(connection)
        assert list((tmp_path / "backups").glob("*.db")) == backups
    finally:
        connection.close()

    reopened = SqliteRuntimeStore(database)
    try:
        assert reopened.list_thread_page(cursor=None, limit=50).threads[0].title == "persisted"
        assert reopened.check_state().thread_count == 1
    finally:
        reopened.close()


def test_migration_backup_failure_leaves_schema8_untouched(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch,
) -> None:
    connection = _database8(tmp_path / "state.db")
    try:
        _thread(connection, "preserved")
        before = _journal(connection)

        def fail_publish(_source: Path, _destination: Path) -> None:
            raise OSError("backup publication failed")

        monkeypatch.setattr(maintenance, "_publish_new_file", fail_publish)
        with pytest.raises(OSError, match="backup publication failed"):
            schema.initialize_schema(connection)
        assert connection.execute("PRAGMA user_version").fetchone()[0] == 8
        assert not _has_changes(connection)
        assert _journal(connection) == before
        assert list((tmp_path / "backups").iterdir()) == []
    finally:
        connection.close()


def test_migration_transaction_failure_rolls_back_new_table_but_keeps_backup(
    tmp_path: Path,
) -> None:
    connection = _database8(tmp_path / "state.db")
    try:
        _thread(connection, "preserved")
        before = _journal(connection)

        def deny_version_write(
            action: int, first: str | None, second: str | None,
            _database: str | None, _trigger: str | None,
        ) -> int:
            if action == sqlite3.SQLITE_PRAGMA and first == "user_version" and second == "9":
                return sqlite3.SQLITE_DENY
            return sqlite3.SQLITE_OK

        connection.set_authorizer(deny_version_write)
        with pytest.raises(RuntimeError, match="backup preserved"):
            schema.initialize_schema(connection)
        connection.set_authorizer(None)
        assert not connection.in_transaction
        assert connection.execute("PRAGMA user_version").fetchone()[0] == 8
        assert not _has_changes(connection)
        assert _journal(connection) == before
        backups = list((tmp_path / "backups").glob("*.db"))
        assert len(backups) == 1
        with sqlite3.connect(backups[0]) as backup:
            assert backup.execute("PRAGMA user_version").fetchone()[0] == 8
            assert _journal(backup) == before
    finally:
        connection.close()


def test_normal_checks_do_not_silently_accept_old_schema(tmp_path: Path) -> None:
    database = tmp_path / "state.db"
    connection = _database8(database)
    try:
        _thread(connection, "old")
        with pytest.raises(RuntimeError, match="does not match 9"):
            maintenance.check_state(connection, database)
        with pytest.raises(RuntimeError, match="does not match 9"):
            maintenance.create_state_backup(connection, database)
        with pytest.raises(RuntimeError, match="incompatible"):
            SqliteRuntimeStore.open_existing(database, read_only=True)
    finally:
        connection.close()


def test_journal_accepts_mixed_schema5_and6_without_rewriting(tmp_path: Path) -> None:
    connection = _database8(tmp_path / "state.db")
    try:
        _thread(connection, "old", 5)
        schema.initialize_schema(connection)
        _thread(connection, "new", 6)
        before = _journal(connection)
        events = projection_events(connection)
        assert [event.schema_version for event in events] == [5, 6]
        assert [event.to_wire()["schemaVersion"] for event in events] == [5, 6]
        maintenance.rebuild_projection_tables(connection)
        assert _journal(connection) == before
        assert maintenance.check_state(connection, tmp_path / "state.db").thread_count == 2
    finally:
        connection.close()


@pytest.mark.parametrize("version", [4, 7])
def test_journal_rejects_unsupported_versions(tmp_path: Path, version: int) -> None:
    connection = _database8(tmp_path / "state.db")
    try:
        _thread(connection, "unsupported", version)
        with pytest.raises(UnsupportedJournalEventVersionError, match="supported versions"):
            replay_events(connection, 0, 10)
    finally:
        connection.close()


def test_file_change_event_requires_schema6(tmp_path: Path) -> None:
    connection = _database8(tmp_path / "state.db")
    try:
        _thread(connection, "wrong-file-version", 5)
        with connection:
            connection.execute("UPDATE events SET event_type = 'file.change_recorded'")
        with pytest.raises(UnsupportedJournalEventVersionError, match="requires.*6"):
            replay_events(connection, 0, 10)
        with connection:
            connection.execute("UPDATE events SET schema_version = 6")
        assert replay_events(connection, 0, 10)[0][0].type == "file.change_recorded"
    finally:
        connection.close()
