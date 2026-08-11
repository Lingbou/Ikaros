from __future__ import annotations

import asyncio
import logging
import uuid
from collections.abc import Awaitable, Callable, Mapping, Sequence
from dataclasses import dataclass
from typing import Protocol

from .cancellation import CancellationToken, RunCancelled
from .domain import ContextItem, JournalEvent
from .providers import (
    ProviderAdapter,
    ProviderMessage,
    ProviderRequest,
    ProviderResolver,
    ReasoningDelta,
    ResponseCompleted,
    TextDelta,
    ToolCallCompleted,
)
from .security import (
    ProtectedStreamGuard,
    contains_protected_value,
    json_contains_protected_value,
)
from .storage import SqliteRuntimeStore
from .tools import ToolCall, ToolExecutionCancelled, ToolExecutor, ToolResult

EventPublisher = Callable[[JournalEvent], Awaitable[None]]
ProtectedValues = Callable[[], Sequence[str]]
_LOGGER = logging.getLogger("ikaros_runtime.agent")
_MAX_REASONING_CHARACTERS = 1_000_000
_PROTECTED_TOOL_OUTPUT_MESSAGE = "Tool output contained protected configuration data."


class RunExecutor(Protocol):
    async def run(self, run_id: str, cancellation: CancellationToken) -> None: ...

    async def cancel(self, run_id: str) -> None: ...


