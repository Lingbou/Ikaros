"""Subagent tool: child sessions spawned from inside a Run."""

from __future__ import annotations

import asyncio
import json
from collections.abc import AsyncIterator
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import pytest

from ikaros_runtime.agent import AgentLoop, AgentScheduler
from ikaros_runtime.cancellation import CancellationToken
from ikaros_runtime.domain import JournalEvent, JsonObject
from ikaros_runtime.providers.base import (
    ProviderEvent,
    ProviderRequest,
    ResponseCompleted,
    TextDelta,
    ToolCallCompleted,
)
from ikaros_runtime.services.subagents import (
    DEFAULT_SUBAGENT_MAX_DEPTH,
    MAX_SUBAGENT_MAX_DEPTH,
    MIN_SUBAGENT_MAX_DEPTH,
    SUBAGENT_MAX_ACTIVE_PER_RUN,
    SUBAGENT_MAX_DEPTH_ENV,
    SubagentError,
    SubagentService,
    subagent_depth_limit,
)
from ikaros_runtime.storage import SqliteRuntimeStore
from ikaros_runtime.tools import (
    FullAccessPolicy,
    SubagentTool,
    ThreadReadTool,
    ToolCall,
    ToolDefinition,
    ToolExecutionContext,
    ToolExecutor,
    ToolRegistry,
    ToolResult,
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
        raise SubagentError(
            "subagent_depth_limit",
            "Subagent depth limit reached (max 2); this Run is already at depth 2.",
        )


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
        del request
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
        del request, cancellation
        yield TextDelta(_LONG_ANSWER)
        yield ResponseCompleted()


class _NestingProvider:
    """Ask every child to spawn one more child, then answer."""

    id = "scripted"
    model_id = "scripted-v1"

    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]:
        del cancellation
        users = [message.content for message in request.messages if message.role == "user"]
        tool_results = [
            message for message in request.messages if message.role == "tool"
        ]
        if tool_results:
            latest = json.loads(tool_results[-1].content)
            if latest.get("errorCode") == "subagent_depth_limit":
                yield TextDelta("depth limit reached")
            else:
                yield TextDelta(f"finished {users[-1]}")
            yield ResponseCompleted()
            return
        yield ToolCallCompleted(
            ToolCall(
                id=f"call_nesting_{len(users)}",
                name="subagent",
                arguments={"prompt": f"{users[-1]} child"},
            )
        )
        yield ResponseCompleted()


class _DispatchProvider:
    """Parent dispatches a slow child with wait=false, then keeps working."""

    id = "scripted"
    model_id = "scripted-v1"

    def __init__(self, child_delay: float = 1.0) -> None:
        self.child_delay = child_delay

    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]:
        users = [message.content for message in request.messages if message.role == "user"]
        tool_results = [
            message for message in request.messages if message.role == "tool"
        ]
        last_user = users[-1]
        if last_user == "parent dispatch task" and not tool_results:
            yield ToolCallCompleted(
                ToolCall(
                    id="call_dispatch_child",
                    name="subagent",
                    arguments={"prompt": "child work", "wait": False},
                )
            )
            yield ResponseCompleted()
            return
        if last_user == "child work":
            await cancellation.sleep(self.child_delay)
            yield TextDelta("child finished")
            yield ResponseCompleted()
            return
        if tool_results:
            yield TextDelta("parent continued")
            yield ResponseCompleted()
            return
        yield TextDelta(f"unexpected task: {last_user}")
        yield ResponseCompleted()


class _InlineProvider:
    """Parent waits for the child inline; the scheduler has only one worker."""

    id = "scripted"
    model_id = "scripted-v1"

    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]:
        del cancellation
        users = [message.content for message in request.messages if message.role == "user"]
        tool_results = [
            message for message in request.messages if message.role == "tool"
        ]
        last_user = users[-1]
        if last_user == "inline parent task" and not tool_results:
            yield ToolCallCompleted(
                ToolCall(
                    id="call_inline_child",
                    name="subagent",
                    arguments={"prompt": "inline child task"},
                )
            )
            yield ResponseCompleted()
            return
        if last_user == "inline child task":
            yield TextDelta("inline child done")
            yield ResponseCompleted()
            return
        if tool_results:
            yield TextDelta("inline parent done")
            yield ResponseCompleted()
            return
        yield TextDelta(f"unexpected task: {last_user}")
        yield ResponseCompleted()


