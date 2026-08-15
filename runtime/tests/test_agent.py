from __future__ import annotations

import asyncio
import json
from collections.abc import AsyncIterator
from pathlib import Path

import pytest

import ikaros_runtime.agent.loop as agent_loop_module
from ikaros_runtime.agent import ContextBuilder, ModelInputPlanner
from ikaros_runtime.agent.loop import AgentLoop
from ikaros_runtime.agent.scheduler import AgentScheduler
from ikaros_runtime.cancellation import CancellationToken, RunCancelled
from ikaros_runtime.domain import JournalEvent, ModelUsage, WorkspaceSummary
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


class BufferedPausingProvider:
    def __init__(self) -> None:
        self.pending_delta_buffered = asyncio.Event()

    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]:
        del request
        yield TextDelta("first")
        yield TextDelta(" pending")
        self.pending_delta_buffered.set()
        await cancellation.sleep(60)
        yield ResponseCompleted()


class ChunkedTextProvider:
    def __init__(self, deltas: list[str], *, fail_after_text: bool = False) -> None:
        self.deltas = deltas
        self.fail_after_text = fail_after_text

    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]:
        del request
        cancellation.raise_if_cancelled()
        for delta in self.deltas:
            yield TextDelta(delta)
        if self.fail_after_text:
            raise RuntimeError("expected provider failure after text")
        yield ResponseCompleted()


class ChangingProtectedValuesProvider:
    def __init__(self, protected_values: list[str], newly_protected: str) -> None:
        self.protected_values = protected_values
        self.newly_protected = newly_protected

    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]:
        del request
        cancellation.raise_if_cancelled()
        yield TextDelta("safe-first")
        yield TextDelta(self.newly_protected)
        self.protected_values.append(self.newly_protected)


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


class StaleResultTool(RecordingTool):
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
            tool_call_id=call.id,
            tool_name=call.name,
            ok=False,
            output="File changed since it was read; read it again before editing.",
            details={"errorCode": "stale_content", "truncated": False},
        )


class ExplodingTool(RecordingTool):
    async def execute(
        self,
        call: ToolCall,
        *,
        cancellation: CancellationToken,
        default_cwd: str | None = None,
    ) -> ToolResult:
        cancellation.raise_if_cancelled()
        del call, default_cwd
        raise RuntimeError("expected tool failure")


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


class NarratedToolLoopProvider(ToolLoopProvider):
    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]:
        cancellation.raise_if_cancelled()
        self.requests.append(request)
        has_tool_result = any(message.role == "tool" for message in request.messages)
        if not has_tool_result:
            yield TextDelta("Let me ")
            yield TextDelta("demonstrate:")
            yield ToolCallCompleted(
                ToolCall(
                    id="call-narrated",
                    name="process_run",
                    arguments={"command": "test-command"},
                )
            )
        else:
            yield TextDelta("final answer")
        yield ResponseCompleted()


class TextAfterToolProvider:
    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]:
        del request
        cancellation.raise_if_cancelled()
        yield ToolCallCompleted(
            ToolCall("call-before-text", "process_run", {"command": "must-not-run"})
        )
        yield TextDelta("late narration")
        yield ResponseCompleted()


class UsageToolLoopProvider(ToolLoopProvider):
    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]:
        cancellation.raise_if_cancelled()
        self.requests.append(request)
        ordinal = len(self.requests)
        has_tool_result = any(message.role == "tool" for message in request.messages)
        if not has_tool_result:
            yield ToolCallCompleted(
                ToolCall(
                    id="call-with-usage",
                    name="process_run",
                    arguments={"command": "test-command"},
                )
            )
        else:
            yield TextDelta("final answer")
        yield ResponseCompleted(
            ModelUsage(
                input_tokens=ordinal * 10,
                output_tokens=ordinal,
                total_tokens=ordinal * 11,
            )
        )


class InvalidCompletedUsageProvider:
    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]:
        del request
        cancellation.raise_if_cancelled()
        yield TextDelta("partial")
        yield ResponseCompleted(ModelUsage(input_tokens=3, output_tokens=1, total_tokens=4))
        yield TextDelta("event after completion")


