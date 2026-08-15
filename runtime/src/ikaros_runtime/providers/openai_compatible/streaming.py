from __future__ import annotations

import asyncio
from collections.abc import AsyncIterator, Awaitable, Callable, Sequence
from contextlib import suppress
from dataclasses import dataclass, field
from typing import Any

import httpx

from ...cancellation import CancellationToken, RunCancelled
from ...domain import ModelUsage
from ...errors import ProviderFailure
from ...json_codec import loads as json_loads
from ...security import contains_protected_value, json_contains_protected_value
from ...tools.core import ToolCall
from ..base import (
    ProviderEvent,
    ReasoningDelta,
    ResponseCompleted,
    ResponseMetadata,
    TextDelta,
    ToolCallCompleted,
)

MAX_SSE_EVENT_CHARACTERS = 1_000_000
MAX_TOOL_ARGUMENT_CHARACTERS = 1_000_000
PROTECTED_RESPONSE_MESSAGE = "provider response contained protected configuration data"
_SQLITE_MAX_INTEGER = (1 << 63) - 1


@dataclass(slots=True)
class ResponseToolAccumulator:
    id: str | None = None
    name: str | None = None
    argument_parts: list[str] = field(default_factory=list)
    argument_characters: int = 0


class ResponseAssembler:
    def __init__(self, secrets: Sequence[str]) -> None:
        self._tools: dict[int, ResponseToolAccumulator] = {}
        self._finish_reason: str | None = None
        self._usage: ModelUsage | None = None
        self._model_id: str | None = None
        self._secrets = tuple(secrets)
        self.saw_output = False

    def consume(self, value: Any) -> tuple[ProviderEvent, ...]:
        if not isinstance(value, dict):
            raise protocol_failure()
        events: list[ProviderEvent] = []
        if "model" in value:
            model_id = value["model"]
            if (
                not isinstance(model_id, str)
                or not model_id
                or len(model_id) > 200
                or any(ord(character) < 32 or ord(character) == 127 for character in model_id)
            ):
                raise protocol_failure()
            if contains_protected_value(model_id, self._secrets):
                raise protected_response_failure()
            if self._model_id is not None and self._model_id != model_id:
                raise protocol_failure()
            if self._model_id is None:
                events.append(ResponseMetadata(model_id=model_id))
            self._model_id = model_id
        if "usage" in value and value["usage"] is not None:
            usage = _parse_usage(value["usage"])
            if self._usage is not None and self._usage != usage:
                raise protocol_failure()
            self._usage = usage
            self.saw_output = True
        choices = value.get("choices")
        if not isinstance(choices, list):
            raise protocol_failure()
        if not choices:
            return tuple(events)
        choice = next(
            (
                candidate
                for candidate in choices
                if isinstance(candidate, dict) and candidate.get("index", 0) == 0
            ),
            None,
        )
        if choice is None:
            return tuple(events)
        delta = choice.get("delta")
        if delta is None:
            delta = {}
        if not isinstance(delta, dict):
            raise protocol_failure()

        if "reasoning_content" in delta:
            reasoning = delta["reasoning_content"]
            if reasoning is not None and not isinstance(reasoning, str):
                raise protocol_failure()
            self.saw_output = True
            events.append(ReasoningDelta(reasoning or ""))

        if "content" in delta:
            content = delta["content"]
            if content is not None and not isinstance(content, str):
                raise protocol_failure()
            if content:
                self.saw_output = True
                events.append(TextDelta(content))

        if "tool_calls" in delta:
            tool_calls = delta["tool_calls"]
            if not isinstance(tool_calls, list):
                raise protocol_failure()
            for fragment in tool_calls:
                self._consume_tool_fragment(fragment)

        finish_reason = choice.get("finish_reason")
        if finish_reason is not None:
            if not isinstance(finish_reason, str):
                raise protocol_failure()
            if self._finish_reason is not None and self._finish_reason != finish_reason:
                raise protocol_failure()
            self._finish_reason = finish_reason
        return tuple(events)

    def finish(
        self,
        *,
        done_seen: bool,
        request_id: str | None = None,
    ) -> tuple[ProviderEvent, ...]:
        if self._finish_reason is None and not done_seen:
            raise protocol_failure()
        events: list[ProviderEvent] = []
        if self._tools:
            if self._finish_reason != "tool_calls":
                raise protocol_failure()
            seen_ids: set[str] = set()
            for index in sorted(self._tools):
                accumulator = self._tools[index]
                if not accumulator.id or not accumulator.name or accumulator.id in seen_ids:
                    raise protocol_failure()
                seen_ids.add(accumulator.id)
                source = "".join(accumulator.argument_parts)
                if any(
                    contains_protected_value(value, self._secrets)
                    for value in (accumulator.id, accumulator.name, source)
                ):
                    raise protected_response_failure()
                try:
                    arguments = {} if not source else json_loads(source)
                except ValueError:
                    raise protocol_failure() from None
                if not isinstance(arguments, dict):
                    raise protocol_failure()
                if json_contains_protected_value(arguments, self._secrets):
                    raise protected_response_failure()
                events.append(
                    ToolCallCompleted(
                        ToolCall(
                            id=accumulator.id,
                            name=accumulator.name,
                            arguments=arguments,
                        )
                    )
                )
        events.append(ResponseCompleted(self._usage, self._model_id, request_id))
        return tuple(events)

    def _consume_tool_fragment(self, value: Any) -> None:
        if not isinstance(value, dict):
            raise protocol_failure()
        index = value.get("index")
        if not isinstance(index, int) or isinstance(index, bool) or index < 0:
            raise protocol_failure()
        accumulator = self._tools.setdefault(index, ResponseToolAccumulator())
        tool_type = value.get("type")
        if tool_type is not None and tool_type != "function":
            raise protocol_failure()
        call_id = value.get("id")
        if call_id is not None:
            if not isinstance(call_id, str) or not call_id:
                raise protocol_failure()
            if accumulator.id is not None and accumulator.id != call_id:
                raise protocol_failure()
            accumulator.id = call_id
        function = value.get("function")
        if function is not None:
            if not isinstance(function, dict):
                raise protocol_failure()
            name = function.get("name")
            if name is not None:
                if not isinstance(name, str) or not name:
                    raise protocol_failure()
                if accumulator.name is not None and accumulator.name != name:
                    raise protocol_failure()
                accumulator.name = name
            arguments = function.get("arguments")
            if arguments is not None:
                if not isinstance(arguments, str):
                    raise protocol_failure()
                accumulator.argument_parts.append(arguments)
                accumulator.argument_characters += len(arguments)
                if accumulator.argument_characters > MAX_TOOL_ARGUMENT_CHARACTERS:
                    raise protocol_failure()
                self.saw_output = True


