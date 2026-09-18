from __future__ import annotations

import asyncio
import json
import os
import secrets
import shlex
import signal
import sqlite3
import subprocess
import sys
from asyncio.subprocess import Process
from pathlib import Path
from typing import Any, cast

import pytest
from websockets.asyncio.client import ClientConnection, connect
from websockets.asyncio.server import ServerConnection
from websockets.exceptions import InvalidStatus

import ikaros_runtime.bootstrap as bootstrap_module
from ikaros_runtime.agent.scheduler import AgentScheduler
from ikaros_runtime.bootstrap import RuntimeApplication
from ikaros_runtime.domain import JOURNAL_EVENT_SCHEMA_VERSION, JournalEvent
from ikaros_runtime.errors import ConfigError, InvalidParamsError
from ikaros_runtime.identity import IdentityResourceError, load_identity_core
from ikaros_runtime.memory import MemoryScope, SqliteMemoryStore
from ikaros_runtime.protocol.spec import PROTOCOL_VERSION
from ikaros_runtime.providers.registry import ConfigStore, ModelInput, ProviderConfig
from ikaros_runtime.run_input import (
    INPUT_BUDGET_MEASUREMENT_VERSION,
    ContextRevision,
    HistoryGroupReferenceV1,
    HistoryItemReferenceV1,
    InputBudgetRecord,
    MemoryReferenceV1,
    OmissionRecordV1,
    StepInput,
)
from ikaros_runtime.security import response_values_contain_protected_value
from ikaros_runtime.server.connection import handle_connection
from ikaros_runtime.server.event_hub import EventHub
from ikaros_runtime.server.host import ServerSettings, _parent_is_alive
from ikaros_runtime.services.turns import TurnService
from ikaros_runtime.storage import SqliteRuntimeStore
from ikaros_runtime.tools.core import ToolDefinition

from .helpers import prepare_turn, run_config

_HIDDEN_PROCESS_FLAGS = int(getattr(subprocess, "CREATE_NO_WINDOW", 0))


@pytest.mark.parametrize("protected", ["newline", "lf", "crlf"])
def test_file_result_vocabulary_has_fixed_security_provenance(protected: str) -> None:
    result = {
        "path": "notes.txt",
        "newline": "crlf",
        "verified": True,
    }

    assert response_values_contain_protected_value(result, [protected]) is False


@pytest.mark.parametrize("protected", ["nextCursor", "snapshotSeq"])
def test_thread_catalog_keys_have_fixed_security_provenance(protected: str) -> None:
    result = {
        "nextCursor": None,
        "snapshotSeq": 0,
    }

    assert response_values_contain_protected_value(result, [protected]) is False


def test_model_input_snapshot_runtime_provenance_is_not_treated_as_a_credential() -> None:
    config = run_config(
        "scripted",
        "scripted-v1",
        identity_core=load_identity_core(),
        tools=(
            ToolDefinition(
                name="runtime-owned-tool",
                description="Runtime owned Tool description",
                input_schema={"type": "object", "properties": {"command": {"type": "string"}}},
            ),
        ),
    )
    budget = InputBudgetRecord(
        mode="bounded",
        measurement_version=INPUT_BUDGET_MEASUREMENT_VERSION,
        maximum_tokens=28672,
        reserved_current_run_tokens=7168,
        instruction_tokens=0,
        context_data_tokens=96,
        tool_tokens=0,
        history_tokens=0,
        current_run_tokens=1,
        memory_tokens=48,
        total_tokens=145,
    )
    memory = (MemoryReferenceV1("memory_00000000000000000000000000000001", 1, "global", 12),)
    omissions = (
        OmissionRecordV1(
            "memory", "memory_00000000000000000000000000000002", "omitted_by_budget", 1, 13
        ),
    )
    history_items = (
        HistoryItemReferenceV1(
            config.user_item_id, config.turn_id, config.run_id, "message", "user", 1
        ),
    )
    revision = ContextRevision(
        history_groups=(HistoryGroupReferenceV1(config.turn_id, (config.user_item_id,)),),
        history_items=history_items,
        memory=memory,
        budget=budget,
        omissions=omissions,
        memory_context_characters=24,
    )
    step = StepInput(
        step_ordinal=1,
        context_revision=1,
        history_items=history_items,
        memory=memory,
        budget=budget,
        omissions=omissions,
        memory_context_characters=24,
    )
    value = {
        "payload": {
            "runConfig": config.to_wire(),
            "contextRevision": revision.to_wire(),
            "stepInput": step.to_wire(),
        }
    }
    assert config.identity_core is not None
    for protected in (
        "full_access",
        "scripted-v1",
        "runtime_instruction",
        "runtime_identity",
        "release",
        config.output_style.content,
        config.identity_core.content,
        "runtime-owned-tool",
        "Runtime owned Tool description",
        "object",
        "properties",
        "command",
        "string",
        "bounded",
        INPUT_BUDGET_MEASUREMENT_VERSION,
        "global",
        "memory",
        "omitted_by_budget",
        config.public_provider_config_fingerprint,
    ):
        assert response_values_contain_protected_value(value, [protected]) is False


@pytest.mark.parametrize(
    "container",
    (
        {"payload": {"runConfig": {"tools": [{"description": "dynamic-secret"}]}}},
        {
            "params": {
                "payload": {"contextRevision": {"historyItems": [{"kind": "dynamic-secret"}]}}
            }
        },
        {"result": {"events": [{"payload": {"stepInput": {"omissions": ["dynamic-secret"]}}}]}},
    ),
)
def test_model_input_snapshot_provenance_accepts_only_legal_roots(
    container: object,
) -> None:
    assert response_values_contain_protected_value(container, ["dynamic-secret"]) is False


def test_nested_model_input_lookalike_does_not_bypass_protected_value_scan() -> None:
    value = {
        "params": {
            "payload": {
                "item": {
                    "data": {
                        "result": {
                            "payload": {"runConfig": {"tools": [{"description": "dynamic-secret"}]}}
                        }
                    }
                }
            }
        }
    }

    assert response_values_contain_protected_value(value, ["dynamic-secret"]) is True


@pytest.mark.parametrize(
    "value",
    (
        {"payload": {"runConfig": {"workspace": {"name": "dynamic-secret"}}}},
        {
            "payload": {
                "runConfig": {
                    "skills": [
                        {
                            "name": "dynamic-secret",
                            "description": "safe",
                            "location": "C:/safe/SKILL.md",
                        }
                    ]
                }
            }
        },
        {
            "payload": {
                "runConfig": {"instructions": {"skillCatalog": {"content": "dynamic-secret"}}}
            }
        },
        {
            "payload": {
                "runConfig": {
                    "providerId": "custom-provider",
                    "modelId": "dynamic-secret",
                }
            }
        },
        {"responseModelId": "dynamic-secret"},
        {"requestId": "dynamic-secret"},
        {"baseUrl": "https://dynamic-secret.example/v1"},
    ),
)
def test_model_input_and_provider_dynamic_values_remain_guarded(
    value: object,
) -> None:
    assert response_values_contain_protected_value(value, ["dynamic-secret"]) is True


def test_non_scripted_model_named_like_the_scripted_model_remains_guarded() -> None:
    value = {
        "payload": {
            "runConfig": {
                "providerId": "custom-provider",
                "modelId": "scripted-v1",
            }
        }
    }

    assert response_values_contain_protected_value(value, ["scripted-v1"]) is True


class AckFailingConnection:
    def __init__(self, messages: list[dict[str, Any]]) -> None:
        self._messages = iter(messages)
        self.sent: list[dict[str, Any]] = []

    def __aiter__(self) -> AckFailingConnection:
        return self

    async def __anext__(self) -> str:
        try:
            message = next(self._messages)
        except StopIteration as error:
            raise StopAsyncIteration from error
        return json.dumps(message)

    async def send(self, message: str) -> None:
        value = json.loads(message)
        if value.get("id") == 2:
            raise ConnectionError("simulated ACK disconnect")
        self.sent.append(value)

    async def close(self, *, code: int, reason: str) -> None:
        del code, reason


class ControlledAckConnection:
    def __init__(self, messages: list[dict[str, Any]], ack_gate: asyncio.Event) -> None:
        self._messages = iter(messages)
        self._ack_gate = ack_gate
        self.ack_started = asyncio.Event()
        self.ack_completed = asyncio.Event()
        self.sent: list[dict[str, Any]] = []

    def __aiter__(self) -> ControlledAckConnection:
        return self

    async def __anext__(self) -> str:
        try:
            message = next(self._messages)
        except StopIteration as error:
            raise StopAsyncIteration from error
        return json.dumps(message)

    async def send(self, message: str) -> None:
        value = json.loads(message)
        if value.get("id") == 2:
            self.ack_started.set()
            await self._ack_gate.wait()
            self.ack_completed.set()
        self.sent.append(value)

    async def close(self, *, code: int, reason: str) -> None:
        del code, reason


async def _start_runtime(token: str, runtime_home: Path) -> tuple[Process, dict[str, Any]]:
    process = await asyncio.create_subprocess_exec(
        sys.executable,
        "-m",
        "ikaros_runtime",
        "serve",
        "--host",
        "127.0.0.1",
        "--port",
        "0",
        f"--token={token}",
        "--parent-pid",
        str(os.getpid()),
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
        env={**os.environ, "IKAROS_HOME": str(runtime_home)},
        creationflags=_HIDDEN_PROCESS_FLAGS,
    )
    try:
        assert process.stdout is not None
        line = await asyncio.wait_for(process.stdout.readline(), timeout=10)
        if not line:
            assert process.stderr is not None
            stderr = (await process.stderr.read()).decode(errors="replace")
            raise AssertionError(f"Runtime exited before readiness: {stderr}")
        return process, json.loads(line)
    except BaseException:
        await _stop_failed_process(process)
        raise


async def _stop_failed_process(process: Process) -> None:
    if process.returncode is not None:
        return
    process.terminate()
    try:
        await asyncio.wait_for(process.wait(), timeout=5)
    except TimeoutError:
        process.kill()
        await process.wait()


async def _rpc(
    connection: ClientConnection,
    request_id: int,
    method: str,
    params: dict[str, Any],
    notifications: list[dict[str, Any]] | None = None,
) -> dict[str, Any]:
    await connection.send(
        json.dumps({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params})
    )
    while True:
        response = json.loads(await connection.recv())
        assert response["jsonrpc"] == "2.0"
        if response.get("method") == "event":
            if notifications is not None:
                notifications.append(response["params"])
            continue
        assert response["id"] == request_id
        return cast(dict[str, Any], response)


async def _raw_rpc(
    connection: ClientConnection,
    request: dict[str, Any],
) -> dict[str, Any]:
    await connection.send(json.dumps(request))
    while True:
        response = json.loads(await connection.recv())
        assert response["jsonrpc"] == "2.0"
        if response.get("method") == "event":
            continue
        return cast(dict[str, Any], response)


async def _collect_run_events(
    connection: ClientConnection,
    run_id: str,
) -> list[dict[str, Any]]:
    events: list[dict[str, Any]] = []
    while True:
        message = json.loads(await asyncio.wait_for(connection.recv(), timeout=10))
        assert message["jsonrpc"] == "2.0"
        assert message["method"] == "event"
        event = message["params"]
        events.append(event)
        if event["type"] == "run.settled" and event["runId"] == run_id:
            return events


async def _replay_until_run_settled(
    connection: ClientConnection,
    run_id: str,
    *,
    request_id: int,
) -> list[dict[str, Any]]:
    cursor = 0
    events: list[dict[str, Any]] = []
    for offset in range(300):
        response = await _rpc(
            connection,
            request_id + offset,
            "event.replay",
            {"afterSeq": cursor, "limit": 1000},
            [],
        )
        page = cast(list[dict[str, Any]], response["result"]["events"])
        if page:
            assert [event["seq"] for event in page] == list(
                range(cursor + 1, cursor + len(page) + 1)
            )
            events.extend(page)
            cursor = int(response["result"]["nextAfterSeq"])
        if any(event["type"] == "run.settled" and event["runId"] == run_id for event in events):
            return events
        await asyncio.sleep(0.01)
    raise AssertionError(f"run {run_id} did not settle")


async def _initialize(uri: str, token: str) -> ClientConnection:
    connection = await connect(
        uri,
        additional_headers={"Authorization": f"Bearer {token}"},
    )
    initialized = await _rpc(
        connection,
        1,
        "initialize",
        {
            "protocolVersion": PROTOCOL_VERSION,
            "client": {"name": "runtime-test", "version": "0.1.0"},
        },
    )
    assert initialized["result"] == {
        "protocolVersion": PROTOCOL_VERSION,
        "server": {"name": "ikaros-runtime", "version": "0.1.0"},
    }
    return connection


async def _shutdown(connection: ClientConnection, process: Process, request_id: int) -> None:
    shutdown = await _rpc(connection, request_id, "runtime.shutdown", {})
    assert shutdown["result"] == {"accepted": True}
    await connection.close()
    assert await asyncio.wait_for(process.wait(), timeout=5) == 0


def _journal_event(seq: int) -> JournalEvent:
    return JournalEvent(
        seq=seq,
        schema_version=JOURNAL_EVENT_SCHEMA_VERSION,
        type="test.event",
        thread_id=None,
        branch_id=None,
        turn_id=None,
        run_id=None,
        item_id=None,
        timestamp="2026-08-11T12:00:00.000Z",
        payload={},
    )


async def _discard_event(_event: JournalEvent) -> None:
    return None


def _database_contents(runtime_home: Path) -> list[bytes]:
    return [path.read_bytes() for path in runtime_home.glob("*.db*")]


class FakeOpenAIEndpoint:
    def __init__(self) -> None:
        self.requests: list[dict[str, Any]] = []
        self._server: asyncio.Server | None = None

    async def start(self) -> str:
        self._server = await asyncio.start_server(self._handle, "127.0.0.1", 0)
        socket = next(iter(self._server.sockets or ()))
        port = int(socket.getsockname()[1])
        return f"http://127.0.0.1:{port}/v1"

    async def close(self) -> None:
        if self._server is None:
            return
        self._server.close()
        await self._server.wait_closed()
        self._server = None

    async def _handle(
        self,
        reader: asyncio.StreamReader,
        writer: asyncio.StreamWriter,
    ) -> None:
        try:
            header_source = await asyncio.wait_for(reader.readuntil(b"\r\n\r\n"), timeout=5)
            header_lines = header_source.decode("ascii").split("\r\n")
            request_line = header_lines[0]
            headers = {
                name.strip().lower(): value.strip()
                for line in header_lines[1:]
                if ":" in line
                for name, value in [line.split(":", 1)]
            }
            content_length = int(headers.get("content-length", "0"))
            content = await asyncio.wait_for(reader.readexactly(content_length), timeout=5)
            request = cast(dict[str, Any], json.loads(content))
            request["_requestLine"] = request_line
            self.requests.append(request)
            body = await self._response_body(request)
            writer.write(
                (
                    "HTTP/1.1 200 OK\r\n"
                    "Content-Type: text/event-stream\r\n"
                    f"Content-Length: {len(body)}\r\n"
                    "Connection: close\r\n"
                    "\r\n"
                ).encode("ascii")
                + body
            )
            await writer.drain()
        finally:
            writer.close()
            await writer.wait_closed()

    async def _response_body(self, request: dict[str, Any]) -> bytes:
        messages = cast(list[dict[str, Any]], request["messages"])
        if messages[-1]["role"] == "tool":
            return _fake_process_followup(messages, "Tool result received by fake provider.")
        last_user = next(message for message in reversed(messages) if message["role"] == "user")
        if last_user["content"] == "run a tool":
            chunks = [
                {
                    "choices": [
                        {
                            "index": 0,
                            "delta": {
                                "reasoning_content": "fake tool reasoning",
                                "tool_calls": [
                                    {
                                        "index": 0,
                                        "id": "call_fake_process",
                                        "type": "function",
                                        "function": {
                                            "name": "process_start",
                                            "arguments": '{"command":"echo gate6-tool"}',
                                        },
                                    }
                                ],
                            },
                            "finish_reason": None,
                        }
                    ]
                },
                {
                    "choices": [
                        {
                            "index": 0,
                            "delta": {},
                            "finish_reason": "tool_calls",
                        }
                    ]
                },
            ]
            return _fake_sse(chunks)
        user_messages = [message for message in messages if message["role"] == "user"]
        if len(user_messages) == 1:
            return _fake_sse_text("First fake response.")
        return _fake_sse_text("Second fake response with context.")


class UsageOpenAIEndpoint(FakeOpenAIEndpoint):
    async def _response_body(self, request: dict[str, Any]) -> bytes:
        assert request["stream_options"] == {"include_usage": True}
        return _fake_sse(
            [
                {
                    "choices": [
                        {
                            "index": 0,
                            "delta": {"content": "Usage recorded by fake provider."},
                            "finish_reason": "stop",
                        }
                    ]
                },
                {
                    "choices": [],
                    "usage": {
                        "prompt_tokens": 7,
                        "completion_tokens": 5,
                        "total_tokens": 12,
                    },
                },
            ]
        )


class FileToolChainEndpoint(FakeOpenAIEndpoint):
    def __init__(self, workspace: Path) -> None:
        super().__init__()
        self._workspace = workspace
        self._calls = [
            ("call_write", "write", {"filePath": "chain.txt", "content": "alpha token"}),
            ("call_read_before", "read", {"filePath": "chain.txt"}),
            (
                "call_edit",
                "edit",
                {"filePath": "chain.txt", "oldString": "alpha", "newString": "omega"},
            ),
            ("call_read_after", "read", {"filePath": "chain.txt"}),
        ]

    async def _response_body(self, request: dict[str, Any]) -> bytes:
        messages = cast(list[dict[str, Any]], request["messages"])
        tool_results = [message for message in messages if message["role"] == "tool"]
        if len(tool_results) == len(self._calls):
            assert self._workspace.joinpath("chain.txt").read_text(encoding="utf-8") == (
                "omega token"
            )
            return _fake_sse_text("File tool chain completed with omega token.")

        call_id, name, arguments = self._calls[len(tool_results)]
        chunks: list[dict[str, Any]] = []
        if not tool_results:
            chunks.append(
                {
                    "choices": [
                        {
                            "index": 0,
                            "delta": {"content": "I will update and verify the file."},
                            "finish_reason": None,
                        }
                    ]
                }
            )
        chunks.extend(
            [
                {
                    "choices": [
                        {
                            "index": 0,
                            "delta": {
                                "tool_calls": [
                                    {
                                        "index": 0,
                                        "id": call_id,
                                        "type": "function",
                                        "function": {
                                            "name": name,
                                            "arguments": json.dumps(
                                                arguments,
                                                separators=(",", ":"),
                                            ),
                                        },
                                    }
                                ]
                            },
                            "finish_reason": None,
                        }
                    ]
                },
                {
                    "choices": [
                        {
                            "index": 0,
                            "delta": {},
                            "finish_reason": "tool_calls",
                        }
                    ]
                },
            ]
        )
        return _fake_sse(chunks)


class InvalidNumericToolArgumentsEndpoint(FakeOpenAIEndpoint):
    def __init__(self, command: str, invalid_number: str) -> None:
        super().__init__()
        self._command = command
        self._invalid_number = invalid_number

    async def _response_body(self, request: dict[str, Any]) -> bytes:
        del request
        arguments = json.dumps({"command": self._command}, separators=(",", ":"))
        arguments = arguments[:-1] + f',"timeoutMs":{self._invalid_number}}}'
        return _fake_sse(
            [
                {
                    "choices": [
                        {
                            "index": 0,
                            "delta": {
                                "tool_calls": [
                                    {
                                        "index": 0,
                                        "id": "call_invalid_numeric",
                                        "type": "function",
                                        "function": {
                                            "name": "process_start",
                                            "arguments": arguments,
                                        },
                                    }
                                ]
                            },
                            "finish_reason": "tool_calls",
                        }
                    ]
                }
            ]
        )


class ProtectedToolEndpoint(FakeOpenAIEndpoint):
    def __init__(self, command: str) -> None:
        super().__init__()
        self._command = command

    async def _response_body(self, request: dict[str, Any]) -> bytes:
        messages = cast(list[dict[str, Any]], request["messages"])
        if messages[-1]["role"] == "tool":
            return _fake_process_followup(messages, "Protected tool result handled safely.")
        arguments = json.dumps({"command": self._command}, separators=(",", ":"))
        return _fake_sse(
            [
                {
                    "choices": [
                        {
                            "index": 0,
                            "delta": {
                                "tool_calls": [
                                    {
                                        "index": 0,
                                        "id": "call_read_config",
                                        "type": "function",
                                        "function": {
                                            "name": "process_start",
                                            "arguments": arguments,
                                        },
                                    }
                                ]
                            },
                            "finish_reason": None,
                        }
                    ]
                },
                {
                    "choices": [
                        {
                            "index": 0,
                            "delta": {},
                            "finish_reason": "tool_calls",
                        }
                    ]
                },
            ]
        )


class SplitProtectedEndpoint(FakeOpenAIEndpoint):
    def __init__(self, protected: str) -> None:
        super().__init__()
        self._protected = protected

    async def _response_body(self, request: dict[str, Any]) -> bytes:
        del request
        split_at = len(self._protected) // 2
        return _fake_sse(
            [
                {
                    "choices": [
                        {
                            "index": 0,
                            "delta": {"content": f"safe prefix {self._protected[:split_at]}"},
                            "finish_reason": None,
                        }
                    ]
                },
                {
                    "choices": [
                        {
                            "index": 0,
                            "delta": {"content": self._protected[split_at:]},
                            "finish_reason": "stop",
                        }
                    ]
                },
            ]
        )


