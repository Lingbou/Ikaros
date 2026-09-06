from __future__ import annotations

from collections.abc import AsyncIterator
from pathlib import Path

import pytest

from ikaros_runtime.agent.history import HistorySelector
from ikaros_runtime.agent.loop import AgentLoop
from ikaros_runtime.cancellation import CancellationToken
from ikaros_runtime.domain import JournalEvent
from ikaros_runtime.errors import ContextBudgetExceededError, ModelInputUnavailableError
from ikaros_runtime.json_codec import dumps as json_dumps
from ikaros_runtime.json_codec import loads as json_loads
from ikaros_runtime.providers.base import ProviderEvent, ProviderRequest, TextDelta
from ikaros_runtime.run_input import (
    ContextItemRecordV1,
    canonical_json,
    config_input_token_counts,
)
from ikaros_runtime.storage import SqliteRuntimeStore
from ikaros_runtime.tools.core import ToolCall

from .helpers import prepare_turn, run_config


def _message_record(
    item_id: str,
    turn_id: str,
    run_id: str,
    role: str,
    content: str,
) -> ContextItemRecordV1:
    return ContextItemRecordV1(
        item_id=item_id,
        turn_id=turn_id,
        run_id=run_id,
        kind="message",
        role=role,
        content=content,
        data={},
    )


def _complete_text_turn(
    store: SqliteRuntimeStore,
    *,
    thread_id: str,
    branch_id: str,
    user_content: str,
    assistant_content: str | None = None,
) -> tuple[str, str]:
    prepared = prepare_turn(
        store,
        thread_id=thread_id,
        branch_id=branch_id,
        content=user_content,
        provider_id="scripted",
        model_id="scripted-v1",
    )
    store.mark_run_running(prepared.run_id)
    if assistant_content is not None:
        item_id, _ = store.create_assistant_item(prepared.run_id)
        store.append_text_delta(item_id, assistant_content)
    store.terminalize_run(prepared.run_id, "completed")
    return prepared.turn_id, prepared.run_id


def test_history_selector_stops_at_first_turn_that_does_not_fit() -> None:
    frame = run_config("scripted", "scripted-v1")
    current = _message_record(
        frame.user_item_id,
        frame.turn_id,
        frame.run_id,
        "user",
        "now",
    )
    instruction_tokens, tool_tokens = config_input_token_counts(frame)
    selector = HistorySelector(
        config=frame,
        current_run_id=frame.run_id,
        current_turn_id=frame.turn_id,
        current_turn_ordinal=4,
        current_records=(current,),
        maximum_tokens=(instruction_tokens + tool_tokens + current.estimated_tokens + 5 + 70),
        reserved_current_run_tokens=5,
    )
    newest = _message_record("item_new", "turn_3", "run_3", "user", "123456")
    blocked = _message_record("item_blocked", "turn_2", "run_2", "user", "1234567")

    assert selector.consider_turn(turn_id="turn_3", ordinal=3, records=(newest,)) is True
    assert selector.consider_turn(turn_id="turn_2", ordinal=2, records=(blocked,)) is False
    with pytest.raises(RuntimeError, match="omission boundary"):
        selector.consider_turn(
            turn_id="turn_1",
            ordinal=1,
            records=(_message_record("item_old", "turn_1", "run_1", "user", "x"),),
        )

    selection = selector.finish()

    assert [record.item_id for record in selection.records] == ["item_new", frame.user_item_id]
    assert selection.omissions[0].source_id == "turn_2"


def test_history_selector_counts_unicode_and_accepts_an_exact_fit() -> None:
    frame = run_config("scripted", "scripted-v1")
    current = _message_record(
        frame.user_item_id,
        frame.turn_id,
        frame.run_id,
        "user",
        "你😀",
    )
    candidate = _message_record("item_old", "turn_old", "run_old", "user", "甲😀乙")
    instructions, tools = config_input_token_counts(frame)
    selector = HistorySelector(
        config=frame,
        current_run_id=frame.run_id,
        current_turn_id=frame.turn_id,
        current_turn_ordinal=2,
        current_records=(current,),
        maximum_tokens=(
            instructions + tools + current.estimated_tokens + candidate.estimated_tokens + 1
        ),
        reserved_current_run_tokens=1,
    )

    assert current.estimated_tokens >= len("你😀".encode())
    assert candidate.estimated_tokens >= len("甲😀乙".encode())
    assert selector.consider_turn(turn_id="turn_old", ordinal=1, records=(candidate,)) is True
    assert selector.finish().omissions == ()