class AgentLoop:
    def __init__(
        self,
        store: SqliteRuntimeStore,
        providers: Mapping[str, ProviderAdapter] | ProviderResolver,
        publish: EventPublisher,
        tool_executor: ToolExecutor | None = None,
        max_steps: int = 16,
        *,
        protected_values: ProtectedValues | None = None,
    ) -> None:
        if max_steps < 1:
            raise ValueError("max_steps must be positive")
        self._store = store
        self._providers = providers
        self._publish = publish
        self._tool_executor = tool_executor
        self._max_steps = max_steps
        self._protected_values = protected_values or _empty_protected_values

    async def run(self, run_id: str, cancellation: CancellationToken) -> None:
        try:
            cancellation.raise_if_cancelled()
            run = self._store.get_run(run_id)
            provider = self._resolve_provider(run.provider_id)
            if provider is None:
                raise ValueError(f"unknown provider: {run.provider_id}")

            if self._tool_executor is not None:
                if run.execution_policy != self._tool_executor.policy_name:
                    raise RuntimeError("run execution policy is not available")
            elif run.execution_policy != "full_access":
                raise RuntimeError("run execution policy is not available")

            await self._publish(self._store.mark_run_running(run_id))
            for _ in range(self._max_steps):
                cancellation.raise_if_cancelled()
                messages = self._provider_messages(
                    self._store.context_items(
                        run.branch_id,
                        through_turn_id=run.turn_id,
                    )
                )
                request = ProviderRequest(
                    model_id=run.model_id,
                    messages=messages,
                    tools=(
                        self._tool_executor.definitions if self._tool_executor is not None else ()
                    ),
                )
                assistant_item_id, tool_calls, reasoning_content = await self._provider_step(
                    run_id,
                    provider,
                    request,
                    cancellation,
                )
                if tool_calls:
                    await self._execute_tool_calls(
                        run_id,
                        tool_calls,
                        cancellation,
                        reasoning_content=reasoning_content,
                    )
                    continue
                if assistant_item_id is None:
                    raise RuntimeError("provider completed without text or a tool call")
                cancellation.raise_if_cancelled()
                await self._publish_terminal_events(
                    self._store.terminalize_run(run_id, "completed")
                )
                return
            raise RuntimeError("provider exceeded the maximum Agent step count")
        except RunCancelled:
            await self._publish_terminal_events(self._store.terminalize_run(run_id, "cancelled"))
        except Exception:
            _LOGGER.error("Run %s failed", run_id)
            await self._publish_terminal_events(self._store.terminalize_run(run_id, "failed"))

    async def cancel(self, run_id: str) -> None:
        await self._publish_terminal_events(self._store.terminalize_run(run_id, "cancelled"))

    async def _publish_terminal_events(self, events: Sequence[JournalEvent]) -> None:
        for event in events:
            await self._publish(event)

    async def _provider_step(
        self,
        run_id: str,
        provider: ProviderAdapter,
        request: ProviderRequest,
        cancellation: CancellationToken,
    ) -> tuple[str | None, tuple[ToolCall, ...], str | None]:
        assistant_item_id: str | None = None
        tool_calls: list[ToolCall] = []
        call_ids: set[str] = set()
        reasoning_parts: list[str] = []
        reasoning_characters = 0
        reasoning_seen = False
        text_seen = False
        protected_values = self._current_protected_values()
        text_guard = ProtectedStreamGuard(protected_values)
        reasoning_guard = ProtectedStreamGuard(protected_values)
        completed = False
        async for event in provider.stream(request, cancellation=cancellation):
            cancellation.raise_if_cancelled()
            self._assert_protected_values_unchanged(protected_values)
            if completed:
                raise RuntimeError("provider emitted an event after response.completed")
            if isinstance(event, TextDelta):
                if tool_calls:
                    raise RuntimeError("provider mixed text and tool calls in one response")
                if not event.delta:
                    continue
                text_seen = True
                safe_delta = text_guard.feed(event.delta)
                if not safe_delta:
                    continue
                if assistant_item_id is None:
                    assistant_item_id, started = self._store.create_assistant_item(run_id)
                    await self._publish(started)
                    self._assert_protected_values_unchanged(protected_values)
                await self._publish(self._store.append_text_delta(assistant_item_id, safe_delta))
            elif isinstance(event, ReasoningDelta):
                reasoning_seen = True
                reasoning_characters += len(event.delta)
                if reasoning_characters > _MAX_REASONING_CHARACTERS:
                    raise RuntimeError("provider reasoning exceeded the supported size")
                safe_delta = reasoning_guard.feed(event.delta)
                if safe_delta:
                    reasoning_parts.append(safe_delta)
            elif isinstance(event, ToolCallCompleted):
                if text_seen:
                    raise RuntimeError("provider mixed text and tool calls in one response")
                call = event.call
                if (
                    not isinstance(call.id, str)
                    or not call.id
                    or not isinstance(call.name, str)
                    or not call.name
                    or not isinstance(call.arguments, dict)
                ):
                    raise RuntimeError("provider emitted an invalid tool call")
                if call.id in call_ids:
                    raise RuntimeError("provider emitted a duplicate tool call ID")
                call_ids.add(call.id)
                tool_calls.append(call)
            elif isinstance(event, ResponseCompleted):
                completed = True
            else:
                raise RuntimeError("provider emitted an unknown event")
        if not completed:
            raise RuntimeError("provider stream ended without response.completed")
        self._assert_protected_values_unchanged(protected_values)
        trailing_text = text_guard.finish()
        if trailing_text:
            if assistant_item_id is None:
                assistant_item_id, started = self._store.create_assistant_item(run_id)
                await self._publish(started)
                self._assert_protected_values_unchanged(protected_values)
            await self._publish(self._store.append_text_delta(assistant_item_id, trailing_text))
        trailing_reasoning = reasoning_guard.finish()
        if trailing_reasoning:
            reasoning_parts.append(trailing_reasoning)
        reasoning_content = "".join(reasoning_parts) if reasoning_seen else None
        return assistant_item_id, tuple(tool_calls), reasoning_content

    async def _execute_tool_calls(
        self,
        run_id: str,
        calls: Sequence[ToolCall],
        cancellation: CancellationToken,
        *,
        reasoning_content: str | None,
    ) -> None:
        executor = self._tool_executor
        if executor is None:
            raise RuntimeError("provider requested a tool but no ToolExecutor is available")
        self._assert_tool_request_safe(calls, reasoning_content)
        step_id = f"step_{uuid.uuid4().hex}"
        item_ids: list[str] = []
        for index, call in enumerate(calls):
            item_id, started = self._store.create_tool_call_item(
                run_id,
                step_id=step_id,
                call_id=call.id,
                tool_name=call.name,
                arguments=call.arguments,
                reasoning_content=reasoning_content if index == 0 else None,
            )
            item_ids.append(item_id)
            await self._publish(started)

        call_items = list(zip(calls, item_ids, strict=True))
        for index, (call, item_id) in enumerate(call_items):
            try:
                cancellation.raise_if_cancelled()
                result = await executor.execute(call, cancellation=cancellation)
            except ToolExecutionCancelled as error:
                await self._publish_tool_result(item_id, call, error.result, "cancelled")
                await self._cancel_unexecuted_tool_calls(call_items[index + 1 :])
                raise RunCancelled from error
            except RunCancelled:
                await self._publish_tool_result(
                    item_id,
                    call,
                    self._cancelled_before_execution(call),
                    "cancelled",
                )
                await self._cancel_unexecuted_tool_calls(call_items[index + 1 :])
                raise
            status = "completed" if result.ok else "failed"
            await self._publish_tool_result(item_id, call, result, status)

    async def _publish_tool_result(
        self,
        item_id: str,
        call: ToolCall,
        result: ToolResult,
        status: str,
    ) -> None:
        result, protected = self._safe_tool_result(call, result)
        if protected and status != "cancelled":
            status = "failed"
        await self._publish_terminal_events(
            self._store.complete_tool_call(
                item_id,
                status=status,
                result=result.to_wire(),
                result_content=result.to_model_content(),
            )
        )

    def _assert_tool_request_safe(
        self,
        calls: Sequence[ToolCall],
        reasoning_content: str | None,
    ) -> None:
        protected_values = self._current_protected_values()
        if not protected_values:
            return
        value = {
            "reasoningContent": reasoning_content,
            "calls": [
                {"id": call.id, "name": call.name, "arguments": call.arguments} for call in calls
            ],
        }
        if json_contains_protected_value(value, protected_values):
            raise RuntimeError("provider tool request contained protected configuration data")

    def _safe_tool_result(
        self,
        call: ToolCall,
        result: ToolResult,
    ) -> tuple[ToolResult, bool]:
        protected_values = self._current_protected_values()
        if not protected_values:
            return result, False
        wire = result.to_wire()
        model_content = result.to_model_content()
        if not (
            json_contains_protected_value(wire, protected_values)
            or contains_protected_value(model_content, protected_values)
        ):
            return result, False
        return (
            ToolResult(
                tool_call_id=call.id,
                tool_name=call.name,
                ok=False,
                output=_PROTECTED_TOOL_OUTPUT_MESSAGE,
                details={
                    "stdout": "",
                    "stderr": "",
                    "exitCode": None,
                    "durationMs": 0,
                    "timedOut": False,
                    "truncated": False,
                    "errorCode": "protected_output",
                },
                cancelled=result.cancelled,
            ),
            True,
        )

    def _current_protected_values(self) -> tuple[str, ...]:
        return tuple(dict.fromkeys(value for value in self._protected_values() if value))

    def _assert_protected_values_unchanged(self, snapshot: Sequence[str]) -> None:
        if self._current_protected_values() != tuple(snapshot):
            raise RuntimeError("protected configuration changed during provider response")

    async def _cancel_unexecuted_tool_calls(
        self,
        call_items: Sequence[tuple[ToolCall, str]],
    ) -> None:
        for call, item_id in call_items:
            await self._publish_tool_result(
                item_id,
                call,
                self._cancelled_before_execution(call),
                "cancelled",
            )

    @staticmethod
    def _cancelled_before_execution(call: ToolCall) -> ToolResult:
        return ToolResult(
            tool_call_id=call.id,
            tool_name=call.name,
            ok=False,
            output="Tool call was not started because the Run was cancelled.",
            details={
                "stdout": "",
                "stderr": "",
                "exitCode": None,
                "durationMs": 0,
                "timedOut": False,
                "truncated": False,
                "errorCode": "cancelled",
            },
            cancelled=True,
        )

    @staticmethod
    def _provider_messages(items: Sequence[ContextItem]) -> list[ProviderMessage]:
        messages: list[ProviderMessage] = []
        index = 0
        while index < len(items):
            item = items[index]
            if item.kind == "message":
                if item.role not in {"user", "assistant"}:
                    raise RuntimeError("message context item has an invalid role")
                messages.append(ProviderMessage(role=item.role, content=item.content))
                index += 1
                continue
            if item.kind == "tool_call":
                step_id = item.data.get("stepId")
                calls: list[ToolCall] = []
                while index < len(items):
                    candidate = items[index]
                    if candidate.kind != "tool_call" or candidate.data.get("stepId") != step_id:
                        break
                    call_id = candidate.data.get("callId")
                    tool_name = candidate.data.get("toolName")
                    arguments = candidate.data.get("arguments")
                    if (
                        not isinstance(call_id, str)
                        or not isinstance(tool_name, str)
                        or not isinstance(arguments, dict)
                    ):
                        raise RuntimeError("tool call context item is invalid")
                    calls.append(ToolCall(call_id, tool_name, arguments))
                    index += 1
                messages.append(
                    ProviderMessage(
                        role="assistant",
                        content="",
                        tool_calls=tuple(calls),
                        reasoning_content=_reasoning_content(item),
                    )
                )
                continue
            if item.kind == "tool_result":
                call_id = item.data.get("callId")
                if not isinstance(call_id, str):
                    raise RuntimeError("tool result context item is invalid")
                messages.append(
                    ProviderMessage(
                        role="tool",
                        content=item.content,
                        tool_call_id=call_id,
                    )
                )
                index += 1
                continue
            raise RuntimeError(f"unknown context item kind: {item.kind}")
        return messages

    def _resolve_provider(self, provider_id: str) -> ProviderAdapter | None:
        if isinstance(self._providers, Mapping):
            return self._providers.get(provider_id)
        return self._providers.resolve(provider_id)