class DelayedProtectedEndpoint(SplitProtectedEndpoint):
    def __init__(self, protected: str) -> None:
        super().__init__(protected)
        self.request_received = asyncio.Event()
        self.release_response = asyncio.Event()

    async def _response_body(self, request: dict[str, Any]) -> bytes:
        self.request_received.set()
        await asyncio.wait_for(self.release_response.wait(), timeout=10)
        return await super()._response_body(request)


def _read_file_command(path: Path) -> str:
    if os.name == "nt":
        escaped = str(path).replace("'", "''")
        return f"Get-Content -Raw -LiteralPath '{escaped}'"
    return f"cat -- {shlex.quote(str(path))}"


def _fake_sse(chunks: list[dict[str, Any]]) -> bytes:
    payloads = [
        f"data: {json.dumps(chunk, separators=(',', ':'))}\n\n".encode() for chunk in chunks
    ]
    payloads.append(b"data: [DONE]\n\n")
    return b"".join(payloads)


def _fake_process_followup(messages: list[dict[str, Any]], final: str) -> bytes:
    result = json.loads(messages[-1]["content"])
    if result.get("state") == "running":
        return _fake_sse(
            [
                {
                    "choices": [
                        {
                            "index": 0,
                            "delta": {
                                "tool_calls": [
                                    {
                                        "index": 0,
                                        "id": f"call_wait_{len(messages)}",
                                        "type": "function",
                                        "function": {
                                            "name": "process_wait",
                                            "arguments": json.dumps(
                                                {
                                                    "processId": result["processId"],
                                                    "timeoutMs": 5000,
                                                }
                                            ),
                                        },
                                    }
                                ]
                            },
                            "finish_reason": "tool_calls",
                        }
                    ],
                }
            ]
        )
    return _fake_sse_text(final)


def _fake_sse_text(content: str) -> bytes:
    return _fake_sse(
        [
            {
                "choices": [
                    {
                        "index": 0,
                        "delta": {"content": content},
                        "finish_reason": "stop",
                    }
                ]
            }
        ]
    )


def test_parent_process_probe_is_non_destructive() -> None:
    assert _parent_is_alive(os.getpid())
    assert not _parent_is_alive(2_147_483_647)


@pytest.mark.asyncio
async def test_provider_configuration_rpc_is_persisted_redacted_and_event_free(
    tmp_path: Path,
) -> None:
    token = secrets.token_hex(32)
    deepseek_secret = "sk-rpc-deepseek-sentinel"
    custom_secret = "sk-rpc-custom-sentinel"
    header_secret = "rpc-header-sentinel"
    process, readiness = await _start_runtime(token, tmp_path)
    uri = f"ws://{readiness['host']}:{readiness['port']}"
    connection = await _initialize(uri, token)
    observed_responses: list[dict[str, Any]] = []
    first_stderr = ""
    try:
        initial_providers = await _rpc(connection, 2, "provider.list", {})
        observed_responses.append(initial_providers)
        assert initial_providers["result"] == {
            "providers": [
                {
                    "id": "deepseek",
                    "displayName": "DeepSeek",
                    "origin": "builtin",
                    "configured": False,
                    "credentialConfigured": False,
                    "health": "unknown",
                }
            ]
        }
        assert not (tmp_path / "config.yaml").exists()

        configured_deepseek = await _rpc(
            connection,
            3,
            "provider.configure",
            {
                "kind": "deepseek",
                "apiKey": deepseek_secret,
                "models": [
                    {
                        "id": "deepseek-chat",
                        "displayName": "DeepSeek Chat",
                        "contextWindow": 32768,
                        "maxOutputTokens": 4096,
                    }
                ],
            },
        )
        observed_responses.append(configured_deepseek)
        assert configured_deepseek["result"]["provider"] == {
            "id": "deepseek",
            "displayName": "DeepSeek",
            "origin": "builtin",
            "configured": True,
            "credentialConfigured": True,
            "health": "unknown",
        }

        configured_custom = await _rpc(
            connection,
            4,
            "provider.configure",
            {
                "kind": "custom",
                "providerId": "local",
                "displayName": "Local",
                "baseUrl": "http://127.0.0.1:8080/v1",
                "apiKey": custom_secret,
                "headers": {"X-Tenant": header_secret},
                "models": [
                    {
                        "id": "local-model",
                        "displayName": "Local Model",
                        "contextWindow": 32768,
                        "maxOutputTokens": 4096,
                    }
                ],
            },
        )
        observed_responses.append(configured_custom)
        assert configured_custom["result"]["provider"]["id"] == "local"

        disabled = await _rpc(
            connection,
            5,
            "model.set_enabled",
            {"providerId": "local", "modelId": "local-model", "enabled": False},
        )
        observed_responses.append(disabled)
        assert disabled["result"]["model"]["enabled"] is False

        providers = await _rpc(connection, 6, "provider.list", {})
        models = await _rpc(connection, 7, "model.list", {})
        replay = await _rpc(connection, 8, "event.replay", {"afterSeq": 0})
        observed_responses.extend([providers, models, replay])
        assert [provider["id"] for provider in providers["result"]["providers"]] == [
            "deepseek",
            "local",
        ]
        assert models["result"]["models"] == [
            {
                "providerId": "deepseek",
                "id": "deepseek-chat",
                "displayName": "DeepSeek Chat",
                "enabled": True,
                "contextWindow": 32768,
                "maxOutputTokens": 4096,
            },
            {
                "providerId": "local",
                "id": "local-model",
                "displayName": "Local Model",
                "enabled": False,
                "contextWindow": 32768,
                "maxOutputTokens": 4096,
            },
        ]
        assert replay["result"]["events"] == []
        await _shutdown(connection, process, 9)
        assert process.stderr is not None
        first_stderr = (await process.stderr.read()).decode(errors="replace")
    finally:
        if process.returncode is None:
            await _stop_failed_process(process)

    serialized_responses = json.dumps(observed_responses)
    for secret in (deepseek_secret, custom_secret, header_secret):
        assert secret not in serialized_responses
        assert secret not in first_stderr
        assert all(secret.encode() not in content for content in _database_contents(tmp_path))
    config_source = (tmp_path / "config.yaml").read_text(encoding="utf-8")
    assert deepseek_secret in config_source
    assert custom_secret in config_source
    assert header_secret in config_source

    restarted, readiness = await _start_runtime(token, tmp_path)
    restarted_uri = f"ws://{readiness['host']}:{readiness['port']}"
    restarted_connection = await _initialize(restarted_uri, token)
    try:
        restored_models = await _rpc(restarted_connection, 2, "model.list", {})
        assert restored_models["result"]["models"][1]["enabled"] is False
        disconnected = await _rpc(
            restarted_connection,
            3,
            "provider.disconnect",
            {"providerId": "deepseek"},
        )
        assert disconnected["result"]["provider"]["configured"] is False
        after_disconnect = await _rpc(restarted_connection, 4, "model.list", {})
        assert after_disconnect["result"]["models"] == [
            {
                "providerId": "local",
                "id": "local-model",
                "displayName": "Local Model",
                "enabled": False,
                "contextWindow": 32768,
                "maxOutputTokens": 4096,
            }
        ]
        removed = await _rpc(
            restarted_connection,
            5,
            "provider.remove",
            {"providerId": "local"},
        )
        assert removed["result"] == {"removed": True, "providerId": "local"}
        await _shutdown(restarted_connection, restarted, 6)
    finally:
        if restarted.returncode is None:
            await _stop_failed_process(restarted)

    assert not (tmp_path / "config.yaml").exists()


@pytest.mark.asyncio
async def test_memory_rpc_is_durable_idempotent_and_event_free(tmp_path: Path) -> None:
    token = secrets.token_hex(32)
    process, readiness = await _start_runtime(token, tmp_path)
    uri = f"ws://{readiness['host']}:{readiness['port']}"
    connection = await _initialize(uri, token)
    params = {
        "kind": "preference",
        "scope": {"type": "global", "key": None},
        "content": "  用户希望技术说明简洁。🌸  ",
        "clientRequestId": "desktop-memory-create-1",
    }
    memory_id = ""
    try:
        created = await _rpc(connection, 2, "memory.create", params)
        assert created["result"]["created"] is True
        assert created["result"]["resultingRevision"] == 1
        memory_id = cast(str, created["result"]["memoryId"])

        listed = await _rpc(
            connection,
            3,
            "memory.list",
            {"scope": {"type": "global", "key": None}},
        )
        assert listed["result"]["nextCursor"] is None
        assert listed["result"]["hasMore"] is False
        assert listed["result"]["memories"] == [
            {
                "id": memory_id,
                "kind": "preference",
                "scope": {"type": "global", "key": None},
                "revision": 1,
                "state": "active",
                "preview": params["content"],
                "createdAt": listed["result"]["memories"][0]["createdAt"],
                "updatedAt": listed["result"]["memories"][0]["updatedAt"],
                "forgottenAt": None,
            }
        ]

        fetched = await _rpc(connection, 4, "memory.get", {"memoryId": memory_id})
        memory = fetched["result"]["memory"]
        assert memory["content"] == params["content"]
        assert memory["provenance"] == {
            "sourceKind": "user_explicit",
            "threadId": None,
            "turnId": None,
            "itemId": None,
            "status": "not_applicable",
        }
        replay = await _rpc(connection, 5, "event.replay", {"afterSeq": 0})
        assert replay["result"]["events"] == []
        protected = "sk-memory-online-credential-sentinel"
        await _rpc(
            connection,
            6,
            "memory.create",
            {
                "kind": "fact",
                "scope": {"type": "global", "key": None},
                "content": f"persisted prefix {protected} suffix",
                "clientRequestId": "desktop-memory-credential-conflict",
            },
        )
        rejected = await _rpc(
            connection,
            7,
            "provider.configure",
            {
                "kind": "deepseek",
                "apiKey": protected,
                "models": [
                    {
                        "id": "deepseek-chat",
                        "displayName": "DeepSeek Chat",
                        "contextWindow": 32768,
                        "maxOutputTokens": 4096,
                    }
                ],
            },
        )
        assert rejected["error"]["code"] == -32602
        assert "credentials conflict" in rejected["error"]["message"]
        assert protected not in json.dumps(rejected)
        assert not (tmp_path / "config.yaml").exists()
        await _shutdown(connection, process, 8)
    finally:
        if process.returncode is None:
            await _stop_failed_process(process)

    assert (tmp_path / "state.db").is_file()
    assert (tmp_path / "memory.db").is_file()

    restarted, readiness = await _start_runtime(token, tmp_path)
    restarted_connection = await _initialize(
        f"ws://{readiness['host']}:{readiness['port']}",
        token,
    )
    try:
        repeated = await _rpc(restarted_connection, 2, "memory.create", params)
        assert repeated["result"] == {
            "memoryId": memory_id,
            "resultingRevision": 1,
            "created": False,
        }
        fetched = await _rpc(
            restarted_connection,
            3,
            "memory.get",
            {"memoryId": memory_id},
        )
        assert fetched["result"]["memory"]["content"] == params["content"]
        await _shutdown(restarted_connection, restarted, 4)
    finally:
        if restarted.returncode is None:
            await _stop_failed_process(restarted)


@pytest.mark.asyncio
async def test_memory_rpc_provenance_correction_forget_and_reset_replay(
    tmp_path: Path,
) -> None:
    token = secrets.token_hex(32)
    process, readiness = await _start_runtime(token, tmp_path)
    connection = await _initialize(f"ws://{readiness['host']}:{readiness['port']}", token)
    source_params: dict[str, Any]
    source_memory_id = ""
    mutation_memory_id = ""
    try:
        created_thread = await _rpc(
            connection,
            2,
            "thread.create",
            {"title": "Memory source", "clientRequestId": "memory-source-thread"},
        )
        thread = created_thread["result"]["thread"]
        started = await _rpc(
            connection,
            3,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": "Remember this exact Session Item.",
                "providerId": "scripted",
                "modelId": "scripted-v1",
            },
        )
        run_id = cast(str, started["result"]["runId"])
        events = await _collect_run_events(connection, run_id)
        latest_seq = max(cast(int, event["seq"]) for event in events)
        history = await _rpc(
            connection,
            4,
            "turn.list",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "limit": 10,
            },
        )
        items = history["result"]["turns"][0]["runs"][0]["items"]
        source_item = next(
            item for item in items if item["kind"] == "message" and item["role"] == "user"
        )
        source_params = {
            "kind": "fact",
            "scope": {"type": "global", "key": None},
            "content": "A fact captured from a Session Item.",
            "clientRequestId": "memory-source-create",
            "source": {"type": "session_item", "itemId": source_item["id"]},
        }
        source_created = await _rpc(connection, 5, "memory.create", source_params)
        source_memory_id = cast(str, source_created["result"]["memoryId"])
        source_fetched = await _rpc(
            connection,
            6,
            "memory.get",
            {"memoryId": source_memory_id},
        )
        assert source_fetched["result"]["memory"]["provenance"] == {
            "sourceKind": "session_item",
            "threadId": thread["id"],
            "turnId": history["result"]["turns"][0]["id"],
            "itemId": source_item["id"],
            "status": "available",
        }

        explicit_created = await _rpc(
            connection,
            7,
            "memory.create",
            {
                "kind": "preference",
                "scope": {"type": "global", "key": None},
                "content": "old preference",
                "clientRequestId": "memory-mutation-create",
            },
        )
        mutation_memory_id = cast(str, explicit_created["result"]["memoryId"])
        correction_params = {
            "memoryId": mutation_memory_id,
            "expectedRevision": 1,
            "content": "corrected preference",
            "clientRequestId": "memory-mutation-correct",
        }
        corrected = await _rpc(connection, 8, "memory.correct", correction_params)
        assert corrected["result"] == {
            "memoryId": mutation_memory_id,
            "resultingRevision": 2,
            "created": True,
        }
        repeated_correct = await _rpc(connection, 9, "memory.correct", correction_params)
        assert repeated_correct["result"]["created"] is False
        stale = await _rpc(
            connection,
            10,
            "memory.correct",
            {
                **correction_params,
                "content": "stale correction",
                "clientRequestId": "memory-mutation-stale",
            },
        )
        assert stale["error"] == {
            "code": -32020,
            "message": "memory operation failed",
            "data": {"reasonCode": "memory_revision_conflict"},
        }
        forget_params = {
            "memoryId": mutation_memory_id,
            "expectedRevision": 2,
            "clientRequestId": "memory-mutation-forget",
        }
        forgotten = await _rpc(connection, 11, "memory.forget", forget_params)
        assert forgotten["result"] == {
            "memoryId": mutation_memory_id,
            "resultingRevision": 3,
            "created": True,
        }
        active = await _rpc(connection, 12, "memory.list", {"state": "active"})
        assert mutation_memory_id not in {memory["id"] for memory in active["result"]["memories"]}
        tombstones = await _rpc(connection, 13, "memory.list", {"state": "forgotten"})
        assert tombstones["result"]["memories"] == [
            {
                **tombstones["result"]["memories"][0],
                "id": mutation_memory_id,
                "revision": 3,
                "state": "forgotten",
                "preview": None,
            }
        ]
        tombstone = await _rpc(connection, 14, "memory.get", {"memoryId": mutation_memory_id})
        assert tombstone["result"]["memory"]["content"] is None
        assert tombstone["result"]["memory"]["state"] == "forgotten"
        no_memory_events = await _rpc(
            connection,
            15,
            "event.replay",
            {"afterSeq": latest_seq, "limit": 1000},
        )
        assert no_memory_events["result"]["events"] == []
        await _shutdown(connection, process, 16)
    finally:
        if process.returncode is None:
            await _stop_failed_process(process)

    memory_before_reset = (tmp_path / "memory.db").read_bytes()
    for path in (
        tmp_path / "state.db",
        tmp_path / "state.db-wal",
        tmp_path / "state.db-shm",
    ):
        if path.exists():
            path.unlink()
    assert (tmp_path / "memory.db").read_bytes() == memory_before_reset

    restarted, restarted_readiness = await _start_runtime(token, tmp_path)
    restarted_connection = await _initialize(
        f"ws://{restarted_readiness['host']}:{restarted_readiness['port']}", token
    )
    try:
        source_replayed = await _rpc(restarted_connection, 2, "memory.create", source_params)
        assert source_replayed["result"] == {
            "memoryId": source_memory_id,
            "resultingRevision": 1,
            "created": False,
        }
        source_after_reset = await _rpc(
            restarted_connection,
            3,
            "memory.get",
            {"memoryId": source_memory_id},
        )
        assert source_after_reset["result"]["memory"]["provenance"]["status"] == ("unavailable")
        forget_replayed = await _rpc(restarted_connection, 4, "memory.forget", forget_params)
        assert forget_replayed["result"] == {
            "memoryId": mutation_memory_id,
            "resultingRevision": 3,
            "created": False,
        }
        await _shutdown(restarted_connection, restarted, 5)
    finally:
        if restarted.returncode is None:
            await _stop_failed_process(restarted)


@pytest.mark.asyncio
async def test_memory_rpc_allows_credentials_equal_to_fixed_protocol_values(
    tmp_path: Path,
) -> None:
    config = ConfigStore(tmp_path)
    for index, protected in enumerate(("fact", "global", "active"), start=1):
        config.configure_custom(
            provider_id=f"fixed-vocabulary-{index}",
            display_name=f"Fixed Vocabulary {index}",
            base_url=f"http://127.0.0.1:{9000 + index}/v1",
            api_key=protected,
            headers=None,
            models=[ModelInput(f"safe-model-{index}", f"Safe Model {index}")],
        )

    token = secrets.token_hex(32)
    process, readiness = await _start_runtime(token, tmp_path)
    connection = await _initialize(f"ws://{readiness['host']}:{readiness['port']}", token)
    try:
        created = await _rpc(
            connection,
            2,
            "memory.create",
            {
                "kind": "fact",
                "scope": {"type": "global", "key": None},
                "content": "Protocol vocabulary remains distinct from user Memory.",
                "clientRequestId": "fixed-vocabulary-request",
            },
        )
        assert "error" not in created
        memory_id = cast(str, created["result"]["memoryId"])

        listed = await _rpc(connection, 3, "memory.list", {"state": "active"})
        assert [entry["id"] for entry in listed["result"]["memories"]] == [memory_id]
        fetched = await _rpc(connection, 4, "memory.get", {"memoryId": memory_id})
        assert fetched["result"]["memory"]["kind"] == "fact"
        await _shutdown(connection, process, 5)
    finally:
        if process.returncode is None:
            await _stop_failed_process(process)


@pytest.mark.asyncio
async def test_memory_rpc_rejects_unencodable_unicode_with_stable_invalid_params(
    tmp_path: Path,
) -> None:
    token = secrets.token_hex(32)
    process, readiness = await _start_runtime(token, tmp_path)
    connection = await _initialize(f"ws://{readiness['host']}:{readiness['port']}", token)
    try:
        requests: tuple[tuple[str, dict[str, Any], str], ...] = (
            (
                "memory.create",
                {
                    "kind": "fact",
                    "scope": {"type": "workspace", "key": "\ud800"},
                    "content": "safe content",
                    "clientRequestId": "safe-request-1",
                },
                "scope",
            ),
            (
                "memory.create",
                {
                    "kind": "fact",
                    "scope": {"type": "global", "key": None},
                    "content": "safe content",
                    "clientRequestId": "\ud800",
                },
                "clientRequestId",
            ),
            ("memory.get", {"memoryId": "\ud800"}, "memoryId"),
        )
        for request_id, (method, params, expected_message) in enumerate(requests, start=2):
            rejected = await _rpc(connection, request_id, method, params)
            assert rejected["error"]["code"] == -32602
            assert expected_message in rejected["error"]["message"]
            assert "codec" not in rejected["error"]["message"]
        await _shutdown(connection, process, 5)
    finally:
        if process.returncode is None:
            await _stop_failed_process(process)


