from __future__ import annotations

import asyncio
import math
from collections.abc import AsyncIterator, Awaitable, Callable, Sequence
from contextlib import suppress
from dataclasses import dataclass, field
from datetime import UTC, datetime
from email.utils import parsedate_to_datetime
from typing import Any, Literal

import httpx

from .cancellation import CancellationToken, RunCancelled
from .config import ModelConfig, ProviderConfig
from .json_codec import dumps as json_dumps
from .json_codec import loads as json_loads
from .providers import (
    ProviderEvent,
    ProviderMessage,
    ProviderRequest,
    ReasoningDelta,
    ResponseCompleted,
    TextDelta,
    ToolCallCompleted,
)
from .security import (
    ProtectedStreamGuard,
    ProtectedValueError,
    contains_protected_value,
    json_contains_protected_value,
)
from .tools import ToolCall, ToolDefinition

ProviderFailureCategory = Literal[
    "authentication",
    "rate_limit",
    "context_overflow",
    "invalid_request",
    "timeout",
    "network",
    "server",
    "cancelled",
    "protocol",
    "unknown",
]

_MAX_SSE_EVENT_CHARACTERS = 1_000_000
_MAX_TOOL_ARGUMENT_CHARACTERS = 1_000_000
_MAX_RETRY_DELAY_SECONDS = 5.0
_REQUEST_ID_HEADERS = ("x-request-id", "request-id", "x-amzn-requestid")
_PROTECTED_RESPONSE_MESSAGE = "provider response contained protected configuration data"
_SAFE_PROVIDER_FAILURE_MESSAGES = frozenset(
    {
        "provider transport is closed",
        "provider request failed",
        "provider response headers timed out",
        "provider connection timed out",
        "provider network request failed",
        "provider request could not be built",
        "provider returned a non-streaming response",
        "provider stream became idle",
        "provider stream timed out",
        "provider stream was interrupted",
        "selected provider model is unavailable",
        "provider authentication failed",
        "provider rate limit was reached",
        "provider rejected the request context size",
        "provider request timed out",
        "provider server failed",
        "provider rejected the request",
        "provider context contains an invalid message",
        "provider returned an invalid streaming response",
        _PROTECTED_RESPONSE_MESSAGE,
    }
)


class ProviderFailure(RuntimeError):
    def __init__(
        self,
        category: ProviderFailureCategory,
        safe_message: str,
        *,
        status_code: int | None = None,
        request_id: str | None = None,
        retryable: bool = False,
        retry_after: float | None = None,
    ) -> None:
        super().__init__(safe_message)
        self.category = category
        self.status_code = status_code
        self.request_id = request_id
        self.retryable = retryable
        self.retry_after = retry_after

    def __repr__(self) -> str:
        return (
            "ProviderFailure("
            f"category={self.category!r}, status_code={self.status_code!r}, "
            f"retryable={self.retryable!r})"
        )


@dataclass(frozen=True, slots=True)
class ProviderTimeouts:
    connect: float = 10.0
    response_header: float = 30.0
    stream_idle: float = 120.0

    def validate(self) -> None:
        if self.connect <= 0 or self.response_header <= 0 or self.stream_idle <= 0:
            raise ValueError("provider timeouts must be positive")


@dataclass(slots=True)
class _ToolAccumulator:
    id: str | None = None
    name: str | None = None
    argument_parts: list[str] = field(default_factory=list)
    argument_characters: int = 0


