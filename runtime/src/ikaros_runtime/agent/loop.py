from __future__ import annotations

import logging
import uuid
from collections.abc import Awaitable, Callable, Mapping, Sequence
from time import monotonic

from ..cancellation import CancellationToken, RunCancelled
from ..domain import ContextItem, JournalEvent, ModelUsage
from ..errors import ProtectedValueError, ProviderFailure
from ..providers.base import (
    ProviderAdapter,
    ProviderMessage,
    ProviderRequest,
    ProviderResolver,
    ReasoningDelta,
    ResponseCompleted,
    TextDelta,
    ToolCallCompleted,
)
from ..security import (
    ProtectedStreamGuard,
    contains_protected_value,
    json_contains_protected_value,
)
from ..storage import SqliteRuntimeStore
from ..tools.core import ToolCall, ToolExecutionCancelled, ToolExecutor, ToolResult

EventPublisher = Callable[[JournalEvent], Awaitable[None]]
ProtectedValues = Callable[[], Sequence[str]]
_LOGGER = logging.getLogger("ikaros_runtime.agent")
_MAX_REASONING_CHARACTERS = 1_000_000
_PROTECTED_TOOL_OUTPUT_MESSAGE = "Tool output contained protected configuration data."
_TEXT_DELTA_FLUSH_CHARACTERS = 256
_TEXT_DELTA_FLUSH_SECONDS = 0.05
_OUTPUT_STYLE_SYSTEM_MESSAGE = ProviderMessage(
    role="system",
    content=(
        "Use a restrained, professional response style. Do not use emoji or decorative "
        "Unicode symbols unless the user explicitly asks for them. Never use them for "
        "decoration, headings, or list markers. Use Markdown hyphen bullets (`- item`) "
        "for ordinary unordered lists; the client will render them as simple round bullets."
    ),
)


