from __future__ import annotations

import base64
import sqlite3
from dataclasses import replace
from pathlib import Path

import pytest

from ikaros_runtime.domain import JOURNAL_EVENT_SCHEMA_VERSION, WorkspaceSummary
from ikaros_runtime.json_codec import dumps as json_dumps
from ikaros_runtime.security import response_values_contain_protected_value
from ikaros_runtime.storage import SqliteRuntimeStore
from ikaros_runtime.storage.journal import latest_sequence as journal_latest_sequence
from ikaros_runtime.storage.thread_history import (
    TurnHistoryCursor,
    decode_turn_history_cursor,
    encode_turn_history_cursor,
)

from .helpers import prepare_turn


def _encoded_cursor(payload: object) -> str:
    encoded = json_dumps(payload, separators=(",", ":")).encode()
    return base64.urlsafe_b64encode(encoded).rstrip(b"=").decode()


def test_thread_get_reads_exact_metadata_without_writing_events(tmp_path: Path) -> None:
    database_path = tmp_path / "state.db"
    workspace_root = tmp_path / "workspace"
    workspace_root.mkdir()
    store = SqliteRuntimeStore(database_path)
    try:
        thread, created = store.create_thread(
            "Metadata",
            workspace=WorkspaceSummary("workspace-1", "Workspace", str(workspace_root)),
        )
        before = store.latest_sequence()

        metadata = store.get_thread(thread.id)

        assert metadata.to_wire() == {
            "thread": thread.to_wire(),
            "snapshotSeq": created.seq,
        }
        assert store.latest_sequence() == before
        with pytest.raises(LookupError, match="thread was not found"):
            store.get_thread("thread_missing")

        store.rebuild_projections()
        assert store.get_thread(thread.id).to_wire() == metadata.to_wire()
    finally:
        store.close()

    reopened = SqliteRuntimeStore(database_path)
    try:
        assert reopened.get_thread(thread.id).to_wire() == metadata.to_wire()
    finally:
        reopened.close()