@pytest.mark.asyncio
async def test_provider_model_discovery_rpc_is_ephemeral_and_secret_guarded(
    tmp_path: Path,
) -> None:
    secret = "sk-rpc-discovery-sentinel"
    captured_provider: ProviderConfig | None = None

    async def discover(provider: ProviderConfig) -> tuple[ModelInput, ...]:
        nonlocal captured_provider
        captured_provider = provider
        return (
            ModelInput("deepseek-v4-flash", "DeepSeek V4 Flash"),
            ModelInput("deepseek-v4-pro", "DeepSeek V4 Pro"),
        )

    store = SqliteRuntimeStore(tmp_path / "state.db")
    kernel = RuntimeApplication(
        store,
        _discard_event,
        memory_store=SqliteMemoryStore(tmp_path / "memory.db"),
        model_discovery=discover,
    )
    connection = AckFailingConnection(
        [
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {"protocolVersion": PROTOCOL_VERSION},
            },
            {
                "jsonrpc": "2.0",
                "id": 3,
                "method": "provider.discover_models",
                "params": {"kind": "deepseek", "apiKey": secret},
            },
        ]
    )
    try:
        await handle_connection(
            cast(ServerConnection, connection),
            asyncio.Event(),
            kernel.router,
            EventHub(next_seq=1),
            kernel.security,
            kernel.finish_command,
        )
        assert connection.sent[1] == {
            "jsonrpc": "2.0",
            "id": 3,
            "result": {
                "models": [
                    {
                        "id": "deepseek-v4-flash",
                        "displayName": "DeepSeek V4 Flash",
                        "contextWindow": 32768,
                        "maxOutputTokens": 4096,
                    },
                    {
                        "id": "deepseek-v4-pro",
                        "displayName": "DeepSeek V4 Pro",
                        "contextWindow": 32768,
                        "maxOutputTokens": 4096,
                    },
                ]
            },
        }
        assert captured_provider is not None
        assert captured_provider.id == "deepseek"
        assert captured_provider.api_key == secret
        assert captured_provider.models == ()
        assert not (tmp_path / "config.yaml").exists()
        assert store.latest_sequence() == 0
        assert all(secret.encode() not in content for content in _database_contents(tmp_path))
        assert secret not in json.dumps(connection.sent)
    finally:
        await kernel.close()
        store.close()


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "params",
    [
        {},
        {"kind": "deepseek"},
        {"kind": "custom", "apiKey": "safe-key"},
        {"kind": "deepseek", "apiKey": 7},
        {"kind": "deepseek", "apiKey": "safe-key", "extra": True},
    ],
)
async def test_provider_model_discovery_rejects_invalid_params(
    tmp_path: Path,
    params: dict[str, Any],
) -> None:
    called = False

    async def discover(_provider: ProviderConfig) -> tuple[ModelInput, ...]:
        nonlocal called
        called = True
        return ()

    store = SqliteRuntimeStore(tmp_path / "state.db")
    kernel = RuntimeApplication(
        store,
        _discard_event,
        memory_store=SqliteMemoryStore(tmp_path / "memory.db"),
        model_discovery=discover,
    )
    try:
        with pytest.raises(InvalidParamsError):
            await kernel.providers.discover_provider_models(params)
        assert called is False
        assert not (tmp_path / "config.yaml").exists()
        assert store.latest_sequence() == 0
    finally:
        await kernel.close()
        store.close()


@pytest.mark.asyncio
async def test_jsonrpc_envelope_and_schema_errors_never_echo_credentials(
    tmp_path: Path,
) -> None:
    token = secrets.token_hex(32)
    api_key = "sk-jsonrpc-envelope-sentinel"
    header_secret = "jsonrpc-header-envelope-sentinel"
    proposed_key = "sk-proposed-request-id-sentinel"
    process, readiness = await _start_runtime(token, tmp_path)
    uri = f"ws://{readiness['host']}:{readiness['port']}"
    connection = await _initialize(uri, token)
    observed: list[dict[str, Any]] = []
    stderr = ""
    try:
        configured = await _rpc(
            connection,
            2,
            "provider.configure",
            {
                "kind": "custom",
                "providerId": "envelope-safe",
                "displayName": "Envelope Safe",
                "baseUrl": "http://127.0.0.1:9/v1",
                "apiKey": api_key,
                "headers": {"X-Protected": header_secret},
                "models": [
                    {
                        "id": "safe-model",
                        "displayName": "Safe Model",
                        "contextWindow": 32768,
                        "maxOutputTokens": 4096,
                    }
                ],
            },
        )
        observed.append(configured)

        protected_id = await _raw_rpc(
            connection,
            {"jsonrpc": "2.0", "id": api_key, "method": "thread.list", "params": {}},
        )
        observed.append(protected_id)
        assert protected_id["id"] is None
        assert protected_id["error"] == {
            "code": -32600,
            "message": "request contains protected configuration data",
        }

        object_id = await _raw_rpc(
            connection,
            {
                "jsonrpc": "2.0",
                "id": {api_key: "innocent"},
                "method": "provider.list",
                "params": {},
            },
        )
        observed.append(object_id)
        assert object_id == {
            "jsonrpc": "2.0",
            "id": None,
            "error": {"code": -32600, "message": "invalid request id"},
        }

        array_id = await _raw_rpc(
            connection,
            {
                "jsonrpc": "2.0",
                "id": [header_secret],
                "method": "provider.list",
                "params": {},
            },
        )
        observed.append(array_id)
        assert array_id["id"] is None
        assert array_id["error"]["code"] == -32600

        boolean_id = await _raw_rpc(
            connection,
            {"jsonrpc": "2.0", "id": True, "method": "provider.list", "params": {}},
        )
        observed.append(boolean_id)
        assert boolean_id["id"] is None
        assert boolean_id["error"]["code"] == -32600

        protected_method = await _raw_rpc(
            connection,
            {"jsonrpc": "2.0", "id": 3, "method": header_secret, "params": {}},
        )
        observed.append(protected_method)
        assert protected_method["id"] == 3
        assert protected_method["error"] == {"code": -32601, "message": "method not found"}

        unknown_thread_field = await _raw_rpc(
            connection,
            {
                "jsonrpc": "2.0",
                "id": 4,
                "method": "thread.create",
                "params": {api_key: "value"},
            },
        )
        observed.append(unknown_thread_field)
        assert unknown_thread_field["error"] == {
            "code": -32602,
            "message": "thread.create contains unsupported parameters",
        }

        unknown_replay_field = await _raw_rpc(
            connection,
            {
                "jsonrpc": "2.0",
                "id": 5,
                "method": "event.replay",
                "params": {header_secret: 0},
            },
        )
        observed.append(unknown_replay_field)
        assert unknown_replay_field["error"] == {
            "code": -32602,
            "message": "event.replay contains unsupported parameters",
        }

        rejected_configuration = await _raw_rpc(
            connection,
            {
                "jsonrpc": "2.0",
                "id": proposed_key,
                "method": "provider.configure",
                "params": {
                    "kind": "custom",
                    "providerId": "must-not-persist",
                    "displayName": "Must Not Persist",
                    "baseUrl": "http://127.0.0.1:9/v1",
                    "apiKey": proposed_key,
                    "models": [
                        {
                            "id": "model",
                            "displayName": "Model",
                            "contextWindow": 32768,
                            "maxOutputTokens": 4096,
                        }
                    ],
                },
            },
        )
        observed.append(rejected_configuration)
        assert rejected_configuration["id"] is None
        assert rejected_configuration["error"]["code"] == -32600

        replay = await _rpc(connection, 6, "event.replay", {"afterSeq": 0, "limit": 1000})
        observed.append(replay)
        assert replay["result"]["events"] == []
        await _shutdown(connection, process, 7)
        assert process.stderr is not None
        stderr = (await process.stderr.read()).decode(errors="replace")
    finally:
        if process.returncode is None:
            await _stop_failed_process(process)

    config_source = (tmp_path / "config.yaml").read_text(encoding="utf-8")
    assert api_key in config_source
    assert header_secret in config_source
    assert proposed_key not in config_source
    serialized = json.dumps(observed, ensure_ascii=False)
    for protected in (api_key, header_secret, proposed_key):
        assert protected not in serialized
        assert protected not in stderr
        assert all(protected.encode() not in content for content in _database_contents(tmp_path))


@pytest.mark.asyncio
async def test_jsonrpc_array_params_are_rejected_without_closing_connection(tmp_path: Path) -> None:
    token = secrets.token_hex(32)
    process, readiness = await _start_runtime(token, tmp_path)
    connection = await _initialize(
        f"ws://{readiness['host']}:{readiness['port']}",
        token,
    )
    try:
        await connection.send('{"jsonrpc":"2.0","id":2,"method":"thread.list","params":[[0]]}')
        response = json.loads(await connection.recv())
        assert response == {
            "jsonrpc": "2.0",
            "id": 2,
            "error": {"code": -32602, "message": "params must be an object"},
        }
        valid_response = await _rpc(connection, 3, "thread.list", {})
        assert valid_response["result"]["threads"] == []
        await _shutdown(connection, process, 4)
    finally:
        if process.returncode is None:
            await _stop_failed_process(process)


@pytest.mark.asyncio
async def test_json_decoder_recursion_failure_returns_parse_error_and_keeps_connection(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    rejected_request = {
        "jsonrpc": "2.0",
        "id": 2,
        "method": "thread.list",
        "params": [[0]],
    }
    rejected_source = json.dumps(rejected_request)
    original_loads = json.loads
    decoder_failures = 0

    def controlled_loads(source: str | bytes | bytearray, **kwargs: Any) -> Any:
        nonlocal decoder_failures
        if source == rejected_source:
            decoder_failures += 1
            raise RecursionError("controlled decoder recursion limit")
        return original_loads(source, **kwargs)

    store = SqliteRuntimeStore(tmp_path / "state.db")
    event_bus = EventHub(next_seq=1)
    kernel = RuntimeApplication(
        store,
        event_bus.publish,
        memory_store=SqliteMemoryStore(tmp_path / "memory.db"),
    )
    connection = AckFailingConnection(
        [
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {"protocolVersion": PROTOCOL_VERSION},
            },
            rejected_request,
            {"jsonrpc": "2.0", "id": 3, "method": "thread.list", "params": {}},
        ]
    )
    # Exercise the production codec's RecursionError conversion without depending
    # on a particular CPython JSON decoder's nesting threshold.
    monkeypatch.setattr(json, "loads", controlled_loads)
    try:
        await handle_connection(
            cast(ServerConnection, connection),
            asyncio.Event(),
            kernel.router,
            event_bus,
            kernel.security,
            kernel.finish_command,
        )
        assert decoder_failures == 1
        assert connection.sent[1] == {
            "jsonrpc": "2.0",
            "id": None,
            "error": {"code": -32700, "message": "parse error"},
        }
        assert connection.sent[2]["id"] == 3
        assert connection.sent[2]["result"]["threads"] == []
    finally:
        await kernel.close()
        store.close()


@pytest.mark.asyncio
async def test_unsafe_numeric_jsonrpc_request_returns_parse_error(tmp_path: Path) -> None:
    token = secrets.token_hex(32)
    process, readiness = await _start_runtime(token, tmp_path)
    connection = await _initialize(
        f"ws://{readiness['host']}:{readiness['port']}",
        token,
    )
    try:
        unsafe_numbers = [
            "1" + ("0" * 400),
            "9007199254740993.0",
            "9.007199254740993e15",
        ]
        for unsafe_number in unsafe_numbers:
            await connection.send(
                '{"jsonrpc":"2.0","id":2,"method":"event.replay","params":{"afterSeq":'
                + unsafe_number
                + "}}"
            )
            response = json.loads(await connection.recv())
            assert response == {
                "jsonrpc": "2.0",
                "id": None,
                "error": {"code": -32700, "message": "parse error"},
            }

        await connection.send(
            '{"jsonrpc":"2.0","id":9007199254740993.0,"method":"thread.list","params":{}}'
        )
        response = json.loads(await connection.recv())
        assert response == {
            "jsonrpc": "2.0",
            "id": None,
            "error": {"code": -32700, "message": "parse error"},
        }
        await _shutdown(connection, process, 3)
    finally:
        if process.returncode is None:
            await _stop_failed_process(process)


@pytest.mark.asyncio
async def test_short_and_schema_shaped_credentials_use_value_provenance(
    tmp_path: Path,
) -> None:
    protected_values = (
        "a",
        "jsonrpc",
        "2.0",
        "full_access",
        "process.run",
        "ikaros-runtime",
        "initialize",
        "provider.list",
    )
    config = ConfigStore(tmp_path)
    config.configure_custom(
        provider_id="zzz",
        display_name="ZZZ",
        base_url="http://127.0.0.1:9/v1",
        api_key=None,
        headers={f"X-Protected-{index}": value for index, value in enumerate(protected_values)},
        models=[ModelInput("zzz", "ZZZ")],
    )

    token = secrets.token_hex(32)
    process, readiness = await _start_runtime(token, tmp_path)
    connection = await _initialize(
        f"ws://{readiness['host']}:{readiness['port']}",
        token,
    )
    try:
        providers = await _rpc(connection, 2, "provider.list", {})
        assert providers["result"]["providers"][1]["id"] == "zzz"
        replay = await _rpc(connection, 3, "event.replay", {"afterSeq": 0})
        assert replay["result"]["events"] == []

        request_id = 4
        for protected in protected_values:
            rejected = await _raw_rpc(
                connection,
                {
                    "jsonrpc": "2.0",
                    "id": protected,
                    "method": "provider.list",
                    "params": {},
                },
            )
            assert rejected["id"] is None
            assert rejected["error"]["code"] == -32600
            request_id += 1

        await _shutdown(connection, process, request_id)
    finally:
        if process.returncode is None:
            await _stop_failed_process(process)


@pytest.mark.asyncio
async def test_protocol_constant_shaped_credentials_do_not_block_turn_events(
    tmp_path: Path,
) -> None:
    protected_values = (
        "jsonrpc",
        "2.0",
        "full_access",
        "process.run",
        "ikaros-runtime",
        "threadId",
        "credentialConfigured",
        "newline",
        "nextOffset",
        "verified",
        "lf",
        "crlf",
        "item.delta",
        "thread.renamed",
        "thread.archived",
        "thread.unarchived",
        "archivedAt",
        "changed",
        "assistant",
        "message",
    )
    config = ConfigStore(tmp_path)
    config.configure_custom(
        provider_id="zzz",
        display_name="ZZZ",
        base_url="http://127.0.0.1:9/v1",
        api_key=None,
        headers={f"X-Protected-{index}": value for index, value in enumerate(protected_values)},
        models=[ModelInput("zzz", "ZZZ")],
    )

    token = secrets.token_hex(32)
    process, readiness = await _start_runtime(token, tmp_path)
    connection = await _initialize(
        f"ws://{readiness['host']}:{readiness['port']}",
        token,
    )
    try:
        created = await _rpc(connection, 2, "thread.create", {"title": "Fixed values"})
        thread = created["result"]["thread"]
        renamed = await _rpc(
            connection,
            3,
            "thread.rename",
            {"threadId": thread["id"], "title": "Renamed fixed values"},
            [],
        )
        assert renamed["result"]["event"]["type"] == "thread.renamed"
        archived = await _rpc(
            connection,
            4,
            "thread.archive",
            {"threadId": thread["id"]},
            [],
        )
        assert archived["result"]["event"]["type"] == "thread.archived"
        restored = await _rpc(
            connection,
            5,
            "thread.unarchive",
            {"threadId": thread["id"]},
            [],
        )
        assert restored["result"]["event"]["type"] == "thread.unarchived"
        started = await _rpc(
            connection,
            6,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": "ZZZ",
                "providerId": "scripted",
                "modelId": "scripted-v1",
            },
        )
        events = await _collect_run_events(connection, started["result"]["runId"])
        settled = next(event for event in events if event["type"] == "run.settled")
        assert settled["payload"]["status"] == "completed"

        replay = await _rpc(connection, 7, "event.replay", {"afterSeq": 0, "limit": 1000})
        assert any(
            event["payload"].get("run", {}).get("executionPolicy") == "full_access"
            for event in replay["result"]["events"]
        )
        await _shutdown(connection, process, 8)
    finally:
        if process.returncode is None:
            await _stop_failed_process(process)


@pytest.mark.asyncio
async def test_generated_identifiers_and_timestamps_keep_public_provenance(
    tmp_path: Path,
) -> None:
    token = secrets.token_hex(32)
    process, readiness = await _start_runtime(token, tmp_path)
    connection = await _initialize(
        f"ws://{readiness['host']}:{readiness['port']}",
        token,
    )
    try:
        created = await _rpc(connection, 2, "thread.create", {"title": "Provenance"})
        thread = created["result"]["thread"]
        configured = await _rpc(
            connection,
            3,
            "provider.configure",
            {
                "kind": "custom",
                "providerId": "generated-provenance",
                "displayName": "Generated Provenance",
                "baseUrl": "http://127.0.0.1:9/v1",
                "apiKey": thread["id"],
                "headers": {"X-Timestamp-Provenance": thread["createdAt"]},
                "models": [
                    {
                        "id": "model",
                        "displayName": "Model",
                        "contextWindow": 32768,
                        "maxOutputTokens": 4096,
                    }
                ],
            },
        )
        assert configured["result"]["provider"]["credentialConfigured"] is True

        listed = await _rpc(connection, 4, "thread.list", {})
        replay = await _rpc(connection, 5, "event.replay", {"afterSeq": 0, "limit": 1000})
        assert listed["result"]["threads"][0]["id"] == thread["id"]
        assert replay["result"]["events"][0]["threadId"] == thread["id"]
        assert replay["result"]["events"][0]["timestamp"] == thread["createdAt"]
        await _shutdown(connection, process, 6)
    finally:
        if process.returncode is None:
            await _stop_failed_process(process)


@pytest.mark.asyncio
async def test_credentials_cannot_reenter_public_fields_or_conversation_events(
    tmp_path: Path,
) -> None:
    api_key = "sk-public-boundary-sentinel"
    header_secret = "header-public-boundary-sentinel"
    historical = "historical-content-sentinel"
    token = secrets.token_hex(32)
    process, readiness = await _start_runtime(token, tmp_path)
    uri = f"ws://{readiness['host']}:{readiness['port']}"
    connection = await _initialize(uri, token)
    observed: list[dict[str, Any]] = []
    stderr = ""
    try:
        invalid_public_model = await _rpc(
            connection,
            2,
            "provider.configure",
            {
                "kind": "custom",
                "providerId": "invalid-public",
                "displayName": "Invalid Public",
                "baseUrl": "http://127.0.0.1:9/v1",
                "apiKey": api_key,
                "models": [
                    {
                        "id": api_key,
                        "displayName": "Invalid Model",
                        "contextWindow": 32768,
                        "maxOutputTokens": 4096,
                    }
                ],
            },
        )
        observed.append(invalid_public_model)
        assert invalid_public_model["error"]["code"] == -32602
        assert api_key not in json.dumps(invalid_public_model)
        assert not (tmp_path / "config.yaml").exists()

        historical_thread = await _rpc(
            connection,
            3,
            "thread.create",
            {"title": historical, "clientRequestId": "historical-thread"},
        )
        assert historical_thread["result"]["thread"]["title"] == historical
        historical_collision = await _rpc(
            connection,
            4,
            "provider.configure",
            {
                "kind": "deepseek",
                "apiKey": historical,
                "models": [
                    {
                        "id": "deepseek-chat",
                        "displayName": "DeepSeek Chat",
                        "contextWindow": 32768,
                        "maxOutputTokens": 4096,
                    }
                ],
            },
        )
        assert historical_collision["error"]["code"] == -32602
        assert historical not in json.dumps(historical_collision)
        assert not (tmp_path / "config.yaml").exists()

        configured = await _rpc(
            connection,
            5,
            "provider.configure",
            {
                "kind": "custom",
                "providerId": "credential-safe",
                "displayName": "Credential Safe",
                "baseUrl": "http://127.0.0.1:9/v1",
                "apiKey": api_key,
                "headers": {"X-Protected": header_secret},
                "models": [
                    {
                        "id": "safe-model",
                        "displayName": "Safe Model",
                        "contextWindow": 32768,
                        "maxOutputTokens": 4096,
                    }
                ],
            },
        )
        observed.append(configured)
        assert configured["result"]["provider"]["credentialConfigured"] is True

        providers = await _rpc(connection, 6, "provider.list", {})
        models = await _rpc(connection, 7, "model.list", {})
        observed.extend([providers, models])

        rejected_title = await _rpc(
            connection,
            8,
            "thread.create",
            {"title": f"New {api_key}", "clientRequestId": "protected-title"},
        )
        observed.append(rejected_title)
        assert rejected_title["error"]["code"] == -32602

        safe_thread_response = await _rpc(
            connection,
            9,
            "thread.create",
            {"title": "Safe thread", "clientRequestId": "safe-thread"},
        )
        observed.append(safe_thread_response)
        safe_thread = safe_thread_response["result"]["thread"]
        rejected_turn = await _rpc(
            connection,
            10,
            "turn.start",
            {
                "threadId": safe_thread["id"],
                "branchId": safe_thread["defaultBranchId"],
                "content": f"Do not persist {header_secret}",
                "providerId": "credential-safe",
                "modelId": "safe-model",
                "clientRequestId": "protected-turn",
            },
        )
        observed.append(rejected_turn)
        assert rejected_turn["error"]["code"] == -32602

        replay = await _rpc(connection, 11, "event.replay", {"afterSeq": 0, "limit": 1000})
        observed.append(replay)
        await _shutdown(connection, process, 12)
        assert process.stderr is not None
        stderr = (await process.stderr.read()).decode(errors="replace")
    finally:
        if process.returncode is None:
            await _stop_failed_process(process)

    config_source = (tmp_path / "config.yaml").read_text(encoding="utf-8")
    serialized_outputs = json.dumps(observed, ensure_ascii=False)
    for protected in (api_key, header_secret):
        assert protected in config_source
        assert protected not in serialized_outputs
        assert protected not in stderr
        assert all(protected.encode() not in content for content in _database_contents(tmp_path))


