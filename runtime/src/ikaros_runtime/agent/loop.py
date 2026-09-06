from __future__ import annotations

import asyncio
import logging
from collections.abc import Awaitable, Callable, Mapping, Sequence
from contextlib import suppress
from dataclasses import replace
from time import monotonic

from ..cancellation import CancellationToken, RunCancelled
from ..domain import JournalEvent, ModelUsage
from ..errors import (
    ContextBudgetExceededError,
    MemoryRetrievalError,
    ModelCallBudgetExceededError,
    ModelInputUnavailableError,
    ProtectedValueError,
    ProviderFailure,
    RunInputDriftError,
)
from ..memory import (
    MaterializedMemoryV1,
    MemoryRetrieverV1,
    MemorySnapshotReferenceV1,
)
from ..providers.base import (
    ProviderAdapter,
    ProviderRequest,
    ProviderResolver,
    ReasoningDelta,
    ResponseCompleted,
    ResponseMetadata,
    TextDelta,
    ToolCallCompleted,
)
from ..run_input import (
    EMPTY_FROZEN_MEMORY_CONTEXT_V1,
    ContextDataBlockV1,
    ContextItemRecordV1,
    FrozenMemoryContextV1,
    InstructionBlockV1,
    MemoryReferenceV1,
    OmissionRecordV1,
    ProviderExecutionSnapshot,
    RunConfig,
    config_input_token_counts,
    validate_tool_environment,
)
from ..security import (
    ProtectedStreamGuard,
    contains_protected_value,
    json_contains_protected_value,
)
from ..storage import SqliteRuntimeStore
from ..tools.core import (
    ToolCall,
    ToolExecutionCancelled,
    ToolExecutionContext,
    ToolExecutor,
    ToolResult,
    ToolTaskCancelled,
)
from ..tools.process_manager import ProcessManager
from .context import (
    ContextBuilder,
    build_history_status_context_data,
    build_memory_context_data,
    memory_context_data_characters,
)
from .model_input import ModelInputPlanner

