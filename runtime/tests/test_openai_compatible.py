from __future__ import annotations

import asyncio
import json
from collections.abc import AsyncIterator, Sequence
from pathlib import Path
from typing import Any, cast

import httpx
import pytest

from ikaros_runtime.cancellation import CancellationToken, RunCancelled
from ikaros_runtime.providers.base import (
    ProviderEvent,
    ProviderMessage,
    ProviderRequest,
    ReasoningDelta,
    ResponseCompleted,
    TextDelta,
    ToolCallCompleted,
)
from ikaros_runtime.providers.openai_compatible.adapter import (
    OpenAICompatibleAdapter,
    ProviderFailure,
    ProviderTimeouts,
)
from ikaros_runtime.providers.openai_compatible.discovery import discover_openai_compatible_models
from ikaros_runtime.providers.registry import (
    ConfigStore,
    ModelConfig,
    ModelInput,
    ProviderConfig,
    RuntimeProviderRegistry,
)
from ikaros_runtime.tools.core import ToolCall, ToolDefinition


class ChunkStream(httpx.AsyncByteStream):
    def __init__(
        self,
        chunks: Sequence[bytes],
        *,
        wait_before: int | None = None,
        gate: asyncio.Event | None = None,
        error_after: int | None = None,
    ) -> None:
        self._chunks = chunks
        self._wait_before = wait_before
        self._gate = gate
        self._error_after = error_after
        self.closed = False
        self.waiting = asyncio.Event()

    async def __aiter__(self) -> AsyncIterator[bytes]:
        for index, chunk in enumerate(self._chunks):
            if self._wait_before == index and self._gate is not None:
                self.waiting.set()
                await self._gate.wait()
            if self._error_after == index:
                raise httpx.ReadError("upstream-body-must-not-leak")
            yield chunk
        if self._wait_before == len(self._chunks) and self._gate is not None:
            self.waiting.set()
            await self._gate.wait()
        if self._error_after == len(self._chunks):
            raise httpx.ReadError("upstream-body-must-not-leak")

    async def aclose(self) -> None:
        self.closed = True


def provider(
    *,
    api_key: str | None = "sk-provider-secret",
    headers: tuple[tuple[str, str], ...] = (("X-Tenant", "header-secret"),),
    supports_tools: bool = True,
) -> ProviderConfig:
    return ProviderConfig(
        id="custom",
        display_name="Custom",
        origin="custom",
        base_url="https://provider.invalid/v1",
        api_key=api_key,
        headers=headers,
        models=(
            ModelConfig(
                id="model",
                display_name="Model",
                enabled=True,
                supports_tools=supports_tools,
            ),
        ),
    )


def request(
    *,
    messages: Sequence[ProviderMessage] | None = None,
    tools: Sequence[ToolDefinition] | None = None,
) -> ProviderRequest:
    return ProviderRequest(
        model_id="model",
        messages=messages or [ProviderMessage(role="user", content="Hello")],
        tools=tools or (),
    )


def sse(value: object) -> bytes:
    return f"data: {json.dumps(value, separators=(',', ':'))}\n\n".encode()


def text_chunk(content: str, *, finish_reason: str | None = None) -> dict[str, object]:
    return {
        "choices": [
            {
                "index": 0,
                "delta": {"content": content},
                "finish_reason": finish_reason,
            }
        ]
    }


async def collect(
    adapter: OpenAICompatibleAdapter,
    provider_request: ProviderRequest | None = None,
    cancellation: CancellationToken | None = None,
) -> list[ProviderEvent]:
    return [
        event
        async for event in adapter.stream(
            provider_request or request(),
            cancellation=cancellation or CancellationToken(),
        )
    ]