@dataclass
class _Harness:
    store: SqliteRuntimeStore
    service: SubagentService
    events: list[JournalEvent]
    loop: AgentLoop
    scheduler: AgentScheduler
    parent_run_id: str
    parent_thread_id: str
    enqueued: list[str]

    async def aclose(self) -> None:
        self.service.close()
        await self.scheduler.close()
        self.store.close()


def _setup(
    tmp_path: Path,
    *,
    provider: Any = None,
    max_depth: int = DEFAULT_SUBAGENT_MAX_DEPTH,
    max_workers: int = 2,
    max_active_per_run: int = SUBAGENT_MAX_ACTIVE_PER_RUN,
    parent_content: str = "parent task",
) -> _Harness:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    events: list[JournalEvent] = []
    enqueued: list[str] = []
    runtime: dict[str, Any] = {}

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    async def run_child(run_id: str, cancellation: CancellationToken) -> None:
        await runtime["loop"].run(run_id, cancellation)

    async def enqueue(run_id: str) -> None:
        enqueued.append(run_id)
        await runtime["scheduler"].enqueue(run_id)

    async def cancel(run_id: str) -> None:
        await runtime["scheduler"].cancel(run_id)

    service = SubagentService(
        store=store,
        run_child=run_child,
        publish=publish,
        enqueue=enqueue,
        cancel=cancel,
        max_depth=max_depth,
        max_active_per_run=max_active_per_run,
    )
    executor = ToolExecutor(ToolRegistry([SubagentTool(service)]), FullAccessPolicy())
    active_provider = provider if provider is not None else ScriptedProvider()

    def notify(config: Any, status: str) -> None:
        del status
        service.notify_run_terminal(config.run_id)

    loop = AgentLoop(
        store,
        {ScriptedProvider.id: active_provider},
        publish,
        executor,
        on_run_terminal=notify,
    )
    runtime["loop"] = loop
    scheduler = AgentScheduler(loop, max_workers=max_workers)
    runtime["scheduler"] = scheduler
    scheduler.start()
    thread, _created = store.create_thread("Parent")
    turn = prepare_turn(
        store,
        thread_id=thread.id,
        branch_id=thread.default_branch_id,
        content=parent_content,
        provider_id="scripted",
        model_id="scripted-v1",
        tools=executor.definitions,
    )
    return _Harness(
        store=store,
        service=service,
        events=events,
        loop=loop,
        scheduler=scheduler,
        parent_run_id=turn.run_id,
        parent_thread_id=thread.id,
        enqueued=enqueued,
    )


async def _wait_for_status(
    store: SqliteRuntimeStore,
    run_id: str,
    expected: set[str],
    *,
    deadline_seconds: float = 5.0,
) -> str:
    deadline = asyncio.get_running_loop().time() + deadline_seconds
    while True:
        status = store.run_status(run_id)
        if status in expected:
            return status
        if asyncio.get_running_loop().time() >= deadline:
            raise AssertionError(
                f"run {run_id} did not reach {sorted(expected)}; last status {status}"
            )
        await asyncio.sleep(0.01)


async def _wait_for_depth(
    service: SubagentService,
    run_id: str,
    expected: int,
    *,
    deadline_seconds: float = 5.0,
) -> None:
    deadline = asyncio.get_running_loop().time() + deadline_seconds
    while service.depth(run_id) != expected:
        if asyncio.get_running_loop().time() >= deadline:
            raise AssertionError(
                f"run {run_id} depth did not reach {expected}; "
                f"last depth {service.depth(run_id)}"
            )
        await asyncio.sleep(0.01)


def _thread_by_title(store: SqliteRuntimeStore, title: str) -> str:
    page = store.list_thread_page(cursor=None, limit=50, archived=False)
    thread = next((candidate for candidate in page.threads if candidate.title == title), None)
    if thread is None:
        raise AssertionError(f"thread {title!r} was not found")
    return thread.id


