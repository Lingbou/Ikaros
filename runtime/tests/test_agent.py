from __future__ import annotations

import asyncio
import json
from collections.abc import AsyncIterator
from pathlib import Path

import pytest

from ikaros_runtime.agent.loop import AgentLoop
from ikaros_runtime.agent.scheduler import AgentScheduler
from ikaros_runtime.cancellation import CancellationToken, RunCancelled
from ikaros_runtime.domain import JournalEvent, WorkspaceSummary
from ikaros_runtime.providers.base import (
    ProviderEvent,
    ProviderRequest,
    ReasoningDelta,
    ResponseCompleted,
    TextDelta,
    ToolCallCompleted,
)
from ikaros_runtime.storage import SqliteRuntimeStore
from ikaros_runtime.tools.core import (
    ToolCall,
    ToolDefinition,
    ToolExecutor,
    ToolRegistry,
    ToolResult,
)
from ikaros_runtime.tools.policy import FullAccessPolicy


class FailingProvider:
    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]:
        cancellation.raise_if_cancelled()
        await asyncio.sleep(0)
        if request.messages:
            raise RuntimeError("expected provider failure")
        yield TextDelta("unreachable")


class FailingOnceExecutor:
    def __init__(self) -> None:
        self.calls: list[str] = []
        self.second_completed = asyncio.Event()

    async def run(self, run_id: str, cancellation: CancellationToken) -> None:
        cancellation.raise_if_cancelled()
        self.calls.append(run_id)
        if len(self.calls) == 1:
            raise RuntimeError("expected executor failure")
        self.second_completed.set()

    async def cancel(self, run_id: str) -> None:
        del run_id


class PausingProvider:
    def __init__(self) -> None:
        self.first_delta_emitted = asyncio.Event()

    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]:
        del request
        yield TextDelta("partial")
        self.first_delta_emitted.set()
        await cancellation.sleep(60)
        yield TextDelta("late")
        yield ResponseCompleted()


class BlockingExecutor:
    def __init__(self) -> None:
        self.calls: list[str] = []
        self.cancelled_queued: list[str] = []
        self.first_started = asyncio.Event()

    async def run(self, run_id: str, cancellation: CancellationToken) -> None:
        self.calls.append(run_id)
        self.first_started.set()
        try:
            await cancellation.sleep(60)
        except RunCancelled:
            return

    async def cancel(self, run_id: str) -> None:
        self.cancelled_queued.append(run_id)


class RecordingTool:
    definition = ToolDefinition(
        name="process_run",
        description="test process tool",
        input_schema={"type": "object"},
    )

    def __init__(self) -> None:
        self.calls: list[ToolCall] = []
        self.default_cwds: list[str | None] = []

    async def execute(
        self,
        call: ToolCall,
        *,
        cancellation: CancellationToken,
        default_cwd: str | None = None,
    ) -> ToolResult:
        cancellation.raise_if_cancelled()
        self.calls.append(call)
        self.default_cwds.append(default_cwd)
        return ToolResult(
            tool_call_id=call.id,
            tool_name=call.name,
            ok=True,
            output="tool-output",
            details={
                "stdout": "tool-output\n",
                "stderr": "",
                "exitCode": 0,
                "durationMs": 3,
                "timedOut": False,
                "truncated": False,
            },
        )


class ProtectedResultTool(RecordingTool):
    def __init__(self, protected: str) -> None:
        super().__init__()
        self.protected = protected

    async def execute(
        self,
        call: ToolCall,
        *,
        cancellation: CancellationToken,
        default_cwd: str | None = None,
    ) -> ToolResult:
        cancellation.raise_if_cancelled()
        del default_cwd
        self.calls.append(call)
        return ToolResult(
            tool_call_id=self.protected,
            tool_name=self.protected,
            ok=True,
            output="otherwise-safe",
            details={self.protected: "otherwise-safe"},
        )


class ToolLoopProvider:
    def __init__(
        self,
        *,
        always_call: bool = False,
        reasoning_content: str | None = None,
    ) -> None:
        self.requests: list[ProviderRequest] = []
        self.always_call = always_call
        self.reasoning_content = reasoning_content

    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]:
        cancellation.raise_if_cancelled()
        self.requests.append(request)
        has_tool_result = any(message.role == "tool" for message in request.messages)
        if self.always_call or not has_tool_result:
            if self.reasoning_content is not None:
                yield ReasoningDelta(self.reasoning_content)
            yield ToolCallCompleted(
                ToolCall(
                    id=f"call-{len(self.requests)}",
                    name="process_run",
                    arguments={"command": "test-command"},
                )
            )
        else:
            yield TextDelta("final answer")
        yield ResponseCompleted()