EventPublisher = Callable[[JournalEvent], Awaitable[None]]
ProtectedValues = Callable[[], Sequence[str]]
ProviderSnapshotResolver = Callable[[str, str], ProviderExecutionSnapshot]
_LOGGER = logging.getLogger("ikaros_runtime.agent")
_MAX_REASONING_CHARACTERS = 1_000_000
_PROTECTED_TOOL_OUTPUT_MESSAGE = "Tool output contained protected configuration data."
_TEXT_DELTA_FLUSH_CHARACTERS = 256
_TEXT_DELTA_FLUSH_SECONDS = 0.05


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
        *,
        process_manager: ProcessManager | None = None,
        protected_values: ProtectedValues | None = None,
        context_builder: ContextBuilder | None = None,
        model_input_planner: ModelInputPlanner | None = None,
        memory_retriever: MemoryRetrieverV1 | None = None,
        provider_snapshot_resolver: ProviderSnapshotResolver | None = None,
        identity_core: InstructionBlockV1 | None = None,
    ) -> None:
        self._store = store
        self._providers = providers
        self._publish = publish
        self._tool_executor = tool_executor
        self._process_manager = process_manager
        self._protected_values = protected_values or _empty_protected_values
        self._context_builder = context_builder if context_builder is not None else ContextBuilder()
        self._model_input_planner = (
            model_input_planner if model_input_planner is not None else ModelInputPlanner()
        )
        self._memory_retriever = memory_retriever
        self._provider_snapshot_resolver = provider_snapshot_resolver
        self._identity_core = identity_core

    async def run(self, run_id: str, cancellation: CancellationToken) -> None:
        work: asyncio.Task[None] | None = None
        cancel_wait: asyncio.Task[None] | None = None
        status = "completed"
        reason: str | None = None
        try:
            cancellation.raise_if_cancelled()
            config = self._store.get_run_config(run_id)
            self._validate_submission_environment(config)
            await self._publish(self._store.mark_run_running(run_id))
            work = asyncio.create_task(self._run_steps(run_id, cancellation))
            cancel_wait = asyncio.create_task(cancellation.wait())
            done, _ = await asyncio.wait(
                {work, cancel_wait},
                timeout=config.max_duration_seconds,
                return_when=asyncio.FIRST_COMPLETED,
            )
            if work in done:
                await work
                cancellation.raise_if_cancelled()
            else:
                status = "cancelled" if cancellation.is_cancelled else "failed"
                reason = "cancelled" if cancellation.is_cancelled else "run_time_limit"
                cancellation.cancel()
                work.cancel()
                with suppress(asyncio.CancelledError, RunCancelled):
                    await work
        except (RunCancelled, asyncio.CancelledError):
            cancellation.cancel()
            status, reason = "cancelled", "cancelled"
        except Exception as error:
            _LOGGER.exception("Run %s failed", run_id)
            status, reason = "failed", _failure_reason_code(error)
        finally:
            if cancel_wait is not None:
                cancel_wait.cancel()
                with suppress(asyncio.CancelledError):
                    await cancel_wait
            if work is not None and not work.done():
                work.cancel()
                with suppress(asyncio.CancelledError, RunCancelled):
                    await work
            if self._process_manager is not None:
                try:
                    await asyncio.wait_for(self._process_manager.close_run(run_id), timeout=10)
                except Exception:
                    status, reason = "failed", "process_cleanup_failed"
            await self._publish_terminal_events(
                self._store.terminalize_run(
                    run_id,
                    status,
                    reason_code=reason,
                    _finish_open_model_step=status != "completed",
                )
            )

    async def _run_steps(self, run_id: str, cancellation: CancellationToken) -> None:
        config = self._store.get_run_config(run_id)
        provider = self._resolve_provider(config.provider_id)
        if provider is None:
            raise ValueError(f"unknown provider: {config.provider_id}")
        workspace_id = config.workspace.id if config.workspace is not None else None
        memory_context = self._select_memory_context(
            config=config,
            query=self._store.get_submission_user_content(run_id),
            workspace_id=workspace_id,
        )
        for step_ordinal in range(1, config.max_model_calls + 1):
            cancellation.raise_if_cancelled()
            prepared = self._store.prepare_model_step(
                run_id,
                step_ordinal=step_ordinal,
                memory_context=memory_context,
            )
            await self._publish(prepared.event)
            try:
                context_data = self._materialize_memory_context(
                    prepared.context_revision.memory,
                    workspace_id=workspace_id,
                    expected_context_data_characters=prepared.context_revision.memory_context_characters,
                )
                plan = self._model_input_planner.build_plan(
                    config=config,
                    items=prepared.items,
                    context_data=(
                        *context_data,
                        *build_history_status_context_data(
                            prepared.context_revision.history_status
                        ),
                    ),
                    budget_snapshot=prepared.step_input.budget,
                )
                request = self._context_builder.build_request(plan)
            except Exception as error:
                await self._publish_terminal_events(
                    self._store.fail_provider_step(
                        run_id,
                        step_ordinal=step_ordinal,
                        outcome="cancelled" if isinstance(error, RunCancelled) else "failed",
                        reason_code=_failure_reason_code(error),
                        assistant_item_id=None,
                    )
                )
                raise
            calls, item_ids = await self._provider_step(
                run_id,
                provider,
                request,
                cancellation,
                step_ordinal=step_ordinal,
            )
            if not calls:
                return
            await self._execute_tool_calls(
                calls,
                item_ids,
                cancellation,
                run_id=run_id,
                thread_id=config.thread_id,
                step_ordinal=step_ordinal,
                default_cwd=config.workspace.root_uri if config.workspace is not None else None,
            )
        raise ModelCallBudgetExceededError("model call budget exhausted")

    async def cancel(self, run_id: str) -> None:
        if self._process_manager is not None:
            await self._process_manager.close_run(run_id)
        await self._publish_terminal_events(
            self._store.terminalize_run(
                run_id,
                "cancelled",
                reason_code="cancelled",
                _finish_open_model_step=True,
            )
        )

    def _select_memory_context(
        self,
        *,
        config: RunConfig,
        query: str,
        workspace_id: str | None,
    ) -> FrozenMemoryContextV1:
        retriever = self._memory_retriever
        if retriever is None:
            return EMPTY_FROZEN_MEMORY_CONTEXT_V1
        self._assert_memory_read_outside_state_transaction()
        instruction_tokens, tool_tokens = config_input_token_counts(config)
        user_tokens = ContextItemRecordV1(
            config.user_item_id, config.turn_id, config.run_id, "message", "user", query, {}
        ).estimated_tokens
        remaining = (
            config.maximum_input_tokens
            - config.reserved_current_run_tokens
            - instruction_tokens
            - tool_tokens
            - user_tokens
        )
        # Memory may use at most half of the remaining input capacity. Also leave
        # 4 Ki estimated tokens for history and an omitted-failure status notice;
        # a small model window therefore drops Memory before blocking user input.
        memory_allowance = max(0, min(remaining // 2, remaining - 4096))

        def fits_memory_capacity(records: Sequence[MaterializedMemoryV1]) -> bool:
            return 4 * (
                sum(record.characters for record in records)
                + memory_context_data_characters(records)
            ) <= memory_allowance

        retrieval = retriever.retrieve(
            query=query,
            workspace_id=workspace_id,
            accept_selection=fits_memory_capacity,
        )
        references = tuple(
            MemoryReferenceV1(
                memory_id=record.memory_id,
                revision=record.revision,
                scope=record.scope,
                characters=record.characters,
            )
            for record in retrieval.selected
        )
        omissions = tuple(
            OmissionRecordV1(
                source_type="memory",
                source_id=omission.memory_id,
                revision=omission.revision,
                characters=omission.characters,
                reason=omission.reason,
            )
            for omission in retrieval.omissions
        )
        return FrozenMemoryContextV1(
            memory=references,
            omissions=omissions,
            memory_characters=retrieval.memory_characters,
            context_data_characters=memory_context_data_characters(retrieval.selected),
        )

    def _materialize_memory_context(
        self,
        references: Sequence[MemoryReferenceV1],
        *,
        workspace_id: str | None,
        expected_context_data_characters: int,
    ) -> tuple[ContextDataBlockV1, ...]:
        if not references:
            if expected_context_data_characters != 0:
                raise MemoryRetrievalError("memory_snapshot_unavailable")
            return ()
        retriever = self._memory_retriever
        if retriever is None:
            raise MemoryRetrievalError("memory_snapshot_unavailable")
        self._assert_memory_read_outside_state_transaction()
        materialized = retriever.materialize_exact(
            tuple(
                MemorySnapshotReferenceV1(
                    memory_id=reference.memory_id,
                    revision=reference.revision,
                    scope=reference.scope,
                    characters=reference.characters,
                )
                for reference in references
            ),
            workspace_id=workspace_id,
        )
        if memory_context_data_characters(materialized) != expected_context_data_characters:
            raise MemoryRetrievalError("memory_snapshot_unavailable")
        return build_memory_context_data(materialized)

    def _assert_memory_read_outside_state_transaction(self) -> None:
        if self._store.in_transaction:
            raise RuntimeError("Memory read attempted during a state database transaction")

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
    ) -> tuple[tuple[ToolCall, ...], tuple[str, ...]]:
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
        response_model_id: str | None = None
        request_id: str | None = None
        step_finished = False
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
                elif isinstance(event, ResponseMetadata):
                    response_model_id, request_id = self._merge_response_metadata(
                        response_model_id,
                        request_id,
                        model_id=event.model_id,
                        request_id=event.request_id,
                    )
                elif isinstance(event, ResponseCompleted):
                    await self._publish_text_delta(assistant_item_id, text_batch.flush())
                    response_usage = event.usage
                    if event.model_id is not None or event.request_id is not None:
                        response_model_id, request_id = self._merge_response_metadata(
                            response_model_id,
                            request_id,
                            model_id=event.model_id,
                            request_id=event.request_id,
                        )
                    completed = True
                else:
                    raise RuntimeError("provider emitted an unknown event")
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
            if assistant_item_id is None and not tool_calls:
                raise RuntimeError("provider completed without text or a tool call")
            self._assert_tool_request_safe(tool_calls, reasoning_content)
            self._assert_response_metadata_safe(response_model_id, request_id)
            completion = self._store.complete_provider_step(
                run_id,
                step_ordinal=step_ordinal,
                assistant_item_id=assistant_item_id,
                tool_calls=tool_calls,
                reasoning_content=reasoning_content,
                usage=response_usage,
                response_model_id=response_model_id,
                request_id=request_id,
            )
            step_finished = True
            await self._publish_terminal_events(completion.events)
            return tuple(tool_calls), completion.tool_call_item_ids
        except (Exception, asyncio.CancelledError) as error:
            if step_finished:
                raise
            # Only text already released by ProtectedStreamGuard is buffered.  Never
            # call finish() on an incomplete/cancelled stream, because that could
            # release a protected trailing prefix.
            if self._current_protected_values() == tuple(protected_values):
                await self._publish_text_delta(assistant_item_id, text_batch.flush())
            terminal_error: Exception = error if isinstance(error, Exception) else RunCancelled()
            if isinstance(error, ProviderFailure) and error.request_id is not None:
                try:
                    response_model_id, request_id = self._merge_response_metadata(
                        response_model_id,
                        request_id,
                        model_id=None,
                        request_id=error.request_id,
                    )
                except RuntimeError as metadata_error:
                    terminal_error = metadata_error
            response_model_id, request_id = self._safe_response_metadata(
                response_model_id,
                request_id,
                protected_snapshot=protected_values,
            )
            outcome = "cancelled" if isinstance(terminal_error, RunCancelled) else "failed"
            await self._publish_terminal_events(
                self._store.fail_provider_step(
                    run_id,
                    step_ordinal=step_ordinal,
                    outcome=outcome,
                    reason_code=_failure_reason_code(terminal_error),
                    assistant_item_id=assistant_item_id,
                    response_model_id=response_model_id,
                    request_id=request_id,
                )
            )
            if terminal_error is not error:
                raise terminal_error from None
            raise

    async def _publish_text_delta(self, item_id: str | None, delta: str | None) -> None:
        if delta is None:
            return
        if item_id is None:
            raise RuntimeError("text delta has no assistant item")
        await self._publish(self._store.append_text_delta(item_id, delta))

    async def _execute_tool_calls(
        self,
        calls: Sequence[ToolCall],
        item_ids: Sequence[str],
        cancellation: CancellationToken,
        *,
        default_cwd: str | None,
        run_id: str,
        thread_id: str,
        step_ordinal: int,
    ) -> None:
        executor = self._tool_executor
        if executor is None:
            raise RuntimeError("provider requested a tool but no ToolExecutor is available")
        call_items = list(zip(calls, item_ids, strict=True))
        for index, (call, item_id) in enumerate(call_items):
            protected_snapshot = self._current_protected_values()
            try:
                cancellation.raise_if_cancelled()
                result = await executor.execute(
                    call,
                    cancellation=cancellation,
                    context=ToolExecutionContext(
                        run_id=run_id,
                        thread_id=thread_id,
                        step_ordinal=step_ordinal,
                        item_id=item_id,
                        default_cwd=default_cwd,
                    ),
                )
            except (ToolExecutionCancelled, ToolTaskCancelled) as error:
                await self._publish_tool_result(
                    item_id,
                    call,
                    error.result,
                    "cancelled",
                    protected_snapshot=protected_snapshot,
                )
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
            await self._publish_tool_result(
                item_id,
                call,
                result,
                status,
                protected_snapshot=protected_snapshot,
            )

    async def _publish_tool_result(
        self,
        item_id: str,
        call: ToolCall,
        result: ToolResult,
        status: str,
        *,
        protected_snapshot: Sequence[str] = (),
    ) -> None:
        if result.file_change is not None:
            result = replace(result, file_change=result.file_change.protected(protected_snapshot))
        result, protected = self._safe_tool_result(call, result)
        if protected and status != "cancelled":
            status = "failed"
        await self._publish_terminal_events(
            self._store.complete_tool_call(
                item_id,
                status=status,
                result=result.to_wire(),
                result_content=result.to_model_content(),
                file_change=result.file_change,
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

    def _assert_response_metadata_safe(
        self,
        response_model_id: str | None,
        request_id: str | None,
    ) -> None:
        values = self._current_protected_values()
        for value in (response_model_id, request_id):
            if value is None:
                continue
            if (
                not value
                or len(value) > 200
                or any(ord(character) < 32 or ord(character) == 127 for character in value)
                or contains_protected_value(value, values)
            ):
                raise RuntimeError("provider response metadata is unsafe")

    def _merge_response_metadata(
        self,
        response_model_id: str | None,
        response_request_id: str | None,
        *,
        model_id: str | None,
        request_id: str | None,
    ) -> tuple[str | None, str | None]:
        if model_id is None and request_id is None:
            raise RuntimeError("provider emitted empty response metadata")
        self._assert_response_metadata_safe(model_id, request_id)
        if response_model_id is not None and model_id is not None and response_model_id != model_id:
            raise RuntimeError("provider emitted conflicting response model IDs")
        if (
            response_request_id is not None
            and request_id is not None
            and response_request_id != request_id
        ):
            raise RuntimeError("provider emitted conflicting request IDs")
        return model_id or response_model_id, request_id or response_request_id

    def _safe_response_metadata(
        self,
        response_model_id: str | None,
        request_id: str | None,
        *,
        protected_snapshot: Sequence[str],
    ) -> tuple[str | None, str | None]:
        protected_values = tuple(
            dict.fromkeys((*protected_snapshot, *self._current_protected_values()))
        )

        def safe(value: str | None) -> str | None:
            if value is None:
                return None
            if (
                not value
                or len(value) > 200
                or any(ord(character) < 32 or ord(character) == 127 for character in value)
                or contains_protected_value(value, protected_values)
            ):
                return None
            return value

        return safe(response_model_id), safe(request_id)

    def _safe_tool_result(
        self,
        call: ToolCall,
        result: ToolResult,
    ) -> tuple[ToolResult, bool]:
        protected_values = self._current_protected_values()
        if result.file_change is not None:
            result = replace(result, file_change=result.file_change.protected(protected_values))
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
                    **(
                        {"path": result.file_change.path}
                        if result.file_change is not None and result.file_change.path is not None
                        else {}
                    ),
                },
                cancelled=result.cancelled,
                file_change=result.file_change,
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

    def _resolve_provider(self, provider_id: str) -> ProviderAdapter | None:
        if isinstance(self._providers, Mapping):
            return self._providers.get(provider_id)
        return self._providers.resolve(provider_id)

    def _validate_submission_environment(self, frame: RunConfig) -> None:
        if frame.identity_core != self._identity_core:
            raise RunInputDriftError("identity_core_changed")

        resolver = self._provider_snapshot_resolver
        try:
            if resolver is None:
                if not isinstance(self._providers, Mapping):
                    raise RunInputDriftError("provider_configuration_changed")
                current_provider = ProviderExecutionSnapshot(
                    provider_id=frame.provider_id,
                    origin="test",
                    base_url=None,
                    model_id=frame.model_id,
                    supports_tools=True,
                )
            else:
                current_provider = resolver(frame.provider_id, frame.model_id)
        except RunInputDriftError:
            raise
        except Exception:
            raise RunInputDriftError("provider_configuration_changed") from None
        current_provider = replace(
            current_provider,
            context_window=frame.context_window,
            max_output_tokens=frame.max_output_tokens,
        )
        if current_provider.fingerprint != frame.public_provider_config_fingerprint:
            raise RunInputDriftError("provider_configuration_changed")

        definitions = self._tool_executor.definitions if self._tool_executor is not None else ()
        if not validate_tool_environment(frame.tools, definitions):
            raise RunInputDriftError("tool_definitions_changed")
        current_policy = (
            self._tool_executor.policy_name if self._tool_executor is not None else "full_access"
        )
        if frame.execution_policy != current_policy:
            raise RunInputDriftError("execution_policy_changed")


def _failure_reason_code(error: Exception) -> str:
    if isinstance(error, ModelCallBudgetExceededError):
        return "model_call_budget_exceeded"
    if isinstance(error, RunCancelled):
        return "cancelled"
    if isinstance(error, RunInputDriftError):
        return error.reason_code
    if isinstance(error, ContextBudgetExceededError):
        return "context_budget_exceeded"
    if isinstance(error, ModelInputUnavailableError):
        return "model_input_unavailable"
    if isinstance(error, MemoryRetrievalError):
        return error.reason_code
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