@pytest.mark.asyncio
async def test_model_discovery_uses_bearer_and_normalizes_catalog() -> None:
    captured: dict[str, Any] = {}

    async def handler(incoming: httpx.Request) -> httpx.Response:
        captured["method"] = incoming.method
        captured["url"] = str(incoming.url)
        captured["headers"] = dict(incoming.headers)
        return httpx.Response(
            200,
            headers={"content-type": "application/json"},
            json={
                "object": "list",
                "data": [
                    {"id": "deepseek-v4-pro", "object": "model"},
                    {"id": "deepseek-v4-flash", "owned_by": "deepseek"},
                    {"id": "deepseek-v4-pro"},
                ],
            },
        )

    discovery_provider = provider(headers=())
    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
        models = await discover_openai_compatible_models(discovery_provider, client=client)

    assert [(model.id, model.display_name) for model in models] == [
        ("deepseek-v4-flash", "DeepSeek V4 Flash"),
        ("deepseek-v4-pro", "DeepSeek V4 Pro"),
    ]
    assert captured["method"] == "GET"
    assert captured["url"] == "https://provider.invalid/v1/models"
    headers = cast(dict[str, str], captured["headers"])
    assert headers["authorization"] == "Bearer sk-provider-secret"
    assert headers["accept"] == "application/json"


@pytest.mark.asyncio
@pytest.mark.parametrize(
    ("status", "category", "message"),
    [
        (401, "authentication", "provider authentication failed"),
        (429, "rate_limit", "provider rate limit was reached"),
    ],
)
async def test_model_discovery_http_failures_are_safe(
    status: int,
    category: str,
    message: str,
) -> None:
    secret = "sk-model-discovery-sentinel"

    async def handler(_incoming: httpx.Request) -> httpx.Response:
        return httpx.Response(status, content=f"upstream {secret}")

    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
        with pytest.raises(ProviderFailure) as captured:
            await discover_openai_compatible_models(
                provider(api_key=secret, headers=()),
                client=client,
            )

    assert captured.value.category == category
    assert str(captured.value) == message
    assert secret not in str(captured.value)
    assert secret not in repr(captured.value)


@pytest.mark.asyncio
async def test_model_discovery_normalizes_timeout_and_network_errors() -> None:
    async def timeout_handler(_incoming: httpx.Request) -> httpx.Response:
        raise httpx.ReadTimeout("timeout-body-must-not-leak")

    async def network_handler(_incoming: httpx.Request) -> httpx.Response:
        raise httpx.ConnectError("network-body-must-not-leak")

    cases = [
        (timeout_handler, "timeout", "provider model discovery timed out"),
        (
            network_handler,
            "network",
            "provider model discovery network request failed",
        ),
    ]
    for handler, category, message in cases:
        async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
            with pytest.raises(ProviderFailure) as captured:
                await discover_openai_compatible_models(provider(headers=()), client=client)
        assert captured.value.category == category
        assert str(captured.value) == message
        assert "must-not-leak" not in str(captured.value)


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "response",
    [
        httpx.Response(200, json={}),
        httpx.Response(200, json={"data": {"id": "model"}}),
        httpx.Response(200, json={"data": [None]}),
        httpx.Response(200, json={"data": [{"id": 7}]}),
        httpx.Response(200, json={"data": [{"id": "bad\nmodel"}]}),
        httpx.Response(200, content=b"not-json"),
    ],
)
async def test_model_discovery_rejects_invalid_schemas(response: httpx.Response) -> None:
    async def handler(_incoming: httpx.Request) -> httpx.Response:
        return response

    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
        with pytest.raises(
            ProviderFailure,
            match="provider returned an invalid models response",
        ):
            await discover_openai_compatible_models(provider(headers=()), client=client)


@pytest.mark.asyncio
async def test_model_discovery_blocks_a_credential_in_remote_model_id() -> None:
    secret = "sk-discovery-echo-sentinel"

    async def handler(_incoming: httpx.Request) -> httpx.Response:
        return httpx.Response(200, json={"data": [{"id": f"model-{secret}"}]})

    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
        with pytest.raises(ProviderFailure) as captured:
            await discover_openai_compatible_models(
                provider(api_key=secret, headers=()),
                client=client,
            )

    assert str(captured.value) == "provider response contained protected configuration data"
    assert secret not in repr(captured.value)


