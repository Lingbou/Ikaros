from __future__ import annotations

import sqlite3
from pathlib import Path

import pytest

from ikaros_runtime.storage import SqliteRuntimeStore
from ikaros_runtime.storage import schema as storage_schema


def test_fresh_database_creates_one_canonical_schema(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        version = int(store._connection.execute("PRAGMA user_version").fetchone()[0])
        tables = {
            row["name"]
            for row in store._connection.execute(
                "SELECT name FROM sqlite_master WHERE type = 'table'"
            ).fetchall()
        }
        event_columns = {
            row["name"] for row in store._connection.execute("PRAGMA table_info(events)")
        }
        thread_columns = {
            row["name"] for row in store._connection.execute("PRAGMA table_info(threads)")
        }
        run_columns = {
            row["name"] for row in store._connection.execute("PRAGMA table_info(runs)")
        }
        usage_columns = {
            row["name"]
            for row in store._connection.execute("PRAGMA table_info(model_usages)")
        }
        run_input_columns = {
            row["name"]
            for row in store._connection.execute("PRAGMA table_info(run_inputs)")
        }
        model_step_columns = {
            row["name"]
            for row in store._connection.execute("PRAGMA table_info(model_steps)")
        }
        item_columns = {
            row["name"] for row in store._connection.execute("PRAGMA table_info(items)")
        }
        indexes = {
            row["name"]
            for row in store._connection.execute(
                "SELECT name FROM sqlite_master WHERE type = 'index'"
            ).fetchall()
        }

        assert version == storage_schema.SCHEMA_VERSION
        assert {
            "events",
            "threads",
            "branches",
            "turns",
            "runs",
            "run_inputs",
            "model_steps",
            "items",
            "model_usages",
        } <= tables
        assert {"turn_id", "run_id", "item_id", "schema_version"} <= event_columns
        assert {"client_request_id", "workspace_json", "archived_at"} <= thread_columns
        assert {
            "client_request_id",
            "execution_policy",
            "started_at",
            "reason_code",
        } <= run_columns
        assert {
            "thread_id",
            "turn_id",
            "run_id",
            "step_ordinal",
            "provider_id",
            "model_id",
            "activity_date",
            "completed_at",
            "total_tokens",
        } <= usage_columns
        assert {
            "run_id",
            "submission_frame_json",
            "run_manifest_json",
            "context_snapshot_json",
        } == run_input_columns
        assert {
            "run_id",
            "step_ordinal",
            "step_manifest_json",
            "prepared_at",
            "outcome",
            "reason_code",
            "response_model_id",
            "request_id",
            "usage_json",
            "activity_date",
            "finished_at",
        } == model_step_columns
        assert "data_json" in item_columns
        assert {
            "events_thread_seq_idx",
            "events_run_seq_idx",
            "events_one_settled_per_run",
            "threads_client_request_id_idx",
            "threads_active_catalog_order_idx",
            "threads_archived_catalog_order_idx",
            "branches_default_thread_idx",
            "runs_client_request_id_idx",
            "runs_turn_history_idx",
            "items_turn_context_idx",
            "model_usages_completed_at_idx",
            "model_usages_activity_date_idx",
        } <= indexes
    finally:
        store.close()


def test_current_database_reopens_without_rewriting_state(tmp_path: Path) -> None:
    database_path = tmp_path / "state.db"
    first = SqliteRuntimeStore(database_path)
    try:
        expected = first.create_thread("Persistent")
    finally:
        first.close()

    reopened = SqliteRuntimeStore(database_path)
    try:
        page = reopened.list_thread_page(cursor=None, limit=50)
        assert page.threads == (expected[0],)
        assert page.snapshot_seq == expected[1].seq
        assert (
            int(reopened._connection.execute("PRAGMA user_version").fetchone()[0])
            == storage_schema.SCHEMA_VERSION
        )
    finally:
        reopened.close()


@pytest.mark.parametrize("version", [1, 2, 3, 4, 5, 6, 7, 999])
def test_non_current_schema_version_requires_explicit_reset(
    tmp_path: Path,
    version: int,
) -> None:
    database_path = tmp_path / "state.db"
    connection = sqlite3.connect(database_path)
    try:
        connection.execute("CREATE TABLE sentinel(value TEXT NOT NULL)")
        connection.execute("INSERT INTO sentinel(value) VALUES ('preserve-me')")
        connection.execute(f"PRAGMA user_version = {version}")
        connection.commit()
    finally:
        connection.close()

    with pytest.raises(RuntimeError, match="incompatible; reset required"):
        SqliteRuntimeStore(database_path)

    connection = sqlite3.connect(database_path)
    try:
        assert int(connection.execute("PRAGMA user_version").fetchone()[0]) == version
        assert connection.execute("SELECT value FROM sentinel").fetchone()[0] == "preserve-me"
    finally:
        connection.close()


def test_unversioned_nonempty_database_requires_explicit_reset(tmp_path: Path) -> None:
    database_path = tmp_path / "state.db"
    connection = sqlite3.connect(database_path)
    try:
        connection.execute("CREATE TABLE sentinel(value TEXT NOT NULL)")
        connection.execute("INSERT INTO sentinel(value) VALUES ('preserve-me')")
        connection.commit()
    finally:
        connection.close()

    with pytest.raises(RuntimeError, match="incompatible; reset required"):
        SqliteRuntimeStore(database_path)

    connection = sqlite3.connect(database_path)
    try:
        assert int(connection.execute("PRAGMA user_version").fetchone()[0]) == 0
        assert connection.execute("SELECT value FROM sentinel").fetchone()[0] == "preserve-me"
    finally:
        connection.close()


def test_failed_fresh_schema_creation_rolls_back_everything(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    database_path = tmp_path / "state.db"
    monkeypatch.setattr(
        storage_schema,
        "_CANONICAL_SCHEMA",
        """
        CREATE TABLE schema_probe(id INTEGER PRIMARY KEY);
        THIS IS NOT VALID SQL;
        """,
    )

    with pytest.raises(sqlite3.OperationalError):
        SqliteRuntimeStore(database_path)

    connection = sqlite3.connect(database_path)
    try:
        version = int(connection.execute("PRAGMA user_version").fetchone()[0])
        probe = connection.execute(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_probe'"
        ).fetchone()

        assert version == 0
        assert probe is None
    finally:
        connection.close()