@pytest.mark.asyncio
async def test_credential_rotation_cannot_commit_an_old_secret_as_public_data(
    tmp_path: Path,
) -> None:
    old_secret = "sk-old-rpc-rotation-sentinel"
    new_secret = "sk-new-rpc-rotation-sentinel"
    token = secrets.token_hex(32)
    process, readiness = await _start_runtime(token, tmp_path)
    connection = await _initialize(
        f"ws://{readiness['host']}:{readiness['port']}",
        token,
    )
    observed: list[dict[str, Any]] = []
    stderr = ""
    try:
        configured = await _rpc(
            connection,
            2,
            "provider.configure",
            {
                "kind": "custom",
                "providerId": "rotation-safe",
                "displayName": "Rotation Safe",
                "baseUrl": "http://127.0.0.1:9/v1",
                "apiKey": old_secret,
                "models": [
                    {
                        "id": "model",
                        "displayName": "Model",
                        "contextWindow": 32768,
                        "maxOutputTokens": 4096,
                    }
                ],
            },
        )
        observed.append(configured)

        rejected_rotation = await _rpc(
            connection,
            3,
            "provider.configure",
            {
                "kind": "custom",
                "providerId": "rotation-safe",
                "displayName": old_secret,
                "baseUrl": "http://127.0.0.1:9/v1",
                "apiKey": new_secret,
                "models": [
                    {
                        "id": "model",
                        "displayName": "Model",
                        "contextWindow": 32768,
                        "maxOutputTokens": 4096,
                    }
                ],
            },
        )
        observed.append(rejected_rotation)
        assert rejected_rotation["error"]["code"] == -32602

        providers = await _rpc(connection, 4, "provider.list", {})
        observed.append(providers)
        custom = next(
            provider
            for provider in providers["result"]["providers"]
            if provider["id"] == "rotation-safe"
        )
        assert custom["displayName"] == "Rotation Safe"
        await _shutdown(connection, process, 5)
        assert process.stderr is not None
        stderr = (await process.stderr.read()).decode(errors="replace")
    finally:
        if process.returncode is None:
            await _stop_failed_process(process)

    persisted = ConfigStore(tmp_path).get_provider("rotation-safe")
    assert persisted is not None
    assert persisted.display_name == "Rotation Safe"
    assert persisted.api_key == old_secret
    config_source = (tmp_path / "config.yaml").read_text(encoding="utf-8")
    assert new_secret not in config_source
    serialized = json.dumps(observed, ensure_ascii=False)
    for protected in (old_secret, new_secret):
        assert protected not in serialized
        assert protected not in stderr
        assert all(protected.encode() not in content for content in _database_contents(tmp_path))


@pytest.mark.asyncio
async def test_openai_compatible_fake_endpoint_runs_multiturn_and_process_tool(
    tmp_path: Path,
) -> None:
    endpoint = FakeOpenAIEndpoint()
    base_url = await endpoint.start()
    token = secrets.token_hex(32)
    process, readiness = await _start_runtime(token, tmp_path)
    uri = f"ws://{readiness['host']}:{readiness['port']}"
    connection = await _initialize(uri, token)
    try:
        configured = await _rpc(
            connection,
            2,
            "provider.configure",
            {
                "kind": "custom",
                "providerId": "fake",
                "displayName": "Fake Provider",
                "baseUrl": base_url,
                "models": [
                    {
                        "id": "fake-model",
                        "displayName": "Fake Model",
                        "contextWindow": 32768,
                        "maxOutputTokens": 4096,
                    }
                ],
            },
        )
        assert configured["result"]["provider"]["configured"] is True
        created = await _rpc(
            connection,
            3,
            "thread.create",
            {"title": "Fake Provider E2E", "clientRequestId": "fake-thread"},
        )
        thread = created["result"]["thread"]

        first = await _rpc(
            connection,
            4,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": "first turn",
                "providerId": "fake",
                "modelId": "fake-model",
                "clientRequestId": "fake-turn-1",
            },
        )
        first_events = await _collect_run_events(connection, first["result"]["runId"])
        assert first_events[-1]["payload"]["status"] == "completed"
        assert any(
            event["type"] == "item.completed"
            and event["payload"].get("item", {}).get("content") == "First fake response."
            for event in first_events
        )

        second = await _rpc(
            connection,
            5,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": "second turn",
                "providerId": "fake",
                "modelId": "fake-model",
                "clientRequestId": "fake-turn-2",
            },
        )
        second_events = await _collect_run_events(connection, second["result"]["runId"])
        assert second_events[-1]["payload"]["status"] == "completed"

        tool_turn = await _rpc(
            connection,
            6,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": "run a tool",
                "providerId": "fake",
                "modelId": "fake-model",
                "clientRequestId": "fake-turn-3",
            },
        )
        tool_events = await _collect_run_events(connection, tool_turn["result"]["runId"])
        assert tool_events[-1]["payload"]["status"] == "completed"
        completed_items = [
            event["payload"]["item"]
            for event in tool_events
            if event["type"] == "item.completed" and "item" in event["payload"]
        ]
        assert any(item["kind"] == "tool_call" for item in completed_items)
        assert any(
            item["kind"] == "tool_result" and "gate6-tool" in item["content"]
            for item in completed_items
        )
        assert any(
            item["kind"] == "message"
            and item["content"] == "Tool result received by fake provider."
            for item in completed_items
        )

        assert len(endpoint.requests) >= 5
        assert any(
            message["role"] == "assistant" and message.get("content") == "First fake response."
            for message in endpoint.requests[1]["messages"]
        )
        tool_followup_messages = endpoint.requests[3]["messages"]
        assert tool_followup_messages[-2]["reasoning_content"] == "fake tool reasoning"
        assert tool_followup_messages[-2]["tool_calls"][0]["id"] == "call_fake_process"
        assert tool_followup_messages[-1]["role"] == "tool"
        assert json.loads(tool_followup_messages[-1]["content"])["state"] == "running"
        terminal_result = json.loads(endpoint.requests[-1]["messages"][-1]["content"])
        assert terminal_result["state"] == "exited" and terminal_result["exitCode"] == 0
        assert "gate6-tool" in terminal_result["output"]

        await _shutdown(connection, process, 7)
    finally:
        if process.returncode is None:
            await _stop_failed_process(process)
        await endpoint.close()


@pytest.mark.asyncio
async def test_openai_fake_endpoint_usage_reaches_journal_and_usage_read(
    tmp_path: Path,
) -> None:
    endpoint = UsageOpenAIEndpoint()
    base_url = await endpoint.start()
    token = secrets.token_hex(32)
    process, readiness = await _start_runtime(token, tmp_path)
    uri = f"ws://{readiness['host']}:{readiness['port']}"
    connection = await _initialize(uri, token)
    try:
        configured = await _rpc(
            connection,
            2,
            "provider.configure",
            {
                "kind": "custom",
                "providerId": "usage-fake",
                "displayName": "Usage Fake Provider",
                "baseUrl": base_url,
                "models": [
                    {
                        "id": "usage-model",
                        "displayName": "Usage Model",
                        "contextWindow": 32768,
                        "maxOutputTokens": 4096,
                    }
                ],
            },
        )
        assert configured["result"]["provider"]["configured"] is True
        created = await _rpc(
            connection,
            3,
            "thread.create",
            {"title": "Usage E2E", "clientRequestId": "usage-e2e-thread"},
        )
        thread = created["result"]["thread"]
        started = await _rpc(
            connection,
            4,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": "record exact usage",
                "providerId": "usage-fake",
                "modelId": "usage-model",
                "clientRequestId": "usage-e2e-turn",
            },
        )
        events = await _collect_run_events(connection, started["result"]["runId"])

        usage_events = [
            event
            for event in events
            if event["type"] == "model.response_finished" and event["payload"]["usage"] is not None
        ]
        assert len(usage_events) == 1
        assert usage_events[0]["payload"]["stepOrdinal"] == 1
        assert usage_events[0]["payload"]["outcome"] == "completed"
        assert usage_events[0]["payload"]["usage"] == {
            "inputTokens": 7,
            "cachedInputTokens": None,
            "outputTokens": 5,
            "reasoningOutputTokens": None,
            "totalTokens": 12,
        }
        assert events.index(usage_events[0]) < len(events) - 1
        assert events[-1]["type"] == "run.settled"
        assert events[-1]["payload"]["status"] == "completed"

        usage = await _rpc(connection, 5, "usage.read", {})
        summary = usage["result"]["summary"]
        assert summary["lifetimeTokens"] == 12
        assert summary["peakDailyTokens"] == 12
        assert isinstance(summary["longestRunningTurnSec"], int)
        assert summary["longestRunningTurnSec"] >= 0
        assert summary["currentStreakDays"] == 1
        assert summary["longestStreakDays"] == 1
        assert len(usage["result"]["dailyUsageBuckets"]) == 1
        assert usage["result"]["dailyUsageBuckets"][0]["tokens"] == 12
        assert len(endpoint.requests) == 1

        await _shutdown(connection, process, 6)
    finally:
        if process.returncode is None:
            await _stop_failed_process(process)
        await endpoint.close()


@pytest.mark.asyncio
async def test_openai_fake_endpoint_runs_the_production_file_tool_chain(
    tmp_path: Path,
) -> None:
    workspace = tmp_path / "workspace"
    workspace.mkdir()
    endpoint = FileToolChainEndpoint(workspace)
    base_url = await endpoint.start()
    token = secrets.token_hex(32)
    process, readiness = await _start_runtime(token, tmp_path / "runtime-home")
    uri = f"ws://{readiness['host']}:{readiness['port']}"
    connection = await _initialize(uri, token)
    run_id = ""
    try:
        configured = await _rpc(
            connection,
            2,
            "provider.configure",
            {
                "kind": "custom",
                "providerId": "file-tools-fake",
                "displayName": "File Tools Fake",
                "baseUrl": base_url,
                "models": [
                    {
                        "id": "fake-model",
                        "displayName": "Fake Model",
                        "contextWindow": 32768,
                        "maxOutputTokens": 4096,
                    }
                ],
            },
        )
        assert configured["result"]["provider"]["configured"] is True
        created = await _rpc(
            connection,
            3,
            "thread.create",
            {
                "title": "File Tools E2E",
                "workspace": {
                    "id": "file-tools-workspace",
                    "name": "File Tools Workspace",
                    "rootUri": str(workspace),
                },
            },
        )
        thread = created["result"]["thread"]
        started = await _rpc(
            connection,
            4,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": "update and verify the workspace file",
                "providerId": "file-tools-fake",
                "modelId": "fake-model",
            },
        )
        run_id = started["result"]["runId"]
        events = await _collect_run_events(connection, run_id)
        assert events[-1]["payload"]["status"] == "completed"
        assert workspace.joinpath("chain.txt").read_text(encoding="utf-8") == "omega token"

        completed = [
            event["payload"]["item"]
            for event in events
            if event["type"] == "item.completed" and "item" in event["payload"]
        ]
        narrated = next(
            item
            for item in completed
            if item["kind"] == "message"
            and item.get("role") == "assistant"
            and item["content"] == "I will update and verify the file."
        )
        tool_calls = [item for item in completed if item["kind"] == "tool_call"]
        tool_results = [item for item in completed if item["kind"] == "tool_result"]
        expected_calls = [
            ("call_write", "write"),
            ("call_read_before", "read"),
            ("call_edit", "edit"),
            ("call_read_after", "read"),
        ]
        assert len(tool_calls) == 4
        assert [
            (item["data"]["callId"], item["data"]["toolName"]) for item in tool_calls
        ] == expected_calls
        assert [item["status"] for item in tool_calls] == ["completed"] * 4

        assert len(tool_results) == 4
        assert [
            (item["data"]["callId"], item["data"]["toolName"]) for item in tool_results
        ] == expected_calls
        assert [
            (
                item["data"]["result"]["toolCallId"],
                item["data"]["result"]["toolName"],
            )
            for item in tool_results
        ] == expected_calls
        assert [item["data"]["toolCallItemId"] for item in tool_results] == [
            item["id"] for item in tool_calls
        ]
        assert [item["data"]["result"]["ok"] for item in tool_results] == [True] * 4

        read_results = [
            item["data"]["result"] for item in tool_results if item["data"]["toolName"] == "read"
        ]
        assert len(read_results) == 2
        assert "alpha token" in read_results[0]["output"]
        assert "omega token" in read_results[1]["output"]
        assert narrated["data"]["stepId"] == tool_calls[0]["data"]["stepId"]
        assert any(
            item["kind"] == "message"
            and item.get("role") == "assistant"
            and item["content"] == "File tool chain completed with omega token."
            for item in completed
        )

        assert len(endpoint.requests) == 5
        expected_provider_history = [expected_calls[:index] for index in range(5)]
        assert [
            [
                (
                    message["tool_call_id"],
                    json.loads(message["content"])["toolName"],
                )
                for message in request["messages"]
                if message["role"] == "tool"
            ]
            for request in endpoint.requests
        ] == expected_provider_history
        assert [
            [
                (call["id"], call["function"]["name"])
                for message in request["messages"]
                if message["role"] == "assistant"
                for call in message.get("tool_calls", [])
            ]
            for request in endpoint.requests
        ] == expected_provider_history

        first_read_for_provider = json.loads(endpoint.requests[2]["messages"][-1]["content"])
        second_read_for_provider = json.loads(endpoint.requests[4]["messages"][-1]["content"])
        assert endpoint.requests[2]["messages"][-1]["tool_call_id"] == "call_read_before"
        assert "alpha token" in first_read_for_provider["output"]
        assert endpoint.requests[4]["messages"][-1]["tool_call_id"] == "call_read_after"
        assert "omega token" in second_read_for_provider["output"]

        expected_schemas = {
            "process_start": {"command"},
            "process_read": {"processId"},
            "process_wait": {"processId"},
                "process_stop": {"processId"},
                "history_read": {"itemId"},
                "read": {"filePath"},
            "write": {"filePath", "content"},
            "edit": {"filePath", "oldString", "newString"},
        }
        for request in endpoint.requests:
            definitions = {
                tool["function"]["name"]: tool["function"]["parameters"]
                for tool in request["tools"]
            }
            assert set(definitions) == set(expected_schemas)
            for name, required in expected_schemas.items():
                assert set(definitions[name]["required"]) == required

        followup_messages = endpoint.requests[1]["messages"]
        narrated_message = next(
            message
            for message in followup_messages
            if message["role"] == "assistant"
            and message.get("content") == "I will update and verify the file."
        )
        assert narrated_message["tool_calls"][0]["id"] == "call_write"
        assert followup_messages[-1]["role"] == "tool"

        replay = await _rpc(connection, 5, "event.replay", {"afterSeq": 0, "limit": 1000})
        replayed_run = [event for event in replay["result"]["events"] if event["runId"] == run_id]
        assert [event["seq"] for event in replayed_run] == [
            event["seq"] for event in events if event["runId"] == run_id
        ]
        await _shutdown(connection, process, 6)
    finally:
        if process.returncode is None:
            await _stop_failed_process(process)
        await endpoint.close()


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "invalid_number",
    ["1e9999", "1" + ("0" * 400)],
    ids=["non-finite", "unsafe-integer"],
)
async def test_invalid_provider_tool_numbers_fail_before_agent_or_sqlite(
    tmp_path: Path,
    invalid_number: str,
) -> None:
    marker = tmp_path / "tool-must-not-run.txt"
    if os.name == "nt":
        escaped_marker = str(marker).replace("'", "''")
        command = f"[IO.File]::WriteAllText('{escaped_marker}', 'ran')"
    else:
        command = f"printf ran > {shlex.quote(str(marker))}"
    endpoint = InvalidNumericToolArgumentsEndpoint(command, invalid_number)
    base_url = await endpoint.start()
    token = secrets.token_hex(32)
    process, readiness = await _start_runtime(token, tmp_path)
    uri = f"ws://{readiness['host']}:{readiness['port']}"
    connection = await _initialize(uri, token)
    try:
        await _rpc(
            connection,
            2,
            "provider.configure",
            {
                "kind": "custom",
                "providerId": "non-finite",
                "displayName": "Non-finite Provider",
                "baseUrl": base_url,
                "models": [
                    {
                        "id": "non-finite-model",
                        "displayName": "Non-finite Model",
                        "contextWindow": 32768,
                        "maxOutputTokens": 4096,
                    }
                ],
            },
        )
        created = await _rpc(
            connection,
            3,
            "thread.create",
            {"title": "Strict JSON", "clientRequestId": "strict-json-thread"},
        )
        thread = created["result"]["thread"]
        started = await _rpc(
            connection,
            4,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": "try invalid arguments",
                "providerId": "non-finite",
                "modelId": "non-finite-model",
                "clientRequestId": "strict-json-turn",
            },
        )
        events = await _collect_run_events(connection, started["result"]["runId"])

        assert events[-1]["payload"]["status"] == "failed"
        assert not any(
            event["payload"].get("item", {}).get("kind") == "tool_call" for event in events
        )
        assert not marker.exists()
        assert all(
            invalid_number.encode() not in content for content in _database_contents(tmp_path)
        )
        assert all(b"Infinity" not in content for content in _database_contents(tmp_path))
        assert all(b"NaN" not in content for content in _database_contents(tmp_path))

        await _shutdown(connection, process, 5)
    finally:
        if process.returncode is None:
            await _stop_failed_process(process)
        await endpoint.close()


@pytest.mark.asyncio
async def test_process_tool_cannot_publish_or_persist_provider_credentials(
    tmp_path: Path,
) -> None:
    api_key = "sk-tool-output-sentinel"
    header_secret = "tool-header-output-sentinel"
    endpoint = ProtectedToolEndpoint(_read_file_command(tmp_path / "config.yaml"))
    base_url = await endpoint.start()
    token = secrets.token_hex(32)
    process, readiness = await _start_runtime(token, tmp_path)
    uri = f"ws://{readiness['host']}:{readiness['port']}"
    connection = await _initialize(uri, token)
    live_events: list[dict[str, Any]] = []
    replay: dict[str, Any] = {}
    stderr = ""
    try:
        configured = await _rpc(
            connection,
            2,
            "provider.configure",
            {
                "kind": "custom",
                "providerId": "protected-tool",
                "displayName": "Protected Tool",
                "baseUrl": base_url,
                "apiKey": api_key,
                "headers": {"X-Protected": header_secret},
                "models": [
                    {
                        "id": "fake-model",
                        "displayName": "Fake Model",
                        "contextWindow": 32768,
                        "maxOutputTokens": 4096,
                    }
                ],
            },
        )
        assert configured["result"]["provider"]["credentialConfigured"] is True
        created = await _rpc(
            connection,
            3,
            "thread.create",
            {"title": "Protected output", "clientRequestId": "protected-thread"},
        )
        thread = created["result"]["thread"]
        started = await _rpc(
            connection,
            4,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": "read protected config",
                "providerId": "protected-tool",
                "modelId": "fake-model",
                "clientRequestId": "protected-turn",
            },
        )
        live_events = await _collect_run_events(connection, started["result"]["runId"])
        assert live_events[-1]["payload"]["status"] == "completed"

        result_items = [
            event["payload"]["item"]
            for event in live_events
            if event["type"] == "item.completed"
            and event["payload"].get("item", {}).get("kind") == "tool_result"
        ]
        assert len(result_items) >= 2
        result_item = result_items[-1]
        assert result_item["status"] == "failed"
        result = result_item["data"]["result"]
        assert result["ok"] is False
        assert result["errorCode"] == "protected_output"
        assert json.loads(result_item["content"]) == result

        assert len(endpoint.requests) >= 3
        model_tool_result = endpoint.requests[-1]["messages"][-1]
        assert model_tool_result["role"] == "tool"
        assert json.loads(model_tool_result["content"])["errorCode"] == "protected_output"

        live_database_files = [
            tmp_path / "state.db",
            tmp_path / "state.db-wal",
            tmp_path / "state.db-shm",
        ]
        assert all(path.is_file() for path in live_database_files)
        for protected in (api_key, header_secret):
            assert all(protected.encode() not in path.read_bytes() for path in live_database_files)

        replay = await _rpc(connection, 5, "event.replay", {"afterSeq": 0, "limit": 1000})
        await _shutdown(connection, process, 6)
        assert process.stderr is not None
        stderr = (await process.stderr.read()).decode(errors="replace")
    finally:
        if process.returncode is None:
            await _stop_failed_process(process)
        await endpoint.close()

    config_source = (tmp_path / "config.yaml").read_text(encoding="utf-8")
    serialized_outputs = json.dumps(
        {
            "live": live_events,
            "replay": replay,
            "providerRequests": endpoint.requests,
        },
        ensure_ascii=False,
    )
    for protected in (api_key, header_secret):
        assert protected in config_source
        assert protected not in serialized_outputs
        assert protected not in stderr
        assert all(protected.encode() not in content for content in _database_contents(tmp_path))


@pytest.mark.asyncio
async def test_agent_boundary_blocks_split_credentials_from_another_provider(
    tmp_path: Path,
) -> None:
    protected = "sk-other-provider-secret-sentinel"
    endpoint = SplitProtectedEndpoint(protected)
    base_url = await endpoint.start()
    token = secrets.token_hex(32)
    process, readiness = await _start_runtime(token, tmp_path)
    uri = f"ws://{readiness['host']}:{readiness['port']}"
    connection = await _initialize(uri, token)
    live_events: list[dict[str, Any]] = []
    replay: dict[str, Any] = {}
    stderr = ""
    try:
        deepseek = await _rpc(
            connection,
            2,
            "provider.configure",
            {
                "kind": "deepseek",
                "apiKey": protected,
                "models": [
                    {
                        "id": "deepseek-chat",
                        "displayName": "DeepSeek Chat",
                        "contextWindow": 32768,
                        "maxOutputTokens": 4096,
                    }
                ],
            },
        )
        assert deepseek["result"]["provider"]["credentialConfigured"] is True
        custom = await _rpc(
            connection,
            3,
            "provider.configure",
            {
                "kind": "custom",
                "providerId": "split-echo",
                "displayName": "Split Echo",
                "baseUrl": base_url,
                "models": [
                    {
                        "id": "fake-model",
                        "displayName": "Fake Model",
                        "contextWindow": 32768,
                        "maxOutputTokens": 4096,
                    }
                ],
            },
        )
        assert custom["result"]["provider"]["credentialConfigured"] is False
        created = await _rpc(
            connection,
            4,
            "thread.create",
            {"title": "Cross-provider boundary", "clientRequestId": "cross-provider-thread"},
        )
        thread = created["result"]["thread"]
        started = await _rpc(
            connection,
            5,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": "echo a protected value",
                "providerId": "split-echo",
                "modelId": "fake-model",
                "clientRequestId": "cross-provider-turn",
            },
        )
        live_events = await _collect_run_events(connection, started["result"]["runId"])
        assert live_events[-1]["payload"]["status"] == "failed"
        replay = await _rpc(connection, 6, "event.replay", {"afterSeq": 0, "limit": 1000})
        await _shutdown(connection, process, 7)
        assert process.stderr is not None
        stderr = (await process.stderr.read()).decode(errors="replace")
    finally:
        if process.returncode is None:
            await _stop_failed_process(process)
        await endpoint.close()

    config_source = (tmp_path / "config.yaml").read_text(encoding="utf-8")
    serialized_outputs = json.dumps(
        {
            "live": live_events,
            "replay": replay,
            "providerRequests": endpoint.requests,
        },
        ensure_ascii=False,
    )
    assert protected in config_source
    assert protected not in serialized_outputs
    assert protected not in stderr
    assert all(protected.encode() not in content for content in _database_contents(tmp_path))