@pytest.mark.asyncio
async def test_request_lowering_headers_and_text_stream() -> None:
    captured: dict[str, Any] = {}
    body = b"".join(
        [
            sse(text_chunk("Hel")),
            sse(text_chunk("lo", finish_reason="stop")),
            b"data: [DONE]\n\n",
        ]
    )

    async def handler(incoming: httpx.Request) -> httpx.Response:
        captured["url"] = str(incoming.url)
        captured["headers"] = dict(incoming.headers)
        captured["body"] = json.loads(incoming.content)
        return httpx.Response(
            200,
            headers={"content-type": "text/event-stream; charset=utf-8"},
            stream=ChunkStream([body[:17], body[17:43], body[43:]]),
        )

    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
        adapter = OpenAICompatibleAdapter(provider(), client=client)
        events = await collect(
            adapter,
            request(
                messages=[
                    ProviderMessage(role="user", content="Run it"),
                    ProviderMessage(
                        role="assistant",
                        content="I will run it now.",
                        tool_calls=(ToolCall("call_1", "process_run", {"command": "echo one"}),),
                        reasoning_content="private reasoning replay",
                    ),
                    ProviderMessage(role="tool", content='{"ok":true}', tool_call_id="call_1"),
                ],
                tools=[
                    ToolDefinition(
                        name="process_run",
                        description="Run a process",
                        input_schema={"type": "object"},
                    )
                ],
            ),
        )

    assert events == [TextDelta("Hel"), TextDelta("lo"), ResponseCompleted()]
    assert captured["url"] == "https://provider.invalid/v1/chat/completions"
    headers = cast(dict[str, str], captured["headers"])
    assert headers["authorization"] == "Bearer sk-provider-secret"
    assert headers["x-tenant"] == "header-secret"
    lowered = cast(dict[str, Any], captured["body"])
    assert lowered["model"] == "model"
    assert lowered["stream"] is True
    assert lowered["messages"][1] == {
        "role": "assistant",
        "content": "I will run it now.",
        "tool_calls": [
            {
                "id": "call_1",
                "type": "function",
                "function": {
                    "name": "process_run",
                    "arguments": '{"command":"echo one"}',
                },
            }
        ],
        "reasoning_content": "private reasoning replay",
    }
    assert lowered["messages"][2] == {
        "role": "tool",
        "tool_call_id": "call_1",
        "content": '{"ok":true}',
    }
    assert lowered["tools"][0]["function"]["name"] == "process_run"


@pytest.mark.asyncio
async def test_custom_provider_without_key_and_tool_disabled_omits_optional_fields() -> None:
    captured: dict[str, Any] = {}

    async def handler(incoming: httpx.Request) -> httpx.Response:
        captured["headers"] = dict(incoming.headers)
        captured["body"] = json.loads(incoming.content)
        return httpx.Response(
            200,
            headers={"content-type": "text/event-stream"},
            stream=ChunkStream([sse(text_chunk("ok", finish_reason="stop")), b"data: [DONE]\n\n"]),
        )

    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
        adapter = OpenAICompatibleAdapter(
            provider(api_key=None, headers=(), supports_tools=False),
            client=client,
        )
        await collect(
            adapter,
            request(
                tools=[ToolDefinition("process_run", "Run", {"type": "object"})],
            ),
        )

    assert "authorization" not in captured["headers"]
    assert "tools" not in captured["body"]


@pytest.mark.asyncio
async def test_sse_framing_comments_multiline_data_and_usage_only_chunks() -> None:
    chunks = [
        b": heartbeat\r\nevent: message\r\n",
        b'data: {"choices": [\r\n',
        b'data: {"index":0,"delta":{"content":"framed"},"finish_reason":"stop"}]}\r\n\r\n',
        sse({"choices": [], "usage": {"total_tokens": 1}}),
        b"data: [DONE]\r\n\r\n",
    ]

    async def handler(_incoming: httpx.Request) -> httpx.Response:
        return httpx.Response(
            200,
            headers={"content-type": "text/event-stream"},
            stream=ChunkStream(chunks),
        )

    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
        events = await collect(OpenAICompatibleAdapter(provider(), client=client))

    assert events == [TextDelta("framed"), ResponseCompleted()]