@pytest.mark.asyncio
async def test_provider_failure_settles_run_once_without_an_empty_message(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    events: list[JournalEvent] = []

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    try:
        thread, _ = store.create_thread("Provider failure")
        prepared = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="fail safely",
            provider_id="failing",
            model_id="failing-v1",
        )
        loop = AgentLoop(store, {"failing": FailingProvider()}, publish)

        await loop.run(prepared.run_id, CancellationToken())

        run_events = [event for event in events if event.run_id == prepared.run_id]
        settled = [event for event in run_events if event.type == "run.settled"]
        assistant_terminal = [
            event
            for event in run_events
            if event.type == "item.completed"
            and event.payload.get("item", {}).get("role") == "assistant"
        ]
        assert len(settled) == 1
        assert settled[0].payload["status"] == "failed"
        assert assistant_terminal == []
    finally:
        store.close()


@pytest.mark.asyncio
async def test_running_provider_stops_after_cancellation_and_preserves_partial_text(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    events: list[JournalEvent] = []

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    try:
        thread, _ = store.create_thread("Cancellation")
        prepared = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="cancel me",
            provider_id="pausing",
            model_id="pausing-v1",
        )
        provider = PausingProvider()
        loop = AgentLoop(store, {"pausing": provider}, publish)
        cancellation = CancellationToken()
        task = asyncio.create_task(loop.run(prepared.run_id, cancellation))

        await asyncio.wait_for(provider.first_delta_emitted.wait(), timeout=1)
        cancellation.cancel()
        await asyncio.wait_for(task, timeout=1)

        deltas = [
            event.payload["delta"]
            for event in events
            if event.type == "item.delta" and event.run_id == prepared.run_id
        ]
        terminal = [
            event
            for event in events
            if event.run_id == prepared.run_id and event.type in {"item.completed", "run.settled"}
        ]
        assert deltas == ["partial"]
        assert [event.type for event in terminal] == ["item.completed", "run.settled"]
        assert terminal[0].payload["item"]["content"] == "partial"
        assert terminal[0].payload["item"]["status"] == "cancelled"
        assert terminal[1].payload["status"] == "cancelled"
    finally:
        store.close()


@pytest.mark.asyncio
async def test_agent_executes_a_scripted_tool_loop_and_persists_provider_context(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    events: list[JournalEvent] = []

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    try:
        thread, _ = store.create_thread("Tool loop")
        prepared = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="run the tool",
            provider_id="tool-loop",
            model_id="tool-loop-v1",
        )
        provider = ToolLoopProvider(reasoning_content="tool reasoning")
        tool = RecordingTool()
        executor = ToolExecutor(ToolRegistry([tool]), FullAccessPolicy())
        loop = AgentLoop(store, {"tool-loop": provider}, publish, executor)

        await loop.run(prepared.run_id, CancellationToken())

        assert [call.id for call in tool.calls] == ["call-1"]
        assert tool.default_cwds == [None]
        assert len(provider.requests) == 2
        second_messages = provider.requests[1].messages
        assert [message.role for message in second_messages] == [
            "user",
            "assistant",
            "tool",
        ]
        assert second_messages[1].tool_calls == (
            ToolCall("call-1", "process_run", {"command": "test-command"}),
        )
        assert second_messages[1].reasoning_content == "tool reasoning"
        assert second_messages[2].tool_call_id == "call-1"
        tool_content = json.loads(second_messages[2].content)
        assert tool_content["toolCallId"] == "call-1"
        assert tool_content["stdout"] == "tool-output\n"

        run_events = [event for event in events if event.run_id == prepared.run_id]
        item_kinds = [
            event.payload["item"]["kind"]
            for event in run_events
            if event.type in {"item.started", "item.completed"} and "item" in event.payload
        ]
        assert item_kinds == [
            "tool_call",
            "tool_call",
            "tool_result",
            "message",
            "message",
        ]
        assert [event.type for event in run_events] == [
            "run.state_changed",
            "item.started",
            "item.completed",
            "item.completed",
            "item.started",
            "item.delta",
            "item.completed",
            "run.settled",
        ]
        assert not any(event.type.startswith("permission.") for event in run_events)
        assert run_events[-1].payload["status"] == "completed"

        before_rebuild = store.context_items(
            thread.default_branch_id,
            through_turn_id=prepared.turn_id,
        )
        store.rebuild_projections()
        assert (
            store.context_items(
                thread.default_branch_id,
                through_turn_id=prepared.turn_id,
            )
            == before_rebuild
        )
    finally:
        store.close()


@pytest.mark.asyncio
async def test_agent_passes_the_thread_workspace_to_tool_execution(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")

    async def publish(_event: JournalEvent) -> None:
        return

    try:
        workspace = tmp_path / "workspace"
        workspace.mkdir()
        thread, _ = store.create_thread(
            "Workspace tool loop",
            workspace=WorkspaceSummary(
                id="workspace-test",
                name="Workspace",
                root_uri=str(workspace.resolve()),
            ),
        )
        prepared = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="run in the workspace",
            provider_id="tool-loop",
            model_id="tool-loop-v1",
        )
        provider = ToolLoopProvider()
        tool = RecordingTool()
        loop = AgentLoop(
            store,
            {"tool-loop": provider},
            publish,
            ToolExecutor(ToolRegistry([tool]), FullAccessPolicy()),
        )

        await loop.run(prepared.run_id, CancellationToken())

        assert tool.default_cwds == [str(workspace.resolve())]
    finally:
        store.close()


