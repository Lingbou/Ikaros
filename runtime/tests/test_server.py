from __future__ import annotations

import asyncio
import json
import os
import secrets
import sys
from asyncio.subprocess import Process
from pathlib import Path
from typing import Any

import pytest
from websockets.asyncio.client import ClientConnection, connect
from websockets.exceptions import InvalidStatus


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
        "--token",
        token,
        "--parent-pid",
        str(os.getpid()),
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
        env={**os.environ, "IKAROS_HOME": str(runtime_home)},
    )
    assert process.stdout is not None
    line = await asyncio.wait_for(process.stdout.readline(), timeout=10)
    if not line:
        assert process.stderr is not None
        stderr = (await process.stderr.read()).decode(errors="replace")
        raise AssertionError(f"Runtime exited before readiness: {stderr}")
    return process, json.loads(line)


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
) -> dict[str, Any]:
    await connection.send(
        json.dumps(
            {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}
        )
    )
    response = json.loads(await connection.recv())
    assert response["jsonrpc"] == "2.0"
    assert response["id"] == request_id
    return response


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
        "capabilities": {"threads": True, "eventReplay": True},
    }
    return connection


async def _shutdown(connection: ClientConnection, process: Process, request_id: int) -> None:
    shutdown = await _rpc(connection, request_id, "runtime.shutdown", {})
    assert shutdown["result"] == {"accepted": True}
    await connection.close()
    assert await asyncio.wait_for(process.wait(), timeout=5) == 0


@pytest.mark.asyncio
async def test_cli_server_requires_authentication_and_completes_handshake(
    tmp_path: Path,
) -> None:
    token = secrets.token_urlsafe(32)
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
        assert empty["result"] == {"threads": []}

        created = await _rpc(first, 3, "thread.create", {"title": "Persistent thread"})
        thread = created["result"]["thread"]
        event = created["result"]["event"]
        assert thread["title"] == "Persistent thread"
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
        assert listed["result"] == {"threads": [thread]}

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