@pytest.mark.asyncio
async def test_reasoning_and_interleaved_tool_calls_are_assembled_by_index() -> None:
    chunks = [
        sse(
            {
                "choices": [
                    {
                        "index": 0,
                        "delta": {
                            "reasoning_content": "plan",
                            "tool_calls": [
                                {
                                    "index": 1,
                                    "id": "call_2",
                                    "type": "function",
                                    "function": {
                                        "name": "process_run",
                                        "arguments": '{"command":"two',
                                    },
                                }
                            ],
                        },
                        "finish_reason": None,
                    }
                ]
            }
        ),
        sse(
            {
                "choices": [
                    {
                        "index": 0,
                        "delta": {
                            "reasoning_content": "",
                            "tool_calls": [
                                {
                                    "index": 0,
                                    "id": "call_1",
                                    "type": "function",
                                    "function": {
                                        "name": "process_run",
                                        "arguments": '{"command":"one"}',
                                    },
                                },
                                {"index": 1, "function": {"arguments": '"}'}},
                            ],
                        },
                        "finish_reason": None,
                    }
                ]
            }
        ),
        sse(
            {
                "choices": [
                    {
                        "index": 0,
                        "delta": {},
                        "finish_reason": "tool_calls",
                    }
                ]
            }
        ),
        b"data: [DONE]\n\n",
    ]

    async def handler(_incoming: httpx.Request) -> httpx.Response:
        return httpx.Response(
            200,
            headers={"content-type": "text/event-stream"},
            stream=ChunkStream(chunks),
        )

    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
        events = await collect(OpenAICompatibleAdapter(provider(), client=client))

    assert events == [
        ReasoningDelta("plan"),
        ReasoningDelta(""),
        ToolCallCompleted(ToolCall("call_1", "process_run", {"command": "one"})),
        ToolCallCompleted(ToolCall("call_2", "process_run", {"command": "two"})),
        ResponseCompleted(),
    ]


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "tool_chunks",
    [
        [
            {
                "index": 0,
                "id": "call",
                "type": "function",
                "function": {"name": "process_run", "arguments": "{"},
            }
        ],
        [
            {
                "index": 0,
                "id": "call",
                "type": "function",
                "function": {"name": "process_run", "arguments": "[]"},
            }
        ],
        [
            {
                "index": 0,
                "id": "call",
                "type": "function",
                "function": {"arguments": "{}"},
            }
        ],
    ],
)
async def test_invalid_tool_calls_fail_before_executor(
    tool_chunks: list[dict[str, object]],
) -> None:
    chunks = [
        sse(
            {
                "choices": [
                    {
                        "index": 0,
                        "delta": {"tool_calls": tool_chunks},
                        "finish_reason": "tool_calls",
                    }
                ]
            }
        ),
        b"data: [DONE]\n\n",
    ]

    async def handler(_incoming: httpx.Request) -> httpx.Response:
        return httpx.Response(
            200,
            headers={"content-type": "text/event-stream"},
            stream=ChunkStream(chunks),
        )

    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
        adapter = OpenAICompatibleAdapter(provider(), client=client, max_retries=0)
        with pytest.raises(ProviderFailure) as captured:
            await collect(adapter)

    assert captured.value.category == "protocol"


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "invalid_number",
    [
        "NaN",
        "Infinity",
        "-Infinity",
        "1e9999",
        "-1e9999",
        "9007199254740992",
        "-9007199254740992",
        "1" + ("0" * 400),
    ],
)
async def test_invalid_json_numbers_never_reach_tool_arguments(invalid_number: str) -> None:
    chunk = sse(
        {
            "choices": [
                {
                    "index": 0,
                    "delta": {
                        "tool_calls": [
                            {
                                "index": 0,
                                "id": "call",
                                "type": "function",
                                "function": {
                                    "name": "process_run",
                                    "arguments": f'{{"timeoutMs":{invalid_number}}}',
                                },
                            }
                        ]
                    },
                    "finish_reason": "tool_calls",
                }
            ]
        }
    )

    async def handler(_incoming: httpx.Request) -> httpx.Response:
        return httpx.Response(
            200,
            headers={"content-type": "text/event-stream"},
            stream=ChunkStream([chunk, b"data: [DONE]\n\n"]),
        )

    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
        with pytest.raises(ProviderFailure) as captured:
            await collect(OpenAICompatibleAdapter(provider(), client=client, max_retries=0))

    assert captured.value.category == "protocol"