class _ResponseAssembler:
    def __init__(self, secrets: Sequence[str]) -> None:
        self._tools: dict[int, _ToolAccumulator] = {}
        self._finish_reason: str | None = None
        self._secrets = tuple(secrets)
        self.saw_output = False

    def consume(self, value: Any) -> tuple[ProviderEvent, ...]:
        if not isinstance(value, dict):
            raise _protocol_failure()
        choices = value.get("choices")
        if not isinstance(choices, list):
            raise _protocol_failure()
        if not choices:
            return ()
        choice = next(
            (
                candidate
                for candidate in choices
                if isinstance(candidate, dict) and candidate.get("index", 0) == 0
            ),
            None,
        )
        if choice is None:
            return ()
        delta = choice.get("delta")
        if delta is None:
            delta = {}
        if not isinstance(delta, dict):
            raise _protocol_failure()

        events: list[ProviderEvent] = []
        if "reasoning_content" in delta:
            reasoning = delta["reasoning_content"]
            if reasoning is not None and not isinstance(reasoning, str):
                raise _protocol_failure()
            self.saw_output = True
            events.append(ReasoningDelta(reasoning or ""))

        if "content" in delta:
            content = delta["content"]
            if content is not None and not isinstance(content, str):
                raise _protocol_failure()
            if content:
                self.saw_output = True
                events.append(TextDelta(content))

        if "tool_calls" in delta:
            tool_calls = delta["tool_calls"]
            if not isinstance(tool_calls, list):
                raise _protocol_failure()
            for fragment in tool_calls:
                self._consume_tool_fragment(fragment)

        finish_reason = choice.get("finish_reason")
        if finish_reason is not None:
            if not isinstance(finish_reason, str):
                raise _protocol_failure()
            if self._finish_reason is not None and self._finish_reason != finish_reason:
                raise _protocol_failure()
            self._finish_reason = finish_reason
        return tuple(events)

    def finish(self, *, done_seen: bool) -> tuple[ProviderEvent, ...]:
        if self._finish_reason is None and not done_seen:
            raise _protocol_failure()
        events: list[ProviderEvent] = []
        if self._tools:
            if self._finish_reason != "tool_calls":
                raise _protocol_failure()
            seen_ids: set[str] = set()
            for index in sorted(self._tools):
                accumulator = self._tools[index]
                if not accumulator.id or not accumulator.name or accumulator.id in seen_ids:
                    raise _protocol_failure()
                seen_ids.add(accumulator.id)
                source = "".join(accumulator.argument_parts)
                if any(
                    contains_protected_value(value, self._secrets)
                    for value in (accumulator.id, accumulator.name, source)
                ):
                    raise _protected_response_failure()
                try:
                    arguments = {} if not source else json_loads(source)
                except ValueError:
                    raise _protocol_failure() from None
                if not isinstance(arguments, dict):
                    raise _protocol_failure()
                if json_contains_protected_value(arguments, self._secrets):
                    raise _protected_response_failure()
                events.append(
                    ToolCallCompleted(
                        ToolCall(
                            id=accumulator.id,
                            name=accumulator.name,
                            arguments=arguments,
                        )
                    )
                )
        events.append(ResponseCompleted())
        return tuple(events)

    def _consume_tool_fragment(self, value: Any) -> None:
        if not isinstance(value, dict):
            raise _protocol_failure()
        index = value.get("index")
        if not isinstance(index, int) or isinstance(index, bool) or index < 0:
            raise _protocol_failure()
        accumulator = self._tools.setdefault(index, _ToolAccumulator())
        tool_type = value.get("type")
        if tool_type is not None and tool_type != "function":
            raise _protocol_failure()
        call_id = value.get("id")
        if call_id is not None:
            if not isinstance(call_id, str) or not call_id:
                raise _protocol_failure()
            if accumulator.id is not None and accumulator.id != call_id:
                raise _protocol_failure()
            accumulator.id = call_id
        function = value.get("function")
        if function is not None:
            if not isinstance(function, dict):
                raise _protocol_failure()
            name = function.get("name")
            if name is not None:
                if not isinstance(name, str) or not name:
                    raise _protocol_failure()
                if accumulator.name is not None and accumulator.name != name:
                    raise _protocol_failure()
                accumulator.name = name
            arguments = function.get("arguments")
            if arguments is not None:
                if not isinstance(arguments, str):
                    raise _protocol_failure()
                accumulator.argument_parts.append(arguments)
                accumulator.argument_characters += len(arguments)
                if accumulator.argument_characters > _MAX_TOOL_ARGUMENT_CHARACTERS:
                    raise _protocol_failure()
                self.saw_output = True


