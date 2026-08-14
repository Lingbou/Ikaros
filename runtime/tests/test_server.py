from __future__ import annotations

import asyncio
import json
import os
import secrets
import shlex
import signal
import subprocess
import sys
from asyncio.subprocess import Process
from pathlib import Path
from typing import Any, cast

import pytest
from websockets.asyncio.client import ClientConnection, connect
from websockets.asyncio.server import ServerConnection
from websockets.exceptions import InvalidStatus

from ikaros_runtime.bootstrap import RuntimeApplication
from ikaros_runtime.domain import JOURNAL_EVENT_SCHEMA_VERSION, JournalEvent
from ikaros_runtime.errors import ConfigError, InvalidParamsError
from ikaros_runtime.providers.registry import ConfigStore, ModelInput, ProviderConfig
from ikaros_runtime.security import response_values_contain_protected_value
from ikaros_runtime.server.connection import handle_connection
from ikaros_runtime.server.event_hub import EventHub
from ikaros_runtime.server.host import _parent_is_alive
from ikaros_runtime.storage import SqliteRuntimeStore

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
            "protocolVersion": 1,
            "client": {"name": "runtime-test", "version": "0.1.0"},
        },
    )
    assert initialized["result"] == {
        "protocolVersion": 1,
        "server": {"name": "ikaros-runtime", "version": "0.1.0"},
        "capabilities": {
            "threads": True,
            "turns": True,
            "eventReplay": True,
            "streaming": True,
            "scriptedProvider": True,
            "runCancellation": True,
            "providers": True,
            "models": True,
            "usage": True,
            "tools": ["process.run", "read", "write", "edit"],
            "executionPolicy": "full_access",
        },
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
    return [path.read_bytes() for path in runtime_home.glob("state.db*")]


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
            return _fake_sse_text("Tool result received by fake provider.")
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
                                            "name": "process_run",
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
                                            "name": "process_run",
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
            return _fake_sse_text("Protected tool result handled safely.")
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
                                            "name": "process_run",
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
                "models": [{"id": "deepseek-chat", "displayName": "DeepSeek Chat"}],
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
                "models": [{"id": "local-model", "displayName": "Local Model"}],
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
            },
            {
                "providerId": "local",
                "id": "local-model",
                "displayName": "Local Model",
                "enabled": False,
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
    kernel = RuntimeApplication(store, _discard_event, model_discovery=discover)
    connection = AckFailingConnection(
        [
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {"protocolVersion": 1},
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
                    {"id": "deepseek-v4-flash", "displayName": "DeepSeek V4 Flash"},
                    {"id": "deepseek-v4-pro", "displayName": "DeepSeek V4 Pro"},
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
    kernel = RuntimeApplication(store, _discard_event, model_discovery=discover)
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
                "models": [{"id": "safe-model", "displayName": "Safe Model"}],
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
                    "models": [{"id": "model", "displayName": "Model"}],
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
async def test_deeply_nested_jsonrpc_request_returns_parse_error(tmp_path: Path) -> None:
    token = secrets.token_hex(32)
    process, readiness = await _start_runtime(token, tmp_path)
    connection = await _initialize(
        f"ws://{readiness['host']}:{readiness['port']}",
        token,
    )
    try:
        nested = ("[" * 4000) + "0" + ("]" * 4000)
        await connection.send(
            '{"jsonrpc":"2.0","id":2,"method":"thread.list","params":' + nested + "}"
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
                "models": [{"id": "model", "displayName": "Model"}],
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
                "models": [{"id": api_key, "displayName": "Invalid Model"}],
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
                "models": [{"id": "deepseek-chat", "displayName": "DeepSeek Chat"}],
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
                "models": [{"id": "safe-model", "displayName": "Safe Model"}],
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
                "models": [{"id": "model", "displayName": "Model"}],
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
                "models": [{"id": "model", "displayName": "Model"}],
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
                "models": [{"id": "fake-model", "displayName": "Fake Model"}],
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

        assert len(endpoint.requests) == 4
        assert any(
            message["role"] == "assistant" and message.get("content") == "First fake response."
            for message in endpoint.requests[1]["messages"]
        )
        tool_followup_messages = endpoint.requests[3]["messages"]
        assert tool_followup_messages[-2]["reasoning_content"] == "fake tool reasoning"
        assert tool_followup_messages[-2]["tool_calls"][0]["id"] == "call_fake_process"
        assert tool_followup_messages[-1]["role"] == "tool"
        assert "gate6-tool" in tool_followup_messages[-1]["content"]

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
                "models": [{"id": "usage-model", "displayName": "Usage Model"}],
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

        usage_events = [event for event in events if event["type"] == "model.usage_recorded"]
        assert len(usage_events) == 1
        assert usage_events[0]["payload"]["stepOrdinal"] == 1
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
                "models": [{"id": "fake-model", "displayName": "Fake Model"}],
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
            (item["data"]["callId"], item["data"]["toolName"])
            for item in tool_results
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
            item["data"]["result"]
            for item in tool_results
            if item["data"]["toolName"] == "read"
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
            "process_run": {"command"},
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
        replayed_run = [
            event for event in replay["result"]["events"] if event["runId"] == run_id
        ]
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
                "models": [{"id": "non-finite-model", "displayName": "Non-finite Model"}],
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
                "models": [{"id": "fake-model", "displayName": "Fake Model"}],
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
        assert len(result_items) == 1
        result_item = result_items[0]
        assert result_item["status"] == "failed"
        result = result_item["data"]["result"]
        assert result["ok"] is False
        assert result["errorCode"] == "protected_output"
        assert json.loads(result_item["content"]) == result

        assert len(endpoint.requests) == 2
        model_tool_result = endpoint.requests[1]["messages"][-1]
        assert model_tool_result["role"] == "tool"
        assert json.loads(model_tool_result["content"])["errorCode"] == "protected_output"

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
                "models": [{"id": "deepseek-chat", "displayName": "DeepSeek Chat"}],
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
                "models": [{"id": "fake-model", "displayName": "Fake Model"}],
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
                "models": [{"id": "fake-model", "displayName": "Fake Model"}],
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
                "models": [{"id": "deepseek-chat", "displayName": "DeepSeek Chat"}],
            },
            live_events,
        )
        assert deepseek["error"]["code"] == -32602
        assert "Run is active" in deepseek["error"]["message"]
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
    prepared = store.prepare_turn(
        thread_id=thread.id,
        branch_id=thread.default_branch_id,
        content="hold configuration",
        provider_id="local",
        model_id="model",
    )
    kernel = RuntimeApplication(store, _discard_event, config_store=config)
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
                    "models": [{"id": "model", "displayName": "Model"}],
                }
            )
        store.terminalize_run(prepared.run_id, "cancelled")
        configured = kernel.providers.configure_provider(
            {
                "kind": "deepseek",
                "apiKey": "full_access",
                "models": [{"id": "model", "displayName": "Model"}],
            }
        )
        provider = cast(dict[str, object], configured["provider"])
        assert provider["configured"] is True
        assert kernel.providers.remove_provider({"providerId": "local"})["removed"] is True
    finally:
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
    prepared = store.prepare_turn(
        thread_id=thread.id,
        branch_id=thread.default_branch_id,
        content="stable content",
        provider_id="local",
        model_id="model",
        client_request_id="stable-turn-request",
    )
    store.terminalize_run(prepared.run_id, "cancelled")
    kernel = RuntimeApplication(store, _discard_event, config_store=config)
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
        assert store._connection.execute("SELECT COUNT(*) FROM runs").fetchone()[0] == 1
    finally:
        store.close()