def _parse_usage(value: Any) -> ModelUsage:
    if not isinstance(value, dict):
        raise protocol_failure()
    input_tokens = _required_token_count(value, "prompt_tokens")
    output_tokens = _required_token_count(value, "completion_tokens")
    total_tokens = _required_token_count(value, "total_tokens")

    cached_input_tokens = _optional_token_count(value, "prompt_cache_hit_tokens")
    prompt_details = value.get("prompt_tokens_details")
    if prompt_details is not None:
        if not isinstance(prompt_details, dict):
            raise protocol_failure()
        nested_cached = _optional_token_count(prompt_details, "cached_tokens")
        if (
            cached_input_tokens is not None
            and nested_cached is not None
            and cached_input_tokens != nested_cached
        ):
            raise protocol_failure()
        if cached_input_tokens is None:
            cached_input_tokens = nested_cached

    reasoning_output_tokens: int | None = None
    completion_details = value.get("completion_tokens_details")
    if completion_details is not None:
        if not isinstance(completion_details, dict):
            raise protocol_failure()
        reasoning_output_tokens = _optional_token_count(
            completion_details,
            "reasoning_tokens",
        )

    if cached_input_tokens is not None and cached_input_tokens > input_tokens:
        raise protocol_failure()
    if reasoning_output_tokens is not None and reasoning_output_tokens > output_tokens:
        raise protocol_failure()

    return ModelUsage(
        input_tokens=input_tokens,
        cached_input_tokens=cached_input_tokens,
        output_tokens=output_tokens,
        reasoning_output_tokens=reasoning_output_tokens,
        total_tokens=total_tokens,
    )