class OpenAICompatibleAdapter:
    def __init__(
        self,
        provider: ProviderConfig,
        *,
        client: httpx.AsyncClient | None = None,
        timeouts: ProviderTimeouts | None = None,
        max_retries: int = 1,
    ) -> None:
        if max_retries < 0:
            raise ValueError("max_retries must be non-negative")
        self._provider = provider
        self._secrets = _provider_secrets(provider)
        self._timeouts = timeouts or ProviderTimeouts()
        self._timeouts.validate()
        self._max_retries = max_retries
        self._owns_client = client is None
        self._client = client or httpx.AsyncClient(
            timeout=httpx.Timeout(
                connect=self._timeouts.connect,
                read=None,
                write=self._timeouts.response_header,
                pool=self._timeouts.connect,
            ),
            follow_redirects=False,
        )
        self._closed = False

    async def aclose(self) -> None:
        if self._closed:
            return
        self._closed = True
        if self._owns_client:
            with suppress(Exception):
                await self._client.aclose()

    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]:
        copied_failure: (
            tuple[
                ProviderFailureCategory,
                str,
                int | None,
                str | None,
                bool,
                float | None,
            ]
            | None
        ) = None
        unknown_failure = False
        try:
            async for event in self._stream_impl(request, cancellation=cancellation):
                yield event
            return
        except RunCancelled:
            raise
        except ProviderFailure as error:
            message = str(error)
            safe_message = (
                message if message in _SAFE_PROVIDER_FAILURE_MESSAGES else "provider request failed"
            )
            copied_failure = (
                error.category,
                safe_message,
                error.status_code,
                error.request_id,
                error.retryable,
                error.retry_after,
            )
        except Exception:
            unknown_failure = True

        if copied_failure is not None:
            category, message, status_code, request_id, retryable, retry_after = copied_failure
            raise ProviderFailure(
                category,
                message,
                status_code=status_code,
                request_id=request_id,
                retryable=retryable,
                retry_after=retry_after,
            ) from None
        if unknown_failure:
            raise ProviderFailure("unknown", "provider request failed") from None
        raise ProviderFailure("unknown", "provider request failed") from None

    async def _stream_impl(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]:
        if self._closed:
            raise ProviderFailure("unknown", "provider transport is closed")
        body = self._request_body(request)
        headers = self._request_headers()
        url = f"{self._provider.base_url.rstrip('/')}/chat/completions"
        for attempt in range(self._max_retries + 1):
            try:
                async for event in self._attempt(
                    url,
                    headers,
                    body,
                    cancellation=cancellation,
                ):
                    yield event
                return
            except RunCancelled:
                raise
            except ProviderFailure as error:
                if attempt >= self._max_retries or not error.retryable:
                    raise
                delay = error.retry_after
                if delay is None:
                    delay = min(0.25 * (2**attempt), _MAX_RETRY_DELAY_SECONDS)
                await cancellation.sleep(min(delay, _MAX_RETRY_DELAY_SECONDS))
        raise ProviderFailure("unknown", "provider request failed")

    async def _attempt(
        self,
        url: str,
        headers: dict[str, str],
        body: dict[str, object],
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]:
        cancellation.raise_if_cancelled()
        try:
            request = self._client.build_request("POST", url, headers=headers, json=body)
            response = await _await_cancellable(
                self._client.send(request, stream=True),
                cancellation=cancellation,
                wait_seconds=self._timeouts.response_header,
                cancelled_result_cleanup=_close_response,
            )
        except RunCancelled:
            raise
        except TimeoutError:
            raise ProviderFailure(
                "timeout",
                "provider response headers timed out",
                retryable=True,
            ) from None
        except httpx.TimeoutException:
            raise ProviderFailure(
                "timeout", "provider connection timed out", retryable=True
            ) from None
        except httpx.RequestError:
            raise ProviderFailure(
                "network", "provider network request failed", retryable=True
            ) from None
        except (TypeError, ValueError):
            raise ProviderFailure(
                "invalid_request", "provider request could not be built"
            ) from None

        try:
            if response.status_code >= 400:
                raise self._http_failure(response)
            content_type = response.headers.get("content-type", "")
            if not content_type.lower().startswith("text/event-stream"):
                raise ProviderFailure("protocol", "provider returned a non-streaming response")
            assembler = _ResponseAssembler(self._secrets)
            text_guard = ProtectedStreamGuard(self._secrets)
            reasoning_guard = ProtectedStreamGuard(self._secrets)
            done_seen = False
            try:
                async for payload in _sse_payloads(
                    response,
                    cancellation=cancellation,
                    idle_timeout=self._timeouts.stream_idle,
                ):
                    if payload.strip() == "[DONE]":
                        done_seen = True
                        break
                    try:
                        value = json_loads(payload)
                    except ValueError:
                        raise _protocol_failure() from None
                    for event in assembler.consume(value):
                        if isinstance(event, TextDelta):
                            safe_delta = text_guard.feed(event.delta)
                            if safe_delta:
                                yield TextDelta(safe_delta)
                        elif isinstance(event, ReasoningDelta):
                            safe_delta = reasoning_guard.feed(event.delta)
                            if safe_delta or not event.delta:
                                yield ReasoningDelta(safe_delta)
                        else:
                            yield event
                trailing_text = text_guard.finish()
                if trailing_text:
                    yield TextDelta(trailing_text)
                trailing_reasoning = reasoning_guard.finish()
                if trailing_reasoning:
                    yield ReasoningDelta(trailing_reasoning)
                for event in assembler.finish(done_seen=done_seen):
                    yield event
            except RunCancelled:
                raise
            except ProtectedValueError:
                raise _protected_response_failure() from None
            except TimeoutError:
                raise ProviderFailure(
                    "timeout",
                    "provider stream became idle",
                    retryable=not assembler.saw_output,
                ) from None
            except httpx.TimeoutException:
                raise ProviderFailure(
                    "timeout",
                    "provider stream timed out",
                    retryable=not assembler.saw_output,
                ) from None
            except httpx.RequestError:
                raise ProviderFailure(
                    "network",
                    "provider stream was interrupted",
                    retryable=not assembler.saw_output,
                ) from None
            except ProviderFailure as error:
                if assembler.saw_output and error.retryable:
                    error.retryable = False
                raise
        finally:
            with suppress(Exception):
                await response.aclose()

    def _request_body(self, request: ProviderRequest) -> dict[str, object]:
        model = self._model(request.model_id)
        body: dict[str, object] = {
            "model": model.id,
            "messages": [_message_to_openai(message) for message in request.messages],
            "stream": True,
        }
        if model.supports_tools and request.tools:
            body["tools"] = [_tool_to_openai(tool) for tool in request.tools]
        return body

    def _request_headers(self) -> dict[str, str]:
        headers = {
            "Accept": "text/event-stream",
            "Content-Type": "application/json",
        }
        if self._provider.api_key is not None:
            headers["Authorization"] = f"Bearer {self._provider.api_key}"
        for name, value in self._provider.headers:
            existing = next((key for key in headers if key.lower() == name.lower()), None)
            if existing is not None:
                del headers[existing]
            headers[name] = value
        return headers

    def _model(self, model_id: str) -> ModelConfig:
        model = next(
            (candidate for candidate in self._provider.models if candidate.id == model_id), None
        )
        if model is None or not model.enabled:
            raise ProviderFailure("invalid_request", "selected provider model is unavailable")
        return model

    def _http_failure(self, response: httpx.Response) -> ProviderFailure:
        status = response.status_code
        request_id = _safe_request_id(response, self._provider)
        retry_after = _retry_after(response.headers.get("retry-after"))
        if status in {401, 403}:
            return ProviderFailure(
                "authentication",
                "provider authentication failed",
                status_code=status,
                request_id=request_id,
            )
        if status == 429:
            return ProviderFailure(
                "rate_limit",
                "provider rate limit was reached",
                status_code=status,
                request_id=request_id,
                retryable=True,
                retry_after=retry_after,
            )
        if status == 413:
            return ProviderFailure(
                "context_overflow",
                "provider rejected the request context size",
                status_code=status,
                request_id=request_id,
            )
        if status == 408:
            return ProviderFailure(
                "timeout",
                "provider request timed out",
                status_code=status,
                request_id=request_id,
                retryable=True,
                retry_after=retry_after,
            )
        if status >= 500:
            return ProviderFailure(
                "server",
                "provider server failed",
                status_code=status,
                request_id=request_id,
                retryable=True,
                retry_after=retry_after,
            )
        return ProviderFailure(
            "invalid_request",
            "provider rejected the request",
            status_code=status,
            request_id=request_id,
        )