def test_history_selector_rejects_non_atomic_tool_pairs() -> None:
    frame = run_config("scripted", "scripted-v1")
    current = _message_record(
        frame.user_item_id,
        frame.turn_id,
        frame.run_id,
        "user",
        "now",
    )
    selector = HistorySelector(
        config=frame,
        current_run_id=frame.run_id,
        current_turn_id=frame.turn_id,
        current_turn_ordinal=2,
        current_records=(current,),
    )
    orphan_call = ContextItemRecordV1(
        item_id="item_call",
        turn_id="turn_old",
        run_id="run_old",
        kind="tool_call",
        role="assistant",
        content="",
        data={
            "stepId": "step_1",
            "callId": "call_1",
            "toolName": "process_run",
            "arguments": {"command": "echo ok"},
        },
    )

    with pytest.raises(ValueError, match="not atomic"):
        selector.consider_turn(
            turn_id="turn_old",
            ordinal=1,
            records=(
                _message_record("item_user", "turn_old", "run_old", "user", "old"),
                orphan_call,
            ),
        )


def test_history_selector_allows_call_id_reuse_in_different_steps() -> None:
    frame = run_config("scripted", "scripted-v1")
    current = _message_record(
        frame.user_item_id,
        frame.turn_id,
        frame.run_id,
        "user",
        "now",
    )
    selector = HistorySelector(
        config=frame,
        current_run_id=frame.run_id,
        current_turn_id=frame.turn_id,
        current_turn_ordinal=2,
        current_records=(current,),
    )
    records: list[ContextItemRecordV1] = [
        _message_record("item_user", "turn_old", "run_old", "user", "old")
    ]
    for step in (1, 2):
        call_item_id = f"item_call_{step}"
        records.extend(
            (
                ContextItemRecordV1(
                    item_id=call_item_id,
                    turn_id="turn_old",
                    run_id="run_old",
                    kind="tool_call",
                    role="assistant",
                    content="",
                    data={
                        "stepId": f"step_{step}",
                        "callId": "call_reused",
                        "toolName": "process_run",
                        "arguments": {"command": f"echo {step}"},
                    },
                ),
                ContextItemRecordV1(
                    item_id=f"item_result_{step}",
                    turn_id="turn_old",
                    run_id="run_old",
                    kind="tool_result",
                    role="tool",
                    content=str(step),
                    data={
                        "stepId": f"step_{step}",
                        "callId": "call_reused",
                        "toolCallItemId": call_item_id,
                        "toolName": "process_run",
                        "result": {"stdout": str(step), "exitCode": 0},
                    },
                ),
            )
        )

    assert selector.consider_turn(
        turn_id="turn_old",
        ordinal=1,
        records=records,
    )


def test_large_budget_matches_existing_context_filter_and_order(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Equivalent history")
        _complete_text_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            user_content="first",
            assistant_content="answer one",
        )
        _complete_text_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            user_content="second",
            assistant_content="answer two",
        )
        current = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="current",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        store.mark_run_running(current.run_id)
        expected = tuple(
            store.context_items(
                thread.default_branch_id,
                through_turn_id=current.turn_id,
            )
        )

        prepared_step = store.prepare_model_step(current.run_id, step_ordinal=1)

        assert prepared_step.items == expected
        assert prepared_step.context_revision.omissions == ()
    finally:
        store.close()