@pytest.mark.asyncio
async def test_agent_replaces_a_tool_result_containing_protected_values(tmp_path: Path) -> None:
    protected = "protected-tool-result-sentinel"
    store = SqliteRuntimeStore(tmp_path / "state.db")
    events: list[JournalEvent] = []

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    try:
        thread, _ = store.create_thread("Protected tool result")
        prepared = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="run the tool safely",
            provider_id="tool-loop",
            model_id="tool-loop-v1",
        )
        provider = ToolLoopProvider()
        tool = ProtectedResultTool(protected)
        loop = AgentLoop(
            store,
            {"tool-loop": provider},
            publish,
            ToolExecutor(ToolRegistry([tool]), FullAccessPolicy()),
            protected_values=lambda: (protected,),
        )

        await loop.run(prepared.run_id, CancellationToken())

        assert len(provider.requests) == 2
        tool_message = provider.requests[1].messages[-1]
        result = json.loads(tool_message.content)
        assert result["toolCallId"] == "call-1"
        assert result["toolName"] == "process_run"
        assert result["ok"] is False
        assert result["errorCode"] == "protected_output"
        assert result["stdout"] == ""
        assert result["stderr"] == ""
        serialized_events = json.dumps([event.to_wire() for event in events])
        assert protected not in serialized_events
        assert protected.encode() not in (tmp_path / "state.db").read_bytes()
    finally:
        store.close()


@pytest.mark.asyncio
async def test_agent_step_limit_settles_an_infinite_tool_loop_once(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    events: list[JournalEvent] = []

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    try:
        thread, _ = store.create_thread("Step bound")
        prepared = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="loop forever",
            provider_id="tool-loop",
            model_id="tool-loop-v1",
        )
        provider = ToolLoopProvider(always_call=True)
        tool = RecordingTool()
        loop = AgentLoop(
            store,
            {"tool-loop": provider},
            publish,
            ToolExecutor(ToolRegistry([tool]), FullAccessPolicy()),
            max_steps=2,
        )

        await loop.run(prepared.run_id, CancellationToken())

        settled = [
            event
            for event in events
            if event.run_id == prepared.run_id and event.type == "run.settled"
        ]
        assert len(provider.requests) == 2
        assert len(tool.calls) == 2
        assert len(settled) == 1
        assert settled[0].payload["status"] == "failed"
    finally:
        store.close()


@pytest.mark.asyncio
async def test_queued_run_is_cancelled_without_calling_executor() -> None:
    executor = BlockingExecutor()
    scheduler = AgentScheduler(executor)
    scheduler.start()
    try:
        await scheduler.enqueue("run-first")
        await asyncio.wait_for(executor.first_started.wait(), timeout=1)
        await scheduler.enqueue("run-queued")

        assert await scheduler.cancel("run-queued") is True
        assert executor.calls == ["run-first"]
        assert executor.cancelled_queued == ["run-queued"]
        assert await scheduler.cancel("run-first") is True
    finally:
        await asyncio.wait_for(scheduler.close(), timeout=1)


@pytest.mark.asyncio
async def test_cancelling_reserved_head_wakes_next_activated_run() -> None:
    executor = BlockingExecutor()
    scheduler = AgentScheduler(executor)
    scheduler.start()
    try:
        scheduler.reserve("run-reserved")
        await scheduler.enqueue("run-next")
        await asyncio.sleep(0)
        assert executor.calls == []

        assert await scheduler.cancel("run-reserved") is True
        await asyncio.wait_for(executor.first_started.wait(), timeout=1)
        assert executor.calls == ["run-next"]
        assert executor.cancelled_queued == ["run-reserved"]
        assert await scheduler.cancel("run-next") is True
    finally:
        await asyncio.wait_for(scheduler.close(), timeout=1)


@pytest.mark.asyncio
async def test_scheduler_close_wakes_and_terminalizes_reserved_head() -> None:
    executor = BlockingExecutor()
    scheduler = AgentScheduler(executor)
    scheduler.start()
    scheduler.reserve("run-reserved")
    await asyncio.sleep(0)

    await asyncio.wait_for(scheduler.close(), timeout=1)

    assert executor.calls == []
    assert executor.cancelled_queued == ["run-reserved"]


@pytest.mark.asyncio
async def test_scheduler_continues_after_an_unhandled_run_failure() -> None:
    executor = FailingOnceExecutor()
    scheduler = AgentScheduler(executor)
    scheduler.start()
    try:
        await scheduler.enqueue("run-first")
        await scheduler.enqueue("run-second")
        await asyncio.wait_for(executor.second_completed.wait(), timeout=1)
        assert executor.calls == ["run-first", "run-second"]
    finally:
        await scheduler.close()