class UsageThenProtectedValuesChangeProvider:
    def __init__(self, protected_values: list[str], newly_protected: str) -> None:
        self.protected_values = protected_values
        self.newly_protected = newly_protected

    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]:
        del request
        cancellation.raise_if_cancelled()
        yield TextDelta("safe response")
        yield ResponseCompleted(ModelUsage(input_tokens=3, output_tokens=1, total_tokens=4))
        self.protected_values.append(self.newly_protected)


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
        assert [
            event.payload["delta"]
            for event in events
            if event.type == "item.delta" and event.run_id == prepared.run_id
        ] == ["partial"]
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
async def test_text_deltas_are_batched_without_changing_final_text_or_event_order(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    events: list[JournalEvent] = []
    append_calls: list[str] = []

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    original_append = store.append_text_delta

    def append_text_delta(item_id: str, delta: str) -> JournalEvent:
        append_calls.append(delta)
        return original_append(item_id, delta)

    monkeypatch.setattr(store, "append_text_delta", append_text_delta)
    monkeypatch.setattr(agent_loop_module, "monotonic", lambda: 0.0)
    try:
        thread, _ = store.create_thread("Batched stream")
        prepared = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="stream many chunks",
            provider_id="chunked",
            model_id="chunked-v1",
        )
        chunks = [str(index % 10) for index in range(1025)]
        loop = AgentLoop(store, {"chunked": ChunkedTextProvider(chunks)}, publish)

        await loop.run(prepared.run_id, CancellationToken())

        deltas = [
            event.payload["delta"]
            for event in events
            if event.type == "item.delta" and event.run_id == prepared.run_id
        ]
        assistant = next(
            event.payload["item"]
            for event in events
            if event.type == "item.completed"
            and event.run_id == prepared.run_id
            and event.payload["item"]["role"] == "assistant"
        )
        assert append_calls == deltas
        assert len(append_calls) == 5
        assert append_calls[0] == chunks[0]
        assert "".join(deltas) == "".join(chunks)
        assert assistant["content"] == "".join(chunks)
        assert [event.type for event in events if event.run_id == prepared.run_id][-2:] == [
            "item.completed",
            "run.settled",
        ]
    finally:
        store.close()


@pytest.mark.asyncio
async def test_text_delta_time_window_flushes_on_the_next_arriving_chunk(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    events: list[JournalEvent] = []
    times = iter((0.0, 0.01, 0.06))
    monkeypatch.setattr(agent_loop_module, "monotonic", lambda: next(times))

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    try:
        thread, _ = store.create_thread("Timed stream")
        prepared = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="flush by time",
            provider_id="chunked",
            model_id="chunked-v1",
        )
        loop = AgentLoop(store, {"chunked": ChunkedTextProvider(["a", "b", "c"])}, publish)

        await loop.run(prepared.run_id, CancellationToken())

        assert [
            event.payload["delta"]
            for event in events
            if event.type == "item.delta" and event.run_id == prepared.run_id
        ] == ["a", "bc"]
    finally:
        store.close()


@pytest.mark.asyncio
async def test_provider_failure_flushes_safe_pending_text_before_terminal_events(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    events: list[JournalEvent] = []
    monkeypatch.setattr(agent_loop_module, "monotonic", lambda: 0.0)

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    try:
        thread, _ = store.create_thread("Failed stream")
        prepared = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="fail after text",
            provider_id="chunked",
            model_id="chunked-v1",
        )
        provider = ChunkedTextProvider(["first", " pending"], fail_after_text=True)
        loop = AgentLoop(store, {"chunked": provider}, publish)

        await loop.run(prepared.run_id, CancellationToken())

        run_events = [event for event in events if event.run_id == prepared.run_id]
        assert [event.payload["delta"] for event in run_events if event.type == "item.delta"] == [
            "first",
            " pending",
        ]
        assert [event.type for event in run_events][-3:] == [
            "item.delta",
            "item.completed",
            "run.settled",
        ]
        assert run_events[-2].payload["item"]["content"] == "first pending"
        assert run_events[-2].payload["item"]["status"] == "failed"
    finally:
        store.close()