@pytest.mark.asyncio
async def test_nonstandard_json_constant_in_ignored_sse_field_is_rejected() -> None:
    chunks = [
        b'data: {"choices":[],"ignored":NaN}\n\n',
        b"data: [DONE]\n\n",
    ]

    async def handler(_incoming: httpx.Request) -> httpx.Response:
        return httpx.Response(
            200,
            headers={"content-type": "text/event-stream"},
            stream=ChunkStream(chunks),
        )

    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
        with pytest.raises(ProviderFailure) as captured:
            await collect(OpenAICompatibleAdapter(provider(), client=client, max_retries=0))

    assert captured.value.category == "protocol"


@pytest.mark.asyncio
async def test_deeply_nested_sse_json_is_a_protocol_failure() -> None:
    nested = ("[" * 4000) + "0" + ("]" * 4000)
    chunks = [f"data: {nested}\n\n".encode(), b"data: [DONE]\n\n"]

    async def handler(_incoming: httpx.Request) -> httpx.Response:
        return httpx.Response(
            200,
            headers={"content-type": "text/event-stream"},
            stream=ChunkStream(chunks),
        )

    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
        with pytest.raises(ProviderFailure) as captured:
            await collect(OpenAICompatibleAdapter(provider(), client=client, max_retries=0))

    assert captured.value.category == "protocol"


@pytest.mark.asyncio
async def test_tool_call_without_terminal_finish_reason_is_rejected() -> None:
    chunk = sse(
        {
            "choices": [
                {
                    "index": 0,
                    "delta": {
                        "tool_calls": [
                            {
                                "index": 0,
                                "id": "call",
                                "type": "function",
                                "function": {"name": "process_run", "arguments": "{}"},
                            }
                        ]
                    },
                    "finish_reason": None,
                }
            ]
        }
    )

    async def handler(_incoming: httpx.Request) -> httpx.Response:
        return httpx.Response(
            200,
            headers={"content-type": "text/event-stream"},
            stream=ChunkStream([chunk, b"data: [DONE]\n\n"]),
        )

    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
        with pytest.raises(ProviderFailure, match="invalid streaming"):
            await collect(OpenAICompatibleAdapter(provider(), client=client, max_retries=0))


@pytest.mark.asyncio
async def test_empty_tool_arguments_are_normalized_to_an_empty_object() -> None:
    chunk = sse(
        {
            "choices": [
                {
                    "index": 0,
                    "delta": {
                        "tool_calls": [
                            {
                                "index": 0,
                                "id": "call",
                                "type": "function",
                                "function": {"name": "process_run", "arguments": ""},
                            }
                        ]
                    },
                    "finish_reason": "tool_calls",
                }
            ]
        }
    )

    async def handler(_incoming: httpx.Request) -> httpx.Response:
        return httpx.Response(
            200,
            headers={"content-type": "text/event-stream"},
            stream=ChunkStream([chunk, b"data: [DONE]\n\n"]),
        )

    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
        events = await collect(OpenAICompatibleAdapter(provider(), client=client))

    assert events == [
        ToolCallCompleted(ToolCall("call", "process_run", {})),
        ResponseCompleted(),
    ]


@pytest.mark.asyncio
async def test_api_key_echo_split_across_text_deltas_is_rejected_before_leaking() -> None:
    secret = "sk-provider-secret"
    chunks = [
        sse(text_chunk(f"safe prefix {secret[:11]}")),
        sse(text_chunk(secret[11:], finish_reason="stop")),
        b"data: [DONE]\n\n",
    ]

    async def handler(_incoming: httpx.Request) -> httpx.Response:
        return httpx.Response(
            200,
            headers={"content-type": "text/event-stream"},
            stream=ChunkStream(chunks),
        )

    emitted: list[ProviderEvent] = []
    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
        adapter = OpenAICompatibleAdapter(provider(api_key=secret), client=client, max_retries=0)
        with pytest.raises(ProviderFailure) as captured:
            async for event in adapter.stream(request(), cancellation=CancellationToken()):
                emitted.append(event)

    assert captured.value.category == "protocol"
    assert secret not in str(captured.value)
    assert secret not in repr(captured.value)
    assert secret not in repr(emitted)