def test_multiple_runs_in_one_turn_have_stable_context_order(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Stable multi-Run order")
        timestamp = "2026-08-16T00:00:00.000Z"
        with store._connection:
            store._connection.execute(
                """
                INSERT INTO turns(
                    id, thread_id, branch_id, ordinal, status, created_at, updated_at
                ) VALUES ('turn_previous', ?, ?, 1, 'completed', ?, ?)
                """,
                (thread.id, thread.default_branch_id, timestamp, timestamp),
            )
            for run_id in ("run_a", "run_b"):
                store._connection.execute(
                    """
                    INSERT INTO runs(
                        id, turn_id, provider_id, model_id, status, created_at,
                        started_at, settled_at, execution_policy
                    ) VALUES (?, 'turn_previous', 'scripted', 'scripted-v1',
                              'completed', ?, ?, ?, 'full_access')
                    """,
                    (run_id, timestamp, timestamp, timestamp),
                )
            store._connection.execute(
                """
                INSERT INTO items(
                    id, turn_id, run_id, ordinal, kind, role, status, content,
                    created_at, updated_at, data_json
                ) VALUES ('item_user', 'turn_previous', 'run_a', 1, 'message',
                          'user', 'completed', 'question', ?, ?, '{}')
                """,
                (timestamp, timestamp),
            )
            store._connection.execute(
                """
                INSERT INTO items(
                    id, turn_id, run_id, ordinal, kind, role, status, content,
                    created_at, updated_at, data_json
                ) VALUES ('item_assistant', 'turn_previous', 'run_b', 1, 'message',
                          'assistant', 'completed', 'answer', ?, ?, '{}')
                """,
                (timestamp, timestamp),
            )
        current = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="continue",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        store.mark_run_running(current.run_id)

        prepared_step = store.prepare_model_step(current.run_id, step_ordinal=1)

        previous_ids = tuple(
            reference.item_id
            for reference in prepared_step.context_revision.history_items
            if reference.turn_id == "turn_previous"
        )
        assert previous_ids == ("item_user", "item_assistant")
    finally:
        store.close()


def test_store_does_not_skip_a_large_recent_turn_for_an_older_small_turn(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("No history holes")
        old_turn_id, _ = _complete_text_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            user_content="old-small",
        )
        blocked_turn_id, _ = _complete_text_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            user_content="x" * 36_000,
        )
        current = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="now",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        store.mark_run_running(current.run_id)

        prepared_step = store.prepare_model_step(current.run_id, step_ordinal=1)

        selected_turns = tuple(
            group.turn_id for group in prepared_step.context_revision.history_groups
        )
        assert selected_turns == (current.turn_id,)
        assert old_turn_id not in selected_turns
        assert prepared_step.context_revision.omissions[0].source_id == blocked_turn_id
    finally:
        store.close()