@pytest.mark.asyncio
async def test_cancellation_flushes_safe_pending_text_before_terminal_events(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    events: list[JournalEvent] = []
    monkeypatch.setattr(agent_loop_module, "monotonic", lambda: 0.0)

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    try:
        thread, _ = store.create_thread("Cancelled buffered stream")
        prepared = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="cancel buffered text",
            provider_id="pausing",
            model_id="pausing-v1",
        )
        provider = BufferedPausingProvider()
        loop = AgentLoop(store, {"pausing": provider}, publish)
        cancellation = CancellationToken()
        task = asyncio.create_task(loop.run(prepared.run_id, cancellation))

        await asyncio.wait_for(provider.pending_delta_buffered.wait(), timeout=1)
        assert [event.payload["delta"] for event in events if event.type == "item.delta"] == [
            "first"
        ]
        cancellation.cancel()
        await asyncio.wait_for(task, timeout=1)

        run_events = [event for event in events if event.run_id == prepared.run_id]
        assert [event.payload["delta"] for event in run_events if event.type == "item.delta"] == [
            "first",
            " pending",
        ]
        assert [event.type for event in run_events][-3:] == [
            "item.delta",
            "item.completed",
            "run.settled",
        ]
        assert run_events[-2].payload["item"]["content"] == "first pending"
        assert run_events[-2].payload["item"]["status"] == "cancelled"
    finally:
        store.close()


@pytest.mark.asyncio
async def test_changed_protected_values_discard_unpersisted_text_batch(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    newly_protected = "newly-protected-buffered-sentinel"
    protected_values: list[str] = []
    store = SqliteRuntimeStore(tmp_path / "state.db")
    events: list[JournalEvent] = []
    monkeypatch.setattr(agent_loop_module, "monotonic", lambda: 0.0)

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    try:
        thread, _ = store.create_thread("Changing protection")
        prepared = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="change protection",
            provider_id="changing",
            model_id="changing-v1",
        )
        provider = ChangingProtectedValuesProvider(protected_values, newly_protected)
        loop = AgentLoop(
            store,
            {"changing": provider},
            publish,
            protected_values=lambda: tuple(protected_values),
        )

        await loop.run(prepared.run_id, CancellationToken())

        serialized_events = json.dumps([event.to_wire() for event in events])
        persisted_rows = store._connection.execute(
            "SELECT payload_json FROM events UNION ALL SELECT content FROM items "
            "UNION ALL SELECT data_json FROM items"
        ).fetchall()
        assert newly_protected not in serialized_events
        assert all(newly_protected not in str(row[0]) for row in persisted_rows)
        run_events = [event for event in events if event.run_id == prepared.run_id]
        assert [event.payload["delta"] for event in run_events if event.type == "item.delta"] == [
            "safe-first"
        ]
        assert run_events[-2].payload["item"]["content"] == "safe-first"
        assert run_events[-1].payload["reasonCode"] == "agent_error"
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
        for request in provider.requests:
            assert request.messages[0].role == "system"
            assert [message.role for message in request.messages].count("system") == 1
            assert "Markdown hyphen bullets (`- item`)" in request.messages[0].content
            assert "unless the user explicitly asks for them" in request.messages[0].content
        second_messages = provider.requests[1].messages
        assert [message.role for message in second_messages] == [
            "system",
            "user",
            "assistant",
            "tool",
        ]
        assert second_messages[2].tool_calls == (
            ToolCall("call-1", "process_run", {"command": "test-command"}),
        )
        assert second_messages[2].reasoning_content == "tool reasoning"
        assert second_messages[3].tool_call_id == "call-1"
        tool_content = json.loads(second_messages[3].content)
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
async def test_agent_records_provider_usage_once_for_each_completed_model_step(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    events: list[JournalEvent] = []

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    try:
        thread, _ = store.create_thread("Usage tool loop")
        prepared = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="run the tool",
            provider_id="usage-tool-loop",
            model_id="usage-model",
        )
        provider = UsageToolLoopProvider()
        loop = AgentLoop(
            store,
            {"usage-tool-loop": provider},
            publish,
            ToolExecutor(ToolRegistry([RecordingTool()]), FullAccessPolicy()),
        )

        await loop.run(prepared.run_id, CancellationToken())

        usage_events = [event for event in events if event.type == "model.usage_recorded"]
        assert [event.payload["stepOrdinal"] for event in usage_events] == [1, 2]
        assert [event.payload["usage"]["totalTokens"] for event in usage_events] == [11, 22]
        rows = store._connection.execute(
            """
            SELECT step_ordinal, provider_id, model_id, input_tokens, output_tokens,
                   total_tokens
            FROM model_usages
            WHERE run_id = ?
            ORDER BY step_ordinal
            """,
            (prepared.run_id,),
        ).fetchall()
        assert [tuple(row) for row in rows] == [
            (1, "usage-tool-loop", "usage-model", 10, 1, 11),
            (2, "usage-tool-loop", "usage-model", 20, 2, 22),
        ]
        assert events[-1].type == "run.settled"
        assert events[-1].payload["status"] == "completed"
    finally:
        store.close()