def _reasoning_content(item: ContextItem) -> str | None:
    value = item.data.get("reasoningContent")
    if value is None:
        return None
    if not isinstance(value, str):
        raise RuntimeError("tool call reasoning context is invalid")
    return value


def _empty_protected_values() -> tuple[str, ...]:
    return ()


@dataclass(slots=True)
class _ScheduledRun:
    cancellation: CancellationToken
    activation: asyncio.Event
    state: str = "queued"


class AgentScheduler:
    def __init__(self, loop: RunExecutor) -> None:
        self._loop = loop
        self._queue: asyncio.Queue[str | None] = asyncio.Queue()
        self._worker: asyncio.Task[None] | None = None
        self._entries: dict[str, _ScheduledRun] = {}
        self._accepting = False

    def start(self, initial_run_ids: Sequence[str] = ()) -> None:
        if self._worker is not None:
            return
        self._accepting = True
        for run_id in initial_run_ids:
            entry = self._register(run_id, state="queued")
            entry.activation.set()
            self._queue.put_nowait(run_id)
        self._worker = asyncio.create_task(self._work(), name="ikaros-agent-scheduler")

    def reserve(self, run_id: str) -> None:
        """Make a persisted Run cancellable before its start ACK is attempted."""
        if self._worker is None or not self._accepting:
            raise RuntimeError("Agent scheduler has not started")
        self._register(run_id, state="reserved")
        self._queue.put_nowait(run_id)

    async def activate(self, run_id: str) -> bool:
        """Queue a reserved Run after the command ACK attempt has finished."""
        entry = self._entries.get(run_id)
        if entry is None:
            return False
        if entry.state != "reserved":
            raise RuntimeError("run is not reserved")
        if not self._accepting:
            return False
        entry.state = "queued"
        entry.activation.set()
        return True

    async def enqueue(self, run_id: str) -> None:
        if self._worker is None or not self._accepting:
            raise RuntimeError("Agent scheduler has not started")
        entry = self._register(run_id, state="queued")
        entry.activation.set()
        self._queue.put_nowait(run_id)

    async def cancel(self, run_id: str) -> bool:
        entry = self._entries.get(run_id)
        if entry is None:
            return False
        entry.cancellation.cancel()
        if entry.state in {"reserved", "queued"}:
            self._entries.pop(run_id, None)
            entry.activation.set()
            await self._loop.cancel(run_id)
        return True

    async def close(self) -> None:
        worker = self._worker
        if worker is None:
            return
        self._accepting = False
        queued_run_ids: list[str] = []
        for run_id, entry in tuple(self._entries.items()):
            entry.cancellation.cancel()
            if entry.state in {"reserved", "queued"}:
                self._entries.pop(run_id, None)
                entry.activation.set()
                queued_run_ids.append(run_id)
        for run_id in queued_run_ids:
            await self._loop.cancel(run_id)
        await self._queue.put(None)
        await worker
        self._worker = None
        self._entries.clear()

    def _register(self, run_id: str, *, state: str) -> _ScheduledRun:
        if run_id in self._entries:
            raise RuntimeError("run is already scheduled")
        entry = _ScheduledRun(CancellationToken(), asyncio.Event(), state=state)
        self._entries[run_id] = entry
        return entry

    async def _work(self) -> None:
        while True:
            run_id = await self._queue.get()
            try:
                if run_id is None:
                    return
                entry = self._entries.get(run_id)
                if entry is None:
                    continue
                if entry.state == "reserved":
                    await entry.activation.wait()
                    entry = self._entries.get(run_id)
                    if entry is None:
                        continue
                if entry.state != "queued":
                    continue
                entry.state = "running"
                try:
                    await self._loop.run(run_id, entry.cancellation)
                except Exception:
                    _LOGGER.error("Unhandled scheduler failure for run %s", run_id)
                finally:
                    self._entries.pop(run_id, None)
            finally:
                self._queue.task_done()