class _TextDeltaBatch:
    """Deterministically coalesce safe provider text without background writes."""

    def __init__(self) -> None:
        self._parts: list[str] = []
        self._characters = 0
        self._first_delta_emitted = False
        self._last_flush_at: float | None = None

    def add(self, delta: str, *, now: float) -> str | None:
        if not self._first_delta_emitted:
            self._first_delta_emitted = True
            self._last_flush_at = now
            return delta
        self._parts.append(delta)
        self._characters += len(delta)
        last_flush_at = self._last_flush_at
        if self._characters >= _TEXT_DELTA_FLUSH_CHARACTERS or (
            last_flush_at is not None and now - last_flush_at >= _TEXT_DELTA_FLUSH_SECONDS
        ):
            return self.flush(now=now)
        return None

    def flush(self, *, now: float | None = None) -> str | None:
        if not self._parts:
            return None
        delta = "".join(self._parts)
        self._parts.clear()
        self._characters = 0
        if now is not None:
            self._last_flush_at = now
        return delta


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
            for step_ordinal in range(1, self._max_steps + 1):
                cancellation.raise_if_cancelled()
                messages = self._provider_messages(
                    self._store.context_items(
                        run.branch_id,
                        through_turn_id=run.turn_id,
                    )
                )
                request = ProviderRequest(
                    model_id=run.model_id,
                    messages=(_OUTPUT_STYLE_SYSTEM_MESSAGE, *messages),
                    tools=(
                        self._tool_executor.definitions if self._tool_executor is not None else ()
                    ),
                )
                assistant_item_id, tool_calls, reasoning_content, step_id = (
                    await self._provider_step(
                        run_id,
                        provider,
                        request,
                        cancellation,
                        step_ordinal=step_ordinal,
                    )
                )
                if tool_calls:
                    await self._execute_tool_calls(
                        run_id,
                        tool_calls,
                        cancellation,
                        default_cwd=(run.workspace.root_uri if run.workspace is not None else None),
                        reasoning_content=reasoning_content,
                        step_id=step_id,
                        assistant_item_id=assistant_item_id,
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
        except Exception as error:
            _LOGGER.exception("Run %s failed", run_id)
            await self._publish_terminal_events(
                self._store.terminalize_run(
                    run_id,
                    "failed",
                    reason_code=_failure_reason_code(error),
                )
            )

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
        *,
        step_ordinal: int,
    ) -> tuple[str | None, tuple[ToolCall, ...], str | None, str | None]:
        assistant_item_id: str | None = None
        tool_calls: list[ToolCall] = []
        call_ids: set[str] = set()
        reasoning_parts: list[str] = []
        reasoning_characters = 0
        reasoning_seen = False
        protected_values = self._current_protected_values()
        text_guard = ProtectedStreamGuard(protected_values)
        reasoning_guard = ProtectedStreamGuard(protected_values)
        text_batch = _TextDeltaBatch()
        completed = False
        response_usage: ModelUsage | None = None
        try:
            async for event in provider.stream(request, cancellation=cancellation):
                cancellation.raise_if_cancelled()
                self._assert_protected_values_unchanged(protected_values)
                if completed:
                    raise RuntimeError("provider emitted an event after response.completed")
                if isinstance(event, TextDelta):
                    if tool_calls:
                        raise RuntimeError("provider emitted text after a completed tool call")
                    if not event.delta:
                        continue
                    safe_delta = text_guard.feed(event.delta)
                    if not safe_delta:
                        continue
                    if assistant_item_id is None:
                        assistant_item_id, started = self._store.create_assistant_item(run_id)
                        await self._publish(started)
                        self._assert_protected_values_unchanged(protected_values)
                    await self._publish_text_delta(
                        assistant_item_id,
                        text_batch.add(safe_delta, now=monotonic()),
                    )
                elif isinstance(event, ReasoningDelta):
                    reasoning_seen = True
                    reasoning_characters += len(event.delta)
                    if reasoning_characters > _MAX_REASONING_CHARACTERS:
                        raise RuntimeError("provider reasoning exceeded the supported size")
                    safe_delta = reasoning_guard.feed(event.delta)
                    if safe_delta:
                        reasoning_parts.append(safe_delta)
                elif isinstance(event, ToolCallCompleted):
                    await self._publish_text_delta(assistant_item_id, text_batch.flush())
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
                    await self._publish_text_delta(assistant_item_id, text_batch.flush())
                    response_usage = event.usage
                    completed = True
                else:
                    raise RuntimeError("provider emitted an unknown event")
        except Exception:
            # Only text already released by ProtectedStreamGuard is buffered.
            # Do not call finish() on an incomplete or cancelled stream.
            if self._current_protected_values() == tuple(protected_values):
                await self._publish_text_delta(assistant_item_id, text_batch.flush())
            raise
        self._assert_protected_values_unchanged(protected_values)
        await self._publish_text_delta(assistant_item_id, text_batch.flush())
        if not completed:
            raise RuntimeError("provider stream ended without response.completed")
        self._assert_protected_values_unchanged(protected_values)
        trailing_text = text_guard.finish()
        if trailing_text:
            if assistant_item_id is None:
                assistant_item_id, started = self._store.create_assistant_item(run_id)
                await self._publish(started)
                self._assert_protected_values_unchanged(protected_values)
            await self._publish_text_delta(
                assistant_item_id,
                text_batch.add(trailing_text, now=monotonic()),
            )
        await self._publish_text_delta(assistant_item_id, text_batch.flush())
        trailing_reasoning = reasoning_guard.finish()
        if trailing_reasoning:
            reasoning_parts.append(trailing_reasoning)
        reasoning_content = "".join(reasoning_parts) if reasoning_seen else None
        if response_usage is not None:
            await self._publish(
                self._store.record_model_usage(
                    run_id,
                    step_ordinal=step_ordinal,
                    usage=response_usage,
                )
            )
        step_id = f"step_{uuid.uuid4().hex}" if tool_calls and assistant_item_id else None
        return assistant_item_id, tuple(tool_calls), reasoning_content, step_id

    async def _publish_text_delta(self, item_id: str | None, delta: str | None) -> None:
        if delta is None:
            return
        if item_id is None:
            raise RuntimeError("text delta has no assistant item")
        await self._publish(self._store.append_text_delta(item_id, delta))

    async def _execute_tool_calls(
        self,
        run_id: str,
        calls: Sequence[ToolCall],
        cancellation: CancellationToken,
        *,
        default_cwd: str | None,
        reasoning_content: str | None,
        step_id: str | None,
        assistant_item_id: str | None,
    ) -> None:
        executor = self._tool_executor
        if executor is None:
            raise RuntimeError("provider requested a tool but no ToolExecutor is available")
        self._assert_tool_request_safe(calls, reasoning_content)
        step_id = step_id or f"step_{uuid.uuid4().hex}"
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

        if assistant_item_id is not None:
            # Journal the calls first so a crash cannot leave a completed
            # narrated tool step whose calls never existed.  The shared step
            # identifier still lets context reconstruction fold both Items
            # back into the original assistant response.
            await self._publish(
                self._store.complete_assistant_item(assistant_item_id, step_id=step_id)
            )

        call_items = list(zip(calls, item_ids, strict=True))
        for index, (call, item_id) in enumerate(call_items):
            try:
                cancellation.raise_if_cancelled()
                result = await executor.execute(
                    call,
                    cancellation=cancellation,
                    default_cwd=default_cwd,
                )
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
                    "durationMs": 0,
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
                "durationMs": 0,
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
                step_id = item.data.get("stepId")
                if item.role == "assistant" and isinstance(step_id, str):
                    index += 1
                    calls, index, reasoning_content = _tool_calls_for_step(
                        items,
                        index,
                        step_id,
                    )
                    if not calls:
                        # Calls from a failed/cancelled Run are omitted from
                        # later context. Preserve its narration as plain text
                        # instead of breaking every future context rebuild.
                        messages.append(ProviderMessage(role="assistant", content=item.content))
                        continue
                    messages.append(
                        ProviderMessage(
                            role="assistant",
                            content=item.content,
                            tool_calls=tuple(calls),
                            reasoning_content=reasoning_content,
                        )
                    )
                    continue
                messages.append(ProviderMessage(role=item.role, content=item.content))
                index += 1
                continue
            if item.kind == "tool_call":
                step_id = item.data.get("stepId")
                if not isinstance(step_id, str):
                    raise RuntimeError("tool call context item has no step ID")
                calls, index, reasoning_content = _tool_calls_for_step(items, index, step_id)
                messages.append(
                    ProviderMessage(
                        role="assistant",
                        content="",
                        tool_calls=tuple(calls),
                        reasoning_content=reasoning_content,
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


def _tool_calls_for_step(
    items: Sequence[ContextItem],
    index: int,
    step_id: str,
) -> tuple[list[ToolCall], int, str | None]:
    calls: list[ToolCall] = []
    reasoning_content: str | None = None
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
        candidate_reasoning = _reasoning_content(candidate)
        if candidate_reasoning is not None:
            if reasoning_content is not None:
                raise RuntimeError("tool call step contains duplicate reasoning context")
            reasoning_content = candidate_reasoning
        calls.append(ToolCall(call_id, tool_name, arguments))
        index += 1
    return calls, index, reasoning_content


def _failure_reason_code(error: Exception) -> str:
    if isinstance(error, ProviderFailure):
        return f"provider_{error.category}"
    if isinstance(error, ProtectedValueError):
        return "protected_value"
    if isinstance(error, RuntimeError) and str(error).startswith("provider "):
        return "provider_protocol"
    if isinstance(error, ValueError) and str(error).startswith("unknown provider"):
        return "provider_unavailable"
    return "agent_error"


def _empty_protected_values() -> tuple[str, ...]:
    return ()