@pytest.mark.asyncio
async def test_active_run_blocks_cross_provider_credential_changes(
    tmp_path: Path,
) -> None:
    protected = "sk-mid-stream-provider-secret-sentinel"
    endpoint = DelayedProtectedEndpoint("Safe delayed response.")
    base_url = await endpoint.start()
    token = secrets.token_hex(32)
    process, readiness = await _start_runtime(token, tmp_path)
    uri = f"ws://{readiness['host']}:{readiness['port']}"
    connection = await _initialize(uri, token)
    live_events: list[dict[str, Any]] = []
    replay: dict[str, Any] = {}
    stderr = ""
    try:
        custom = await _rpc(
            connection,
            2,
            "provider.configure",
            {
                "kind": "custom",
                "providerId": "delayed-echo",
                "displayName": "Delayed Echo",
                "baseUrl": base_url,
                "models": [
                    {
                        "id": "fake-model",
                        "displayName": "Fake Model",
                        "contextWindow": 32768,
                        "maxOutputTokens": 4096,
                    }
                ],
            },
        )
        assert custom["result"]["provider"]["credentialConfigured"] is False
        created = await _rpc(
            connection,
            3,
            "thread.create",
            {"title": "Mid-stream boundary", "clientRequestId": "mid-stream-thread"},
        )
        thread = created["result"]["thread"]
        started = await _rpc(
            connection,
            4,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": "wait before echoing",
                "providerId": "delayed-echo",
                "modelId": "fake-model",
                "clientRequestId": "mid-stream-turn",
            },
        )
        await asyncio.wait_for(endpoint.request_received.wait(), timeout=10)
        deepseek = await _rpc(
            connection,
            5,
            "provider.configure",
            {
                "kind": "deepseek",
                "apiKey": protected,
                "models": [
                    {
                        "id": "deepseek-chat",
                        "displayName": "DeepSeek Chat",
                        "contextWindow": 32768,
                        "maxOutputTokens": 4096,
                    }
                ],
            },
            live_events,
        )
        assert deepseek["error"]["code"] == -32602
        assert "Run is active" in deepseek["error"]["message"]
        limits = await _rpc(
            connection,
            8,
            "model.set_limits",
            {
                "providerId": "delayed-echo",
                "modelId": "fake-model",
                "contextWindow": 65536,
                "maxOutputTokens": 8192,
            },
            live_events,
        )
        assert limits["result"]["model"]["maxOutputTokens"] == 8192
        assert endpoint.requests[0]["max_tokens"] == 4096
        endpoint.release_response.set()
        live_events.extend(await _collect_run_events(connection, started["result"]["runId"]))
        assert live_events[-1]["payload"]["status"] == "completed"
        replay = await _rpc(connection, 6, "event.replay", {"afterSeq": 0, "limit": 1000})
        await _shutdown(connection, process, 7)
        assert process.stderr is not None
        stderr = (await process.stderr.read()).decode(errors="replace")
    finally:
        endpoint.release_response.set()
        if process.returncode is None:
            await _stop_failed_process(process)
        await endpoint.close()

    config_source = (tmp_path / "config.yaml").read_text(encoding="utf-8")
    serialized_outputs = json.dumps(
        {
            "live": live_events,
            "replay": replay,
            "providerRequests": endpoint.requests,
        },
        ensure_ascii=False,
    )
    assert protected not in config_source
    assert protected not in serialized_outputs
    assert protected not in stderr
    assert all(protected.encode() not in content for content in _database_contents(tmp_path))


def test_active_run_blocks_provider_mutation(tmp_path: Path) -> None:
    config = ConfigStore(tmp_path)
    config.configure_custom(
        provider_id="local",
        display_name="Local",
        base_url="http://127.0.0.1:8080/v1",
        api_key=None,
        headers=None,
        models=[ModelInput("model", "Model")],
    )
    config.configure_custom(
        provider_id="other",
        display_name="Other",
        base_url="http://127.0.0.1:8081/v1",
        api_key="sk-other-active-run-sentinel",
        headers=None,
        models=[ModelInput("other-model", "Other Model")],
    )
    store = SqliteRuntimeStore(tmp_path / "state.db")
    thread, _event = store.create_thread("Active provider")
    prepared = prepare_turn(
        store,
        thread_id=thread.id,
        branch_id=thread.default_branch_id,
        content="hold configuration",
        provider_id="local",
        model_id="model",
    )
    memory_store = SqliteMemoryStore(tmp_path / "memory.db")
    kernel = RuntimeApplication(
        store,
        _discard_event,
        memory_store=memory_store,
        config_store=config,
    )
    try:
        with pytest.raises(InvalidParamsError, match="Run is active"):
            kernel.providers.remove_provider({"providerId": "local"})
        with pytest.raises(InvalidParamsError, match="Run is active"):
            kernel.providers.remove_provider({"providerId": "other"})
        with pytest.raises(InvalidParamsError, match="Run is active"):
            kernel.providers.configure_provider(
                {
                    "kind": "deepseek",
                    "apiKey": "full_access",
                    "models": [
                        {
                            "id": "model",
                            "displayName": "Model",
                            "contextWindow": 32768,
                            "maxOutputTokens": 4096,
                        }
                    ],
                }
            )
        store.terminalize_run(prepared.run_id, "cancelled")
        configured = kernel.providers.configure_provider(
            {
                "kind": "deepseek",
                "apiKey": "full_access",
                "models": [
                    {
                        "id": "model",
                        "displayName": "Model",
                        "contextWindow": 32768,
                        "maxOutputTokens": 4096,
                    }
                ],
            }
        )
        provider = cast(dict[str, object], configured["provider"])
        assert provider["configured"] is True
        assert kernel.providers.remove_provider({"providerId": "local"})["removed"] is True
    finally:
        memory_store.close()
        store.close()


def test_idempotent_turn_retry_survives_provider_removal(tmp_path: Path) -> None:
    config = ConfigStore(tmp_path)
    config.configure_custom(
        provider_id="local",
        display_name="Local",
        base_url="http://127.0.0.1:9/v1",
        api_key=None,
        headers=None,
        models=[ModelInput("model", "Model")],
    )
    store = SqliteRuntimeStore(tmp_path / "state.db")
    thread, _ = store.create_thread("Retry after removal")
    prepared = prepare_turn(
        store,
        thread_id=thread.id,
        branch_id=thread.default_branch_id,
        content="stable content",
        provider_id="local",
        model_id="model",
        client_request_id="stable-turn-request",
    )
    store.terminalize_run(prepared.run_id, "cancelled")
    memory_store = SqliteMemoryStore(tmp_path / "memory.db")
    kernel = RuntimeApplication(
        store,
        _discard_event,
        memory_store=memory_store,
        config_store=config,
    )
    params: dict[str, object] = {
        "threadId": thread.id,
        "branchId": thread.default_branch_id,
        "content": "stable content",
        "providerId": "local",
        "modelId": "model",
        "clientRequestId": "stable-turn-request",
    }
    try:
        assert kernel.providers.remove_provider({"providerId": "local"})["removed"] is True

        repeated = kernel.turns.start_turn(params)

        assert repeated.result == {
            "turnId": prepared.turn_id,
            "runId": prepared.run_id,
            "threadId": thread.id,
            "branchId": thread.default_branch_id,
        }
        assert repeated.events_after_ack == ()
        assert repeated.run_after_ack is None
        with pytest.raises(InvalidParamsError, match="clientRequestId"):
            kernel.turns.start_turn({**params, "content": "different content"})
        with pytest.raises(InvalidParamsError, match="fields"):
            kernel.turns.start_turn({**params, "executionLimits": {"maxModelCalls": 100}})
        assert store._connection.execute("SELECT COUNT(*) FROM runs").fetchone()[0] == 1
    finally:
        memory_store.close()
        store.close()


def test_kernel_rejects_config_credentials_already_present_in_the_journal(
    tmp_path: Path,
) -> None:
    protected = "manual-config-journal-sentinel"
    store = SqliteRuntimeStore(tmp_path / "state.db")
    thread, _ = store.create_thread(protected)
    store.rename_thread(thread.id, "Current safe title")
    assert store.list_thread_page(cursor=None, limit=50).threads[0].title == "Current safe title"
    config = ConfigStore(tmp_path)
    config.configure_deepseek(api_key=protected, models=[ModelInput("model", "Model")])

    memory_store = SqliteMemoryStore(tmp_path / "memory.db")
    try:
        with pytest.raises(ConfigError, match="conflict") as captured:
            RuntimeApplication(
                store,
                _discard_event,
                memory_store=memory_store,
                config_store=config,
            )
        assert protected not in str(captured.value)
    finally:
        memory_store.close()
        store.close()


def test_online_provider_configuration_rejects_credentials_in_historical_events(
    tmp_path: Path,
) -> None:
    protected = "online-historical-journal-sentinel"
    store = SqliteRuntimeStore(tmp_path / "state.db")
    thread, _ = store.create_thread(protected)
    store.rename_thread(thread.id, "Current safe title")
    config = ConfigStore(tmp_path)
    memory_store = SqliteMemoryStore(tmp_path / "memory.db")
    kernel = RuntimeApplication(
        store,
        _discard_event,
        memory_store=memory_store,
        config_store=config,
    )
    before = config.path.read_bytes() if config.path.exists() else None

    try:
        with pytest.raises(InvalidParamsError, match="credentials conflict") as captured:
            kernel.providers.configure_provider(
                {
                    "kind": "deepseek",
                    "apiKey": protected,
                    "models": [
                        {
                            "id": "model",
                            "displayName": "Model",
                            "contextWindow": 32768,
                            "maxOutputTokens": 4096,
                        }
                    ],
                }
            )
        assert protected not in str(captured.value)
        assert (config.path.read_bytes() if config.path.exists() else None) == before
        replayed, _ = store.replay_events(0, 100)
        assert any(
            event.type == "thread.created" and event.payload["thread"]["title"] == protected
            for event in replayed
        )
    finally:
        memory_store.close()
        store.close()


def test_queued_run_config_preserves_exact_user_content_across_restart_and_rebuild(
    tmp_path: Path,
) -> None:
    content = " \r\nuser-body-sentinel\nwith trailing space  "
    database_path = tmp_path / "state.db"
    store = SqliteRuntimeStore(database_path)

    class ReserveOnlyScheduler:
        def reserve(self, _run_id: str) -> None:
            return None

    service = TurnService(
        store,
        cast(AgentScheduler, ReserveOnlyScheduler()),
        ConfigStore(tmp_path),
        lambda _value: None,
        load_identity_core(),
    )
    thread, _ = store.create_thread("Exact queued input")
    outcome = service.start_turn(
        {
            "threadId": thread.id,
            "branchId": thread.default_branch_id,
            "content": content,
            "providerId": "scripted",
            "modelId": "scripted-v1",
        }
    )
    run_id = str(outcome.result["runId"])

    def persisted_input(current: SqliteRuntimeStore) -> tuple[dict[str, object], str, str]:
        row = current._connection.execute(
            "SELECT config_json FROM run_configs WHERE run_id = ?",
            (run_id,),
        ).fetchone()
        assert row is not None
        user = current._connection.execute(
            "SELECT content FROM items WHERE run_id = ? AND role = 'user'",
            (run_id,),
        ).fetchone()
        assert user is not None
        assert (
            current._connection.execute(
                "SELECT COUNT(*) FROM context_revisions WHERE run_id = ?", (run_id,)
            ).fetchone()[0]
            == 0
        )
        return current.get_run_config(run_id).to_wire(), str(row[0]), str(user[0])

    before = persisted_input(store)
    assert before[2] == content
    assert "user-body-sentinel" not in json.dumps(before[:2], ensure_ascii=False)
    store.close()

    reopened = SqliteRuntimeStore(database_path)
    try:
        assert reopened.run_status(run_id) == "queued"
        assert persisted_input(reopened) == before
        reopened.rebuild_projections()
        assert reopened.run_status(run_id) == "queued"
        assert persisted_input(reopened) == before
    finally:
        reopened.close()


@pytest.mark.asyncio
async def test_invalid_config_fails_before_sqlite_and_never_echoes_source(
    tmp_path: Path,
) -> None:
    secret = "sk-invalid-startup-sentinel"
    (tmp_path / "config.yaml").write_text(
        f"version: [\napi_key: {secret}\n",
        encoding="utf-8",
    )
    process = await asyncio.create_subprocess_exec(
        sys.executable,
        "-m",
        "ikaros_runtime",
        "serve",
        "--host",
        "127.0.0.1",
        "--port",
        "0",
        f"--token={secrets.token_hex(32)}",
        "--parent-pid",
        str(os.getpid()),
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
        env={**os.environ, "IKAROS_HOME": str(tmp_path)},
        creationflags=_HIDDEN_PROCESS_FLAGS,
    )

    assert process.stdout is not None
    assert await asyncio.wait_for(process.stdout.readline(), timeout=10) == b""
    assert await asyncio.wait_for(process.wait(), timeout=10) != 0
    assert process.stderr is not None
    stderr = (await process.stderr.read()).decode(errors="replace")
    assert secret not in stderr
    assert not (tmp_path / "state.db").exists()


@pytest.mark.asyncio
async def test_startup_credential_conflict_fails_before_recovery_mutates_state(
    tmp_path: Path,
) -> None:
    protected = "sk-startup-recovery-conflict-sentinel"
    store = SqliteRuntimeStore(tmp_path / "state.db")
    thread, _ = store.create_thread(protected)
    prepared = prepare_turn(
        store,
        thread_id=thread.id,
        branch_id=thread.default_branch_id,
        content="running before rejected startup",
        provider_id="scripted",
        model_id="scripted-v1",
    )
    store.mark_run_running(prepared.run_id)
    store.create_assistant_item(prepared.run_id)
    before_events, before_latest_seq = store.replay_events(0, 1000)
    store.close()

    config = ConfigStore(tmp_path)
    config.configure_deepseek(
        api_key=protected,
        models=[ModelInput("deepseek-chat", "DeepSeek Chat")],
    )

    process = await asyncio.create_subprocess_exec(
        sys.executable,
        "-m",
        "ikaros_runtime",
        "serve",
        "--host",
        "127.0.0.1",
        "--port",
        "0",
        f"--token={secrets.token_hex(32)}",
        "--parent-pid",
        str(os.getpid()),
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
        env={**os.environ, "IKAROS_HOME": str(tmp_path)},
        creationflags=_HIDDEN_PROCESS_FLAGS,
    )

    assert process.stdout is not None
    assert await asyncio.wait_for(process.stdout.readline(), timeout=10) == b""
    assert await asyncio.wait_for(process.wait(), timeout=10) != 0
    assert process.stderr is not None
    stderr = (await process.stderr.read()).decode(errors="replace")
    assert protected not in stderr

    reopened = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        after_events, after_latest_seq = reopened.replay_events(0, 1000)
        assert reopened.run_status(prepared.run_id) == "running"
        assert after_latest_seq == before_latest_seq
        assert after_events == before_events
    finally:
        reopened.close()


@pytest.mark.asyncio
async def test_memory_credential_conflict_fails_before_recovery_mutates_state(
    tmp_path: Path,
) -> None:
    protected = "sk-memory-startup-conflict-sentinel"
    store = SqliteRuntimeStore(tmp_path / "state.db")
    thread, _ = store.create_thread("Memory credential conflict")
    prepared = prepare_turn(
        store,
        thread_id=thread.id,
        branch_id=thread.default_branch_id,
        content="running before Memory conflict",
        provider_id="scripted",
        model_id="scripted-v1",
    )
    store.mark_run_running(prepared.run_id)
    before_events, before_latest_seq = store.replay_events(0, 1000)
    store.close()

    memory_store = SqliteMemoryStore(tmp_path / "memory.db")
    memory_store.create_memory_once(
        kind="fact",
        scope=MemoryScope("global", None),
        content=f"persisted prefix {protected} suffix",
        client_request_id="memory-startup-conflict",
    )
    memory_store.close()
    config = ConfigStore(tmp_path)
    config.configure_deepseek(
        api_key=protected,
        models=[ModelInput("deepseek-chat", "DeepSeek Chat")],
    )

    process = await asyncio.create_subprocess_exec(
        sys.executable,
        "-m",
        "ikaros_runtime",
        "serve",
        "--host",
        "127.0.0.1",
        "--port",
        "0",
        f"--token={secrets.token_hex(32)}",
        "--parent-pid",
        str(os.getpid()),
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
        env={**os.environ, "IKAROS_HOME": str(tmp_path)},
        creationflags=_HIDDEN_PROCESS_FLAGS,
    )

    assert process.stdout is not None
    assert await asyncio.wait_for(process.stdout.readline(), timeout=10) == b""
    assert await asyncio.wait_for(process.wait(), timeout=10) != 0
    assert process.stderr is not None
    stderr = (await process.stderr.read()).decode(errors="replace")
    assert protected not in stderr

    reopened = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        after_events, after_latest_seq = reopened.replay_events(0, 1000)
        assert reopened.run_status(prepared.run_id) == "running"
        assert after_latest_seq == before_latest_seq
        assert after_events == before_events
    finally:
        reopened.close()


