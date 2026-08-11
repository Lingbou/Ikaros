from __future__ import annotations

from pathlib import Path

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
        store.complete_assistant_item(item_id)
        store.settle_run(prepared.run_id, "completed")
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
