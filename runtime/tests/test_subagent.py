"""Subagent tool: child sessions spawned from inside a Run."""

from __future__ import annotations

import asyncio
from collections.abc import AsyncIterator
from pathlib import Path
from typing import Any

import pytest

from ikaros_runtime.agent import AgentLoop
from ikaros_runtime.cancellation import CancellationToken
from ikaros_runtime.domain import JournalEvent, JsonObject
from ikaros_runtime.providers.base import (
    ProviderEvent,
    ProviderRequest,
    ResponseCompleted,
    TextDelta,
)
from ikaros_runtime.services.subagents import SubagentError, SubagentService
from ikaros_runtime.storage import SqliteRuntimeStore
from ikaros_runtime.tools import (
    FullAccessPolicy,
    SubagentTool,
    ToolCall,
    ToolExecutionContext,
    ToolExecutor,
    ToolRegistry,
)

from .fakes.scripted_provider import ScriptedProvider
from .helpers import prepare_turn


class _BackendStub:
    async def spawn(self, **_: object) -> JsonObject:  # pragma: no cover - unused
        raise AssertionError("backend stub must not be called")


class _RecordingBackend:
    def __init__(self, payload: JsonObject) -> None:
        self.payload = payload
        self.calls: list[dict[str, object]] = []

    async def spawn(self, **kwargs: object) -> JsonObject:
        self.calls.append(kwargs)
        return self.payload


class _FailingBackend:
    async def spawn(self, **_: object) -> JsonObject:
        raise SubagentError("subagent_depth_limit", "Subagents cannot spawn further subagents.")


class _SlowProvider:
    id = "scripted"
    model_id = "scripted-v1"

    def __init__(self, delay: float = 30.0) -> None:
        self.delay = delay

    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]:
        await cancellation.sleep(self.delay)
        yield TextDelta("slow child finished")
        yield ResponseCompleted()


_LONG_ANSWER = "L" * 5_000


class _LongAnswerProvider:
    id = "scripted"
    model_id = "scripted-v1"

    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]:
        yield TextDelta(_LONG_ANSWER)
        yield ResponseCompleted()


def _setup(
    tmp_path: Path,
    *,
    provider: Any = None,
) -> tuple[SqliteRuntimeStore, SubagentService, list[JournalEvent], str, str]:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    events: list[JournalEvent] = []

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    executor = ToolExecutor(ToolRegistry([SubagentTool(_BackendStub())]), FullAccessPolicy())
    active_provider = provider if provider is not None else ScriptedProvider()
    loop = AgentLoop(store, {ScriptedProvider.id: active_provider}, publish, executor)
    service = SubagentService(
        store=store,
        run_child=loop.run,
        publish=publish,
    )
    thread, _created = store.create_thread("Parent")
    turn = prepare_turn(
        store,
        thread_id=thread.id,
        branch_id=thread.default_branch_id,
        content="parent task",
        provider_id="scripted",
        model_id="scripted-v1",
        tools=executor.definitions,
    )
    store.mark_run_running(turn.run_id)
    return store, service, events, turn.run_id, thread.id


@pytest.mark.asyncio
async def test_subagent_spawns_a_child_thread_and_returns_the_final_message(
    tmp_path: Path,
) -> None:
    store, service, events, parent_run, parent_thread = _setup(tmp_path)
    try:
        result = await service.spawn(
            parent_run_id=parent_run,
            request_id="item_child_one",
            prompt="child task",
            title=None,
            cancellation=CancellationToken(),
        )
        assert result["status"] == "completed"
        assert result["summary"] == "Scripted response to: child task"
        assert result["threadId"] != parent_thread
        page = store.list_thread_page(cursor=None, limit=50, archived=False)
        child = next(thread for thread in page.threads if thread.id == result["threadId"])
        assert child.title == "Subagent: child task"
        assert store.run_status(str(result["runId"])) == "completed"
        assert any(event.type == "run.settled" for event in events)
    finally:
        store.close()


@pytest.mark.asyncio
async def test_subagent_replays_idempotently_for_one_tool_call(tmp_path: Path) -> None:
    store, service, _events, parent_run, _parent_thread = _setup(tmp_path)
    try:
        first = await service.spawn(
            parent_run_id=parent_run,
            request_id="item_child_replay",
            prompt="replay child",
            title=None,
            cancellation=CancellationToken(),
        )
        second = await service.spawn(
            parent_run_id=parent_run,
            request_id="item_child_replay",
            prompt="replay child",
            title=None,
            cancellation=CancellationToken(),
        )
        assert second["threadId"] == first["threadId"]
        assert second["runId"] == first["runId"]
        assert second["status"] == "completed"
        assert second["summary"] == first["summary"]
    finally:
        store.close()


