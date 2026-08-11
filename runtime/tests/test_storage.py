from __future__ import annotations

import sqlite3
from pathlib import Path

import pytest

from ikaros_runtime.storage import SqliteRuntimeStore


def test_projections_can_be_rebuilt_from_the_event_journal(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        expected, event = store.create_thread("Rebuild me")
        assert event.seq == 1

        store.rebuild_projections()

        assert store.list_threads() == [expected]
        replayed, latest_seq = store.replay_events(0, 100)
        assert replayed == [event]
        assert latest_seq == 1
    finally:
        store.close()


def test_client_request_ids_make_thread_and_turn_creation_idempotent(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, created_event, thread_created = store.create_thread_once(
            "Exactly once",
            "create-request",
        )
        repeated_thread, repeated_event, repeated_created = store.create_thread_once(
            "Exactly once",
            "create-request",
        )

        assert thread_created is True
        assert repeated_created is False
        assert repeated_thread == thread
        assert repeated_event == created_event

        prepared = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="only one turn",
            provider_id="scripted",
            model_id="scripted-v1",
            client_request_id="turn-request",
        )
        repeated = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="only one turn",
            provider_id="scripted",
            model_id="scripted-v1",
            client_request_id="turn-request",
        )

        assert prepared.newly_created is True
        assert repeated.newly_created is False
        assert repeated.run_id == prepared.run_id
        assert repeated.turn_id == prepared.turn_id
        assert repeated.initial_events == ()
        assert store._connection.execute("SELECT COUNT(*) FROM threads").fetchone()[0] == 1
        assert store._connection.execute("SELECT COUNT(*) FROM turns").fetchone()[0] == 1
        assert store._connection.execute("SELECT COUNT(*) FROM runs").fetchone()[0] == 1

        store.rebuild_projections()
        rebuilt_thread, _, rebuilt_created = store.create_thread_once(
            "Exactly once",
            "create-request",
        )
        rebuilt_turn = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="only one turn",
            provider_id="scripted",
            model_id="scripted-v1",
            client_request_id="turn-request",
        )
        assert rebuilt_thread.id == thread.id
        assert rebuilt_thread.title == thread.title
        assert rebuilt_thread.default_branch_id == thread.default_branch_id
        assert rebuilt_created is False
        assert rebuilt_turn.run_id == prepared.run_id
        assert rebuilt_turn.newly_created is False

        with pytest.raises(LookupError, match="different thread.create parameters"):
            store.create_thread_once("Different title", "create-request")
        with pytest.raises(LookupError, match="different turn.start parameters"):
            store.prepare_turn(
                thread_id=thread.id,
                branch_id=thread.default_branch_id,
                content="different content",
                provider_id="scripted",
                model_id="scripted-v1",
                client_request_id="turn-request",
            )
    finally:
        store.close()


def test_agent_projections_rebuild_without_rewriting_the_journal(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Agent history")
        prepared = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="hello",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        store.mark_run_running(prepared.run_id)
        item_id, _ = store.create_assistant_item(prepared.run_id)
        store.append_text_delta(item_id, "world")
        terminal_events = store.terminalize_run(prepared.run_id, "completed")
        assert [event.type for event in terminal_events] == ["item.completed", "run.settled"]
        before, latest_seq = store.replay_events(0, 100)

        store.rebuild_projections()

        after, rebuilt_latest_seq = store.replay_events(0, 100)
        assert after == before
        assert rebuilt_latest_seq == latest_seq
        assert store.context_messages(
            thread.default_branch_id,
            through_turn_id=prepared.turn_id,
        ) == [
            ("user", "hello"),
            ("assistant", "world"),
        ]
    finally:
        store.close()


def test_terminalize_run_is_atomic_idempotent_and_uniquely_settled(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Atomic terminal state")
        prepared = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="keep partial output",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        store.mark_run_running(prepared.run_id)
        item_id, _ = store.create_assistant_item(prepared.run_id)
        store.append_text_delta(item_id, "partial")
        store._connection.execute(
            """
            CREATE TRIGGER abort_run_settled
            BEFORE INSERT ON events
            WHEN NEW.event_type = 'run.settled'
            BEGIN
                SELECT RAISE(ABORT, 'settled insert failed');
            END;
            """
        )

        with pytest.raises(sqlite3.IntegrityError, match="settled insert failed"):
            store.terminalize_run(prepared.run_id, "cancelled")

        item = store._connection.execute(
            "SELECT status, content FROM items WHERE id = ?", (item_id,)
        ).fetchone()
        assert store.run_status(prepared.run_id) == "running"
        assert dict(item) == {"status": "streaming", "content": "partial"}
        replayed, _ = store.replay_events(0, 100)
        assert not any(
            event.run_id == prepared.run_id
            and (
                event.type == "run.settled"
                or (
                    event.type == "item.completed"
                    and event.payload.get("item", {}).get("role") == "assistant"
                )
            )
            for event in replayed
        )

        store._connection.execute("DROP TRIGGER abort_run_settled")
        events = store.terminalize_run(prepared.run_id, "cancelled")
        assert [event.type for event in events] == ["item.completed", "run.settled"]
        assert events[0].payload["item"]["status"] == "cancelled"
        assert events[0].payload["item"]["content"] == "partial"
        assert events[1].payload["status"] == "cancelled"
        assert store.terminalize_run(prepared.run_id, "cancelled") == ()

        with pytest.raises(sqlite3.IntegrityError):
            store._connection.execute(
                """
                INSERT INTO events(event_type, run_id, created_at, payload_json)
                VALUES ('run.settled', ?, '2026-08-11T00:00:00.000Z', '{}')
                """,
                (prepared.run_id,),
            )
        store._connection.rollback()
    finally:
        store.close()


def test_recovery_fails_running_runs_and_orders_queued_runs_by_event_seq(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Recovery")
        running = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="running",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        store.mark_run_running(running.run_id)
        item_id, _ = store.create_assistant_item(running.run_id)
        store.append_text_delta(item_id, "preserved partial")
        first_queued = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="first queued",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        second_queued = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="second queued",
            provider_id="scripted",
            model_id="scripted-v1",
        )

        recovery = store.recover_incomplete_runs()

        assert recovery.queued_run_ids == (first_queued.run_id, second_queued.run_id)
        assert store.run_status(running.run_id) == "failed"
        assert store.run_status(first_queued.run_id) == "queued"
        replayed, _ = store.replay_events(0, 100)
        running_terminal = [
            event
            for event in replayed
            if event.run_id == running.run_id
            and (
                event.type == "run.settled"
                or (
                    event.type == "item.completed"
                    and event.payload.get("item", {}).get("role") == "assistant"
                )
            )
        ]
        assert [event.type for event in running_terminal] == [
            "item.completed",
            "run.settled",
        ]
        assert running_terminal[0].payload["item"]["content"] == "preserved partial"
        assert running_terminal[0].payload["item"]["status"] == "failed"
        assert running_terminal[1].payload["reasonCode"] == "runtime_interrupted"
    finally:
        store.close()


def test_context_does_not_include_later_queued_turns(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Queued context")
        first = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="alpha",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        second = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="beta",
            provider_id="scripted",
            model_id="scripted-v1",
        )

        assert store.context_messages(
            thread.default_branch_id,
            through_turn_id=first.turn_id,
        ) == [("user", "alpha")]
        assert store.context_messages(
            thread.default_branch_id,
            through_turn_id=second.turn_id,
        ) == [("user", "alpha"), ("user", "beta")]
    finally:
        store.close()