@pytest.mark.asyncio
async def test_incompatible_memory_schema_fails_before_recovery_and_is_not_rewritten(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    thread, _ = store.create_thread("Incompatible Memory schema")
    prepared = prepare_turn(
        store,
        thread_id=thread.id,
        branch_id=thread.default_branch_id,
        content="running before rejected Memory schema",
        provider_id="scripted",
        model_id="scripted-v1",
    )
    store.mark_run_running(prepared.run_id)
    before_events, before_latest_seq = store.replay_events(0, 1000)
    store.close()

    memory_path = tmp_path / "memory.db"
    connection = sqlite3.connect(memory_path)
    connection.execute("CREATE TABLE foreign_data(value TEXT)")
    connection.execute("PRAGMA user_version = 99")
    connection.commit()
    connection.close()
    before_memory = memory_path.read_bytes()

    process = await asyncio.create_subprocess_exec(
        sys.executable,
        "-m",
        "ikaros_runtime",
        "serve",
        "--host",
        "127.0.0.1",
        "--port",
        "0",
        f"--token={secrets.token_hex(32)}",
        "--parent-pid",
        str(os.getpid()),
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
        env={**os.environ, "IKAROS_HOME": str(tmp_path)},
        creationflags=_HIDDEN_PROCESS_FLAGS,
    )

    assert process.stdout is not None
    assert await asyncio.wait_for(process.stdout.readline(), timeout=10) == b""
    assert await asyncio.wait_for(process.wait(), timeout=10) != 0
    assert process.stderr is not None
    stderr = (await process.stderr.read()).decode(errors="replace")
    assert "memory database schema is incompatible" in stderr
    assert memory_path.read_bytes() == before_memory
    assert not await asyncio.to_thread(Path(f"{memory_path}-wal").exists)
    assert not await asyncio.to_thread(Path(f"{memory_path}-shm").exists)

    reopened = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        after_events, after_latest_seq = reopened.replay_events(0, 1000)
        assert reopened.run_status(prepared.run_id) == "running"
        assert after_latest_seq == before_latest_seq
        assert after_events == before_events
    finally:
        reopened.close()


@pytest.mark.asyncio
async def test_missing_identity_fails_before_recovery_mutates_state(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    thread, _ = store.create_thread("Identity startup failure")
    prepared = prepare_turn(
        store,
        thread_id=thread.id,
        branch_id=thread.default_branch_id,
        content="running before missing identity",
        provider_id="scripted",
        model_id="scripted-v1",
    )
    store.mark_run_running(prepared.run_id)
    before_events, before_latest_seq = store.replay_events(0, 1000)
    store.close()

    def missing_identity() -> None:
        raise IdentityResourceError("Ikaros identity resource is unavailable")

    monkeypatch.setattr(bootstrap_module, "load_identity_core", missing_identity)
    settings = ServerSettings(
        host="127.0.0.1",
        port=0,
        token=secrets.token_hex(32),
        parent_pid=os.getpid(),
        runtime_home=tmp_path,
    )

    with pytest.raises(IdentityResourceError, match="resource is unavailable"):
        await bootstrap_module.run_runtime_server(settings)

    reopened = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        after_events, after_latest_seq = reopened.replay_events(0, 1000)
        assert reopened.run_status(prepared.run_id) == "running"
        assert after_latest_seq == before_latest_seq
        assert after_events == before_events
    finally:
        reopened.close()


@pytest.mark.asyncio
async def test_event_bus_releases_notifications_in_sequence_order() -> None:
    observed: list[int] = []
    event_bus = EventHub(next_seq=5)
    event_bus.subscribe(lambda event: observed.append(event.seq))

    await event_bus.publish(_journal_event(6))
    assert observed == []

    await event_bus.publish(_journal_event(5))
    assert observed == [5, 6]


@pytest.mark.asyncio
async def test_event_bus_drops_a_slow_sink_without_blocking_other_subscribers() -> None:
    observed: list[int] = []
    event_bus = EventHub(next_seq=1)

    def full_sink(_event: JournalEvent) -> None:
        raise asyncio.QueueFull

    event_bus.subscribe(full_sink)
    event_bus.subscribe(lambda event: observed.append(event.seq))

    await event_bus.publish(_journal_event(1))
    await event_bus.publish(_journal_event(2))

    assert observed == [1, 2]


@pytest.mark.asyncio
@pytest.mark.parametrize("credential_position", ["key", "value"])
async def test_response_guard_replaces_an_unexpected_protected_payload(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    credential_position: str,
) -> None:
    protected = "sk-unexpected-response-sentinel"
    config = ConfigStore(tmp_path)
    config.configure_deepseek(
        api_key=protected,
        models=[ModelInput("deepseek-chat", "DeepSeek Chat")],
    )
    store = SqliteRuntimeStore(tmp_path / "state.db")
    event_bus = EventHub(next_seq=1)
    kernel = RuntimeApplication(
        store,
        event_bus.publish,
        memory_store=SqliteMemoryStore(tmp_path / "memory.db"),
        config_store=config,
    )
    connection = AckFailingConnection(
        [
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {"protocolVersion": PROTOCOL_VERSION},
            },
            {
                "jsonrpc": "2.0",
                "id": 3,
                "method": "thread.list",
                "params": {},
            },
        ]
    )
    unexpected = (
        {protected: "otherwise-safe"} if credential_position == "key" else {"value": protected}
    )
    monkeypatch.setattr(kernel.threads, "list", lambda _params: unexpected)

    try:
        await handle_connection(
            cast(ServerConnection, connection),
            asyncio.Event(),
            kernel.router,
            event_bus,
            kernel.security,
            kernel.finish_command,
        )
        guarded = next(value for value in connection.sent if value.get("error"))
        assert guarded == {
            "jsonrpc": "2.0",
            "id": None,
            "error": {
                "code": -32603,
                "message": "response contained protected configuration data",
            },
        }
        assert protected not in json.dumps(connection.sent)
    finally:
        await kernel.close()
        store.close()


@pytest.mark.asyncio
async def test_accepted_turn_runs_even_when_the_ack_connection_disconnects(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    thread, _ = store.create_thread("ACK disconnect")
    event_bus = EventHub(next_seq=store.latest_sequence() + 1)
    kernel = RuntimeApplication(
        store,
        event_bus.publish,
        memory_store=SqliteMemoryStore(tmp_path / "memory.db"),
    )
    kernel.start()
    connection = AckFailingConnection(
        [
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {"protocolVersion": PROTOCOL_VERSION},
            },
            {
                "jsonrpc": "2.0",
                "id": 2,
                "method": "turn.start",
                "params": {
                    "threadId": thread.id,
                    "branchId": thread.default_branch_id,
                    "content": "continue after disconnect",
                    "providerId": "scripted",
                    "modelId": "scripted-v1",
                },
            },
        ]
    )

    try:
        with pytest.raises(ConnectionError, match="simulated ACK disconnect"):
            await handle_connection(
                cast(ServerConnection, connection),
                asyncio.Event(),
                kernel.router,
                event_bus,
                kernel.security,
                kernel.finish_command,
            )

        settled: list[JournalEvent] = []
        for _ in range(100):
            replayed, _latest_seq = store.replay_events(0, 1000)
            settled = [event for event in replayed if event.type == "run.settled"]
            if settled:
                break
            await asyncio.sleep(0.01)

        assert len(settled) == 1
        assert settled[0].payload["status"] == "completed"
    finally:
        await kernel.close()
        store.close()


@pytest.mark.asyncio
async def test_accepted_cancel_runs_even_when_the_ack_connection_disconnects(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    thread, _ = store.create_thread("Cancel ACK disconnect")
    event_bus = EventHub(next_seq=store.latest_sequence() + 1)
    kernel = RuntimeApplication(
        store,
        event_bus.publish,
        memory_store=SqliteMemoryStore(tmp_path / "memory.db"),
    )
    kernel.start()
    started = kernel.turns.start_turn(
        {
            "threadId": thread.id,
            "branchId": thread.default_branch_id,
            "content": "x" * 4000,
            "providerId": "scripted",
            "modelId": "scripted-v1",
        }
    )
    run_id = cast(str, started.result["runId"])
    await kernel.finish_command(started)

    try:
        for _ in range(100):
            if store.run_status(run_id) == "running":
                break
            await asyncio.sleep(0.01)
        assert store.run_status(run_id) == "running"
        connection = AckFailingConnection(
            [
                {
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "initialize",
                    "params": {"protocolVersion": PROTOCOL_VERSION},
                },
                {
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "run.cancel",
                    "params": {"runId": run_id},
                },
            ]
        )

        with pytest.raises(ConnectionError, match="simulated ACK disconnect"):
            await handle_connection(
                cast(ServerConnection, connection),
                asyncio.Event(),
                kernel.router,
                event_bus,
                kernel.security,
                kernel.finish_command,
            )

        for _ in range(100):
            if store.run_status(run_id) == "cancelled":
                break
            await asyncio.sleep(0.01)
        assert store.run_status(run_id) == "cancelled"
        replayed, _ = store.replay_events(0, 1000)
        settled = [
            event for event in replayed if event.type == "run.settled" and event.run_id == run_id
        ]
        assert len(settled) == 1
        assert settled[0].payload["status"] == "cancelled"
    finally:
        await kernel.close()
        store.close()


@pytest.mark.asyncio
async def test_cancel_between_turn_prepare_and_activation_prevents_execution(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    thread, _ = store.create_thread("Cross-client cancellation")
    event_bus = EventHub(next_seq=store.latest_sequence() + 1)
    kernel = RuntimeApplication(
        store,
        event_bus.publish,
        memory_store=SqliteMemoryStore(tmp_path / "memory.db"),
    )
    kernel.start()

    try:
        started = kernel.turns.start_turn(
            {
                "threadId": thread.id,
                "branchId": thread.default_branch_id,
                "content": "must never reach the provider",
                "providerId": "scripted",
                "modelId": "scripted-v1",
            }
        )
        run_id = cast(str, started.result["runId"])
        cancelled = kernel.turns.cancel_run({"runId": run_id})
        assert cancelled.result == {
            "accepted": True,
            "runId": run_id,
            "status": "queued",
        }

        # Simulate two client handlers whose post-ACK actions interleave: the
        # cancel ACK completes before the original turn.start ACK completes.
        await kernel.finish_command(cancelled)
        await kernel.finish_command(started)
        await asyncio.sleep(0.02)

        assert store.run_status(run_id) == "cancelled"
        replayed, _ = store.replay_events(0, 1000)
        run_events = [event for event in replayed if event.run_id == run_id]
        assert not any(
            event.type in {"item.started", "item.delta"}
            or (event.type == "run.state_changed" and event.payload["status"] == "running")
            for event in run_events
        )
        settled = [event for event in run_events if event.type == "run.settled"]
        assert len(settled) == 1
        assert settled[0].payload["status"] == "cancelled"
    finally:
        await kernel.close()
        store.close()


@pytest.mark.asyncio
async def test_reverse_ack_order_preserves_persisted_turn_execution_order(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    thread, _ = store.create_thread("Reverse ACK order")
    event_bus = EventHub(next_seq=store.latest_sequence() + 1)
    kernel = RuntimeApplication(
        store,
        event_bus.publish,
        memory_store=SqliteMemoryStore(tmp_path / "memory.db"),
    )
    kernel.start()
    first_ack_gate = asyncio.Event()
    second_ack_gate = asyncio.Event()
    second_ack_gate.set()
    first_connection = ControlledAckConnection(
        [
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {"protocolVersion": PROTOCOL_VERSION},
            },
            {
                "jsonrpc": "2.0",
                "id": 2,
                "method": "turn.start",
                "params": {
                    "threadId": thread.id,
                    "branchId": thread.default_branch_id,
                    "content": "alpha",
                    "providerId": "scripted",
                    "modelId": "scripted-v1",
                },
            },
        ],
        first_ack_gate,
    )
    second_connection = ControlledAckConnection(
        [
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {"protocolVersion": PROTOCOL_VERSION},
            },
            {
                "jsonrpc": "2.0",
                "id": 2,
                "method": "turn.start",
                "params": {
                    "threadId": thread.id,
                    "branchId": thread.default_branch_id,
                    "content": "beta",
                    "providerId": "scripted",
                    "modelId": "scripted-v1",
                },
            },
        ],
        second_ack_gate,
    )
    tasks: list[asyncio.Task[None]] = []

    try:
        first_task = asyncio.create_task(
            handle_connection(
                cast(ServerConnection, first_connection),
                asyncio.Event(),
                kernel.router,
                event_bus,
                kernel.security,
                kernel.finish_command,
            )
        )
        tasks.append(first_task)
        await asyncio.wait_for(first_connection.ack_started.wait(), timeout=1)

        second_task = asyncio.create_task(
            handle_connection(
                cast(ServerConnection, second_connection),
                asyncio.Event(),
                kernel.router,
                event_bus,
                kernel.security,
                kernel.finish_command,
            )
        )
        tasks.append(second_task)
        await asyncio.wait_for(second_connection.ack_completed.wait(), timeout=1)
        await asyncio.wait_for(second_task, timeout=1)
        assert not first_connection.ack_completed.is_set()
        assert not first_task.done()

        second_response = next(value for value in second_connection.sent if value.get("id") == 2)
        second_run_id = cast(str, second_response["result"]["runId"])
        assert store.run_status(second_run_id) == "queued"
        replayed_before_first_ack, _ = store.replay_events(0, 1000)
        assert not any(
            event.type == "run.state_changed"
            and event.run_id == second_run_id
            and event.payload["status"] == "running"
            for event in replayed_before_first_ack
        )

        first_ack_gate.set()
        await asyncio.wait_for(first_task, timeout=1)
        first_response = next(value for value in first_connection.sent if value.get("id") == 2)
        first_run_id = cast(str, first_response["result"]["runId"])

        replayed: list[JournalEvent] = []
        for _ in range(500):
            replayed, _ = store.replay_events(0, 1000)
            settled_ids = [
                event.run_id
                for event in replayed
                if event.type == "run.settled" and event.run_id in {first_run_id, second_run_id}
            ]
            if len(settled_ids) == 2:
                break
            await asyncio.sleep(0.005)

        assert settled_ids == [first_run_id, second_run_id]
        second_answer = next(
            event.payload["item"]["content"]
            for event in replayed
            if event.type == "item.completed"
            and event.run_id == second_run_id
            and event.payload.get("item", {}).get("role") == "assistant"
        )
        assert second_answer == (
            "Previous user: alpha\n"
            "Previous assistant: Scripted response to: alpha\n"
            "Current user: beta"
        )
    finally:
        first_ack_gate.set()
        second_ack_gate.set()
        await asyncio.gather(*tasks, return_exceptions=True)
        await kernel.close()
        store.close()


@pytest.mark.asyncio
async def test_natural_completion_can_win_after_cancel_is_accepted(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    thread, _ = store.create_thread("Completion race")
    event_bus = EventHub(next_seq=store.latest_sequence() + 1)
    kernel = RuntimeApplication(
        store,
        event_bus.publish,
        memory_store=SqliteMemoryStore(tmp_path / "memory.db"),
    )
    kernel.start()

    try:
        started = kernel.turns.start_turn(
            {
                "threadId": thread.id,
                "branchId": thread.default_branch_id,
                "content": "completion-wins-" + ("x" * 1000),
                "providerId": "scripted",
                "modelId": "scripted-v1",
            }
        )
        run_id = cast(str, started.result["runId"])
        await kernel.finish_command(started)
        for _ in range(100):
            if store.run_status(run_id) == "running":
                break
            await asyncio.sleep(0.001)
        assert store.run_status(run_id) == "running"

        # The response represents the ACK-time decision. Post-ACK cancellation
        # is intentionally delayed here so the natural terminal transaction wins.
        cancellation = kernel.turns.cancel_run({"runId": run_id})
        assert cancellation.result == {
            "accepted": True,
            "runId": run_id,
            "status": "running",
        }
        for _ in range(1000):
            if store.run_status(run_id) == "completed":
                break
            await asyncio.sleep(0.001)
        assert store.run_status(run_id) == "completed"

        await kernel.finish_command(cancellation)
        assert store.run_status(run_id) == "completed"
        replayed, _ = store.replay_events(0, 2000)
        settled = [
            event for event in replayed if event.type == "run.settled" and event.run_id == run_id
        ]
        assert len(settled) == 1
        assert settled[0].payload["status"] == "completed"
    finally:
        await kernel.close()
        store.close()


@pytest.mark.asyncio
async def test_cli_server_requires_authentication_and_completes_handshake(
    tmp_path: Path,
) -> None:
    token = "-option-shaped-launch-token"
    process, ready = await _start_runtime(token, tmp_path)
    uri = f"ws://{ready['host']}:{ready['port']}"

    try:
        assert ready["type"] == "ikaros_runtime.ready"
        assert ready["protocolVersion"] == PROTOCOL_VERSION
        assert ready["host"] == "127.0.0.1"
        assert ready["port"] > 0
        assert ready["pid"] > 0
        assert (tmp_path / "state.db").is_file()
        assert not (tmp_path / "config.yaml").exists()

        with pytest.raises(InvalidStatus) as unauthorized:
            async with connect(uri):
                await asyncio.sleep(0)
        assert unauthorized.value.response.status_code == 401

        connection = await _initialize(uri, token)
        await _shutdown(connection, process, 2)
    finally:
        await _stop_failed_process(process)


@pytest.mark.asyncio
async def test_thread_and_event_journal_survive_runtime_restart(tmp_path: Path) -> None:
    first_token = secrets.token_urlsafe(32)
    first_process, first_ready = await _start_runtime(first_token, tmp_path)
    try:
        first = await _initialize(
            f"ws://{first_ready['host']}:{first_ready['port']}",
            first_token,
        )
        empty = await _rpc(first, 2, "thread.list", {})
        assert empty["result"] == {
            "threads": [],
            "nextCursor": None,
            "hasMore": False,
            "snapshotSeq": 0,
        }

        created = await _rpc(first, 3, "thread.create", {"title": "Persistent thread"})
        thread = created["result"]["thread"]
        event = created["result"]["event"]
        assert thread["title"] == "Persistent thread"
        assert thread["workspace"] is None
        assert thread["defaultBranchId"].startswith("branch_")
        assert event["seq"] == 1
        assert event["type"] == "thread.created"
        assert event["threadId"] == thread["id"]
        assert event["branchId"] == thread["defaultBranchId"]

        replayed = await _rpc(first, 4, "event.replay", {"afterSeq": 0})
        assert replayed["result"] == {
            "events": [event],
            "latestSeq": 1,
            "nextAfterSeq": 1,
            "hasMore": False,
        }
        await _shutdown(first, first_process, 5)
    finally:
        await _stop_failed_process(first_process)

    second_token = secrets.token_urlsafe(32)
    second_process, second_ready = await _start_runtime(second_token, tmp_path)
    try:
        second = await _initialize(
            f"ws://{second_ready['host']}:{second_ready['port']}",
            second_token,
        )
        listed = await _rpc(second, 2, "thread.list", {})
        assert listed["result"] == {
            "threads": [thread],
            "nextCursor": None,
            "hasMore": False,
            "snapshotSeq": 1,
        }

        caught_up = await _rpc(second, 3, "event.replay", {"afterSeq": 1})
        assert caught_up["result"] == {
            "events": [],
            "latestSeq": 1,
            "nextAfterSeq": 1,
            "hasMore": False,
        }

        next_created = await _rpc(second, 4, "thread.create", {})
        assert next_created["result"]["event"]["seq"] == 2
        page = await _rpc(second, 5, "event.replay", {"afterSeq": 0, "limit": 1})
        assert [event["seq"] for event in page["result"]["events"]] == [1]
        assert page["result"]["latestSeq"] == 2
        assert page["result"]["nextAfterSeq"] == 1
        assert page["result"]["hasMore"] is True
        await _shutdown(second, second_process, 6)
    finally:
        await _stop_failed_process(second_process)


@pytest.mark.asyncio
async def test_thread_workspace_rpc_survives_list_replay_and_restart(tmp_path: Path) -> None:
    workspace_root = tmp_path / "research"
    workspace_root.mkdir()
    workspace = {
        "id": "workspace-research",
        "name": "Research",
        "rootUri": str(workspace_root.resolve()),
    }
    first_token = secrets.token_urlsafe(32)
    first_process, first_ready = await _start_runtime(first_token, tmp_path)
    try:
        first = await _initialize(
            f"ws://{first_ready['host']}:{first_ready['port']}",
            first_token,
        )
        created = await _rpc(
            first,
            2,
            "thread.create",
            {
                "title": "Workspace thread",
                "workspace": workspace,
                "clientRequestId": "workspace-thread-create",
            },
        )
        thread = created["result"]["thread"]
        assert thread["workspace"] == workspace
        assert created["result"]["event"]["payload"]["thread"]["workspace"] == workspace
        repeated = await _rpc(
            first,
            3,
            "thread.create",
            {
                "title": "Workspace thread",
                "workspace": workspace,
                "clientRequestId": "workspace-thread-create",
            },
        )
        assert repeated["result"] == created["result"]
        conflicting = await _rpc(
            first,
            4,
            "thread.create",
            {
                "title": "Workspace thread",
                "workspace": {"id": "workspace-other", "name": "Other", "rootUri": None},
                "clientRequestId": "workspace-thread-create",
            },
        )
        assert conflicting["error"]["code"] == -32602
        listed = await _rpc(first, 5, "thread.list", {})
        assert listed["result"]["threads"] == [thread]
        await _shutdown(first, first_process, 6)
    finally:
        await _stop_failed_process(first_process)

    second_token = secrets.token_urlsafe(32)
    second_process, second_ready = await _start_runtime(second_token, tmp_path)
    try:
        second = await _initialize(
            f"ws://{second_ready['host']}:{second_ready['port']}",
            second_token,
        )
        listed = await _rpc(second, 2, "thread.list", {})
        assert listed["result"]["threads"] == [thread]
        await _shutdown(second, second_process, 3)
    finally:
        await _stop_failed_process(second_process)


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "workspace",
    [
        "workspace",
        {},
        {"id": "id", "name": "name"},
        {"id": "id", "name": "name", "rootUri": None, "extra": True},
        {"id": 1, "name": "name", "rootUri": None},
        {"id": "id", "name": "", "rootUri": None},
        {"id": "x" * 201, "name": "name", "rootUri": None},
        {"id": "id", "name": "x" * 201, "rootUri": None},
        {"id": "id", "name": "name", "rootUri": 1},
        {"id": "id", "name": "name", "rootUri": "relative/workspace"},
    ],
)
async def test_thread_create_rejects_invalid_workspace(
    tmp_path: Path,
    workspace: object,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    kernel = RuntimeApplication(
        store,
        _discard_event,
        memory_store=SqliteMemoryStore(tmp_path / "memory.db"),
    )
    try:
        with pytest.raises(InvalidParamsError, match="workspace"):
            kernel.threads.create({"title": "Invalid workspace", "workspace": workspace})
        assert store.list_thread_page(cursor=None, limit=50).threads == ()
        assert store.latest_sequence() == 0
    finally:
        await kernel.close()
        store.close()


@pytest.mark.asyncio
@pytest.mark.parametrize(
    "params",
    [
        {"unknown": True},
        {"cursor": None},
        {"cursor": ""},
        {"cursor": "not*base64url"},
        {"limit": True},
        {"limit": 0},
        {"limit": 101},
        {"limit": 1.5},
        {"archived": None},
        {"archived": "true"},
    ],
)
async def test_thread_list_rejects_invalid_pagination_params(
    tmp_path: Path,
    params: dict[str, object],
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    kernel = RuntimeApplication(
        store,
        _discard_event,
        memory_store=SqliteMemoryStore(tmp_path / "memory.db"),
    )
    try:
        routed = await kernel.router.dispatch(1, "thread.list", params)

        error = routed.response.get("error")
        assert isinstance(error, dict)
        assert error["code"] == -32602
    finally:
        await kernel.close()
        store.close()


@pytest.mark.asyncio
async def test_usage_read_returns_exact_empty_shape_and_rejects_parameters(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    kernel = RuntimeApplication(
        store,
        _discard_event,
        memory_store=SqliteMemoryStore(tmp_path / "memory.db"),
    )
    try:
        routed = await kernel.router.dispatch(1, "usage.read", {})
        assert routed.response == {
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "summary": {
                    "lifetimeTokens": None,
                    "peakDailyTokens": None,
                    "longestRunningTurnSec": None,
                    "currentStreakDays": 0,
                    "longestStreakDays": 0,
                },
                "dailyUsageBuckets": [],
            },
        }

        rejected = await kernel.router.dispatch(2, "usage.read", {"days": 7})
        assert rejected.response == {
            "jsonrpc": "2.0",
            "id": 2,
            "error": {
                "code": -32602,
                "message": "usage.read does not accept parameters",
            },
        }
    finally:
        await kernel.close()
        store.close()


@pytest.mark.asyncio
async def test_thread_list_defaults_to_fifty_and_returns_next_page(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    kernel = RuntimeApplication(
        store,
        _discard_event,
        memory_store=SqliteMemoryStore(tmp_path / "memory.db"),
    )
    try:
        for index in range(51):
            kernel.threads.create({"title": f"Thread {index}"})

        first = kernel.threads.list({})
        assert len(first["threads"]) == 50
        assert first["hasMore"] is True
        assert isinstance(first["nextCursor"], str)
        assert first["snapshotSeq"] == 51

        second = kernel.threads.list({"cursor": first["nextCursor"]})
        assert len(second["threads"]) == 1
        assert second["hasMore"] is False
        assert second["nextCursor"] is None
        assert second["snapshotSeq"] == 51
    finally:
        await kernel.close()
        store.close()


@pytest.mark.asyncio
@pytest.mark.parametrize(
    ("method", "params"),
    [
        ("thread.get", {}),
        ("thread.get", {"threadId": ""}),
        ("thread.get", {"threadId": "thread", "extra": True}),
        ("thread.rename", {}),
        ("thread.rename", {"threadId": "thread", "title": 7}),
        ("thread.archive", {}),
        ("thread.archive", {"threadId": "thread", "extra": True}),
        ("thread.unarchive", {"threadId": ""}),
        ("turn.list", {}),
        ("turn.list", {"threadId": "thread"}),
        ("turn.list", {"threadId": "thread", "branchId": ""}),
        ("turn.list", {"threadId": "thread", "branchId": "branch", "cursor": None}),
        ("turn.list", {"threadId": "thread", "branchId": "branch", "limit": True}),
        ("turn.list", {"threadId": "thread", "branchId": "branch", "limit": 101}),
        (
            "turn.list",
            {"threadId": "thread", "branchId": "branch", "unsupported": True},
        ),
    ],
)
async def test_thread_history_reads_reject_invalid_params(
    tmp_path: Path,
    method: str,
    params: dict[str, object],
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    kernel = RuntimeApplication(
        store,
        _discard_event,
        memory_store=SqliteMemoryStore(tmp_path / "memory.db"),
    )
    try:
        routed = await kernel.router.dispatch(1, method, params)

        error = routed.response.get("error")
        assert isinstance(error, dict)
        assert error["code"] == -32602
        assert store.latest_sequence() == 0
    finally:
        await kernel.close()
        store.close()


@pytest.mark.asyncio
async def test_thread_workspace_cannot_contain_configured_credentials(tmp_path: Path) -> None:
    protected = "workspace-configured-credential"
    config = ConfigStore(tmp_path)
    config.configure_custom(
        provider_id="local",
        display_name="Local",
        base_url="http://127.0.0.1:9/v1",
        api_key=protected,
        headers=None,
        models=[ModelInput("model", "Model")],
    )
    store = SqliteRuntimeStore(tmp_path / "state.db")
    kernel = RuntimeApplication(
        store,
        _discard_event,
        memory_store=SqliteMemoryStore(tmp_path / "memory.db"),
        config_store=config,
    )
    try:
        with pytest.raises(InvalidParamsError, match="protected configuration"):
            kernel.threads.create(
                {
                    "title": "Protected workspace",
                    "workspace": {
                        "id": "workspace-id",
                        "name": protected,
                        "rootUri": None,
                    },
                }
            )
        assert store.list_thread_page(cursor=None, limit=50).threads == ()
        assert store.latest_sequence() == 0
    finally:
        await kernel.close()
        store.close()


@pytest.mark.asyncio
async def test_thread_create_normalizes_workspace_root_and_list_survives_deletion(
    tmp_path: Path,
) -> None:
    root = tmp_path / "workspace" / "nested"
    root.mkdir(parents=True)
    store = SqliteRuntimeStore(tmp_path / "state.db")
    kernel = RuntimeApplication(
        store,
        _discard_event,
        memory_store=SqliteMemoryStore(tmp_path / "memory.db"),
    )
    try:
        created = kernel.threads.create(
            {
                "title": "Normalized workspace",
                "workspace": {
                    "id": " workspace-id ",
                    "name": " Workspace Name ",
                    "rootUri": str(root / ".." / "nested"),
                },
            }
        )
        assert created.result["thread"]["workspace"] == {
            "id": "workspace-id",
            "name": "Workspace Name",
            "rootUri": str(root.resolve()),
        }

        root.rmdir()
        assert kernel.threads.list({})["threads"] == [created.result["thread"]]
    finally:
        await kernel.close()
        store.close()


@pytest.mark.asyncio
async def test_client_request_ids_make_mutating_commands_exactly_once(tmp_path: Path) -> None:
    token = secrets.token_urlsafe(32)
    process, ready = await _start_runtime(token, tmp_path)
    try:
        connection = await _initialize(f"ws://{ready['host']}:{ready['port']}", token)
        create_params = {
            "title": "Idempotent commands",
            "clientRequestId": "desktop-create-request",
        }
        first_created = await _rpc(connection, 2, "thread.create", create_params)
        repeated_created = await _rpc(connection, 3, "thread.create", create_params, [])
        assert repeated_created["result"] == first_created["result"]
        thread = first_created["result"]["thread"]

        start_params = {
            "threadId": thread["id"],
            "branchId": thread["defaultBranchId"],
            "content": "execute exactly once",
            "providerId": "scripted",
            "modelId": "scripted-v1",
            "clientRequestId": "desktop-turn-request",
        }
        first_started = await _rpc(connection, 4, "turn.start", start_params, [])
        repeated_started = await _rpc(connection, 5, "turn.start", start_params, [])
        assert repeated_started["result"] == first_started["result"]

        run_id = first_started["result"]["runId"]
        replayed = await _replay_until_run_settled(
            connection,
            run_id,
            request_id=6,
        )
        run_events = [event for event in replayed if event["runId"] == run_id]
        assert sum(event["type"] == "run.settled" for event in run_events) == 1
        assert (
            sum(
                event["type"] == "item.completed"
                and event["payload"].get("item", {}).get("role") == "user"
                for event in run_events
            )
            == 1
        )
        user_event = next(
            event
            for event in run_events
            if event["type"] == "item.completed"
            and event["payload"].get("item", {}).get("role") == "user"
        )
        assert user_event["payload"]["clientRequestId"] == "desktop-turn-request"
        listed = await _rpc(connection, 100, "thread.list", {}, [])
        assert len(listed["result"]["threads"]) == 1
        await _shutdown(connection, process, 101)
    finally:
        await _stop_failed_process(process)


@pytest.mark.asyncio
async def test_thread_lifecycle_mutations_flow_through_rpc_journal_and_catalog(
    tmp_path: Path,
) -> None:
    token = secrets.token_urlsafe(32)
    process, ready = await _start_runtime(token, tmp_path)
    try:
        connection = await _initialize(f"ws://{ready['host']}:{ready['port']}", token)
        created = await _rpc(connection, 2, "thread.create", {"title": "Original"})
        thread = created["result"]["thread"]
        assert thread["archivedAt"] is None

        renamed = await _rpc(
            connection,
            3,
            "thread.rename",
            {"threadId": thread["id"], "title": "  Renamed  "},
        )
        assert renamed["result"]["changed"] is True
        assert renamed["result"]["thread"]["title"] == "Renamed"
        assert renamed["result"]["event"]["type"] == "thread.renamed"

        archived = await _rpc(
            connection,
            4,
            "thread.archive",
            {"threadId": thread["id"]},
        )
        assert archived["result"]["changed"] is True
        assert archived["result"]["thread"]["archivedAt"] is not None
        assert archived["result"]["event"]["type"] == "thread.archived"

        active_page = await _rpc(connection, 5, "thread.list", {}, [])
        assert active_page["result"]["threads"] == []
        archived_page = await _rpc(
            connection,
            6,
            "thread.list",
            {"archived": True},
            [],
        )
        assert archived_page["result"]["threads"] == [archived["result"]["thread"]]
        metadata = await _rpc(
            connection,
            7,
            "thread.get",
            {"threadId": thread["id"]},
            [],
        )
        assert metadata["result"]["thread"] == archived["result"]["thread"]

        repeated = await _rpc(
            connection,
            8,
            "thread.archive",
            {"threadId": thread["id"]},
            [],
        )
        assert repeated["result"] == {
            "thread": archived["result"]["thread"],
            "changed": False,
            "event": None,
        }

        restored = await _rpc(
            connection,
            9,
            "thread.unarchive",
            {"threadId": thread["id"]},
            [],
        )
        assert restored["result"]["thread"]["archivedAt"] is None
        assert restored["result"]["event"]["type"] == "thread.unarchived"

        replayed = await _rpc(
            connection,
            10,
            "event.replay",
            {"afterSeq": 0, "limit": 100},
            [],
        )
        assert [event["type"] for event in replayed["result"]["events"]] == [
            "thread.created",
            "thread.renamed",
            "thread.archived",
            "thread.unarchived",
        ]
        await _shutdown(connection, process, 11)
    finally:
        await _stop_failed_process(process)


@pytest.mark.asyncio
async def test_scripted_provider_streams_two_contextual_turns_and_settles_once(
    tmp_path: Path,
) -> None:
    token = secrets.token_urlsafe(32)
    process, ready = await _start_runtime(token, tmp_path)
    try:
        connection = await _initialize(f"ws://{ready['host']}:{ready['port']}", token)
        created = await _rpc(connection, 2, "thread.create", {"title": "Two turns"})
        thread = created["result"]["thread"]

        first_started = await _rpc(
            connection,
            3,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": "alpha",
                "providerId": "scripted",
                "modelId": "scripted-v1",
            },
        )
        first_run_id = first_started["result"]["runId"]
        first_events = await _collect_run_events(connection, first_run_id)
        first_types = [event["type"] for event in first_events]
        assert first_types[:5] == [
            "item.completed",
            "run.state_changed",
            "run.state_changed",
            "model.input_prepared",
            "item.started",
        ]
        assert first_types.count("item.delta") >= 2
        assert first_types[-3:] == [
            "item.completed",
            "model.response_finished",
            "run.settled",
        ]
        assert first_types.count("model.input_prepared") == 1
        assert first_types.count("model.response_finished") == 1
        assert (
            sum(
                event["type"] == "run.settled" and event["runId"] == first_run_id
                for event in first_events
            )
            == 1
        )
        first_answer = next(
            event["payload"]["item"]["content"]
            for event in first_events
            if event["type"] == "item.completed" and event["payload"]["item"]["role"] == "assistant"
        )
        assert first_answer == "Scripted response to: alpha"

        second_started = await _rpc(
            connection,
            4,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": "beta",
                "providerId": "scripted",
                "modelId": "scripted-v1",
            },
        )
        second_run_id = second_started["result"]["runId"]
        second_events = await _collect_run_events(connection, second_run_id)
        second_answer = next(
            event["payload"]["item"]["content"]
            for event in second_events
            if event["type"] == "item.completed" and event["payload"]["item"]["role"] == "assistant"
        )
        assert second_answer == (
            "Previous user: alpha\n"
            "Previous assistant: Scripted response to: alpha\n"
            "Current user: beta"
        )
        assert [event["seq"] for event in first_events + second_events] == sorted(
            event["seq"] for event in first_events + second_events
        )
        assert sum(event["type"] == "run.settled" for event in second_events) == 1
        await _shutdown(connection, process, 5)
    finally:
        await _stop_failed_process(process)


@pytest.mark.asyncio
async def test_thread_get_and_turn_list_read_the_complete_runtime_history(
    tmp_path: Path,
) -> None:
    token = secrets.token_urlsafe(32)
    process, ready = await _start_runtime(token, tmp_path)
    try:
        connection = await _initialize(f"ws://{ready['host']}:{ready['port']}", token)
        created = await _rpc(connection, 2, "thread.create", {"title": "Readable history"})
        thread = created["result"]["thread"]
        started = await _rpc(
            connection,
            3,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": "remember this turn",
                "providerId": "scripted",
                "modelId": "scripted-v1",
            },
        )
        run_id = started["result"]["runId"]
        await _collect_run_events(connection, run_id)
        before = await _rpc(
            connection,
            4,
            "event.replay",
            {"afterSeq": 0, "limit": 1000},
            [],
        )

        metadata = await _rpc(
            connection,
            5,
            "thread.get",
            {"threadId": thread["id"]},
            [],
        )
        history = await _rpc(
            connection,
            6,
            "turn.list",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "limit": 10,
            },
            [],
        )
        after = await _rpc(
            connection,
            7,
            "event.replay",
            {"afterSeq": 0, "limit": 1000},
            [],
        )

        metadata_thread = metadata["result"]["thread"]
        assert {
            key: metadata_thread[key]
            for key in ("id", "title", "defaultBranchId", "workspace", "createdAt")
        } == {
            key: thread[key] for key in ("id", "title", "defaultBranchId", "workspace", "createdAt")
        }
        assert metadata_thread["updatedAt"] >= thread["updatedAt"]
        assert metadata["result"]["snapshotSeq"] == before["result"]["latestSeq"]
        assert history["result"]["snapshotSeq"] == before["result"]["latestSeq"]
        assert history["result"]["hasMore"] is False
        assert history["result"]["nextCursor"] is None
        turns = history["result"]["turns"]
        assert len(turns) == 1
        assert turns[0]["ordinal"] == 1
        assert [run["id"] for run in turns[0]["runs"]] == [run_id]
        items = turns[0]["runs"][0]["items"]
        assert [(item["role"], item["content"]) for item in items] == [
            ("user", "remember this turn"),
            ("assistant", "Scripted response to: remember this turn"),
        ]
        assert after["result"]["latestSeq"] == before["result"]["latestSeq"]
        assert after["result"]["events"] == before["result"]["events"]

        missing_branch = await _rpc(
            connection,
            8,
            "turn.list",
            {"threadId": thread["id"], "branchId": "branch_missing"},
            [],
        )
        assert missing_branch["error"]["code"] == -32602
        await _shutdown(connection, process, 9)
    finally:
        await _stop_failed_process(process)


@pytest.mark.asyncio
async def test_scripted_provider_runs_a_real_command_and_continues_the_conversation(
    tmp_path: Path,
) -> None:
    token = secrets.token_urlsafe(32)
    process, ready = await _start_runtime(token, tmp_path)
    try:
        connection = await _initialize(f"ws://{ready['host']}:{ready['port']}", token)
        created = await _rpc(connection, 2, "thread.create", {"title": "Tool turn"})
        thread = created["result"]["thread"]
        command = (
            "Write-Output 'gate5-tool-output'"
            if os.name == "nt"
            else "printf 'gate5-tool-output\\n'"
        )
        prompt = f"/process.run {command}"
        started = await _rpc(
            connection,
            3,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": prompt,
                "providerId": "scripted",
                "modelId": "scripted-v1",
            },
        )
        run_id = started["result"]["runId"]
        events = await _collect_run_events(connection, run_id)
        run_events = [event for event in events if event["runId"] == run_id]
        tool_call_events = [
            event
            for event in run_events
            if event["type"] in {"item.started", "item.completed"}
            and event["payload"].get("item", {}).get("kind") == "tool_call"
        ]
        tool_results = [
            event
            for event in run_events
            if event["type"] == "item.completed"
            and event["payload"].get("item", {}).get("kind") == "tool_result"
        ]
        assistant = next(
            event["payload"]["item"]
            for event in run_events
            if event["type"] == "item.completed"
            and event["payload"].get("item", {}).get("kind") == "message"
            and event["payload"]["item"].get("role") == "assistant"
        )

        assert [event["payload"]["item"]["status"] for event in tool_call_events] == [
            status for _ in tool_results for status in ("running", "completed")
        ]
        assert len(tool_results) >= 2
        results = [event["payload"]["item"]["data"]["result"] for event in tool_results]
        assert results[0]["toolName"] == "process_start"
        assert results[0]["state"] == "running"
        assert all(result["toolName"] == "process_wait" for result in results[1:])
        result = results[-1]
        assert result["state"] == "exited"
        assert result["exitCode"] == 0
        assert result["ok"] is True
        assert "gate5-tool-output" in result["output"]
        process_facts = [
            event["payload"]["process"]
            for event in run_events
            if event["type"] == "process.recorded"
        ]
        assert process_facts[-1]["state"] == "exited"
        assert "gate5-tool-output" in process_facts[-1]["stdout"]
        assert "gate5-tool-output" in assistant["content"]
        assert not any(event["type"].startswith("permission.") for event in run_events)
        assert sum(event["type"] == "run.settled" for event in run_events) == 1

        replayed = await _replay_until_run_settled(
            connection,
            run_id,
            request_id=100,
        )
        replayed_run = [event for event in replayed if event["runId"] == run_id]
        assert [event["seq"] for event in replayed_run] == [event["seq"] for event in run_events]

        follow_up = await _rpc(
            connection,
            200,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": "what happened",
                "providerId": "scripted",
                "modelId": "scripted-v1",
            },
        )
        follow_up_events = await _collect_run_events(
            connection,
            follow_up["result"]["runId"],
        )
        follow_up_answer = next(
            event["payload"]["item"]["content"]
            for event in follow_up_events
            if event["type"] == "item.completed"
            and event["payload"].get("item", {}).get("role") == "assistant"
        )
        assert f"Previous user: {prompt}" in follow_up_answer
        assert "Previous assistant: Command exited with code 0." in follow_up_answer
        assert "Current user: what happened" in follow_up_answer
        await _shutdown(connection, process, 201)
    finally:
        await _stop_failed_process(process)


