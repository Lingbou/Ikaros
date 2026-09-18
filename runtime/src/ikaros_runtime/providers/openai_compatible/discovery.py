from __future__ import annotations

import asyncio
import re
from collections.abc import Sequence
from contextlib import suppress

import httpx

from ...errors import ProviderFailure
from ...json_codec import loads as json_loads
from ...security import contains_protected_value
from ..base import ModelInput, ProviderConfig
from ..model_defaults import default_model_capacity
from .adapter import (
    ProviderTimeouts,
    parse_retry_after,
    provider_request_headers,
    provider_secrets,
)
from .streaming import PROTECTED_RESPONSE_MESSAGE, protected_response_failure

_MAX_MODELS_RESPONSE_BYTES = 1_000_000
_MAX_DISCOVERED_MODELS = 1_000


async def discover_openai_compatible_models(
    provider: ProviderConfig,
    *,
    client: httpx.AsyncClient | None = None,
    timeouts: ProviderTimeouts | None = None,
) -> tuple[ModelInput, ...]:
    """Fetch a bounded OpenAI-compatible model catalog without persisting it."""

    resolved_timeouts = timeouts or ProviderTimeouts()
    resolved_timeouts.validate()
    owns_client = client is None
    transport = client or httpx.AsyncClient(
        timeout=httpx.Timeout(
            connect=resolved_timeouts.connect,
            read=resolved_timeouts.response_header,
            write=resolved_timeouts.response_header,
            pool=resolved_timeouts.connect,
        ),
        follow_redirects=False,
    )
    response: httpx.Response | None = None
    try:
        try:
            request = transport.build_request(
                "GET",
                f"{provider.base_url.rstrip('/')}/models",
                headers=provider_request_headers(provider, accept="application/json"),
            )
            async with asyncio.timeout(resolved_timeouts.response_header):
                response = await transport.send(request, stream=True, follow_redirects=False)
        except TimeoutError:
            raise ProviderFailure("timeout", "provider model discovery timed out") from None
        except httpx.TimeoutException:
            raise ProviderFailure("timeout", "provider model discovery timed out") from None
        except httpx.RequestError:
            raise ProviderFailure(
                "network", "provider model discovery network request failed"
            ) from None
        except (TypeError, ValueError):
            raise ProviderFailure(
                "invalid_request", "provider model discovery request could not be built"
            ) from None

        if response.status_code >= 400:
            raise _model_discovery_http_failure(response)
        if response.status_code != 200:
            raise _models_protocol_failure()
        body = await _bounded_models_body(
            response,
            wait_seconds=resolved_timeouts.response_header,
        )
        return _parse_models_response(body, provider_secrets(provider))
    except ProviderFailure as error:
        safe_messages = {
            "provider authentication failed",
            "provider rate limit was reached",
            "provider model discovery timed out",
            "provider model discovery network request failed",
            "provider model discovery request could not be built",
            "provider model discovery failed",
            "provider returned an invalid models response",
            PROTECTED_RESPONSE_MESSAGE,
        }
        message = str(error)
        raise ProviderFailure(
            error.category,
            message if message in safe_messages else "provider model discovery failed",
            status_code=error.status_code,
            retryable=error.retryable,
            retry_after=error.retry_after,
        ) from None
    except Exception:
        raise ProviderFailure("unknown", "provider model discovery failed") from None
    finally:
        if response is not None:
            with suppress(Exception):
                await response.aclose()
        if owns_client:
            with suppress(Exception):
                await transport.aclose()


async def _bounded_models_body(response: httpx.Response, *, wait_seconds: float) -> bytes:
    declared_length = response.headers.get("content-length")
    if declared_length is not None:
        try:
            if int(declared_length) > _MAX_MODELS_RESPONSE_BYTES:
                raise _models_protocol_failure()
        except ValueError:
            raise _models_protocol_failure() from None
    chunks: list[bytes] = []
    size = 0
    try:
        async with asyncio.timeout(wait_seconds):
            async for chunk in response.aiter_bytes():
                size += len(chunk)
                if size > _MAX_MODELS_RESPONSE_BYTES:
                    raise _models_protocol_failure()
                chunks.append(chunk)
    except TimeoutError:
        raise ProviderFailure("timeout", "provider model discovery timed out") from None
    except httpx.TimeoutException:
        raise ProviderFailure("timeout", "provider model discovery timed out") from None
    except httpx.RequestError:
        raise ProviderFailure(
            "network", "provider model discovery network request failed"
        ) from None
    return b"".join(chunks)


def _parse_models_response(source: bytes, secrets: Sequence[str]) -> tuple[ModelInput, ...]:
    try:
        value = json_loads(source)
    except (UnicodeError, ValueError):
        raise _models_protocol_failure() from None
    if not isinstance(value, dict) or not isinstance(value.get("data"), list):
        raise _models_protocol_failure()
    rows = value["data"]
    if len(rows) > _MAX_DISCOVERED_MODELS:
        raise _models_protocol_failure()
    by_id: dict[str, ModelInput] = {}
    for row in rows:
        if not isinstance(row, dict):
            raise _models_protocol_failure()
        model_id = row.get("id")
        if not isinstance(model_id, str):
            raise _models_protocol_failure()
        model_id = model_id.strip()
        if (
            not model_id
            or len(model_id) > 200
            or any(ord(character) < 32 or ord(character) == 127 for character in model_id)
        ):
            raise _models_protocol_failure()
        if contains_protected_value(model_id, secrets):
            raise protected_response_failure()
        defaults = default_model_capacity(model_id)
        by_id.setdefault(
            model_id,
            ModelInput(
                model_id,
                _friendly_model_name(model_id),
                defaults.context_window,
            ),
        )
    return tuple(sorted(by_id.values(), key=lambda model: (model.id.casefold(), model.id)))


def _friendly_model_name(model_id: str) -> str:
    parts = [part for part in re.split(r"[-_./\s]+", model_id) if part]
    if not parts:
        return model_id

    def friendly_part(part: str) -> str:
        lowered = part.casefold()
        if lowered == "deepseek":
            return "DeepSeek"
        if re.fullmatch(r"[a-zA-Z]\d+(?:\.\d+)?", part):
            return part[0].upper() + part[1:]
        if part.isupper():
            return part
        return part[0].upper() + part[1:]

    return " ".join(friendly_part(part) for part in parts)


def _model_discovery_http_failure(response: httpx.Response) -> ProviderFailure:
    status = response.status_code
    if status in {401, 403}:
        return ProviderFailure(
            "authentication",
            "provider authentication failed",
            status_code=status,
        )
    if status == 429:
        return ProviderFailure(
            "rate_limit",
            "provider rate limit was reached",
            status_code=status,
            retryable=True,
            retry_after=parse_retry_after(response.headers.get("retry-after")),
        )
    if status == 408:
        return ProviderFailure(
            "timeout",
            "provider model discovery timed out",
            status_code=status,
            retryable=True,
        )
    if status >= 500:
        return ProviderFailure(
            "server",
            "provider model discovery failed",
            status_code=status,
            retryable=True,
        )
    return ProviderFailure(
        "invalid_request",
        "provider model discovery failed",
        status_code=status,
    )


def _models_protocol_failure() -> ProviderFailure:
    return ProviderFailure("protocol", "provider returned an invalid models response")