def test_turn_list_hydrates_every_run_and_materialized_item(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("History")
        prepared = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="run the tool",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        store.mark_run_running(prepared.run_id)
        assistant_item_id, _ = store.create_assistant_item(prepared.run_id)
        store.append_text_delta(assistant_item_id, "materialized answer")
        store.complete_assistant_item(assistant_item_id, step_id="step-1")
        tool_item_id, _ = store.create_tool_call_item(
            prepared.run_id,
            step_id="step-1",
            call_id="call-1",
            tool_name="process_run",
            arguments={"command": "echo history"},
        )
        store.complete_tool_call(
            tool_item_id,
            status="completed",
            result={"ok": True, "stdout": "history", "durationMs": 1},
            result_content='{"stdout":"history"}',
        )
        store.terminalize_run(prepared.run_id, "completed")

        retry_run_id = "run_retry"
        retry_item_id = "item_retry"
        retry_timestamp = "9999-12-31T23:59:59.999Z"
        with store._connection:
            store._connection.execute(
                """
                INSERT INTO runs(
                    id, turn_id, provider_id, model_id, status, created_at, settled_at,
                    execution_policy
                ) VALUES (?, ?, 'scripted', 'scripted-v1', 'failed', ?, ?, 'full_access')
                """,
                (retry_run_id, prepared.turn_id, retry_timestamp, retry_timestamp),
            )
            config = replace(store.get_run_config(prepared.run_id), run_id=retry_run_id)
            store._connection.execute(
                "INSERT INTO run_configs(run_id, config_json) VALUES (?, ?)",
                (retry_run_id, json_dumps(config.to_wire())),
            )
            store._connection.execute(
                """
                INSERT INTO items(
                    id, turn_id, run_id, ordinal, kind, role, status, content,
                    created_at, updated_at, data_json
                ) VALUES (?, ?, ?, 1, 'message', 'assistant', 'failed', 'retry failed',
                          ?, ?, '{"reason":"test"}')
                """,
                (
                    retry_item_id,
                    prepared.turn_id,
                    retry_run_id,
                    retry_timestamp,
                    retry_timestamp,
                ),
            )

        monkeypatch.setattr(
            "ikaros_runtime.storage.thread_history._MAX_SQL_PARAMETERS",
            1,
        )
        before = store.latest_sequence()
        page = store.list_turn_page(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            cursor=None,
            limit=50,
        )
        wire = page.to_wire()

        assert page.snapshot_seq == before
        assert page.has_more is False
        assert page.next_cursor is None
        assert len(wire["turns"]) == 1
        turn = wire["turns"][0]
        assert turn["id"] == prepared.turn_id
        assert [run["id"] for run in turn["runs"]] == [prepared.run_id, retry_run_id]
        first_run, retry_run = turn["runs"]
        assert [item["kind"] for item in first_run["items"]] == [
            "message",
            "message",
            "tool_call",
            "tool_result",
        ]
        assert first_run["items"][1]["content"] == "materialized answer"
        assert first_run["items"][2]["data"]["arguments"] == {"command": "echo history"}
        assert first_run["items"][3]["data"]["result"]["stdout"] == "history"
        assert retry_run["status"] == "failed"
        assert retry_run["items"][0]["data"] == {"reason": "test"}
        assert store.latest_sequence() == before
    finally:
        store.close()


@pytest.mark.parametrize(
    ("status", "reason"),
    [
        ("completed", None),
        ("failed", "provider_unavailable"),
        ("failed", "runtime_interrupted"),
        ("cancelled", "cancelled"),
    ],
)
def test_run_failure_reason_survives_history_restart_and_rebuild(
    tmp_path: Path,
    status: str,
    reason: str | None,
) -> None:
    database_path = tmp_path / "state.db"
    store = SqliteRuntimeStore(database_path)
    try:
        thread, _ = store.create_thread("Failure history")
        prepared = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="inspect this project",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        queued = store.list_turn_page(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            cursor=None,
            limit=50,
        ).to_wire()["turns"][0]["runs"][0]
        assert queued["reasonCode"] is None
        store.mark_run_running(prepared.run_id)
        events = store.terminalize_run(prepared.run_id, status, reason_code=reason)
        settled = next(event for event in events if event.type == "run.settled")
        assert settled.payload.get("reasonCode") == reason
    finally:
        store.close()

    reopened = SqliteRuntimeStore(database_path)
    try:
        for rebuild in (False, True):
            if rebuild:
                reopened.rebuild_projections()
            run = reopened.list_turn_page(
                thread_id=thread.id,
                branch_id=thread.default_branch_id,
                cursor=None,
                limit=50,
            ).to_wire()["turns"][0]["runs"][0]
            assert run["status"] == status
            assert run["reasonCode"] == reason
    finally:
        reopened.close()


def test_turn_history_pages_latest_first_but_each_page_is_chronological(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Pagination")
        for ordinal in range(1, 7):
            prepare_turn(
                store,
                thread_id=thread.id,
                branch_id=thread.default_branch_id,
                content=f"turn {ordinal}",
                provider_id="scripted",
                model_id="scripted-v1",
            )

        first = store.list_turn_page(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            cursor=None,
            limit=2,
        )
        assert [turn.ordinal for turn in first.turns] == [5, 6]
        assert first.has_more is True
        assert first.next_cursor is not None
        first_snapshot = first.snapshot_seq

        prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="arrived after the first page",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        second = store.list_turn_page(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            cursor=first.next_cursor,
            limit=2,
        )
        third = store.list_turn_page(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            cursor=second.next_cursor,
            limit=2,
        )

        assert [turn.ordinal for turn in second.turns] == [3, 4]
        assert [turn.ordinal for turn in third.turns] == [1, 2]
        assert second.snapshot_seq > first_snapshot
        assert third.has_more is False
        assert third.next_cursor is None
        loaded = [turn.ordinal for page in (first, second, third) for turn in page.turns]
        assert sorted(loaded) == [1, 2, 3, 4, 5, 6]
        assert len(loaded) == len(set(loaded))
    finally:
        store.close()


def test_turn_history_empty_page_and_cursor_scope_validation(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Empty")
        empty = store.list_turn_page(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            cursor=None,
            limit=50,
        )
        assert empty.turns == ()
        assert empty.has_more is False
        assert empty.next_cursor is None

        cursor = encode_turn_history_cursor(
            TurnHistoryCursor(
                thread_id=thread.id,
                branch_id=thread.default_branch_id,
                ordinal=1,
            )
        )
        assert (
            decode_turn_history_cursor(
                cursor,
                thread_id=thread.id,
                branch_id=thread.default_branch_id,
            ).ordinal
            == 1
        )
        with pytest.raises(ValueError, match="turn.list cursor is invalid"):
            decode_turn_history_cursor(
                cursor,
                thread_id="thread_other",
                branch_id=thread.default_branch_id,
            )
        with pytest.raises(ValueError, match="turn.list cursor is invalid"):
            decode_turn_history_cursor(
                _encoded_cursor(
                    {
                        "v": 2,
                        "threadId": thread.id,
                        "branchId": thread.default_branch_id,
                        "ordinal": 1,
                    }
                ),
                thread_id=thread.id,
                branch_id=thread.default_branch_id,
            )
        with pytest.raises(ValueError, match="turn.list cursor is invalid") as raised:
            decode_turn_history_cursor(
                cursor + "=",
                thread_id=thread.id,
                branch_id=thread.default_branch_id,
            )
        assert cursor not in str(raised.value)

        with pytest.raises(LookupError, match="thread or branch was not found"):
            store.list_turn_page(
                thread_id=thread.id,
                branch_id="branch_missing",
                cursor=None,
                limit=50,
            )
    finally:
        store.close()


def test_turn_page_and_watermark_share_one_sqlite_snapshot(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    database_path = tmp_path / "state.db"
    store = SqliteRuntimeStore(database_path)
    writer = sqlite3.connect(database_path)
    try:
        thread, _ = store.create_thread("Snapshot")
        original = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="original",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        real_latest_sequence = journal_latest_sequence
        inserted = False

        def interleaved_latest_sequence(connection: sqlite3.Connection) -> int:
            nonlocal inserted
            snapshot_seq = real_latest_sequence(connection)
            if not inserted:
                inserted = True
                timestamp = "9999-12-31T23:59:59.999Z"
                with writer:
                    writer.execute(
                        """
                        INSERT INTO turns(
                            id, thread_id, branch_id, ordinal, status, created_at, updated_at
                        ) VALUES ('turn_interleaved', ?, ?, 2, 'queued', ?, ?)
                        """,
                        (thread.id, thread.default_branch_id, timestamp, timestamp),
                    )
                    writer.execute(
                        """
                        INSERT INTO runs(
                            id, turn_id, provider_id, model_id, status, created_at,
                            execution_policy
                        ) VALUES (
                            'run_interleaved', 'turn_interleaved', 'scripted',
                            'scripted-v1', 'queued', ?, 'full_access'
                        )
                        """,
                        (timestamp,),
                    )
                    writer.execute(
                        """
                        INSERT INTO items(
                            id, turn_id, run_id, ordinal, kind, role, status, content,
                            created_at, updated_at
                        ) VALUES (
                            'item_interleaved', 'turn_interleaved', 'run_interleaved',
                            1, 'message', 'user', 'completed', 'interleaved', ?, ?
                        )
                        """,
                        (timestamp, timestamp),
                    )
                    writer.execute(
                        """
                        INSERT INTO events(
                            schema_version, event_type, thread_id, branch_id, turn_id,
                            run_id, item_id, created_at, payload_json
                        ) VALUES (?, 'item.completed', ?, ?, 'turn_interleaved',
                                  'run_interleaved', 'item_interleaved', ?, '{}')
                        """,
                        (
                            JOURNAL_EVENT_SCHEMA_VERSION,
                            thread.id,
                            thread.default_branch_id,
                            timestamp,
                        ),
                    )
            return snapshot_seq

        monkeypatch.setattr(
            "ikaros_runtime.storage.thread_history.latest_sequence",
            interleaved_latest_sequence,
        )
        page = store.list_turn_page(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            cursor=None,
            limit=50,
        )

        assert [turn.id for turn in page.turns] == [original.turn_id]
        assert page.snapshot_seq == 3
        monkeypatch.setattr(
            "ikaros_runtime.storage.thread_history.latest_sequence",
            real_latest_sequence,
        )
        refreshed = store.list_turn_page(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            cursor=None,
            limit=50,
        )
        assert [turn.id for turn in refreshed.turns] == [
            original.turn_id,
            "turn_interleaved",
        ]
        assert refreshed.snapshot_seq == 4
    finally:
        writer.close()
        store.close()


def test_turn_history_queries_use_projection_indexes(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Query plan")
        prepared = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="indexed",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        turn_plan = store._connection.execute(
            """
            EXPLAIN QUERY PLAN
            SELECT id, thread_id, branch_id, ordinal, status, created_at, updated_at
            FROM turns
            WHERE thread_id = ? AND branch_id = ? AND ordinal < ?
            ORDER BY ordinal DESC
            LIMIT 51
            """,
            (thread.id, thread.default_branch_id, 100),
        ).fetchall()
        run_plan = store._connection.execute(
            """
            EXPLAIN QUERY PLAN
            SELECT id, turn_id, provider_id, model_id, execution_policy, status,
                   created_at, settled_at
            FROM runs
            WHERE turn_id IN (?)
            ORDER BY turn_id ASC, created_at ASC, id ASC
            """,
            (prepared.turn_id,),
        ).fetchall()
        item_plan = store._connection.execute(
            """
            EXPLAIN QUERY PLAN
            SELECT id, turn_id, run_id, ordinal, kind, role, status, content,
                   data_json, created_at, updated_at
            FROM items
            WHERE run_id IN (?)
            ORDER BY run_id ASC, ordinal ASC, id ASC
            """,
            (prepared.run_id,),
        ).fetchall()

        assert any("SEARCH" in str(row["detail"]) for row in turn_plan)
        assert any("runs_turn_history_idx" in str(row["detail"]) for row in run_plan)
        assert any("SEARCH" in str(row["detail"]) for row in item_plan)
    finally:
        store.close()


@pytest.mark.parametrize("protected", ["runs", "items"])
def test_history_collection_keys_have_fixed_security_provenance(protected: str) -> None:
    assert (
        response_values_contain_protected_value(
            {"turns": [{"runs": [{"items": []}]}]},
            [protected],
        )
        is False
    )
    assert (
        response_values_contain_protected_value(
            {"turns": [{"runs": [{"items": [{"content": protected}]}]}]},
            [protected],
        )
        is True
    )