@pytest.mark.asyncio
async def test_skill_vertical_slice_reads_instructions_runs_script_and_persists_item_order(
    tmp_path: Path,
) -> None:
    skill_directory = tmp_path / "skills" / "demo"
    skill_directory.mkdir(parents=True)
    script = skill_directory / "skill_script.py"
    script.write_text("print('skill-script-output')\n", encoding="utf-8")
    command = subprocess.list2cmdline([sys.executable, str(script)])
    skill_file = skill_directory / "SKILL.md"
    skill_file.write_text(
        "\n".join(
            (
                "---",
                "name: demo",
                "description: Run the deterministic Skill script.",
                "---",
                "",
                f"IKAROS_SCRIPT_COMMAND: {command}",
                "",
            )
        ),
        encoding="utf-8",
    )
    token = secrets.token_urlsafe(32)
    process, ready = await _start_runtime(token, tmp_path)
    try:
        connection = await _initialize(f"ws://{ready['host']}:{ready['port']}", token)
        catalog = await _rpc(connection, 2, "skill.list", {})
        assert catalog["result"]["diagnostics"] == []
        assert catalog["result"]["skills"] == [
            {
                "name": "demo",
                "description": "Run the deterministic Skill script.",
                "location": str(skill_file.resolve()),
                "enabled": True,
            }
        ]
        created = await _rpc(connection, 3, "thread.create", {"title": "Skill turn"})
        thread = created["result"]["thread"]
        started = await _rpc(
            connection,
            4,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": "/skill.run demo",
                "providerId": "scripted",
                "modelId": "scripted-v1",
            },
        )
        run_id = started["result"]["runId"]
        run_events = [
            event
            for event in await _collect_run_events(connection, run_id)
            if event["runId"] == run_id
        ]
        initial = next(
            event
            for event in run_events
            if event["type"] == "item.completed" and "run" in event["payload"]
        )
        assert initial["payload"]["run"]["skills"] == [
            {
                "name": "demo",
                "description": "Run the deterministic Skill script.",
                "location": str(skill_file.resolve()),
            }
        ]
        history = await _rpc(
            connection,
            5,
            "turn.list",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "limit": 10,
            },
        )
        items = history["result"]["turns"][0]["runs"][0]["items"]
        assert (items[0]["kind"], items[0]["role"]) == ("message", "user")
        assert (items[-1]["kind"], items[-1]["role"]) == ("message", "assistant")
        middle = items[1:-1]
        assert len(middle) % 2 == 0
        assert [(item["kind"], item["role"]) for item in middle] == [
            pair
            for _ in range(len(middle) // 2)
            for pair in (("tool_call", "assistant"), ("tool_result", "tool"))
        ]
        tool_names = [item["data"]["toolName"] for item in items if item["kind"] == "tool_call"]
        assert tool_names[:2] == ["read", "process_start"]
        assert len(tool_names) >= 3 and all(name == "process_wait" for name in tool_names[2:])
        process_result = [
            item["data"]["result"] for item in items if item["kind"] == "tool_result"
        ][-1]
        assert process_result["state"] == "exited"
        assert process_result["exitCode"] == 0
        assert "skill-script-output" in process_result["output"]
        assert "skill-script-output" in items[-1]["content"]
        assert sum(event["type"] == "run.settled" for event in run_events) == 1
        await _shutdown(connection, process, 6)
    finally:
        await _stop_failed_process(process)


@pytest.mark.asyncio
async def test_cancelling_process_wait_preserves_partial_output_and_clears_running_tool(
    tmp_path: Path,
) -> None:
    token = secrets.token_urlsafe(32)
    process, ready = await _start_runtime(token, tmp_path)
    try:
        connection = await _initialize(f"ws://{ready['host']}:{ready['port']}", token)
        created = await _rpc(connection, 2, "thread.create", {"title": "Cancel tool"})
        thread = created["result"]["thread"]
        command = (
            "Write-Output 'before-stop'; Start-Sleep -Seconds 60"
            if os.name == "nt"
            else "printf 'before-stop\\n'; sleep 60"
        )
        started = await _rpc(
            connection,
            3,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": f"/process.run {command}",
                "providerId": "scripted",
                "modelId": "scripted-v1",
            },
        )
        run_id = started["result"]["runId"]
        events: list[dict[str, Any]] = []
        waiting = False
        saw_output = False
        while True:
            message = json.loads(await asyncio.wait_for(connection.recv(), timeout=10))
            assert message["method"] == "event"
            event = message["params"]
            events.append(event)
            if (
                event["runId"] == run_id
                and event["type"] == "item.started"
                and event["payload"].get("item", {}).get("data", {}).get("toolName")
                == "process_wait"
            ):
                waiting = True
            if event["type"] == "process.recorded":
                saw_output = saw_output or "before-stop" in event["payload"]["process"]["stdout"]
            if waiting and saw_output:
                break
        notifications: list[dict[str, Any]] = []
        cancelled = await _rpc(
            connection,
            4,
            "run.cancel",
            {"runId": run_id},
            notifications,
        )
        assert cancelled["result"] == {
            "accepted": True,
            "runId": run_id,
            "status": "running",
        }
        events.extend(notifications)
        events.extend(await _collect_run_events(connection, run_id))
        run_events = [event for event in events if event["runId"] == run_id]
        tool_call_terminal = next(
            event["payload"]["item"]
            for event in run_events
            if event["type"] == "item.completed"
            and event["payload"].get("item", {}).get("kind") == "tool_call"
            and event["payload"]["item"]["status"] == "cancelled"
        )
        tool_result = next(
            event["payload"]["item"]
            for event in run_events
            if event["type"] == "item.completed"
            and event["payload"].get("item", {}).get("kind") == "tool_result"
            and event["payload"]["item"]["status"] == "cancelled"
        )
        settled = [event for event in run_events if event["type"] == "run.settled"]

        assert tool_call_terminal["status"] == "cancelled"
        assert tool_result["status"] == "cancelled"
        assert tool_result["data"]["result"]["cancelled"] is True
        assert "before-stop" in tool_result["data"]["result"]["output"]
        process_facts = [
            event["payload"]["process"]
            for event in run_events
            if event["type"] == "process.recorded"
        ]
        assert process_facts[-1]["state"] == "terminated"
        assert "before-stop" in process_facts[-1]["stdout"]
        assert len(settled) == 1
        assert settled[0]["payload"]["status"] == "cancelled"
        assert not any(
            event["type"] == "item.completed"
            and event["payload"].get("item", {}).get("role") == "assistant"
            and event["payload"].get("item", {}).get("kind") == "message"
            for event in run_events
        )
        await _shutdown(connection, process, 5)
    finally:
        await _stop_failed_process(process)