def _run_id_for_thread(store: SqliteRuntimeStore, thread_id: str) -> str:
    payload = store.read_thread_slice(thread_id=thread_id)
    run_ids = {
        str(item["runId"])
        for item in payload["items"]
        if isinstance(item, dict) and item.get("runId")
    }
    if len(run_ids) == 1:
        return run_ids.pop()
    active_run = payload["thread"].get("activeRun")
    if isinstance(active_run, dict) and active_run.get("id"):
        return str(active_run["id"])
    raise AssertionError(f"thread {thread_id} does not expose one Run ID")


def _assert_journal_order(events: list[JournalEvent]) -> None:
    sequences = [event.seq for event in events]
    assert sequences == sorted(sequences)
    assert len(set(sequences)) == len(sequences)


def _assert_run_settles_last(events: list[JournalEvent], run_id: str) -> None:
    run_events = [event for event in events if event.run_id == run_id]
    assert run_events, f"run {run_id} published no events"
    assert run_events[-1].type == "run.settled"
    states = [
        event.payload.get("status")
        for event in run_events
        if event.type == "run.state_changed"
    ]
    assert states
    assert states[-1] == "running"
    assert all(state in {"queued", "running"} for state in states)


def test_subagent_depth_limit_environment() -> None:
    assert DEFAULT_SUBAGENT_MAX_DEPTH == 2
    assert MIN_SUBAGENT_MAX_DEPTH == 1
    assert MAX_SUBAGENT_MAX_DEPTH == 8
    assert subagent_depth_limit({}) == 2
    assert subagent_depth_limit({SUBAGENT_MAX_DEPTH_ENV: "3"}) == 3
    for raw in ("0", "9", "not-a-number", ""):
        with pytest.raises(ValueError):
            subagent_depth_limit({SUBAGENT_MAX_DEPTH_ENV: raw})


@pytest.mark.asyncio
async def test_subagent_nests_to_configured_depth_and_rejects_beyond(
    tmp_path: Path,
) -> None:
    harness = _setup(tmp_path, provider=_NestingProvider(), max_depth=2)
    try:
        await harness.loop.run(harness.parent_run_id, CancellationToken())

        assert harness.store.run_status(harness.parent_run_id) == "completed"
        child_thread_id = _thread_by_title(harness.store, "Subagent: parent task child")
        grandchild_thread_id = _thread_by_title(
            harness.store, "Subagent: parent task child child"
        )
        child_run_id = _run_id_for_thread(harness.store, child_thread_id)
        grandchild_run_id = _run_id_for_thread(harness.store, grandchild_thread_id)
        assert harness.store.run_status(child_run_id) == "completed"
        assert harness.store.run_status(grandchild_run_id) == "completed"
        assert harness.store.final_assistant_message(grandchild_thread_id) == (
            "depth limit reached"
        )
        titles = [
            thread.title
            for thread in harness.store.list_thread_page(
                cursor=None, limit=50, archived=False
            ).threads
        ]
        assert "Subagent: parent task child child child" not in titles
        grandchild_items = harness.store.read_thread_slice(thread_id=grandchild_thread_id)[
            "items"
        ]
        assert any(
            isinstance(item, dict)
            and "subagent_depth_limit" in str(item.get("content", ""))
            for item in grandchild_items
        )
        assert harness.service.depth(grandchild_run_id) == 0
        _assert_journal_order(harness.events)
        for run_id in (child_run_id, grandchild_run_id):
            _assert_run_settles_last(harness.events, run_id)
    finally:
        await harness.aclose()


