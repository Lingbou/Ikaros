from __future__ import annotations

import json
from pathlib import Path

import pytest

from ikaros_runtime.cancellation import CancellationToken
from ikaros_runtime.storage import SqliteRuntimeStore
from ikaros_runtime.tools import (
    ThreadListTool,
    ThreadReadTool,
    ToolCall,
    ToolExecutionContext,
)

from .helpers import prepare_turn


def _complete_turn(
    store: SqliteRuntimeStore,
    *,
    thread_id: str,
    branch_id: str,
    content: str,
    answer: str,
) -> tuple[str, str]:
    prepared = prepare_turn(
        store,
        thread_id=thread_id,
        branch_id=branch_id,
        content=content,
        provider_id="scripted",
        model_id="scripted-v1",
    )
    store.mark_run_running(prepared.run_id)
    assistant_id, _ = store.create_assistant_item(prepared.run_id)
    store.append_text_delta(assistant_id, answer)
    store.complete_assistant_item(assistant_id, step_id="step-1")
    store.terminalize_run(prepared.run_id, "completed")
    return store.get_run_config(prepared.run_id).user_item_id, assistant_id


def _context(thread_id: str) -> ToolExecutionContext:
    return ToolExecutionContext("run_caller", 1, "item_caller", thread_id)


@pytest.mark.asyncio
async def test_thread_list_returns_stable_catalog_and_archived_filter(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        first, _ = store.create_thread("First")
        second, _ = store.create_thread("Second")
        archived, _ = store.create_thread("Archived")
        store.set_thread_archived(archived.id, archived=True)

        tool = ThreadListTool(store.list_threads_for_model)
        result = await tool.execute(
            ToolCall("list", "thread_list", {}),
            cancellation=CancellationToken(),
            context=_context(first.id),
        )
        assert result.ok is True
        payload = json.loads(result.output)
        assert set(entry["id"] for entry in payload["threads"]) == {first.id, second.id}
        assert payload["hasMore"] is False
        assert result.details["threadCount"] == 2
        for entry in payload["threads"]:
            assert set(entry) == {"id", "title", "updatedAt", "archived"}
            assert entry["archived"] is False
            assert isinstance(entry["updatedAt"], str) and entry["updatedAt"]
        assert {entry["title"] for entry in payload["threads"]} == {"First", "Second"}

        repeated = await tool.execute(
            ToolCall("list", "thread_list", {}),
            cancellation=CancellationToken(),
            context=_context(first.id),
        )
        assert repeated.output == result.output

        archived_result = await tool.execute(
            ToolCall("list", "thread_list", {"archived": True}),
            cancellation=CancellationToken(),
            context=_context(first.id),
        )
        archived_payload = json.loads(archived_result.output)
        assert [entry["id"] for entry in archived_payload["threads"]] == [archived.id]
        assert archived_payload["threads"][0]["archived"] is True

        limited = await tool.execute(
            ToolCall("list", "thread_list", {"limit": 1}),
            cancellation=CancellationToken(),
            context=_context(first.id),
        )
        limited_payload = json.loads(limited.output)
        assert len(limited_payload["threads"]) == 1
        assert limited_payload["hasMore"] is True
    finally:
        store.close()


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "arguments",
    [
        {"limit": 0},
        {"limit": 51},
        {"limit": True},
        {"archived": "yes"},
        {"unknown": 1},
    ],
)
async def test_thread_list_rejects_invalid_arguments(
    tmp_path: Path,
    arguments: dict[str, object],
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Thread")
        tool = ThreadListTool(store.list_threads_for_model)
        result = await tool.execute(
            ToolCall("list", "thread_list", arguments),
            cancellation=CancellationToken(),
            context=_context(thread.id),
        )
        assert result.ok is False
        assert result.details["errorCode"] == "invalid_arguments"
    finally:
        store.close()


@pytest.mark.asyncio
async def test_thread_read_returns_the_newest_settled_slice_with_metadata(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        caller, _ = store.create_thread("Caller")
        target, _ = store.create_thread("Target")
        _complete_turn(
            store,
            thread_id=target.id,
            branch_id=target.default_branch_id,
            content="first question",
            answer="first answer",
        )
        second_user, second_assistant = _complete_turn(
            store,
            thread_id=target.id,
            branch_id=target.default_branch_id,
            content="second question",
            answer="second answer",
        )

        tool = ThreadReadTool(store.read_thread_slice)
        result = await tool.execute(
            ToolCall("read", "thread_read", {"threadId": target.id, "before": 1, "after": 0}),
            cancellation=CancellationToken(),
            context=_context(caller.id),
        )

        assert result.ok is True
        payload = json.loads(result.output)
        assert payload["thread"]["id"] == target.id
        assert payload["thread"]["title"] == "Target"
        assert payload["thread"]["archived"] is False
        assert payload["thread"]["updatedAt"] == store.get_thread(target.id).thread.updated_at
        assert payload["targetItemId"] == second_assistant
        assert [item["id"] for item in payload["items"]] == [second_user, second_assistant]
        assert payload["truncated"] is False
        answer = payload["items"][1]
        assert answer["kind"] == "message"
        assert answer["role"] == "assistant"
        assert answer["content"] == "second answer"
        assert answer["createdAt"] and answer["updatedAt"]
        assert result.details["threadId"] == target.id
        assert result.details["itemId"] == second_assistant
        assert result.details["truncated"] is False
    finally:
        store.close()


@pytest.mark.asyncio
async def test_thread_read_pages_around_an_explicit_anchor(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        caller, _ = store.create_thread("Caller")
        target, _ = store.create_thread("Target")
        first_user, first_assistant = _complete_turn(
            store,
            thread_id=target.id,
            branch_id=target.default_branch_id,
            content="question one",
            answer="answer one",
        )
        second_user, second_assistant = _complete_turn(
            store,
            thread_id=target.id,
            branch_id=target.default_branch_id,
            content="question two",
            answer="answer two",
        )
        third_user, third_assistant = _complete_turn(
            store,
            thread_id=target.id,
            branch_id=target.default_branch_id,
            content="question three",
            answer="answer three",
        )

        tool = ThreadReadTool(store.read_thread_slice)
        centered = await tool.execute(
            ToolCall(
                "read",
                "thread_read",
                {"threadId": target.id, "itemId": second_user, "before": 1, "after": 1},
            ),
            cancellation=CancellationToken(),
            context=_context(caller.id),
        )
        payload = json.loads(centered.output)
        assert payload["targetItemId"] == second_user
        assert [item["id"] for item in payload["items"]] == [
            first_assistant,
            second_user,
            second_assistant,
        ]

        tail = await tool.execute(
            ToolCall(
                "read",
                "thread_read",
                {"threadId": target.id, "itemId": third_assistant, "before": 1, "after": 5},
            ),
            cancellation=CancellationToken(),
            context=_context(caller.id),
        )
        tail_payload = json.loads(tail.output)
        assert [item["id"] for item in tail_payload["items"]] == [
            third_user,
            third_assistant,
        ]
    finally:
        store.close()


@pytest.mark.asyncio
async def test_thread_read_truncates_oversized_slices_explicitly(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        caller, _ = store.create_thread("Caller")
        target, _ = store.create_thread("Target")
        _complete_turn(
            store,
            thread_id=target.id,
            branch_id=target.default_branch_id,
            content="long question " * 200,
            answer="long answer " * 400,
        )

        tool = ThreadReadTool(store.read_thread_slice)
        result = await tool.execute(
            ToolCall("read", "thread_read", {"threadId": target.id, "maxChars": 256}),
            cancellation=CancellationToken(),
            context=_context(caller.id),
        )

        assert result.ok is True
        payload = json.loads(result.output)
        assert payload["truncated"] is True
        assert len(result.output) < 2_000
        assert "[truncated]" in result.output
        assert result.details["truncated"] is True
        assert len(payload["items"]) == 1
    finally:
        store.close()


@pytest.mark.asyncio
async def test_thread_read_reports_missing_threads_items_and_unsettled_items(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        caller, _ = store.create_thread("Caller")
        target, _ = store.create_thread("Target")
        other, _ = store.create_thread("Other")
        target_user, _ = _complete_turn(
            store,
            thread_id=target.id,
            branch_id=target.default_branch_id,
            content="target question",
            answer="target answer",
        )

        tool = ThreadReadTool(store.read_thread_slice)
        missing_thread = await tool.execute(
            ToolCall("read", "thread_read", {"threadId": "thread_missing"}),
            cancellation=CancellationToken(),
            context=_context(caller.id),
        )
        assert missing_thread.ok is False
        assert missing_thread.details["errorCode"] == "thread_not_found"

        missing_item = await tool.execute(
            ToolCall(
                "read",
                "thread_read",
                {"threadId": target.id, "itemId": "item_missing"},
            ),
            cancellation=CancellationToken(),
            context=_context(caller.id),
        )
        assert missing_item.ok is False
        assert missing_item.details["errorCode"] == "item_not_found"

        # A valid Item ID from another Thread must never leak across Threads.
        cross_thread = await tool.execute(
            ToolCall(
                "read",
                "thread_read",
                {"threadId": other.id, "itemId": target_user},
            ),
            cancellation=CancellationToken(),
            context=_context(caller.id),
        )
        assert cross_thread.ok is False
        assert cross_thread.details["errorCode"] == "item_not_found"
        assert "target question" not in cross_thread.output

        prepared = prepare_turn(
            store,
            thread_id=target.id,
            branch_id=target.default_branch_id,
            content="in flight question",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        store.mark_run_running(prepared.run_id)
        pending_id, _ = store.create_assistant_item(prepared.run_id)
        unsettled = await tool.execute(
            ToolCall("read", "thread_read", {"threadId": target.id, "itemId": pending_id}),
            cancellation=CancellationToken(),
            context=_context(caller.id),
        )
        assert unsettled.ok is False
        assert unsettled.details["errorCode"] == "item_not_settled"

        empty = await tool.execute(
            ToolCall("read", "thread_read", {"threadId": other.id}),
            cancellation=CancellationToken(),
            context=_context(caller.id),
        )
        assert empty.ok is True
        empty_payload = json.loads(empty.output)
        assert empty_payload["targetItemId"] is None
        assert empty_payload["items"] == []
        assert empty_payload["truncated"] is False
    finally:
        store.close()


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "arguments",
    [
        {"threadId": "thread_x", "before": 42},
        {"threadId": "thread_x", "after": -1},
        {"threadId": "thread_x", "before": True},
        {"threadId": "thread_x", "maxChars": 255},
        {"threadId": "thread_x", "maxChars": 24_001},
        {"threadId": "thread_x", "itemId": ""},
        {"threadId": ""},
        {"threadId": "thread_x", "unknown": 1},
    ],
)
async def test_thread_read_rejects_invalid_arguments(
    tmp_path: Path,
    arguments: dict[str, object],
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Thread")
        tool = ThreadReadTool(store.read_thread_slice)
        result = await tool.execute(
            ToolCall("read", "thread_read", arguments),
            cancellation=CancellationToken(),
            context=_context(thread.id),
        )
        assert result.ok is False
        assert result.details["errorCode"] == "invalid_arguments"
    finally:
        store.close()


@pytest.mark.asyncio
async def test_thread_tools_leave_the_journal_and_threads_untouched(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        caller, _ = store.create_thread("Caller")
        target, _ = store.create_thread("Target")
        target_user, _ = _complete_turn(
            store,
            thread_id=target.id,
            branch_id=target.default_branch_id,
            content="target question",
            answer="target answer",
        )
        events_before = store.replay_events(0, 10_000)[0]
        thread_before = store.get_thread(target.id).thread

        list_result = await ThreadListTool(store.list_threads_for_model).execute(
            ToolCall("list", "thread_list", {"archived": True}),
            cancellation=CancellationToken(),
            context=_context(caller.id),
        )
        read_result = await ThreadReadTool(store.read_thread_slice).execute(
            ToolCall(
                "read",
                "thread_read",
                {"threadId": target.id, "itemId": target_user},
            ),
            cancellation=CancellationToken(),
            context=_context(caller.id),
        )

        assert list_result.ok is True
        assert read_result.ok is True
        assert store.replay_events(0, 10_000)[0] == events_before
        assert store.latest_sequence() == len(events_before)
        thread_after = store.get_thread(target.id).thread
        assert thread_after.updated_at == thread_before.updated_at
        assert thread_after.archived_at is None
        assert store.get_thread(caller.id).thread.title == "Caller"
    finally:
        store.close()