async def _sse_payloads(
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
            line = await _await_cancellable(
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
        if data_characters > _MAX_SSE_EVENT_CHARACTERS:
            raise _protocol_failure()
        data_lines.append(value)


async def _await_cancellable[T](
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


async def _close_response(response: httpx.Response) -> None:
    await response.aclose()


def _message_to_openai(message: ProviderMessage) -> dict[str, object]:
    if message.role == "user":
        return {"role": "user", "content": message.content}
    if message.role == "assistant" and message.tool_calls:
        value: dict[str, object] = {
            "role": "assistant",
            "content": None,
            "tool_calls": [
                {
                    "id": call.id,
                    "type": "function",
                    "function": {
                        "name": call.name,
                        "arguments": json_dumps(
                            call.arguments,
                            ensure_ascii=False,
                            separators=(",", ":"),
                        ),
                    },
                }
                for call in message.tool_calls
            ],
        }
        if message.reasoning_content is not None:
            value["reasoning_content"] = message.reasoning_content
        return value
    if message.role == "assistant":
        return {"role": "assistant", "content": message.content}
    if message.role == "tool" and message.tool_call_id:
        return {
            "role": "tool",
            "tool_call_id": message.tool_call_id,
            "content": message.content,
        }
    raise ProviderFailure("protocol", "provider context contains an invalid message")


def _tool_to_openai(tool: ToolDefinition) -> dict[str, object]:
    return {
        "type": "function",
        "function": {
            "name": tool.name,
            "description": tool.description,
            "parameters": tool.input_schema,
        },
    }


def _protocol_failure() -> ProviderFailure:
    return ProviderFailure("protocol", "provider returned an invalid streaming response")


def _protected_response_failure() -> ProviderFailure:
    return ProviderFailure("protocol", _PROTECTED_RESPONSE_MESSAGE)


def _provider_secrets(provider: ProviderConfig) -> tuple[str, ...]:
    return tuple(
        dict.fromkeys(
            value
            for value in (
                provider.api_key,
                *(header_value for _header_name, header_value in provider.headers),
            )
            if value
        )
    )


def _retry_after(value: str | None) -> float | None:
    if value is None:
        return None
    try:
        seconds = float(value)
    except ValueError:
        try:
            parsed = parsedate_to_datetime(value)
        except (TypeError, ValueError, OverflowError):
            return None
        if parsed.tzinfo is None:
            parsed = parsed.replace(tzinfo=UTC)
        seconds = (parsed - datetime.now(UTC)).total_seconds()
    if not math.isfinite(seconds):
        return None
    if seconds < 0:
        return 0.0
    return min(seconds, _MAX_RETRY_DELAY_SECONDS)


def _safe_request_id(response: httpx.Response, provider: ProviderConfig) -> str | None:
    value = next(
        (response.headers[name] for name in _REQUEST_ID_HEADERS if name in response.headers),
        None,
    )
    if value is None or not value or len(value) > 200:
        return None
    if any(ord(character) < 32 or ord(character) == 127 for character in value):
        return None
    secrets = _provider_secrets(provider)
    if any(secret in value for secret in secrets):
        return None
    return value