@pytest.mark.asyncio
async def test_subagent_wait_false_parent_completion_does_not_kill_child(
    tmp_path: Path,
) -> None:
    harness = _setup(
        tmp_path,
        provider=_DispatchProvider(child_delay=1.0),
        parent_content="parent dispatch task",
        max_workers=2,
    )
    try:
        await harness.loop.run(harness.parent_run_id, CancellationToken())
        assert harness.store.run_status(harness.parent_run_id) == "completed"

        child_thread_id = _thread_by_title(harness.store, "Subagent: child work")
        child_run_id = _run_id_for_thread(harness.store, child_thread_id)
        assert harness.enqueued == [child_run_id]
        assert harness.store.run_status(child_run_id) in {"queued", "running"}
        read_tool = ThreadReadTool(harness.service.read_thread)
        read_result = await read_tool.execute(
            ToolCall("call_read_child", "thread_read", {"threadId": child_thread_id}),
            cancellation=CancellationToken(),
            context=ToolExecutionContext(
                harness.parent_run_id,
                1,
                "item_read_child",
                harness.parent_thread_id,
                None,
            ),
        )
        assert read_result.ok
        active_payload = json.loads(read_result.output)
        assert active_payload["thread"]["activeRun"] == {
            "id": child_run_id,
            "status": harness.store.run_status(child_run_id),
        }
        assert "active Run" in active_payload["thread"]["notice"]

        assert (
            await _wait_for_status(
                harness.store, child_run_id, {"completed"}, deadline_seconds=5
            )
            == "completed"
        )
        assert harness.store.final_assistant_message(child_thread_id) == "child finished"
        settled_result = await read_tool.execute(
            ToolCall("call_read_child_settled", "thread_read", {"threadId": child_thread_id}),
            cancellation=CancellationToken(),
            context=ToolExecutionContext(
                harness.parent_run_id,
                2,
                "item_read_child_settled",
                harness.parent_thread_id,
                None,
            ),
        )
        assert settled_result.ok
        settled_payload = json.loads(settled_result.output)
        assert settled_payload["thread"].get("activeRun") is None
        _assert_journal_order(harness.events)
        _assert_run_settles_last(harness.events, child_run_id)
    finally:
        await harness.aclose()


@pytest.mark.asyncio
async def test_subagent_parent_cancellation_propagates_to_non_blocking_child(
    tmp_path: Path,
) -> None:
    harness = _setup(tmp_path, provider=_SlowProvider(), max_workers=1)
    token = CancellationToken()
    try:
        result = await harness.service.spawn(
            parent_run_id=harness.parent_run_id,
            request_id="item_child_cancel_non_blocking",
            prompt="slow child",
            title=None,
            wait=False,
            cancellation=token,
        )
        child_run_id = str(result["runId"])
        assert result["status"] in {"queued", "running"}
        assert harness.service.depth(child_run_id) == 1

        token.cancel()
        assert (
            await _wait_for_status(
                harness.store, child_run_id, {"cancelled"}, deadline_seconds=5
            )
            == "cancelled"
        )
        await _wait_for_depth(harness.service, child_run_id, 0)
    finally:
        await harness.aclose()


@pytest.mark.asyncio
async def test_subagent_non_blocking_concurrency_limit(tmp_path: Path) -> None:
    harness = _setup(tmp_path, provider=_SlowProvider(), max_workers=1)
    token = CancellationToken()
    try:
        results = []
        for index in range(SUBAGENT_MAX_ACTIVE_PER_RUN):
            results.append(
                await harness.service.spawn(
                    parent_run_id=harness.parent_run_id,
                    request_id=f"item_child_limit_{index}",
                    prompt=f"slow child {index}",
                    title=None,
                    wait=False,
                    cancellation=token,
                )
            )
        with pytest.raises(SubagentError) as raised:
            await harness.service.spawn(
                parent_run_id=harness.parent_run_id,
                request_id="item_child_limit_overflow",
                prompt="slow child overflow",
                title=None,
                wait=False,
                cancellation=token,
            )
        assert raised.value.code == "subagent_concurrency_limit"

        token.cancel()
        for result in results:
            await _wait_for_status(
                harness.store, str(result["runId"]), {"cancelled"}, deadline_seconds=5
            )
    finally:
        await harness.aclose()


@pytest.mark.asyncio
async def test_subagent_wait_true_inline_avoids_scheduler_deadlock(
    tmp_path: Path,
) -> None:
    harness = _setup(
        tmp_path,
        provider=_InlineProvider(),
        max_workers=1,
        parent_content="inline parent task",
    )
    try:
        await harness.scheduler.enqueue(harness.parent_run_id)
        await _wait_for_status(
            harness.store, harness.parent_run_id, {"completed"}, deadline_seconds=5
        )
        child_thread_id = _thread_by_title(harness.store, "Subagent: inline child task")
        child_run_id = _run_id_for_thread(harness.store, child_thread_id)
        assert harness.store.run_status(child_run_id) == "completed"
        assert harness.store.final_assistant_message(child_thread_id) == "inline child done"
        assert harness.store.final_assistant_message(harness.parent_thread_id) == (
            "inline parent done"
        )
        assert harness.enqueued == []
        _assert_journal_order(harness.events)
        _assert_run_settles_last(harness.events, child_run_id)
        _assert_run_settles_last(harness.events, harness.parent_run_id)
    finally:
        await harness.aclose()


