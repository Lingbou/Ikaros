from __future__ import annotations

from pathlib import Path

import pytest

from ikaros_runtime.cancellation import CancellationToken
from ikaros_runtime.storage import SqliteRuntimeStore
from ikaros_runtime.tools import HistoryReadTool, ToolCall, ToolExecutionContext

from .helpers import prepare_turn


@pytest.mark.asyncio
async def test_history_read_returns_settled_items_around_target(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("History")
        prepared = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="remember the exact request",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        store.mark_run_running(prepared.run_id)
        assistant_id, _ = store.create_assistant_item(prepared.run_id)
        store.append_text_delta(assistant_id, "the exact answer")
        store.complete_assistant_item(assistant_id, step_id="step-1")
        store.terminalize_run(prepared.run_id, "completed")

        tool = HistoryReadTool(store.read_history_slice)
        result = await tool.execute(
            ToolCall("read", "history_read", {"itemId": prepared.run_id.replace("run_", "item_")}),
            cancellation=CancellationToken(),
            context=ToolExecutionContext(prepared.run_id, 1, assistant_id, thread.id),
        )

        # The generated User Item ID is in the initial event payload.
        assert result.ok is False  # run IDs are not Item IDs and must not be guessed
        user_item_id = store.get_run_config(prepared.run_id).user_item_id
        result = await tool.execute(
            ToolCall("read", "history_read", {"itemId": user_item_id, "before": 0, "after": 1}),
            cancellation=CancellationToken(),
            context=ToolExecutionContext(prepared.run_id, 1, assistant_id, thread.id),
        )
        assert result.ok is True
        assert "remember the exact request" in result.output
        assert "the exact answer" in result.output
    finally:
        store.close()


@pytest.mark.asyncio
async def test_history_read_rejects_cross_thread_and_invalid_limits(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        first, _ = store.create_thread("First")
        second, _ = store.create_thread("Second")
        prepared = prepare_turn(
            store,
            thread_id=first.id,
            branch_id=first.default_branch_id,
            content="private history",
            provider_id="scripted",
            model_id="scripted-v1",
        )
        tool = HistoryReadTool(store.read_history_slice)
        user_item_id = store.get_run_config(prepared.run_id).user_item_id
        context = ToolExecutionContext(prepared.run_id, 1, user_item_id, second.id)
        result = await tool.execute(
            ToolCall("read", "history_read", {"itemId": user_item_id}),
            cancellation=CancellationToken(),
            context=context,
        )
        assert result.ok is False
        assert result.details["errorCode"] == "history_unavailable"

        result = await tool.execute(
            ToolCall("read", "history_read", {"itemId": user_item_id, "maxChars": 1}),
            cancellation=CancellationToken(),
            context=ToolExecutionContext(prepared.run_id, 1, user_item_id, first.id),
        )
        assert result.ok is False
        assert result.details["errorCode"] == "invalid_arguments"
    finally:
        store.close()