@pytest.mark.asyncio
async def test_header_echo_split_across_reasoning_deltas_is_rejected() -> None:
    secret = "header-secret"
    chunks = [
        sse(
            {
                "choices": [
                    {
                        "index": 0,
                        "delta": {"reasoning_content": secret[:7]},
                        "finish_reason": None,
                    }
                ]
            }
        ),
        sse(
            {
                "choices": [
                    {
                        "index": 0,
                        "delta": {"reasoning_content": secret[7:]},
                        "finish_reason": "stop",
                    }
                ]
            }
        ),
        b"data: [DONE]\n\n",
    ]

    async def handler(_incoming: httpx.Request) -> httpx.Response:
        return httpx.Response(
            200,
            headers={"content-type": "text/event-stream"},
            stream=ChunkStream(chunks),
        )

    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
        with pytest.raises(ProviderFailure) as captured:
            await collect(OpenAICompatibleAdapter(provider(), client=client, max_retries=0))

    assert captured.value.category == "protocol"
    assert secret not in str(captured.value)
    assert secret not in repr(captured.value)


@pytest.mark.asyncio
async def test_header_echo_split_across_tool_argument_fragments_is_rejected() -> None:
    secret = "header-secret"
    chunks = [
        sse(
            {
                "choices": [
                    {
                        "index": 0,
                        "delta": {
                            "tool_calls": [
                                {
                                    "index": 0,
                                    "id": "call",
                                    "type": "function",
                                    "function": {
                                        "name": "process_run",
                                        "arguments": f'{{"command":"{secret[:7]}',
                                    },
                                }
                            ]
                        },
                        "finish_reason": None,
                    }
                ]
            }
        ),
        sse(
            {
                "choices": [
                    {
                        "index": 0,
                        "delta": {
                            "tool_calls": [
                                {
                                    "index": 0,
                                    "function": {"arguments": f'{secret[7:]}"}}'},
                                }
                            ]
                        },
                        "finish_reason": "tool_calls",
                    }
                ]
            }
        ),
        b"data: [DONE]\n\n",
    ]

    async def handler(_incoming: httpx.Request) -> httpx.Response:
        return httpx.Response(
            200,
            headers={"content-type": "text/event-stream"},
            stream=ChunkStream(chunks),
        )

    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
        with pytest.raises(ProviderFailure) as captured:
            await collect(OpenAICompatibleAdapter(provider(), client=client, max_retries=0))

    assert captured.value.category == "protocol"
    assert secret not in str(captured.value)
    assert secret not in repr(captured.value)


@pytest.mark.asyncio
@pytest.mark.parametrize(
    ("status", "category", "retryable"),
    [
        (401, "authentication", False),
        (429, "rate_limit", True),
        (413, "context_overflow", False),
        (400, "invalid_request", False),
        (500, "server", True),
    ],
)
async def test_http_errors_are_normalized_without_body_or_secret_values(
    status: int,
    category: str,
    retryable: bool,
) -> None:
    secret = "sk-provider-secret"

    async def handler(_incoming: httpx.Request) -> httpx.Response:
        return httpx.Response(
            status,
            headers={
                "content-type": "application/json",
                "x-request-id": f"request-{secret}",
                "retry-after": "0",
            },
            content=f'{{"error":{{"message":"body-{secret}"}}}}',
        )

    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
        adapter = OpenAICompatibleAdapter(provider(api_key=secret), client=client, max_retries=0)
        with pytest.raises(ProviderFailure) as captured:
            await collect(adapter)

    error = captured.value
    assert error.category == category
    assert error.retryable is retryable
    assert error.request_id is None
    assert secret not in str(error)
    assert secret not in repr(error)


@pytest.mark.asyncio
async def test_short_header_value_cannot_escape_as_request_id() -> None:
    secret = "a"

    async def handler(_incoming: httpx.Request) -> httpx.Response:
        return httpx.Response(
            401,
            headers={"content-type": "application/json", "x-request-id": secret},
            content=b'{"error":{"message":"unauthorized"}}',
        )

    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
        adapter = OpenAICompatibleAdapter(
            provider(api_key=None, headers=(("X-Protected", secret),)),
            client=client,
            max_retries=0,
        )
        with pytest.raises(ProviderFailure) as captured:
            await collect(adapter)

    assert captured.value.request_id is None