def test_multi_tool_call_turn_is_selected_as_one_atomic_group(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Atomic tools")
        previous = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="run both",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        store.mark_run_running(previous.run_id)
        store.prepare_model_step(previous.run_id, step_ordinal=1)
        completed = store.complete_provider_step(
            previous.run_id,
            step_ordinal=1,
            assistant_item_id=None,
            tool_calls=(
                ToolCall("call_1", "process_run", {"command": "first"}),
                ToolCall("call_2", "process_run", {"command": "second"}),
            ),
            reasoning_content="run both",
            usage=None,
            response_model_id=None,
            request_id=None,
        )
        for index, item_id in enumerate(completed.tool_call_item_ids, start=1):
            store.complete_tool_call(
                item_id,
                status="completed",
                result={"stdout": f"result {index}", "exitCode": 0},
                result_content=f"result {index}",
            )
        store.terminalize_run(previous.run_id, "completed")
        current = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="continue",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        store.mark_run_running(current.run_id)

        prepared_step = store.prepare_model_step(current.run_id, step_ordinal=1)
        previous_items = tuple(
            reference
            for reference in prepared_step.context_revision.history_items
            if reference.turn_id == previous.turn_id
        )

        assert [item.kind for item in previous_items] == [
            "message",
            "tool_call",
            "tool_call",
            "tool_result",
            "tool_result",
        ]
    finally:
        store.close()


def test_cancelled_tool_items_do_not_create_orphan_provider_messages(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Cancelled tools")
        cancelled = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="cancel this",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        store.mark_run_running(cancelled.run_id)
        store.prepare_model_step(cancelled.run_id, step_ordinal=1)
        store.complete_provider_step(
            cancelled.run_id,
            step_ordinal=1,
            assistant_item_id=None,
            tool_calls=(ToolCall("call_cancel", "process_run", {"command": "wait"}),),
            reasoning_content=None,
            usage=None,
            response_model_id=None,
            request_id=None,
        )
        store.terminalize_run(cancelled.run_id, "cancelled", reason_code="cancelled")
        current = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="continue",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        store.mark_run_running(current.run_id)

        prepared_step = store.prepare_model_step(current.run_id, step_ordinal=1)
        cancelled_kinds = tuple(
            reference.kind
            for reference in prepared_step.context_revision.history_items
            if reference.turn_id == cancelled.turn_id
        )

        assert cancelled_kinds == ("message", "tool_call", "tool_result")
    finally:
        store.close()


def test_later_tool_steps_load_frozen_ids_and_current_run_without_branch_scan(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Sixteen steps")
        current = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="repeat",
            provider_id="scripted",
            model_id="scripted-v1",
            max_model_calls=16,
        )
        store.mark_run_running(current.run_id)
        first = store.prepare_model_step(current.run_id, step_ordinal=1)
        queries: list[str] = []
        store._connection.set_trace_callback(queries.append)
        frozen_ids = tuple(item.item_id for item in first.context_revision.history_items)

        for ordinal in range(1, 16):
            completed = store.complete_provider_step(
                current.run_id,
                step_ordinal=ordinal,
                assistant_item_id=None,
                tool_calls=(
                    ToolCall(
                        f"call_{ordinal}",
                        "process_run",
                        {"command": f"echo {ordinal}"},
                    ),
                ),
                reasoning_content=None,
                usage=None,
                response_model_id=None,
                request_id=None,
            )
            store.complete_tool_call(
                completed.tool_call_item_ids[0],
                status="completed",
                result={"stdout": str(ordinal), "exitCode": 0},
                result_content=str(ordinal),
            )
            next_step = store.prepare_model_step(
                current.run_id,
                step_ordinal=ordinal + 1,
            )
            assert (
                tuple(item.item_id for item in next_step.context_revision.history_items)
                == frozen_ids
            )

        normalized = "\n".join(query.upper() for query in queries)
        assert "WHERE I.ID IN" in normalized
        assert "WHERE I.RUN_ID =" in normalized
        assert "T.ORDINAL <=" not in normalized
    finally:
        store.close()


def test_current_run_may_grow_past_reserve_while_total_stays_within_budget(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Tool growth overflow")
        current = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="run it",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        store.mark_run_running(current.run_id)
        store.prepare_model_step(current.run_id, step_ordinal=1)
        completed = store.complete_provider_step(
            current.run_id,
            step_ordinal=1,
            assistant_item_id=None,
            tool_calls=(ToolCall("call_big", "process_run", {"command": "big"}),),
            reasoning_content=None,
            usage=None,
            response_model_id=None,
            request_id=None,
        )
        store.complete_tool_call(
            completed.tool_call_item_ids[0],
            status="completed",
            result={"stdout": "x" * 13_000, "exitCode": 0},
            result_content="x" * 13_000,
        )
        second_step = store.prepare_model_step(current.run_id, step_ordinal=2)

        growth = (
            second_step.step_input.budget.current_run_tokens
            - second_step.context_revision.budget.current_run_tokens
        )
        maximum = second_step.step_input.budget.maximum_tokens
        assert growth > second_step.step_input.budget.reserved_current_run_tokens
        assert maximum is not None
        assert second_step.step_input.budget.total_tokens <= maximum
    finally:
        store.close()


def test_current_run_fails_only_when_actual_total_exceeds_budget(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Tool growth overflow")
        current = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="run it",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        store.mark_run_running(current.run_id)
        store.prepare_model_step(current.run_id, step_ordinal=1)
        completed = store.complete_provider_step(
            current.run_id,
            step_ordinal=1,
            assistant_item_id=None,
            tool_calls=(ToolCall("call_big", "process_run", {"command": "big"}),),
            reasoning_content=None,
            usage=None,
            response_model_id=None,
            request_id=None,
        )
        store.complete_tool_call(
            completed.tool_call_item_ids[0],
            status="completed",
            result={"stdout": "x" * 50_000, "exitCode": 0},
            result_content="x" * 50_000,
        )
        latest_seq = store.latest_sequence()

        with pytest.raises(ContextBudgetExceededError, match="context_budget_exceeded"):
            store.prepare_model_step(current.run_id, step_ordinal=2)

        assert store.latest_sequence() == latest_seq
    finally:
        store.close()


def test_first_step_preparation_rolls_back_snapshot_and_event_together(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Atomic model input")
        current = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="prepare atomically",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        store.mark_run_running(current.run_id)
        latest_seq = store.latest_sequence()

        def fail_append_event(**_kwargs: object) -> JournalEvent:
            raise RuntimeError("injected append failure")

        monkeypatch.setattr(store, "_append_event", fail_append_event)

        with pytest.raises(RuntimeError, match="injected append failure"):
            store.prepare_model_step(current.run_id, step_ordinal=1)

        assert store.latest_sequence() == latest_seq
        row = store._connection.execute(
            "SELECT record_json FROM context_revisions WHERE run_id = ?",
            (current.run_id,),
        ).fetchone()
        assert row is None
        assert (
            store._connection.execute(
                "SELECT COUNT(*) FROM model_calls WHERE run_id = ?",
                (current.run_id,),
            ).fetchone()[0]
            == 0
        )
    finally:
        store.close()


def test_later_step_rejects_a_missing_frozen_item(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Missing frozen history")
        _complete_text_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            user_content="previous",
            assistant_content="answer",
        )
        current = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="continue",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        store.mark_run_running(current.run_id)
        first = store.prepare_model_step(current.run_id, step_ordinal=1)
        completed = store.complete_provider_step(
            current.run_id,
            step_ordinal=1,
            assistant_item_id=None,
            tool_calls=(ToolCall("call_1", "process_run", {"command": "echo ok"}),),
            reasoning_content=None,
            usage=None,
            response_model_id=None,
            request_id=None,
        )
        store.complete_tool_call(
            completed.tool_call_item_ids[0],
            status="completed",
            result={"stdout": "ok", "exitCode": 0},
            result_content="ok",
        )
        frozen_item_id = next(
            reference.item_id
            for reference in first.context_revision.history_items
            if reference.run_id != current.run_id
        )
        with store._connection:
            store._connection.execute("DELETE FROM items WHERE id = ?", (frozen_item_id,))

        with pytest.raises(ModelInputUnavailableError, match="model_input_unavailable"):
            store.prepare_model_step(current.run_id, step_ordinal=2)

        assert (
            store._connection.execute(
                "SELECT COUNT(*) FROM model_calls WHERE run_id = ?",
                (current.run_id,),
            ).fetchone()[0]
            == 1
        )
    finally:
        store.close()


def test_1000_turn_history_stops_paging_after_budget_boundary(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Paged history")
        timestamp = "2026-08-16T00:00:00.000Z"
        with store._connection:
            for ordinal in range(1, 1001):
                turn_id = f"turn_{ordinal:04d}"
                run_id = f"run_{ordinal:04d}"
                item_id = f"item_{ordinal:04d}"
                store._connection.execute(
                    """
                    INSERT INTO turns(
                        id, thread_id, branch_id, ordinal, status, created_at, updated_at
                    )
                    VALUES (?, ?, ?, ?, 'completed', ?, ?)
                    """,
                    (
                        turn_id,
                        thread.id,
                        thread.default_branch_id,
                        ordinal,
                        timestamp,
                        timestamp,
                    ),
                )
                store._connection.execute(
                    """
                    INSERT INTO runs(
                        id, turn_id, provider_id, model_id, status, created_at,
                        started_at, settled_at, execution_policy
                    ) VALUES (?, ?, 'scripted', 'scripted-v1', 'completed', ?, ?, ?, 'full_access')
                    """,
                    (run_id, turn_id, timestamp, timestamp, timestamp),
                )
                store._connection.execute(
                    """
                    INSERT INTO items(
                        id, turn_id, run_id, ordinal, kind, role, status, content,
                        created_at, updated_at, data_json
                    ) VALUES (?, ?, ?, 1, 'message', 'user', 'completed', ?, ?, ?, '{}')
                    """,
                    (item_id, turn_id, run_id, "x" * 200, timestamp, timestamp),
                )
        current = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="now",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        store.mark_run_running(current.run_id)
        queries: list[str] = []
        store._connection.set_trace_callback(queries.append)

        prepared_step = store.prepare_model_step(current.run_id, step_ordinal=1)

        page_queries = [
            query
            for query in queries
            if "FROM TURNS" in query.upper() and "ORDER BY ORDINAL DESC" in query.upper()
        ]
        assert 1 < len(page_queries) < 10
        assert prepared_step.context_revision.omissions
        assert len(prepared_step.context_revision.history_groups) < 1001
    finally:
        store.close()


def test_1000_turn_journal_rebuild_replays_the_same_bounded_snapshot(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Journal rebuild paging")
        for ordinal in range(1, 1001):
            _complete_text_turn(
                store,
                thread_id=thread.id,
                branch_id=thread.default_branch_id,
                user_content=f"{ordinal:04d}:" + ("x" * 195),
            )
        current = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="now",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        store.mark_run_running(current.run_id)
        prepared_step = store.prepare_model_step(current.run_id, step_ordinal=1)
        expected_snapshot = canonical_json(prepared_step.context_revision.to_wire())
        queries: list[str] = []
        store._connection.set_trace_callback(queries.append)

        store.rebuild_projections()

        rebuilt_snapshot = store._connection.execute(
            "SELECT record_json FROM context_revisions WHERE run_id = ?",
            (current.run_id,),
        ).fetchone()
        assert rebuilt_snapshot is not None
        assert rebuilt_snapshot["record_json"] == expected_snapshot
        page_queries = [
            query
            for query in queries
            if "FROM TURNS" in query.upper() and "ORDER BY ORDINAL DESC" in query.upper()
        ]
        assert 1 < len(page_queries) < 10
    finally:
        store.close()


def test_context_revision_rebuild_is_deterministic(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Rebuild selection")
        _complete_text_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            user_content="old",
            assistant_content="answer",
        )
        _complete_text_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            user_content="x" * 36_000,
        )
        current = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="now",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        store.mark_run_running(current.run_id)
        store.prepare_model_step(current.run_id, step_ordinal=1)
        before = str(
            store._connection.execute(
                "SELECT record_json FROM context_revisions WHERE run_id = ?",
                (current.run_id,),
            ).fetchone()[0]
        )
        assert "omitted_by_budget" in before

        store.rebuild_projections()

        after = str(
            store._connection.execute(
                "SELECT record_json FROM context_revisions WHERE run_id = ?",
                (current.run_id,),
            ).fetchone()[0]
        )
        assert after == before
    finally:
        store.close()


def test_projection_rebuild_rejects_a_tampered_omission_boundary(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Tampered omission")
        _complete_text_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            user_content="old",
            assistant_content="answer",
        )
        _complete_text_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            user_content="x" * 36_000,
        )
        current = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="now",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        store.mark_run_running(current.run_id)
        store.prepare_model_step(current.run_id, step_ordinal=1)
        row = store._connection.execute(
            """
            SELECT seq, payload_json FROM events
            WHERE run_id = ? AND event_type = 'model.input_prepared'
            """,
            (current.run_id,),
        ).fetchone()
        payload = json_loads(str(row["payload_json"]))
        for container in (payload["contextRevision"], payload["stepInput"]):
            container["omissions"][0]["sourceId"] = "turn_tampered_boundary"
        with store._connection:
            store._connection.execute(
                "UPDATE events SET payload_json = ? WHERE seq = ?",
                (
                    json_dumps(payload, separators=(",", ":"), ensure_ascii=False),
                    row["seq"],
                ),
            )

        with pytest.raises(RuntimeError, match="initial Context Revision is not canonical"):
            store.rebuild_projections()
    finally:
        store.close()


def test_turns_queued_after_the_context_boundary_are_never_selected(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Stable boundary")
        current = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="current",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        store.mark_run_running(current.run_id)
        future = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="queued later",
            provider_id="scripted",
            model_id="scripted-v1",
        )

        prepared_step = store.prepare_model_step(current.run_id, step_ordinal=1)

        selected_turn_ids = {
            reference.turn_id for reference in prepared_step.context_revision.history_items
        }
        assert current.turn_id in selected_turn_ids
        assert future.turn_id not in selected_turn_ids
    finally:
        store.close()


class _NeverCalledProvider:
    def __init__(self) -> None:
        self.calls = 0

    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]:
        del request, cancellation
        self.calls += 1
        yield TextDelta("unexpected")


@pytest.mark.asyncio
async def test_oversized_current_request_fails_before_provider_call(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    events: list[JournalEvent] = []
    provider = _NeverCalledProvider()

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    try:
        thread, _ = store.create_thread("Oversized current Run")
        current = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="x" * 40_000,
            provider_id="scripted",
            model_id="scripted-v1",
        )
        loop = AgentLoop(store, {"scripted": provider}, publish)

        await loop.run(current.run_id, CancellationToken())

        assert provider.calls == 0
        assert store.run_status(current.run_id) == "failed"
        assert events[-1].type == "run.settled"
        assert events[-1].payload["reasonCode"] == "context_budget_exceeded"
        assert not any(event.type == "model.input_prepared" for event in events)
    finally:
        store.close()


@pytest.mark.asyncio
async def test_corrupt_history_fails_with_stable_reason_before_provider_call(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    events: list[JournalEvent] = []
    provider = _NeverCalledProvider()

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    try:
        thread, _ = store.create_thread("Unavailable model input")
        previous_turn_id, _ = _complete_text_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            user_content="previous",
            assistant_content="answer",
        )
        current = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="continue",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        with store._connection:
            store._connection.execute(
                "DELETE FROM items WHERE turn_id = ? AND role = 'user'",
                (previous_turn_id,),
            )
        loop = AgentLoop(store, {"scripted": provider}, publish)

        await loop.run(current.run_id, CancellationToken())

        assert provider.calls == 0
        assert store.run_status(current.run_id) == "failed"
        assert events[-1].type == "run.settled"
        assert events[-1].payload["reasonCode"] == "model_input_unavailable"
        assert not any(event.type == "model.input_prepared" for event in events)
    finally:
        store.close()