@pytest.mark.asyncio
async def test_agent_keeps_completed_step_usage_when_the_following_tool_crashes(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    events: list[JournalEvent] = []

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    try:
        thread, _ = store.create_thread("Usage before tool failure")
        prepared = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="run the failing tool",
            provider_id="usage-tool-loop",
            model_id="usage-model",
        )
        loop = AgentLoop(
            store,
            {"usage-tool-loop": UsageToolLoopProvider()},
            publish,
            ToolExecutor(ToolRegistry([ExplodingTool()]), FullAccessPolicy()),
        )

        await loop.run(prepared.run_id, CancellationToken())

        usage_events = [event for event in events if event.type == "model.usage_recorded"]
        assert len(usage_events) == 1
        assert usage_events[0].payload["stepOrdinal"] == 1
        assert usage_events[0].payload["usage"]["totalTokens"] == 11
        assert store.read_usage().summary.lifetime_tokens == 11
        assert events[-1].type == "run.settled"
        assert events[-1].payload["status"] == "failed"
        assert events[-1].payload["reasonCode"] == "agent_error"
    finally:
        store.close()


@pytest.mark.asyncio
async def test_agent_does_not_record_usage_until_the_full_stream_is_validated(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    events: list[JournalEvent] = []

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    try:
        thread, _ = store.create_thread("Invalid completed stream")
        prepared = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="reject malformed provider output",
            provider_id="invalid-completed",
            model_id="invalid-completed-v1",
        )
        loop = AgentLoop(
            store,
            {"invalid-completed": InvalidCompletedUsageProvider()},
            publish,
        )

        await loop.run(prepared.run_id, CancellationToken())

        assert not any(event.type == "model.usage_recorded" for event in events)
        assert (
            store._connection.execute(
                "SELECT COUNT(*) FROM model_usages WHERE run_id = ?",
                (prepared.run_id,),
            ).fetchone()[0]
            == 0
        )
        settled = [event for event in events if event.type == "run.settled"]
        assert len(settled) == 1
        assert settled[0].payload["status"] == "failed"
        assert settled[0].payload["reasonCode"] == "provider_protocol"
    finally:
        store.close()


@pytest.mark.asyncio
async def test_agent_does_not_record_usage_when_trailing_security_validation_fails(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    events: list[JournalEvent] = []
    protected_values: list[str] = []

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    try:
        thread, _ = store.create_thread("Usage rejected after security validation")
        prepared = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="reject stale security snapshot",
            provider_id="usage-protection-change",
            model_id="usage-model",
        )
        provider = UsageThenProtectedValuesChangeProvider(
            protected_values,
            "newly-protected-sentinel",
        )
        loop = AgentLoop(
            store,
            {"usage-protection-change": provider},
            publish,
            protected_values=lambda: tuple(protected_values),
        )

        await loop.run(prepared.run_id, CancellationToken())

        assert not any(event.type == "model.usage_recorded" for event in events)
        assert store.read_usage().summary.lifetime_tokens is None
        assert events[-1].type == "run.settled"
        assert events[-1].payload["status"] == "failed"
        assert events[-1].payload["reasonCode"] == "agent_error"
    finally:
        store.close()