@pytest.mark.asyncio
async def test_subagent_wait_false_replays_idempotently(tmp_path: Path) -> None:
    harness = _setup(tmp_path, provider=_SlowProvider(), max_workers=1)
    token = CancellationToken()
    try:
        first = await harness.service.spawn(
            parent_run_id=harness.parent_run_id,
            request_id="item_child_replay_non_blocking",
            prompt="slow replay child",
            title=None,
            wait=False,
            cancellation=token,
        )
        second = await harness.service.spawn(
            parent_run_id=harness.parent_run_id,
            request_id="item_child_replay_non_blocking",
            prompt="slow replay child",
            title=None,
            wait=False,
            cancellation=token,
        )
        assert second["threadId"] == first["threadId"]
        assert second["runId"] == first["runId"]
        assert harness.enqueued == [first["runId"]]

        token.cancel()
        await _wait_for_status(
            harness.store, str(first["runId"]), {"cancelled"}, deadline_seconds=5
        )
    finally:
        await harness.aclose()


@pytest.mark.asyncio
async def test_subagent_spawns_a_child_thread_and_returns_the_final_message(
    tmp_path: Path,
) -> None:
    harness = _setup(tmp_path)
    try:
        result = await harness.service.spawn(
            parent_run_id=harness.parent_run_id,
            request_id="item_child_one",
            prompt="child task",
            title=None,
            cancellation=CancellationToken(),
        )
        assert result["status"] == "completed"
        assert result["summary"] == "Scripted response to: child task"
        assert result["threadId"] != harness.parent_thread_id
        child = next(
            thread
            for thread in harness.store.list_thread_page(
                cursor=None, limit=50, archived=False
            ).threads
            if thread.id == result["threadId"]
        )
        assert child.title == "Subagent: child task"
        assert harness.store.run_status(str(result["runId"])) == "completed"
        _assert_run_settles_last(harness.events, str(result["runId"]))
    finally:
        await harness.aclose()


@pytest.mark.asyncio
async def test_subagent_waits_for_slow_child_work_without_a_wall_clock_cutoff(
    tmp_path: Path,
) -> None:
    harness = _setup(tmp_path, provider=_SlowProvider(delay=0.2))
    try:
        result = await harness.service.spawn(
            parent_run_id=harness.parent_run_id,
            request_id="item_child_slow",
            prompt="slow child",
            title=None,
            cancellation=CancellationToken(),
        )
        assert result["status"] == "completed"
        assert result["summary"] == "slow child finished"
    finally:
        await harness.aclose()


