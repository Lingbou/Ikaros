from __future__ import annotations

import asyncio
import json
import os
import secrets
import sys
from asyncio.subprocess import Process
from typing import Any

import pytest
from websockets.asyncio.client import connect
from websockets.exceptions import InvalidStatus


async def _start_runtime(token: str) -> tuple[Process, dict[str, Any]]:
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


@pytest.mark.asyncio
async def test_cli_server_requires_authentication_and_completes_handshake() -> None:
    token = secrets.token_urlsafe(32)
    process, ready = await _start_runtime(token)
    uri = f"ws://{ready['host']}:{ready['port']}"

    try:
        assert ready["type"] == "ikaros_runtime.ready"
        assert ready["protocolVersion"] == 1
        assert ready["host"] == "127.0.0.1"
        assert ready["port"] > 0
        assert ready["pid"] > 0

        with pytest.raises(InvalidStatus) as unauthorized:
            async with connect(uri):
                pass
        assert unauthorized.value.response.status_code == 401

        async with connect(uri, additional_headers={"Authorization": f"Bearer {token}"}) as ws:
            await ws.send(
                json.dumps(
                    {
                        "jsonrpc": "2.0",
                        "id": 1,
                        "method": "initialize",
                        "params": {
                            "protocolVersion": 1,
                            "client": {"name": "runtime-test", "version": "0.1.0"},
                        },
                    }
                )
            )
            initialized = json.loads(await ws.recv())
            assert initialized["result"] == {
                "protocolVersion": 1,
                "server": {"name": "ikaros-runtime", "version": "0.1.0"},
                "capabilities": {},
            }

            await ws.send(
                json.dumps(
                    {
                        "jsonrpc": "2.0",
                        "id": 2,
                        "method": "runtime.shutdown",
                        "params": {},
                    }
                )
            )
            shutdown = json.loads(await ws.recv())
            assert shutdown["result"] == {"accepted": True}

        assert await asyncio.wait_for(process.wait(), timeout=5) == 0
    finally:
        await _stop_failed_process(process)