@pytest.mark.asyncio
async def test_queued_turns_stream_events_in_seq_order_and_run_serially(tmp_path: Path) -> None:
    token = secrets.token_urlsafe(32)
    process, ready = await _start_runtime(token, tmp_path)
    try:
        connection = await _initialize(f"ws://{ready['host']}:{ready['port']}", token)
        notifications: list[dict[str, Any]] = []
        created = await _rpc(
            connection,
            2,
            "thread.create",
            {"title": "Queued turns"},
            notifications,
        )
        thread = created["result"]["thread"]
        first_content = f"alpha-{'x' * 1200}"
        first = await _rpc(
            connection,
            3,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": first_content,
                "providerId": "scripted",
                "modelId": "scripted-v1",
            },
            notifications,
        )
        second = await _rpc(
            connection,
            4,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": "beta",
                "providerId": "scripted",
                "modelId": "scripted-v1",
            },
            notifications,
        )
        run_ids = {first["result"]["runId"], second["result"]["runId"]}
        settled_ids = {
            event["runId"]
            for event in notifications
            if event["type"] == "run.settled" and event["runId"] in run_ids
        }
        while settled_ids != run_ids:
            message = json.loads(await asyncio.wait_for(connection.recv(), timeout=10))
            assert message["method"] == "event"
            event = message["params"]
            notifications.append(event)
            if event["type"] == "run.settled" and event["runId"] in run_ids:
                settled_ids.add(event["runId"])

        sequences = [event["seq"] for event in notifications]
        assert sequences == sorted(sequences)
        assert len(sequences) == len(set(sequences))
        first_settled_seq = next(
            event["seq"]
            for event in notifications
            if event["type"] == "run.settled" and event["runId"] == first["result"]["runId"]
        )
        second_running_seq = next(
            event["seq"]
            for event in notifications
            if event["type"] == "run.state_changed"
            and event["runId"] == second["result"]["runId"]
            and event["payload"]["status"] == "running"
        )
        assert second_running_seq > first_settled_seq
        second_answer = next(
            event["payload"]["item"]["content"]
            for event in notifications
            if event["type"] == "item.completed"
            and event["runId"] == second["result"]["runId"]
            and event["payload"]["item"]["role"] == "assistant"
        )
        assert second_answer.startswith(f"Previous user: {first_content}")
        assert second_answer.endswith("Current user: beta")
        await _shutdown(connection, process, 5)
    finally:
        await _stop_failed_process(process)


@pytest.mark.asyncio
async def test_jsonrpc_cancel_queued_run_never_executes_provider_and_replays_once(
    tmp_path: Path,
) -> None:
    token = secrets.token_urlsafe(32)
    process, ready = await _start_runtime(token, tmp_path)
    try:
        connection = await _initialize(f"ws://{ready['host']}:{ready['port']}", token)
        created = await _rpc(connection, 2, "thread.create", {"title": "Queued cancel"})
        thread = created["result"]["thread"]
        notifications: list[dict[str, Any]] = []
        active = await _rpc(
            connection,
            3,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": "hold-active-" + ("x" * 12000),
                "providerId": "scripted",
                "modelId": "scripted-v1",
            },
            notifications,
        )
        active_run_id = active["result"]["runId"]
        while not any(
            event["type"] == "item.delta" and event["runId"] == active_run_id
            for event in notifications
        ):
            message = json.loads(await asyncio.wait_for(connection.recv(), timeout=5))
            assert message["method"] == "event"
            notifications.append(message["params"])

        queued = await _rpc(
            connection,
            4,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": "queued-provider-must-not-run",
                "providerId": "scripted",
                "modelId": "scripted-v1",
            },
            notifications,
        )
        queued_run_id = queued["result"]["runId"]
        before_ack: list[dict[str, Any]] = []
        cancelled = await _rpc(
            connection,
            5,
            "run.cancel",
            {"runId": queued_run_id},
            before_ack,
        )
        assert cancelled["result"] == {
            "accepted": True,
            "runId": queued_run_id,
            "status": "queued",
        }
        assert not any(
            event["type"] == "run.settled" and event["runId"] == queued_run_id
            for event in before_ack
        )

        replayed = await _replay_until_run_settled(
            connection,
            queued_run_id,
            request_id=6,
        )
        run_events = [event for event in replayed if event["runId"] == queued_run_id]
        assert not any(
            event["type"] in {"item.started", "item.delta"}
            or (event["type"] == "run.state_changed" and event["payload"]["status"] == "running")
            for event in run_events
        )
        settled = [event for event in run_events if event["type"] == "run.settled"]
        assert len(settled) == 1
        assert settled[0]["payload"]["status"] == "cancelled"

        inspection = SqliteRuntimeStore(tmp_path / "state.db")
        try:
            assert inspection.run_status(queued_run_id) == "cancelled"
        finally:
            inspection.close()
        await _shutdown(connection, process, 400)
    finally:
        await _stop_failed_process(process)


@pytest.mark.asyncio
async def test_runtime_shutdown_settles_active_and_queued_runs_without_recovery(
    tmp_path: Path,
) -> None:
    first_token = secrets.token_urlsafe(32)
    first_process, first_ready = await _start_runtime(first_token, tmp_path)
    try:
        first = await _initialize(
            f"ws://{first_ready['host']}:{first_ready['port']}",
            first_token,
        )
        created = await _rpc(first, 2, "thread.create", {"title": "Clean shutdown"})
        thread = created["result"]["thread"]
        notifications: list[dict[str, Any]] = []
        active = await _rpc(
            first,
            3,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": "shutdown-active-" + ("x" * 12000),
                "providerId": "scripted",
                "modelId": "scripted-v1",
            },
            notifications,
        )
        active_run_id = active["result"]["runId"]
        while not any(
            event["type"] == "item.delta" and event["runId"] == active_run_id
            for event in notifications
        ):
            message = json.loads(await asyncio.wait_for(first.recv(), timeout=5))
            assert message["method"] == "event"
            notifications.append(message["params"])

        queued = await _rpc(
            first,
            4,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": "shutdown-queued",
                "providerId": "scripted",
                "modelId": "scripted-v1",
            },
            notifications,
        )
        queued_run_id = queued["result"]["runId"]
        await _shutdown(first, first_process, 5)
    finally:
        await _stop_failed_process(first_process)

    inspection = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        assert inspection.run_status(active_run_id) == "cancelled"
        assert inspection.run_status(queued_run_id) == "cancelled"
        before_restart, before_restart_latest = inspection.replay_events(0, 5000)
        for run_id in (active_run_id, queued_run_id):
            settled = [
                event
                for event in before_restart
                if event.type == "run.settled" and event.run_id == run_id
            ]
            assert len(settled) == 1
            assert settled[0].payload["status"] == "cancelled"
        assert not any(
            event.type == "run.state_changed"
            and event.run_id == queued_run_id
            and event.payload["status"] == "running"
            for event in before_restart
        )
    finally:
        inspection.close()

    second_token = secrets.token_urlsafe(32)
    second_process, second_ready = await _start_runtime(second_token, tmp_path)
    try:
        second = await _initialize(
            f"ws://{second_ready['host']}:{second_ready['port']}",
            second_token,
        )
        await asyncio.sleep(0.05)
        replayed = await _rpc(second, 2, "event.replay", {"afterSeq": 0, "limit": 1000})
        assert replayed["result"]["latestSeq"] == before_restart_latest
        wire_events = cast(list[dict[str, Any]], replayed["result"]["events"])
        for run_id in (active_run_id, queued_run_id):
            wire_settled = [
                event
                for event in wire_events
                if event["type"] == "run.settled" and event["runId"] == run_id
            ]
            assert len(wire_settled) == 1
            assert wire_settled[0]["payload"]["status"] == "cancelled"
        await _shutdown(second, second_process, 3)
    finally:
        await _stop_failed_process(second_process)


@pytest.mark.asyncio
async def test_run_cancel_ack_precedes_canonical_cancelled_events(tmp_path: Path) -> None:
    token = secrets.token_urlsafe(32)
    process, ready = await _start_runtime(token, tmp_path)
    try:
        connection = await _initialize(f"ws://{ready['host']}:{ready['port']}", token)
        created = await _rpc(connection, 2, "thread.create", {"title": "Stop"})
        thread = created["result"]["thread"]
        notifications: list[dict[str, Any]] = []
        started = await _rpc(
            connection,
            3,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": "cancel-" + ("x" * 5000),
                "providerId": "scripted",
                "modelId": "scripted-v1",
            },
            notifications,
        )
        run_id = started["result"]["runId"]
        while not any(
            event["type"] == "item.delta" and event["runId"] == run_id for event in notifications
        ):
            message = json.loads(await asyncio.wait_for(connection.recv(), timeout=5))
            assert message["method"] == "event"
            notifications.append(message["params"])

        before_ack: list[dict[str, Any]] = []
        cancelled = await _rpc(
            connection,
            4,
            "run.cancel",
            {"runId": run_id},
            before_ack,
        )
        assert cancelled["result"] == {
            "accepted": True,
            "runId": run_id,
            "status": "running",
        }
        assert not any(
            event["type"] == "run.settled" and event["runId"] == run_id for event in before_ack
        )

        terminal_events = await _collect_run_events(connection, run_id)
        terminal = [
            event
            for event in terminal_events
            if event["runId"] == run_id
            and event["type"] in {"item.completed", "run.settled"}
            and (
                event["type"] == "run.settled"
                or event["payload"].get("item", {}).get("role") == "assistant"
            )
        ]
        assert [event["type"] for event in terminal] == ["item.completed", "run.settled"]
        assert terminal[0]["payload"]["item"]["status"] == "cancelled"
        assert terminal[0]["payload"]["item"]["content"]
        assert terminal[1]["payload"]["status"] == "cancelled"

        repeated = await _rpc(connection, 5, "run.cancel", {"runId": run_id})
        assert repeated["result"] == {
            "accepted": False,
            "runId": run_id,
            "status": "cancelled",
        }
        unknown = await _rpc(connection, 6, "run.cancel", {"runId": "run_missing"})
        assert unknown["error"]["code"] == -32602
        await _shutdown(connection, process, 7)
    finally:
        await _stop_failed_process(process)


@pytest.mark.asyncio
async def test_websocket_disconnect_does_not_cancel_run_and_replay_has_no_gaps(
    tmp_path: Path,
) -> None:
    token = secrets.token_urlsafe(32)
    process, ready = await _start_runtime(token, tmp_path)
    uri = f"ws://{ready['host']}:{ready['port']}"
    try:
        first = await _initialize(uri, token)
        created = await _rpc(first, 2, "thread.create", {"title": "Reconnect"})
        thread = created["result"]["thread"]
        started = await _rpc(
            first,
            3,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": "reconnect-" + ("x" * 3000),
                "providerId": "scripted",
                "modelId": "scripted-v1",
            },
        )
        run_id = started["result"]["runId"]
        while True:
            message = json.loads(await asyncio.wait_for(first.recv(), timeout=5))
            if message["params"]["type"] == "item.delta":
                break
        await first.close()

        second = await _initialize(uri, token)
        replayed = await _replay_until_run_settled(second, run_id, request_id=2)
        run_events = [event for event in replayed if event["runId"] == run_id]
        assert process.returncode is None
        assert [event["seq"] for event in replayed] == list(range(1, len(replayed) + 1))
        assert len({event["seq"] for event in replayed}) == len(replayed)
        assert sum(event["type"] == "run.settled" for event in run_events) == 1
        assert (
            next(event for event in run_events if event["type"] == "run.settled")["payload"][
                "status"
            ]
            == "completed"
        )
        await _shutdown(second, process, 400)
    finally:
        await _stop_failed_process(process)


@pytest.mark.asyncio
async def test_runtime_restart_fails_running_run_and_resumes_queued_run(
    tmp_path: Path,
) -> None:
    first_token = secrets.token_urlsafe(32)
    first_process, first_ready = await _start_runtime(first_token, tmp_path)
    try:
        first = await _initialize(
            f"ws://{first_ready['host']}:{first_ready['port']}",
            first_token,
        )
        try:
            created = await _rpc(first, 2, "thread.create", {"title": "Crash recovery"})
            thread = created["result"]["thread"]
            notifications: list[dict[str, Any]] = []
            running = await _rpc(
                first,
                3,
                "turn.start",
                {
                    "threadId": thread["id"],
                    "branchId": thread["defaultBranchId"],
                    "content": "crash-" + ("x" * 1200),
                    "providerId": "scripted",
                    "modelId": "scripted-v1",
                },
                notifications,
            )
            queued = await _rpc(
                first,
                4,
                "turn.start",
                {
                    "threadId": thread["id"],
                    "branchId": thread["defaultBranchId"],
                    "content": "resume queued",
                    "providerId": "scripted",
                    "modelId": "scripted-v1",
                },
                notifications,
            )
            running_id = running["result"]["runId"]
            queued_id = queued["result"]["runId"]
            while not any(
                event["type"] == "item.delta" and event["runId"] == running_id
                for event in notifications
            ):
                message = json.loads(await asyncio.wait_for(first.recv(), timeout=5))
                notifications.append(message["params"])

            os.kill(int(first_ready["pid"]), signal.SIGTERM)
            await asyncio.wait_for(first_process.wait(), timeout=5)
        finally:
            await first.close()
    finally:
        await _stop_failed_process(first_process)

    second_token = secrets.token_urlsafe(32)
    second_process, second_ready = await _start_runtime(second_token, tmp_path)
    try:
        second = await _initialize(
            f"ws://{second_ready['host']}:{second_ready['port']}",
            second_token,
        )
        replayed = await _replay_until_run_settled(second, queued_id, request_id=2)
        running_settled = [
            event
            for event in replayed
            if event["type"] == "run.settled" and event["runId"] == running_id
        ]
        queued_settled = [
            event
            for event in replayed
            if event["type"] == "run.settled" and event["runId"] == queued_id
        ]
        partial = next(
            event
            for event in replayed
            if event["type"] == "item.completed"
            and event["runId"] == running_id
            and event["payload"].get("item", {}).get("role") == "assistant"
        )
        prepared_steps = [
            event
            for event in replayed
            if event["type"] == "model.input_prepared" and event["runId"] == running_id
        ]
        finished_steps = [
            event
            for event in replayed
            if event["type"] == "model.response_finished" and event["runId"] == running_id
        ]
        assert len(prepared_steps) == 1
        assert len(finished_steps) == 1
        assert (
            finished_steps[0]["payload"]["stepOrdinal"]
            == prepared_steps[0]["payload"]["stepOrdinal"]
        )
        assert finished_steps[0]["payload"]["outcome"] == "failed"
        assert finished_steps[0]["payload"]["reasonCode"] == "runtime_interrupted"
        assert len(running_settled) == 1
        assert running_settled[0]["payload"]["status"] == "failed"
        assert running_settled[0]["payload"]["reasonCode"] == "runtime_interrupted"
        assert partial["payload"]["item"]["status"] == "failed"
        assert partial["payload"]["item"]["content"]
        assert len(queued_settled) == 1
        assert queued_settled[0]["payload"]["status"] == "completed"
        await _shutdown(second, second_process, 400)
    finally:
        await _stop_failed_process(second_process)


@pytest.mark.asyncio
async def test_overlapping_runtime_waits_for_home_owner_without_recovering_its_run(
    tmp_path: Path,
) -> None:
    endpoint = DelayedProtectedEndpoint("Owner response")
    base_url = await endpoint.start()
    first_token = secrets.token_urlsafe(32)
    first_process, first_ready = await _start_runtime(first_token, tmp_path)
    first = await _initialize(
        f"ws://{first_ready['host']}:{first_ready['port']}",
        first_token,
    )
    second_process: Process | None = None
    second: ClientConnection | None = None
    try:
        configured = await _rpc(
            first,
            10,
            "provider.configure",
            {
                "kind": "custom",
                "providerId": "owner-test",
                "displayName": "Owner test",
                "baseUrl": base_url,
                "models": [
                    {
                        "id": "model",
                        "displayName": "Model",
                        "contextWindow": 32768,
                        "maxOutputTokens": 4096,
                    }
                ],
            },
        )
        assert "result" in configured
        created = await _rpc(first, 2, "thread.create", {"title": "Single home owner"})
        thread = created["result"]["thread"]
        notifications: list[dict[str, Any]] = []
        started = await _rpc(
            first,
            3,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": "Wait while the other runtime starts",
                "providerId": "owner-test",
                "modelId": "model",
            },
            notifications,
        )
        run_id = started["result"]["runId"]
        while not any(
            event["type"] == "run.state_changed"
            and event["runId"] == run_id
            and event["payload"]["status"] == "running"
            for event in notifications
        ):
            message = json.loads(await asyncio.wait_for(first.recv(), timeout=5))
            notifications.append(message["params"])

        await asyncio.wait_for(endpoint.request_received.wait(), timeout=10)
        second_token = secrets.token_urlsafe(32)
        second_process = await asyncio.create_subprocess_exec(
            sys.executable,
            "-m",
            "ikaros_runtime",
            "serve",
            "--host",
            "127.0.0.1",
            "--port",
            "0",
            f"--token={second_token}",
            "--parent-pid",
            str(os.getpid()),
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            env={**os.environ, "IKAROS_HOME": str(tmp_path)},
            creationflags=_HIDDEN_PROCESS_FLAGS,
        )
        assert second_process.stdout is not None
        second_readiness = asyncio.create_task(second_process.stdout.readline())
        await asyncio.sleep(0.25)
        assert not second_readiness.done()
        assert second_process.returncode is None

        before_release = await _rpc(first, 4, "event.replay", {"afterSeq": 0, "limit": 1000})
        assert not any(
            event["type"] == "run.settled" and event["runId"] == run_id
            for event in before_release["result"]["events"]
        )

        await _shutdown(first, first_process, 5)
        readiness_line = await asyncio.wait_for(second_readiness, timeout=10)
        if not readiness_line:
            assert second_process.stderr is not None
            stderr = (await second_process.stderr.read()).decode(errors="replace")
            pytest.fail(f"waiting Runtime exited before readiness: {stderr}")
        second_ready = json.loads(readiness_line)
        second = await _initialize(
            f"ws://{second_ready['host']}:{second_ready['port']}",
            second_token,
        )
        replay = await _rpc(second, 2, "event.replay", {"afterSeq": 0, "limit": 1000})
        run_events = [event for event in replay["result"]["events"] if event["runId"] == run_id]
        settled = [event for event in run_events if event["type"] == "run.settled"]
        assert len(settled) == 1
        assert settled[0]["payload"]["status"] == "cancelled"
        assert all(
            event["payload"].get("reasonCode") != "runtime_interrupted" for event in run_events
        )
        await _shutdown(second, second_process, 3)
    finally:
        endpoint.release_response.set()
        await endpoint.close()
        await first.close()
        if second is not None:
            await second.close()
        await _stop_failed_process(first_process)
        if second_process is not None:
            await _stop_failed_process(second_process)