def _required_token_count(value: dict[str, Any], key: str) -> int:
    if key not in value:
        raise protocol_failure()
    count = value[key]
    if (
        not isinstance(count, int)
        or isinstance(count, bool)
        or count < 0
        or count > _SQLITE_MAX_INTEGER
    ):
        raise protocol_failure()
    return count


def _optional_token_count(value: dict[str, Any], key: str) -> int | None:
    if key not in value or value[key] is None:
        return None
    count = value[key]
    if (
        not isinstance(count, int)
        or isinstance(count, bool)
        or count < 0
        or count > _SQLITE_MAX_INTEGER
    ):
        raise protocol_failure()
    return count


async def sse_payloads(
    response: httpx.Response,
    *,
    cancellation: CancellationToken,
    idle_timeout: float,
) -> AsyncIterator[str]:
    lines = response.aiter_lines().__aiter__()
    data_lines: list[str] = []
    data_characters = 0
    while True:
        try:
            line = await await_cancellable(
                anext(lines),
                cancellation=cancellation,
                wait_seconds=idle_timeout,
            )
        except StopAsyncIteration:
            if data_lines:
                yield "\n".join(data_lines)
            return
        if line == "":
            if data_lines:
                yield "\n".join(data_lines)
                data_lines = []
                data_characters = 0
            continue
        if line.startswith(":"):
            continue
        field, separator, raw_value = line.partition(":")
        if field != "data":
            continue
        value = raw_value[1:] if separator and raw_value.startswith(" ") else raw_value
        data_characters += len(value)
        if data_characters > MAX_SSE_EVENT_CHARACTERS:
            raise protocol_failure()
        data_lines.append(value)


async def await_cancellable[T](
    awaitable: Awaitable[T],
    *,
    cancellation: CancellationToken,
    wait_seconds: float,
    cancelled_result_cleanup: Callable[[T], Awaitable[None]] | None = None,
) -> T:
    cancellation.raise_if_cancelled()
    operation = asyncio.ensure_future(awaitable)
    cancelled = asyncio.create_task(cancellation.wait())
    try:
        done, _pending = await asyncio.wait(
            {operation, cancelled},
            timeout=wait_seconds,
            return_when=asyncio.FIRST_COMPLETED,
        )
        if operation in done:
            result = operation.result()
            if cancelled in done:
                if cancelled_result_cleanup is not None:
                    with suppress(Exception):
                        await cancelled_result_cleanup(result)
                raise RunCancelled
            return result
        if cancelled in done:
            operation.cancel()
            await asyncio.gather(operation, return_exceptions=True)
            raise RunCancelled
        operation.cancel()
        await asyncio.gather(operation, return_exceptions=True)
        raise TimeoutError
    except BaseException:
        operation.cancel()
        await asyncio.gather(operation, return_exceptions=True)
        raise
    finally:
        cancelled.cancel()
        await asyncio.gather(cancelled, return_exceptions=True)


async def close_response(response: httpx.Response) -> None:
    await response.aclose()


def protocol_failure() -> ProviderFailure:
    return ProviderFailure("protocol", "provider returned an invalid streaming response")


def protected_response_failure() -> ProviderFailure:
    return ProviderFailure("protocol", PROTECTED_RESPONSE_MESSAGE)