@pytest.mark.asyncio
async def test_subagent_depth_limit_rejects_nested_spawning(tmp_path: Path) -> None:
    store, service, _events, parent_run, _parent_thread = _setup(tmp_path)
    try:
        service._depth_by_run[parent_run] = 1
        with pytest.raises(SubagentError) as raised:
            await service.spawn(
                parent_run_id=parent_run,
                request_id="item_child_nested",
                prompt="nested child",
                title=None,
                cancellation=CancellationToken(),
            )
        assert raised.value.code == "subagent_depth_limit"
    finally:
        store.close()


@pytest.mark.asyncio
async def test_subagent_waits_for_slow_child_work_without_a_wall_clock_cutoff(
    tmp_path: Path,
) -> None:
    store, service, _events, parent_run, _parent_thread = _setup(
        tmp_path,
        provider=_SlowProvider(delay=0.2),
    )
    try:
        result = await service.spawn(
            parent_run_id=parent_run,
            request_id="item_child_slow",
            prompt="slow child",
            title=None,
            cancellation=CancellationToken(),
        )
        assert result["status"] == "completed"
    finally:
        store.close()


@pytest.mark.asyncio
async def test_subagent_returns_the_full_final_message_without_truncation(
    tmp_path: Path,
) -> None:
    store, service, _events, parent_run, _parent_thread = _setup(
        tmp_path,
        provider=_LongAnswerProvider(),
    )
    try:
        result = await service.spawn(
            parent_run_id=parent_run,
            request_id="item_child_long",
            prompt="long child",
            title=None,
            cancellation=CancellationToken(),
        )
        assert result["status"] == "completed"
        assert result["summary"] == _LONG_ANSWER
    finally:
        store.close()


@pytest.mark.asyncio
async def test_subagent_parent_cancellation_cancels_the_child_run(tmp_path: Path) -> None:
    store, service, _events, parent_run, _parent_thread = _setup(
        tmp_path,
        provider=_SlowProvider(),
    )
    token = CancellationToken()
    try:
        task = asyncio.create_task(
            service.spawn(
                parent_run_id=parent_run,
                request_id="item_child_cancel",
                prompt="cancel child",
                title=None,
                cancellation=token,
            )
        )
        await asyncio.sleep(0.05)
        token.cancel()
        result = await asyncio.wait_for(task, timeout=10)
        assert result["status"] == "cancelled"
    finally:
        store.close()


@pytest.mark.asyncio
async def test_subagent_tool_validates_arguments_and_projects_results() -> None:
    backend = _RecordingBackend(
        {
            "threadId": "thread_child",
            "runId": "run_child",
            "status": "completed",
            "summary": "done",
        }
    )
    tool = SubagentTool(backend)
    context = ToolExecutionContext("run_parent", 1, "item_child", "thread_parent", None)
    missing = await tool.execute(
        ToolCall("call_missing", "subagent", {}),
        cancellation=CancellationToken(),
        context=context,
    )
    assert not missing.ok
    assert missing.details["errorCode"] == "invalid_arguments"
    unknown = await tool.execute(
        ToolCall("call_unknown", "subagent", {"prompt": "x", "extra": True}),
        cancellation=CancellationToken(),
        context=context,
    )
    assert not unknown.ok
    assert unknown.details["errorCode"] == "invalid_arguments"
    accepted = await tool.execute(
        ToolCall("call_ok", "subagent", {"prompt": "do it", "title": "Child"}),
        cancellation=CancellationToken(),
        context=context,
    )
    assert accepted.ok
    assert accepted.output == "done"
    assert accepted.details["threadId"] == "thread_child"
    assert backend.calls and backend.calls[-1]["request_id"] == "item_child"

    denied = await SubagentTool(_FailingBackend()).execute(
        ToolCall("call_depth", "subagent", {"prompt": "nested"}),
        cancellation=CancellationToken(),
        context=context,
    )
    assert not denied.ok
    assert denied.details["errorCode"] == "subagent_depth_limit"

@pytest.mark.asyncio
async def test_subagent_tool_returns_the_full_summary_without_truncation() -> None:
    long_summary = "R" * 5_000
    tool = SubagentTool(
        _RecordingBackend(
            {
                "threadId": "thread_child",
                "runId": "run_child",
                "status": "completed",
                "summary": long_summary,
            }
        )
    )
    context = ToolExecutionContext("run_parent", 1, "item_child", "thread_parent", None)
    result = await tool.execute(
        ToolCall("call_long", "subagent", {"prompt": "long"}),
        cancellation=CancellationToken(),
        context=context,
    )
    assert result.ok
    assert result.output == long_summary
    assert result.details == {
        "threadId": "thread_child",
        "runId": "run_child",
        "status": "completed",
    }