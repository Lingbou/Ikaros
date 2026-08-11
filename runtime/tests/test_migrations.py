from __future__ import annotations

import sqlite3
from pathlib import Path

import pytest

import ikaros_runtime.storage as storage_module
from ikaros_runtime.storage import SqliteRuntimeStore


def _create_v1_database(database_path: Path) -> None:
    connection = sqlite3.connect(database_path)
    try:
        connection.executescript(f"{storage_module._INITIAL_SCHEMA}\nPRAGMA user_version = 1;")
    finally:
        connection.close()


def _create_v2_database(database_path: Path) -> None:
    _create_v1_database(database_path)
    connection = sqlite3.connect(database_path)
    try:
        connection.executescript(f"{storage_module._MIGRATION_2}\nPRAGMA user_version = 2;")
    finally:
        connection.close()


def _create_v3_database(database_path: Path) -> None:
    _create_v2_database(database_path)
    connection = sqlite3.connect(database_path)
    try:
        connection.executescript(f"{storage_module._MIGRATION_3}\nPRAGMA user_version = 3;")
    finally:
        connection.close()


def _create_v4_database(database_path: Path) -> None:
    _create_v3_database(database_path)
    connection = sqlite3.connect(database_path)
    try:
        connection.executescript(f"{storage_module._MIGRATION_4}\nPRAGMA user_version = 4;")
    finally:
        connection.close()


def test_v1_database_is_upgraded_to_v5(tmp_path: Path) -> None:
    database_path = tmp_path / "state.db"
    _create_v1_database(database_path)

    store = SqliteRuntimeStore(database_path)
    try:
        version = int(store._connection.execute("PRAGMA user_version").fetchone()[0])
        event_columns = {
            row["name"] for row in store._connection.execute("PRAGMA table_info(events)").fetchall()
        }
        tables = {
            row["name"]
            for row in store._connection.execute(
                "SELECT name FROM sqlite_master WHERE type = 'table'"
            ).fetchall()
        }
        event_indexes = {
            row["name"] for row in store._connection.execute("PRAGMA index_list(events)").fetchall()
        }
        thread_columns = {
            row["name"]
            for row in store._connection.execute("PRAGMA table_info(threads)").fetchall()
        }
        run_columns = {
            row["name"] for row in store._connection.execute("PRAGMA table_info(runs)").fetchall()
        }

        item_columns = {
            row["name"] for row in store._connection.execute("PRAGMA table_info(items)").fetchall()
        }

        assert version == 5
        assert {"turn_id", "run_id", "item_id"} <= event_columns
        assert {"turns", "runs", "items"} <= tables
        assert "events_one_settled_per_run" in event_indexes
        assert "client_request_id" in thread_columns
        assert "client_request_id" in run_columns
        assert "execution_policy" in run_columns
        assert "data_json" in item_columns
    finally:
        store.close()


def test_failed_migration_rolls_back_all_schema_changes(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    database_path = tmp_path / "state.db"
    _create_v1_database(database_path)
    monkeypatch.setattr(
        storage_module,
        "_MIGRATION_2",
        """
        ALTER TABLE events ADD COLUMN turn_id TEXT;
        CREATE TABLE migration_probe(id INTEGER PRIMARY KEY);
        THIS IS NOT VALID SQL;
        """,
    )

    with pytest.raises(sqlite3.OperationalError):
        SqliteRuntimeStore(database_path)

    connection = sqlite3.connect(database_path)
    connection.row_factory = sqlite3.Row
    try:
        version = int(connection.execute("PRAGMA user_version").fetchone()[0])
        event_columns = {
            row["name"] for row in connection.execute("PRAGMA table_info(events)").fetchall()
        }
        probe = connection.execute(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'migration_probe'"
        ).fetchone()

        assert version == 1
        assert "turn_id" not in event_columns
        assert probe is None
    finally:
        connection.close()


def test_failed_v3_migration_rolls_back_index_and_version(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    database_path = tmp_path / "state.db"
    _create_v2_database(database_path)
    monkeypatch.setattr(
        storage_module,
        "_MIGRATION_3",
        """
        CREATE UNIQUE INDEX migration_probe ON events(run_id);
        THIS IS NOT VALID SQL;
        """,
    )

    with pytest.raises(sqlite3.OperationalError):
        SqliteRuntimeStore(database_path)

    connection = sqlite3.connect(database_path)
    connection.row_factory = sqlite3.Row
    try:
        version = int(connection.execute("PRAGMA user_version").fetchone()[0])
        indexes = {
            row["name"] for row in connection.execute("PRAGMA index_list(events)").fetchall()
        }
        assert version == 2
        assert "migration_probe" not in indexes
    finally:
        connection.close()


def test_failed_v4_migration_rolls_back_columns_and_version(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    database_path = tmp_path / "state.db"
    _create_v3_database(database_path)
    monkeypatch.setattr(
        storage_module,
        "_MIGRATION_4",
        """
        ALTER TABLE threads ADD COLUMN client_request_id TEXT;
        THIS IS NOT VALID SQL;
        """,
    )

    with pytest.raises(sqlite3.OperationalError):
        SqliteRuntimeStore(database_path)

    connection = sqlite3.connect(database_path)
    connection.row_factory = sqlite3.Row
    try:
        version = int(connection.execute("PRAGMA user_version").fetchone()[0])
        thread_columns = {
            row["name"] for row in connection.execute("PRAGMA table_info(threads)").fetchall()
        }
        assert version == 3
        assert "client_request_id" not in thread_columns
    finally:
        connection.close()


def test_failed_v5_migration_rolls_back_columns_and_version(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    database_path = tmp_path / "state.db"
    _create_v4_database(database_path)
    monkeypatch.setattr(
        storage_module,
        "_MIGRATION_5",
        """
        ALTER TABLE runs ADD COLUMN execution_policy TEXT NOT NULL DEFAULT 'full_access';
        THIS IS NOT VALID SQL;
        """,
    )

    with pytest.raises(sqlite3.OperationalError):
        SqliteRuntimeStore(database_path)

    connection = sqlite3.connect(database_path)
    connection.row_factory = sqlite3.Row
    try:
        version = int(connection.execute("PRAGMA user_version").fetchone()[0])
        run_columns = {
            row["name"] for row in connection.execute("PRAGMA table_info(runs)").fetchall()
        }
        assert version == 4
        assert "execution_policy" not in run_columns
    finally:
        connection.close()
