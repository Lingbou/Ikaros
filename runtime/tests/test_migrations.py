from __future__ import annotations

import sqlite3
from pathlib import Path

import pytest

import ikaros_runtime.storage as storage_module
from ikaros_runtime.storage import SqliteRuntimeStore


def _create_v1_database(database_path: Path) -> None:
    connection = sqlite3.connect(database_path)
    try:
        connection.executescript(
            f"{storage_module._INITIAL_SCHEMA}\nPRAGMA user_version = 1;"
        )
    finally:
        connection.close()


def test_v1_database_is_upgraded_to_v2(tmp_path: Path) -> None:
    database_path = tmp_path / "state.db"
    _create_v1_database(database_path)

    store = SqliteRuntimeStore(database_path)
    try:
        version = int(store._connection.execute("PRAGMA user_version").fetchone()[0])
        event_columns = {
            row["name"]
            for row in store._connection.execute("PRAGMA table_info(events)").fetchall()
        }
        tables = {
            row["name"]
            for row in store._connection.execute(
                "SELECT name FROM sqlite_master WHERE type = 'table'"
            ).fetchall()
        }

        assert version == 2
        assert {"turn_id", "run_id", "item_id"} <= event_columns
        assert {"turns", "runs", "items"} <= tables
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