@pytest.mark.asyncio
async def test_unexpected_transport_error_is_detached_and_redacted(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    secret = "unexpected-transport-secret"
    attempts = 0

    async with httpx.AsyncClient() as client:

        def explode(*_args: object, **_kwargs: object) -> httpx.Request:
            nonlocal attempts
            attempts += 1
            raise RuntimeError(secret)

        monkeypatch.setattr(client, "build_request", explode)
        adapter = OpenAICompatibleAdapter(provider(), client=client, max_retries=3)
        with pytest.raises(ProviderFailure) as captured:
            await collect(adapter)

    error = captured.value
    assert attempts == 1
    assert error.category == "unknown"
    assert secret not in str(error)
    assert secret not in repr(error)
    assert error.__cause__ is None
    assert error.__context__ is None


@pytest.mark.asyncio
async def test_retry_occurs_only_before_stream_output() -> None:
    attempts = 0

    async def handler(_incoming: httpx.Request) -> httpx.Response:
        nonlocal attempts
        attempts += 1
        if attempts == 1:
            return httpx.Response(500, headers={"retry-after": "0"})
        return httpx.Response(
            200,
            headers={"content-type": "text/event-stream"},
            stream=ChunkStream([sse(text_chunk("retried", finish_reason="stop"))]),
        )

    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
        events = await collect(OpenAICompatibleAdapter(provider(), client=client))

    assert attempts == 2
    assert events == [TextDelta("retried"), ResponseCompleted()]


@pytest.mark.asyncio
async def test_non_finite_retry_after_uses_safe_default_backoff() -> None:
    attempts = 0

    async def handler(_incoming: httpx.Request) -> httpx.Response:
        nonlocal attempts
        attempts += 1
        if attempts == 1:
            return httpx.Response(429, headers={"retry-after": "NaN"})
        return httpx.Response(
            200,
            headers={"content-type": "text/event-stream"},
            stream=ChunkStream([sse(text_chunk("retried", finish_reason="stop"))]),
        )

    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
        events = await collect(OpenAICompatibleAdapter(provider(), client=client))

    assert attempts == 2
    assert events == [TextDelta("retried"), ResponseCompleted()]


@pytest.mark.asyncio
async def test_midstream_failure_is_not_retried_after_first_delta() -> None:
    attempts = 0

    async def handler(_incoming: httpx.Request) -> httpx.Response:
        nonlocal attempts
        attempts += 1
        return httpx.Response(
            200,
            headers={"content-type": "text/event-stream"},
            stream=ChunkStream([sse(text_chunk("partial"))], error_after=1),
        )

    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
        adapter = OpenAICompatibleAdapter(provider(), client=client)
        stream = adapter.stream(request(), cancellation=CancellationToken())
        assert await anext(stream) == TextDelta("partial")
        with pytest.raises(ProviderFailure) as captured:
            await anext(stream)

    assert captured.value.category == "network"
    assert captured.value.retryable is False
    assert "upstream-body-must-not-leak" not in str(captured.value)
    assert attempts == 1


@pytest.mark.asyncio
async def test_cancellation_interrupts_response_header_wait() -> None:
    entered = asyncio.Event()
    handler_cancelled = asyncio.Event()

    async def handler(_incoming: httpx.Request) -> httpx.Response:
        entered.set()
        try:
            await asyncio.Event().wait()
        except asyncio.CancelledError:
            handler_cancelled.set()
            raise
        raise AssertionError("unreachable")

    cancellation = CancellationToken()
    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
        adapter = OpenAICompatibleAdapter(provider(), client=client)
        task = asyncio.create_task(collect(adapter, cancellation=cancellation))
        await asyncio.wait_for(entered.wait(), timeout=1)
        cancellation.cancel()
        with pytest.raises(RunCancelled):
            await asyncio.wait_for(task, timeout=1)
        await asyncio.wait_for(handler_cancelled.wait(), timeout=1)


@pytest.mark.asyncio
async def test_response_is_closed_when_headers_and_cancellation_complete_together() -> None:
    entered = asyncio.Event()
    release = asyncio.Event()
    response_stream = ChunkStream([])

    async def handler(_incoming: httpx.Request) -> httpx.Response:
        entered.set()
        await release.wait()
        return httpx.Response(
            200,
            headers={"content-type": "text/event-stream"},
            stream=response_stream,
        )

    cancellation = CancellationToken()
    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
        adapter = OpenAICompatibleAdapter(provider(), client=client)
        task = asyncio.create_task(collect(adapter, cancellation=cancellation))
        await asyncio.wait_for(entered.wait(), timeout=1)
        cancellation.cancel()
        release.set()
        with pytest.raises(RunCancelled):
            await asyncio.wait_for(task, timeout=1)

    assert response_stream.closed


@pytest.mark.asyncio
async def test_response_header_timeout_cancels_transport_and_is_normalized() -> None:
    entered = asyncio.Event()
    handler_cancelled = asyncio.Event()

    async def handler(_incoming: httpx.Request) -> httpx.Response:
        entered.set()
        try:
            await asyncio.Event().wait()
        except asyncio.CancelledError:
            handler_cancelled.set()
            raise
        raise AssertionError("unreachable")

    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
        adapter = OpenAICompatibleAdapter(
            provider(),
            client=client,
            timeouts=ProviderTimeouts(connect=1, response_header=0.01, stream_idle=1),
            max_retries=0,
        )
        with pytest.raises(ProviderFailure) as captured:
            await collect(adapter)
        await asyncio.wait_for(handler_cancelled.wait(), timeout=1)

    assert entered.is_set()
    assert captured.value.category == "timeout"


@pytest.mark.asyncio
async def test_stream_idle_timeout_and_cancellation_close_response() -> None:
    gate = asyncio.Event()
    idle_stream = ChunkStream([], wait_before=0, gate=gate)

    async def idle_handler(_incoming: httpx.Request) -> httpx.Response:
        return httpx.Response(
            200,
            headers={"content-type": "text/event-stream"},
            stream=idle_stream,
        )

    async with httpx.AsyncClient(transport=httpx.MockTransport(idle_handler)) as client:
        adapter = OpenAICompatibleAdapter(
            provider(),
            client=client,
            timeouts=ProviderTimeouts(connect=1, response_header=1, stream_idle=0.01),
            max_retries=0,
        )
        with pytest.raises(ProviderFailure) as captured:
            await collect(adapter)
    assert captured.value.category == "timeout"
    assert idle_stream.closed

    cancel_gate = asyncio.Event()
    cancel_stream = ChunkStream([], wait_before=0, gate=cancel_gate)

    async def cancel_handler(_incoming: httpx.Request) -> httpx.Response:
        return httpx.Response(
            200,
            headers={"content-type": "text/event-stream"},
            stream=cancel_stream,
        )

    cancellation = CancellationToken()
    async with httpx.AsyncClient(transport=httpx.MockTransport(cancel_handler)) as client:
        adapter = OpenAICompatibleAdapter(provider(), client=client)
        task = asyncio.create_task(collect(adapter, cancellation=cancellation))
        await asyncio.wait_for(cancel_stream.waiting.wait(), timeout=1)
        cancellation.cancel()
        with pytest.raises(RunCancelled):
            await asyncio.wait_for(task, timeout=1)
    assert cancel_stream.closed


@pytest.mark.asyncio
async def test_provider_registry_caches_replaces_and_closes_clients(tmp_path: Path) -> None:
    config = ConfigStore(tmp_path)
    config.configure_custom(
        provider_id="local",
        display_name="Local",
        base_url="http://127.0.0.1:8080/v1",
        api_key=None,
        headers=None,
        models=[ModelInput("model", "Model")],
    )
    created: list[OpenAICompatibleAdapter] = []

    def factory(configured: ProviderConfig) -> OpenAICompatibleAdapter:
        adapter = OpenAICompatibleAdapter(configured)
        created.append(adapter)
        return adapter

    registry = RuntimeProviderRegistry(config, adapter_factory=factory)
    first = registry.resolve("local")
    assert first is registry.resolve("local")
    config.configure_custom(
        provider_id="local",
        display_name="Local changed",
        base_url="http://127.0.0.1:8080/v1",
        api_key=None,
        headers=None,
        models=[ModelInput("model", "Model")],
    )
    registry.configuration_changed("local")
    await asyncio.sleep(0)
    second = registry.resolve("local")
    assert second is not first
    assert len(created) == 2
    await registry.close()
    assert all(adapter._closed for adapter in created)