@pytest.mark.asyncio
async def test_subagent_parent_cancellation_cancels_the_inline_child(
    tmp_path: Path,
) -> None:
    harness = _setup(tmp_path, provider=_SlowProvider())
    token = CancellationToken()
    try:
        task = asyncio.create_task(
            harness.service.spawn(
                parent_run_id=harness.parent_run_id,
                request_id="item_child_cancel_inline",
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
        await harness.aclose()


@pytest.mark.asyncio
async def test_subagent_returns_the_full_final_message_without_truncation(
    tmp_path: Path,
) -> None:
    harness = _setup(tmp_path, provider=_LongAnswerProvider())
    try:
        result = await harness.service.spawn(
            parent_run_id=harness.parent_run_id,
            request_id="item_child_long",
            prompt="long child",
            title=None,
            cancellation=CancellationToken(),
        )
        assert result["status"] == "completed"
        assert result["summary"] == _LONG_ANSWER
    finally:
        await harness.aclose()


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
    invalid_wait = await tool.execute(
        ToolCall("call_bad_wait", "subagent", {"prompt": "x", "wait": "yes"}),
        cancellation=CancellationToken(),
        context=context,
    )
    assert not invalid_wait.ok
    assert invalid_wait.details["errorCode"] == "invalid_arguments"
    accepted = await tool.execute(
        ToolCall("call_ok", "subagent", {"prompt": "do it", "title": "Child"}),
        cancellation=CancellationToken(),
        context=context,
    )
    assert accepted.ok
    assert accepted.output == "done"
    assert accepted.details["threadId"] == "thread_child"
    assert backend.calls and backend.calls[-1]["request_id"] == "item_child"
    assert backend.calls[-1]["wait"] is True

    dispatch_backend = _RecordingBackend(
        {
            "threadId": "thread_dispatched",
            "runId": "run_dispatched",
            "status": "queued",
            "summary": "",
        }
    )
    dispatched = await SubagentTool(dispatch_backend).execute(
        ToolCall("call_dispatch", "subagent", {"prompt": "later", "wait": False}),
        cancellation=CancellationToken(),
        context=context,
    )
    assert dispatched.ok
    assert dispatched.details["status"] == "queued"
    assert "dispatched" in dispatched.output
    assert "thread_dispatched" in dispatched.output
    assert dispatch_backend.calls[-1]["wait"] is False

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

class _MutatingProbeTool:
    """A mutating Tool the child Run calls while the parent holds its gate."""

    def __init__(self) -> None:
        self.calls = 0

    @property
    def definition(self) -> ToolDefinition:
        return ToolDefinition(
            name="probe_write",
            description="probe",
            input_schema={
                "type": "object",
                "properties": {},
                "additionalProperties": False,
            },
            mutating=True,
        )

    async def execute(
        self,
        call: ToolCall,
        *,
        cancellation: CancellationToken,
        context: ToolExecutionContext,
    ) -> ToolResult:
        self.calls += 1
        return ToolResult(
            tool_call_id=call.id,
            tool_name=call.name,
            ok=True,
            output="probe ok",
            details={"durationMs": 0, "truncated": False},
        )


class _NestedProvider:
    """Parent delegates, the child mutates, then both answer."""

    id = "scripted"
    model_id = "scripted-v1"

    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]:
        cancellation.raise_if_cancelled()
        users = [message.content for message in request.messages if message.role == "user"]
        has_results = any(message.role == "tool" for message in request.messages)
        last_user = users[-1] if users else ""
        if last_user == "parent task" and not has_results:
            yield ToolCallCompleted(
                ToolCall(
                    id="call_parent",
                    name="subagent",
                    arguments={"prompt": "child task"},
                )
            )
            yield ResponseCompleted()
            return
        if last_user == "child task" and not has_results:
            yield ToolCallCompleted(
                ToolCall(id="call_child", name="probe_write", arguments={})
            )
            yield ResponseCompleted()
            return
        yield TextDelta("done")
        yield ResponseCompleted()


@pytest.mark.asyncio
async def test_inline_subagent_child_can_use_mutating_tools(tmp_path: Path) -> None:
    """A nested Run must not deadlock behind its caller's tool gate."""

    store = SqliteRuntimeStore(tmp_path / "state.db")
    events: list[JournalEvent] = []

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    probe = _MutatingProbeTool()
    loop_holder: dict[str, AgentLoop] = {}

    async def run_child(run_id: str, cancellation: CancellationToken) -> None:
        await loop_holder["loop"].run(run_id, cancellation)

    service = SubagentService(store=store, run_child=run_child, publish=publish)
    executor = ToolExecutor(
        ToolRegistry([probe, SubagentTool(service), ThreadReadTool(store.read_thread_slice)]),
        FullAccessPolicy(),
    )
    provider = _NestedProvider()
    loop = AgentLoop(store, {provider.id: provider}, publish, executor)
    loop_holder["loop"] = loop
    try:
        thread, _created = store.create_thread("Parent")
        prepared = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="parent task",
            provider_id=provider.id,
            model_id=provider.model_id,
            tools=executor.definitions,
        )

        await asyncio.wait_for(loop.run(prepared.run_id, CancellationToken()), timeout=15)

        assert store.run_status(prepared.run_id) == "completed"
        assert probe.calls == 1
    finally:
        store.close()