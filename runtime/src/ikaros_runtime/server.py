from __future__ import annotations

import asyncio
import json
import logging
import os
import secrets
import sys
from dataclasses import dataclass
from http import HTTPStatus
from typing import Any

from websockets.asyncio.server import ServerConnection, serve
from websockets.http11 import Request, Response

from . import __version__

PROTOCOL_VERSION = 1
_LOGGER = logging.getLogger("ikaros_runtime")
_LOOPBACK_HOSTS = frozenset({"127.0.0.1", "::1"})


@dataclass(frozen=True, slots=True)
class ServerSettings:
    host: str
    port: int
    token: str
    parent_pid: int

    def validate(self) -> None:
        if self.host not in _LOOPBACK_HOSTS:
            raise ValueError("the Runtime server must bind to a loopback address")
        if not 0 <= self.port <= 65535:
            raise ValueError("port must be between 0 and 65535")
        if not self.token:
            raise ValueError("launch token must not be empty")
        if self.parent_pid <= 0:
            raise ValueError("parent PID must be positive")


def _parent_is_alive(parent_pid: int) -> bool:
    try:
        os.kill(parent_pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


async def _watch_parent(parent_pid: int, stop_event: asyncio.Event) -> None:
    while not stop_event.is_set():
        await asyncio.sleep(1)
        if not _parent_is_alive(parent_pid):
            _LOGGER.info("Desktop parent process exited; stopping Runtime.")
            stop_event.set()


def _jsonrpc_error(request_id: object, code: int, message: str) -> dict[str, object]:
    return {
        "jsonrpc": "2.0",
        "id": request_id,
        "error": {"code": code, "message": message},
    }


def _initialize_result() -> dict[str, object]:
    return {
        "protocolVersion": PROTOCOL_VERSION,
        "server": {"name": "ikaros-runtime", "version": __version__},
        "capabilities": {},
    }


async def _handle_connection(connection: ServerConnection, stop_event: asyncio.Event) -> None:
    initialized = False
    async for raw_message in connection:
        if not isinstance(raw_message, str):
            await connection.send(json.dumps(_jsonrpc_error(None, -32600, "text messages only")))
            continue

        try:
            request: Any = json.loads(raw_message)
        except json.JSONDecodeError:
            await connection.send(json.dumps(_jsonrpc_error(None, -32700, "parse error")))
            continue

        if not isinstance(request, dict) or request.get("jsonrpc") != "2.0":
            await connection.send(json.dumps(_jsonrpc_error(None, -32600, "invalid request")))
            continue

        request_id = request.get("id")
        method = request.get("method")
        params = request.get("params", {})

        if method == "initialize":
            if not isinstance(params, dict) or params.get("protocolVersion") != PROTOCOL_VERSION:
                response = _jsonrpc_error(request_id, -32001, "unsupported protocol version")
            elif initialized:
                response = _jsonrpc_error(request_id, -32002, "connection already initialized")
            else:
                initialized = True
                response = {"jsonrpc": "2.0", "id": request_id, "result": _initialize_result()}
        elif not initialized:
            response = _jsonrpc_error(request_id, -32000, "initialize must be called first")
        elif method == "runtime.shutdown":
            response = {"jsonrpc": "2.0", "id": request_id, "result": {"accepted": True}}
        else:
            response = _jsonrpc_error(request_id, -32601, "method not found")

        await connection.send(json.dumps(response, separators=(",", ":")))
        if method == "runtime.shutdown" and "result" in response:
            stop_event.set()
            return


async def run_server(settings: ServerSettings) -> None:
    settings.validate()
    logging.basicConfig(level=logging.INFO, stream=sys.stderr)
    stop_event = asyncio.Event()
    expected_authorization = f"Bearer {settings.token}"

    def authenticate(connection: ServerConnection, request: Request) -> Response | None:
        authorization = request.headers.get("Authorization", "")
        if secrets.compare_digest(authorization, expected_authorization):
            return None
        return connection.respond(HTTPStatus.UNAUTHORIZED, "Unauthorized\n")

    async with serve(
        lambda connection: _handle_connection(connection, stop_event),
        settings.host,
        settings.port,
        process_request=authenticate,
    ) as server:
        socket = next(iter(server.sockets))
        selected_port = int(socket.getsockname()[1])
        readiness = {
            "type": "ikaros_runtime.ready",
            "protocolVersion": PROTOCOL_VERSION,
            "host": settings.host,
            "port": selected_port,
            "pid": os.getpid(),
        }
        print(json.dumps(readiness, separators=(",", ":")), flush=True)
        parent_watcher = asyncio.create_task(_watch_parent(settings.parent_pid, stop_event))
        try:
            await stop_event.wait()
        finally:
            parent_watcher.cancel()
            await asyncio.gather(parent_watcher, return_exceptions=True)
