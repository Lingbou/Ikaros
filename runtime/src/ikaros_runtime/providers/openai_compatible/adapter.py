from __future__ import annotations

import math
from collections.abc import AsyncIterator
from contextlib import suppress
from dataclasses import dataclass
from datetime import UTC, datetime
from email.utils import parsedate_to_datetime

import httpx

from ...cancellation import CancellationToken, RunCancelled
from ...errors import ProviderFailure as ProviderFailure
from ...errors import ProviderFailureCategory
from ...json_codec import dumps as json_dumps
from ...json_codec import loads as json_loads
from ...security import ProtectedStreamGuard, ProtectedValueError
from ...tools.core import ToolDefinition
from ..base import (
    ModelConfig,
    ProviderConfig,
    ProviderEvent,
    ProviderMessage,
    ProviderRequest,
    ReasoningDelta,
    TextDelta,
)
from .streaming import (
    PROTECTED_RESPONSE_MESSAGE,
    ResponseAssembler,
    await_cancellable,
    close_response,
    protected_response_failure,
    protocol_failure,
    sse_payloads,
)

_MAX_RETRY_DELAY_SECONDS = 5.0
_REQUEST_ID_HEADERS = ("x-request-id", "request-id", "x-amzn-requestid")
_PROTECTED_RESPONSE_MESSAGE = PROTECTED_RESPONSE_MESSAGE
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


@dataclass(frozen=True, slots=True)
class ProviderTimeouts:
    connect: float = 10.0
    response_header: float = 30.0
    stream_idle: float = 120.0

    def validate(self) -> None:
        if self.connect <= 0 or self.response_header <= 0 or self.stream_idle <= 0:
            raise ValueError("provider timeouts must be positive")


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
        self._secrets = provider_secrets(provider)
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
        self._usage_stream_options_supported: dict[str, bool] = {}

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
        headers = self._request_headers()
        url = f"{self._provider.base_url.rstrip('/')}/chat/completions"
        upstream_model_id = self._model(request.model_id).id
        cached_usage_support = self._usage_stream_options_supported.get(upstream_model_id)
        include_usage = (
            self._provider.origin == "builtin" or cached_usage_support is not False
        )
        fallback_without_usage = (
            self._provider.origin == "custom"
            and cached_usage_support is None
            and include_usage
        )
        fallback_attempt = False
        attempt = 0
        while True:
            body = self._request_body(request, include_usage=include_usage)
            try:
                async for event in self._attempt(
                    url,
                    headers,
                    body,
                    cancellation=cancellation,
                ):
                    yield event
                if self._provider.origin == "custom":
                    if include_usage:
                        self._usage_stream_options_supported[upstream_model_id] = True
                    elif fallback_attempt:
                        self._usage_stream_options_supported[upstream_model_id] = False
                return
            except RunCancelled:
                raise
            except ProviderFailure as error:
                if fallback_without_usage and error.status_code in {400, 422}:
                    include_usage = False
                    fallback_without_usage = False
                    fallback_attempt = True
                    continue
                if attempt >= self._max_retries or not error.retryable:
                    raise
                delay = error.retry_after
                if delay is None:
                    delay = min(0.25 * (2**attempt), _MAX_RETRY_DELAY_SECONDS)
                await cancellation.sleep(min(delay, _MAX_RETRY_DELAY_SECONDS))
                attempt += 1

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
            response = await await_cancellable(
                self._client.send(request, stream=True),
                cancellation=cancellation,
                wait_seconds=self._timeouts.response_header,
                cancelled_result_cleanup=close_response,
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
            assembler = ResponseAssembler(self._secrets)
            text_guard = ProtectedStreamGuard(self._secrets)
            reasoning_guard = ProtectedStreamGuard(self._secrets)
            done_seen = False
            try:
                async for payload in sse_payloads(
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
                        raise protocol_failure() from None
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
                raise protected_response_failure() from None
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

    def _request_body(
        self,
        request: ProviderRequest,
        *,
        include_usage: bool,
    ) -> dict[str, object]:
        model = self._model(request.model_id)
        body: dict[str, object] = {
            "model": model.id,
            "messages": [_message_to_openai(message) for message in request.messages],
            "stream": True,
        }
        if include_usage:
            body["stream_options"] = {"include_usage": True}
        if model.supports_tools and request.tools:
            body["tools"] = [_tool_to_openai(tool) for tool in request.tools]
        return body

    def _request_headers(self) -> dict[str, str]:
        return provider_request_headers(
            self._provider,
            accept="text/event-stream",
            content_type="application/json",
        )

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
        retry_after = parse_retry_after(response.headers.get("retry-after"))
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


def provider_request_headers(
    provider: ProviderConfig,
    *,
    accept: str,
    content_type: str | None = None,
) -> dict[str, str]:
    headers = {"Accept": accept}
    if content_type is not None:
        headers["Content-Type"] = content_type
    if provider.api_key is not None:
        headers["Authorization"] = f"Bearer {provider.api_key}"
    for name, value in provider.headers:
        existing = next((key for key in headers if key.lower() == name.lower()), None)
        if existing is not None:
            del headers[existing]
        headers[name] = value
    return headers


def _message_to_openai(message: ProviderMessage) -> dict[str, object]:
    if message.role == "system":
        return {"role": "system", "content": message.content}
    if message.role == "user":
        return {"role": "user", "content": message.content}
    if message.role == "assistant" and message.tool_calls:
        value: dict[str, object] = {
            "role": "assistant",
            "content": message.content or None,
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


def provider_secrets(provider: ProviderConfig) -> tuple[str, ...]:
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


def parse_retry_after(value: str | None) -> float | None:
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
    secrets = provider_secrets(provider)
    if any(secret in value for secret in secrets):
        return None
    return value