def test_kernel_rejects_config_credentials_already_present_in_the_journal(
    tmp_path: Path,
) -> None:
    protected = "manual-config-journal-sentinel"
    store = SqliteRuntimeStore(tmp_path / "state.db")
    store.create_thread(protected)
    config = ConfigStore(tmp_path)
    config.configure_deepseek(api_key=protected, models=[ModelInput("model", "Model")])

    try:
        with pytest.raises(ConfigError, match="conflict") as captured:
            RuntimeApplication(store, _discard_event, config_store=config)
        assert protected not in str(captured.value)
    finally:
        store.close()


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
    prepared = store.prepare_turn(
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
    kernel = RuntimeApplication(store, event_bus.publish, config_store=config)
    connection = AckFailingConnection(
        [
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {"protocolVersion": 1},
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
    kernel = RuntimeApplication(store, event_bus.publish)
    kernel.start()
    connection = AckFailingConnection(
        [
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {"protocolVersion": 1},
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
    kernel = RuntimeApplication(store, event_bus.publish)
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
                    "params": {"protocolVersion": 1},
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
    kernel = RuntimeApplication(store, event_bus.publish)
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
    kernel = RuntimeApplication(store, event_bus.publish)
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
                "params": {"protocolVersion": 1},
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
                "params": {"protocolVersion": 1},
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
    kernel = RuntimeApplication(store, event_bus.publish)
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
        assert ready["protocolVersion"] == 1
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
    kernel = RuntimeApplication(store, _discard_event)
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
    kernel = RuntimeApplication(store, _discard_event)
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
    kernel = RuntimeApplication(store, _discard_event)
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
    kernel = RuntimeApplication(store, _discard_event)
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
    kernel = RuntimeApplication(store, _discard_event)
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
    kernel = RuntimeApplication(store, _discard_event, config_store=config)
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
    kernel = RuntimeApplication(store, _discard_event)
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
        assert first_types[:4] == [
            "item.completed",
            "run.state_changed",
            "run.state_changed",
            "item.started",
        ]
        assert first_types.count("item.delta") >= 2
        assert first_types[-2:] == ["item.completed", "run.settled"]
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
            key: thread[key]
            for key in ("id", "title", "defaultBranchId", "workspace", "createdAt")
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
            "running",
            "completed",
        ]
        assert len(tool_results) == 1
        result = tool_results[0]["payload"]["item"]["data"]["result"]
        assert result["toolName"] == "process_run"
        assert result["exitCode"] == 0
        assert result["ok"] is True
        assert "gate5-tool-output" in result["stdout"]
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
async def test_cancelling_process_run_preserves_partial_output_and_clears_running_tool(
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
        while True:
            message = json.loads(await asyncio.wait_for(connection.recv(), timeout=10))
            assert message["method"] == "event"
            event = message["params"]
            events.append(event)
            if (
                event["runId"] == run_id
                and event["type"] == "item.started"
                and event["payload"].get("item", {}).get("kind") == "tool_call"
            ):
                break
        await asyncio.sleep(0.75)
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
        )
        tool_result = next(
            event["payload"]["item"]
            for event in run_events
            if event["type"] == "item.completed"
            and event["payload"].get("item", {}).get("kind") == "tool_result"
        )
        settled = [event for event in run_events if event["type"] == "run.settled"]

        assert tool_call_terminal["status"] == "cancelled"
        assert tool_result["status"] == "cancelled"
        assert tool_result["data"]["result"]["cancelled"] is True
        assert "before-stop" in tool_result["data"]["result"]["stdout"]
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
    first_token = secrets.token_urlsafe(32)
    first_process, first_ready = await _start_runtime(first_token, tmp_path)
    first = await _initialize(
        f"ws://{first_ready['host']}:{first_ready['port']}",
        first_token,
    )
    second_process: Process | None = None
    second: ClientConnection | None = None
    try:
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
                "content": "lock-owner-" + ("x" * 24_000),
                "providerId": "scripted",
                "modelId": "scripted-v1",
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
        await first.close()
        if second is not None:
            await second.close()
        await _stop_failed_process(first_process)
        if second_process is not None:
            await _stop_failed_process(second_process)