@pytest.mark.asyncio
async def test_agent_accepts_narration_and_tool_calls_in_one_provider_response(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    events: list[JournalEvent] = []

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    try:
        thread, _ = store.create_thread("Narrated tool loop")
        prepared = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="show the tool",
            provider_id="narrated",
            model_id="narrated-v1",
        )
        provider = NarratedToolLoopProvider()
        tool = RecordingTool()
        loop = AgentLoop(
            store,
            {"narrated": provider},
            publish,
            ToolExecutor(ToolRegistry([tool]), FullAccessPolicy()),
        )

        await loop.run(prepared.run_id, CancellationToken())

        assert [call.id for call in tool.calls] == ["call-narrated"]
        assert len(provider.requests) == 2
        second_messages = provider.requests[1].messages
        assert [message.role for message in second_messages] == [
            "system",
            "user",
            "assistant",
            "tool",
        ]
        assert second_messages[2].content == "Let me demonstrate:"
        assert second_messages[2].tool_calls == (
            ToolCall("call-narrated", "process_run", {"command": "test-command"}),
        )
        narrated_deltas = [
            event
            for event in events
            if event.run_id == prepared.run_id
            and event.type == "item.delta"
            and event.payload["delta"] in {"Let me ", "demonstrate:"}
        ]
        first_tool_call = next(
            event
            for event in events
            if event.run_id == prepared.run_id
            and event.type == "item.started"
            and event.payload["item"]["kind"] == "tool_call"
        )
        assert [event.payload["delta"] for event in narrated_deltas] == [
            "Let me ",
            "demonstrate:",
        ]
        assert all(event.seq < first_tool_call.seq for event in narrated_deltas)
        narrated_terminal = [
            event
            for event in events
            if event.type == "item.completed"
            and event.payload.get("item", {}).get("role") == "assistant"
            and event.payload.get("item", {}).get("content") == "Let me demonstrate:"
        ]
        assert len(narrated_terminal) == 1
        assert narrated_terminal[0].payload["item"]["status"] == "completed"
        assert events[-1].type == "run.settled"
        assert events[-1].payload["status"] == "completed"

        before_rebuild = store.context_items(
            thread.default_branch_id,
            through_turn_id=prepared.turn_id,
        )
        store.rebuild_projections()
        rebuilt = store.context_items(
            thread.default_branch_id,
            through_turn_id=prepared.turn_id,
        )
        assert rebuilt == before_rebuild
        replay_plan = ModelInputPlanner().build_plan(
            model_id="narrated-v1",
            items=rebuilt,
        )
        replayed = ContextBuilder().build_request(replay_plan).messages
        assert replayed[2].content == "Let me demonstrate:"
        assert replayed[2].tool_calls[0].id == "call-narrated"
    finally:
        store.close()


@pytest.mark.asyncio
async def test_agent_rejects_text_emitted_after_a_completed_tool_call(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    events: list[JournalEvent] = []

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    try:
        thread, _ = store.create_thread("Late tool text")
        prepared = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="reject malformed ordering",
            provider_id="late-text",
            model_id="late-text-v1",
        )
        tool = RecordingTool()
        loop = AgentLoop(
            store,
            {"late-text": TextAfterToolProvider()},
            publish,
            ToolExecutor(ToolRegistry([tool]), FullAccessPolicy()),
        )

        await loop.run(prepared.run_id, CancellationToken())

        assert tool.calls == []
        settled = [event for event in events if event.type == "run.settled"]
        assert len(settled) == 1
        assert settled[0].payload["status"] == "failed"
        assert settled[0].payload["reasonCode"] == "provider_protocol"
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
        serialized_events = json.dumps([event.to_wire() for event in events])
        assert protected not in serialized_events
        assert protected.encode() not in (tmp_path / "state.db").read_bytes()
    finally:
        store.close()


@pytest.mark.asyncio
async def test_agent_returns_stale_content_to_the_provider_and_rebuilds_it(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    events: list[JournalEvent] = []

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    try:
        thread, _ = store.create_thread("Stale edit result")
        prepared = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="edit safely",
            provider_id="tool-loop",
            model_id="tool-loop-v1",
        )
        provider = ToolLoopProvider()
        loop = AgentLoop(
            store,
            {"tool-loop": provider},
            publish,
            ToolExecutor(ToolRegistry([StaleResultTool()]), FullAccessPolicy()),
        )

        await loop.run(prepared.run_id, CancellationToken())

        result = json.loads(provider.requests[1].messages[-1].content)
        assert result["ok"] is False
        assert result["errorCode"] == "stale_content"
        result_event = next(
            event
            for event in events
            if event.type == "item.completed"
            and event.payload.get("item", {}).get("kind") == "tool_result"
        )
        assert result_event.payload["item"]["status"] == "failed"
        assert result_event.payload["item"]["data"]["result"]["errorCode"] == "stale_content"

        before_rebuild = store.context_items(
            thread.default_branch_id,
            through_turn_id=prepared.turn_id,
        )
        store.rebuild_projections()
        after_rebuild = store.context_items(
            thread.default_branch_id,
            through_turn_id=prepared.turn_id,
        )
        assert after_rebuild == before_rebuild
        rebuilt_result = next(item for item in after_rebuild if item.kind == "tool_result")
        assert json.loads(rebuilt_result.content)["errorCode"] == "stale_content"
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
