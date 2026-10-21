"""Tool calls from one model step run concurrently under a read/write gate."""

from __future__ import annotations

import asyncio
from collections.abc import AsyncIterator, Sequence
from pathlib import Path
from typing import Any

import pytest

from ikaros_runtime.agent import AgentLoop
from ikaros_runtime.cancellation import CancellationToken
from ikaros_runtime.domain import JournalEvent
from ikaros_runtime.providers.base import (
    ProviderEvent,
    ProviderRequest,
    ResponseCompleted,
    TextDelta,
    ToolCallCompleted,
)
from ikaros_runtime.storage import SqliteRuntimeStore
from ikaros_runtime.tools import (
    FullAccessPolicy,
    ToolCall,
    ToolDefinition,
    ToolExecutionContext,
    ToolExecutor,
    ToolRegistry,
    ToolResult,
)

from .helpers import prepare_turn

_PROVIDER_ID = "parallel"
_MODEL_ID = "parallel-v1"


class _Probe:
    """Record concurrency and let the test release each Tool by hand."""

    def __init__(self) -> None:
        self.active = 0
        self.max_active = 0
        self.started: dict[str, asyncio.Event] = {}
        self.release: dict[str, asyncio.Event] = {}

    def _events(self, name: str) -> tuple[asyncio.Event, asyncio.Event]:
        return (
            self.started.setdefault(name, asyncio.Event()),
            self.release.setdefault(name, asyncio.Event()),
        )

    def started_event(self, name: str) -> asyncio.Event:
        return self._events(name)[0]

    def release_event(self, name: str) -> asyncio.Event:
        return self._events(name)[1]

    async def wait_started(self, *names: str) -> None:
        await asyncio.wait_for(
            asyncio.gather(*(self.started_event(name).wait() for name in names)),
            timeout=5,
        )

    def release_all(self) -> None:
        for event in self.release.values():
            event.set()


class _ProbeTool:
    def __init__(self, name: str, probe: _Probe, *, mutating: bool = False) -> None:
        self.definition = ToolDefinition(
            name=name,
            description=f"probe tool {name}",
            input_schema={
                "type": "object",
                "properties": {},
                "additionalProperties": False,
            },
            mutating=mutating,
        )
        self._name = name
        self._probe = probe

    async def execute(
        self,
        call: ToolCall,
        *,
        cancellation: CancellationToken,
        context: ToolExecutionContext,
    ) -> ToolResult:
        probe = self._probe
        probe.active += 1
        probe.max_active = max(probe.max_active, probe.active)
        probe.started_event(self._name).set()
        try:
            await probe.release_event(self._name).wait()
        finally:
            probe.active -= 1
        return ToolResult(
            tool_call_id=call.id,
            tool_name=call.name,
            ok=True,
            output=f"{self._name} finished",
            details={"durationMs": 0, "truncated": False},
        )


class _ParallelProvider:
    """Emit every Tool call in the first step, then finish."""

    id = _PROVIDER_ID
    model_id = _MODEL_ID

    def __init__(self, names: Sequence[str]) -> None:
        self._names = tuple(names)
        self.requests: list[ProviderRequest] = []

    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]:
        cancellation.raise_if_cancelled()
        self.requests.append(request)
        if len(self.requests) == 1:
            for index, name in enumerate(self._names):
                yield ToolCallCompleted(
                    ToolCall(id=f"call-{index}", name=name, arguments={})
                )
            yield ResponseCompleted()
            return
        yield TextDelta("all tools finished")
        yield ResponseCompleted()


def _setup(
    tmp_path: Path,
    tools: Sequence[_ProbeTool],
    names: Sequence[str],
) -> tuple[SqliteRuntimeStore, AgentLoop, list[JournalEvent], str]:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    events: list[JournalEvent] = []
    provider = _ParallelProvider(names)

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    executor = ToolExecutor(ToolRegistry(list(tools)), FullAccessPolicy())
    thread, _ = store.create_thread("Parallel")
    prepared = prepare_turn(
        store,
        thread_id=thread.id,
        branch_id=thread.default_branch_id,
        content="run these tools together",
        provider_id=_PROVIDER_ID,
        model_id=_MODEL_ID,
        tools=executor.definitions,
    )
    loop = AgentLoop(store, {_PROVIDER_ID: provider}, publish, executor)
    return store, loop, events, prepared.run_id


def _tool_results(events: Sequence[JournalEvent]) -> list[dict[str, Any]]:
    return [
        event.payload["item"]
        for event in events
        if event.type == "item.completed"
        and event.payload.get("item", {}).get("kind") == "tool_result"
    ]


@pytest.mark.asyncio
async def test_read_only_tool_calls_overlap(tmp_path: Path) -> None:
    probe = _Probe()
    store, loop, events, run_id = _setup(
        tmp_path,
        [_ProbeTool("alpha", probe), _ProbeTool("beta", probe)],
        ["alpha", "beta"],
    )
    try:
        task = asyncio.create_task(loop.run(run_id, CancellationToken()))
        await probe.wait_started("alpha", "beta")
        assert probe.max_active == 2
        probe.release_all()
        await asyncio.wait_for(task, timeout=5)

        assert store.run_status(run_id) == "completed"
        assert [item["data"]["toolName"] for item in _tool_results(events)] == [
            "alpha",
            "beta",
        ]
    finally:
        store.close()


@pytest.mark.asyncio
async def test_mutating_tool_call_holds_the_gate_alone(tmp_path: Path) -> None:
    probe = _Probe()
    store, loop, events, run_id = _setup(
        tmp_path,
        [
            _ProbeTool("writer", probe, mutating=True),
            _ProbeTool("reader", probe),
        ],
        ["writer", "reader"],
    )
    try:
        task = asyncio.create_task(loop.run(run_id, CancellationToken()))
        await probe.wait_started("writer")
        await asyncio.sleep(0.05)
        assert probe.started_event("reader").is_set() is False
        assert probe.max_active == 1

        probe.release_event("writer").set()
        await probe.wait_started("reader")
        assert probe.max_active == 1
        probe.release_all()
        await asyncio.wait_for(task, timeout=5)

        assert store.run_status(run_id) == "completed"
        assert [item["data"]["toolName"] for item in _tool_results(events)] == [
            "writer",
            "reader",
        ]
    finally:
        store.close()


@pytest.mark.asyncio
async def test_tool_results_are_published_in_call_order(tmp_path: Path) -> None:
    probe = _Probe()
    store, loop, events, run_id = _setup(
        tmp_path,
        [_ProbeTool("first", probe), _ProbeTool("second", probe)],
        ["first", "second"],
    )
    try:
        task = asyncio.create_task(loop.run(run_id, CancellationToken()))
        await probe.wait_started("first", "second")
        # The second call finishes first; publication still follows call order.
        probe.release_event("second").set()
        await asyncio.sleep(0.05)
        assert not probe.release_event("first").is_set()
        probe.release_event("first").set()
        await asyncio.wait_for(task, timeout=5)

        assert store.run_status(run_id) == "completed"
        items = _tool_results(events)
        assert [item["data"]["toolName"] for item in items] == ["first", "second"]
        ordinals = [item["ordinal"] for item in items]
        assert ordinals == sorted(ordinals)
    finally:
        store.close()