from __future__ import annotations

import asyncio
import ctypes
import logging
import math
import os
import secrets
import sys
from collections.abc import Callable
from ctypes import wintypes
from dataclasses import dataclass
from http import HTTPStatus
from pathlib import Path
from typing import Any, cast

from websockets.asyncio.server import ServerConnection, serve
from websockets.exceptions import ConnectionClosed
from websockets.http11 import Request, Response

from . import __version__
from .config import ConfigError, ConfigStore
from .domain import JournalEvent
from .json_codec import dumps as json_dumps
from .json_codec import loads as json_loads
from .kernel import CommandOutcome, InvalidParamsError, RuntimeKernel
from .runtime_lock import RuntimeHomeLock
from .storage import SqliteRuntimeStore

PROTOCOL_VERSION = 1
_LOGGER = logging.getLogger("ikaros_runtime")
_LOOPBACK_HOSTS = frozenset({"127.0.0.1", "::1"})
_EVENT_QUEUE_LIMIT = 1024
_SEND_TIMEOUT_SECONDS = 5.0

EventSink = Callable[[JournalEvent], None]


class EventBus:
    def __init__(self, *, next_seq: int) -> None:
        if next_seq < 1:
            raise ValueError("next event sequence must be positive")
        self._sinks: set[EventSink] = set()
        self._pending: dict[int, JournalEvent] = {}
        self._next_seq = next_seq
        self._lock = asyncio.Lock()

    def subscribe(self, sink: EventSink) -> Callable[[], None]:
        self._sinks.add(sink)

        def unsubscribe() -> None:
            self._sinks.discard(sink)

        return unsubscribe

    async def publish(self, event: JournalEvent) -> None:
        async with self._lock:
            if event.seq < self._next_seq or event.seq in self._pending:
                return
            self._pending[event.seq] = event
            while ready := self._pending.pop(self._next_seq, None):
                self._next_seq += 1
                for sink in tuple(self._sinks):
                    try:
                        sink(ready)
                    except Exception:
                        self._sinks.discard(sink)


@dataclass(frozen=True, slots=True)
class ServerSettings:
    host: str
    port: int
    token: str
    parent_pid: int
    runtime_home: Path

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
    if sys.platform == "win32":
        return _windows_process_is_alive(parent_pid)
    try:
        os.kill(parent_pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def _windows_process_is_alive(process_id: int) -> bool:
    process_query_limited_information = 0x1000
    still_active = 259
    error_access_denied = 5

    win_dll = cast(Callable[..., Any], ctypes.__dict__["WinDLL"])
    get_last_error = cast(Callable[[], int], ctypes.__dict__["get_last_error"])
    kernel32 = win_dll("kernel32", use_last_error=True)
    open_process = kernel32.OpenProcess
    open_process.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    open_process.restype = wintypes.HANDLE
    get_exit_code_process = kernel32.GetExitCodeProcess
    get_exit_code_process.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
    get_exit_code_process.restype = wintypes.BOOL
    close_handle = kernel32.CloseHandle
    close_handle.argtypes = [wintypes.HANDLE]
    close_handle.restype = wintypes.BOOL

    process_handle = open_process(process_query_limited_information, False, process_id)
    if not process_handle:
        return get_last_error() == error_access_denied
    try:
        exit_code = wintypes.DWORD()
        if not get_exit_code_process(process_handle, ctypes.byref(exit_code)):
            return True
        return exit_code.value == still_active
    finally:
        close_handle(process_handle)


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


def _valid_request_id(value: object) -> bool:
    if value is None or isinstance(value, str):
        return True
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return False
    return not isinstance(value, float) or math.isfinite(value)


def _initialize_result() -> dict[str, object]:
    return {
        "protocolVersion": PROTOCOL_VERSION,
        "server": {"name": "ikaros-runtime", "version": __version__},
        "capabilities": {
            "threads": True,
            "turns": True,
            "eventReplay": True,
            "streaming": True,
            "scriptedProvider": True,
            "tools": ["process.run"],
            "executionPolicy": "full_access",
            "runCancellation": True,
            "providers": True,
            "models": True,
        },
    }


def _configuration_write_values(method: object, params: object) -> tuple[str, ...]:
    if method != "provider.configure" or not isinstance(params, dict):
        return ()
    values: list[str] = []
    api_key = params.get("apiKey")
    if isinstance(api_key, str) and api_key:
        values.append(api_key)
    headers = params.get("headers")
    if isinstance(headers, dict):
        values.extend(value for value in headers.values() if isinstance(value, str) and value)
    return tuple(dict.fromkeys(values))


async def _handle_connection(
    connection: ServerConnection,
    stop_event: asyncio.Event,
    kernel: RuntimeKernel,
    event_bus: EventBus,
) -> None:
    initialized = False
    send_lock = asyncio.Lock()
    unsubscribe: Callable[[], None] | None = None
    event_queue: asyncio.Queue[JournalEvent] = asyncio.Queue(maxsize=_EVENT_QUEUE_LIMIT)
    event_sender: asyncio.Task[None] | None = None
    overflow_close: asyncio.Task[None] | None = None

    async def send_json(
        value: dict[str, object],
        *,
        additional_protected_values: tuple[str, ...] = (),
    ) -> None:
        if kernel.response_contains_protected_value(value, additional_protected_values):
            value = _jsonrpc_error(
                None,
                -32603,
                "response contained protected configuration data",
            )
        try:
            payload = json_dumps(value, separators=(",", ":"))
        except (TypeError, ValueError):
            payload = json_dumps(
                _jsonrpc_error(None, -32603, "internal error"),
                separators=(",", ":"),
            )
        async with send_lock:
            await connection.send(payload)

    async def send_event(event: JournalEvent) -> None:
        await send_json({"jsonrpc": "2.0", "method": "event", "params": event.to_wire()})

    def enqueue_event(event: JournalEvent) -> None:
        nonlocal overflow_close
        try:
            event_queue.put_nowait(event)
        except asyncio.QueueFull:
            if overflow_close is None:
                overflow_close = asyncio.create_task(
                    connection.close(code=1013, reason="event consumer too slow")
                )
            raise

    async def send_events() -> None:
        try:
            while True:
                event = await event_queue.get()
                try:
                    async with asyncio.timeout(_SEND_TIMEOUT_SECONDS):
                        await send_event(event)
                finally:
                    event_queue.task_done()
        except asyncio.CancelledError:
            raise
        except Exception:
            await connection.close(code=1011, reason="event stream failed")

    try:
        async for raw_message in connection:
            if not isinstance(raw_message, str):
                await send_json(_jsonrpc_error(None, -32600, "text messages only"))
                continue

            try:
                request: Any = json_loads(raw_message)
            except ValueError:
                await send_json(_jsonrpc_error(None, -32700, "parse error"))
                continue

            if not isinstance(request, dict) or request.get("jsonrpc") != "2.0":
                await send_json(_jsonrpc_error(None, -32600, "invalid request"))
                continue

            request_id = request.get("id")
            method = request.get("method")
            params = request.get("params", {})
            outcome: CommandOutcome | None = None

            if not _valid_request_id(request_id):
                await send_json(_jsonrpc_error(None, -32600, "invalid request id"))
                continue

            configuration_values = (
                *kernel.protected_values(),
                *_configuration_write_values(method, params),
            )
            if kernel.rpc_request_values_contain_protected_value(
                request_id,
                configuration_values,
            ):
                await send_json(
                    _jsonrpc_error(
                        None,
                        -32600,
                        "request contains protected configuration data",
                    )
                )
                continue

            if not isinstance(method, str):
                await send_json(_jsonrpc_error(None, -32600, "invalid request method"))
                continue

            if method == "initialize":
                if (
                    not isinstance(params, dict)
                    or params.get("protocolVersion") != PROTOCOL_VERSION
                ):
                    response = _jsonrpc_error(request_id, -32001, "unsupported protocol version")
                elif initialized:
                    response = _jsonrpc_error(request_id, -32002, "connection already initialized")
                else:
                    initialized = True
                    response = {"jsonrpc": "2.0", "id": request_id, "result": _initialize_result()}
            elif not initialized:
                response = _jsonrpc_error(request_id, -32000, "initialize must be called first")
            elif not isinstance(params, dict):
                response = _jsonrpc_error(request_id, -32602, "params must be an object")
            else:
                try:
                    if method == "runtime.shutdown":
                        result = {"accepted": True}
                    elif method == "thread.create":
                        outcome = kernel.create_thread(params)
                        result = outcome.result
                    elif method == "thread.list":
                        result = kernel.list_threads(params)
                    elif method == "provider.list":
                        result = kernel.list_providers(params)
                    elif method == "provider.configure":
                        result = kernel.configure_provider(params)
                    elif method == "provider.disconnect":
                        result = kernel.disconnect_provider(params)
                    elif method == "provider.remove":
                        result = kernel.remove_provider(params)
                    elif method == "model.list":
                        result = kernel.list_models(params)
                    elif method == "model.set_enabled":
                        result = kernel.set_model_enabled(params)
                    elif method == "turn.start":
                        outcome = kernel.start_turn(params)
                        result = outcome.result
                    elif method == "run.cancel":
                        outcome = kernel.cancel_run(params)
                        result = outcome.result
                    elif method == "event.replay":
                        result = kernel.replay_events(params)
                    else:
                        response = _jsonrpc_error(request_id, -32601, "method not found")
                        result = None
                    if result is not None:
                        response = {"jsonrpc": "2.0", "id": request_id, "result": result}
                except InvalidParamsError as error:
                    response = _jsonrpc_error(request_id, -32602, str(error))
                except Exception:
                    _LOGGER.error("Unhandled Runtime method failure")
                    response = _jsonrpc_error(request_id, -32603, "internal error")

            initialized_now = (
                method == "initialize" and "result" in response and unsubscribe is None
            )
            if initialized_now:
                unsubscribe = event_bus.subscribe(enqueue_event)
            accepted_outcome = outcome if outcome is not None and "result" in response else None
            shutdown_accepted = method == "runtime.shutdown" and "result" in response
            try:
                async with asyncio.timeout(_SEND_TIMEOUT_SECONDS):
                    await send_json(
                        response,
                        additional_protected_values=_configuration_write_values(method, params),
                    )
            finally:
                if accepted_outcome is not None:
                    await kernel.finish_command(accepted_outcome)
                if shutdown_accepted:
                    stop_event.set()
            if initialized_now:
                event_sender = asyncio.create_task(
                    send_events(),
                    name="ikaros-runtime-event-sender",
                )
            if shutdown_accepted:
                return
    except ConnectionClosed:
        return
    finally:
        if unsubscribe is not None:
            unsubscribe()
        if event_sender is not None:
            event_sender.cancel()
            await asyncio.gather(event_sender, return_exceptions=True)
        if overflow_close is not None:
            await asyncio.gather(overflow_close, return_exceptions=True)


async def run_server(settings: ServerSettings) -> None:
    settings.validate()
    runtime_lock = await RuntimeHomeLock.acquire(settings.runtime_home)
    store: SqliteRuntimeStore | None = None
    kernel: RuntimeKernel | None = None
    try:
        logging.basicConfig(level=logging.INFO, stream=sys.stderr)
        stop_event = asyncio.Event()
        expected_authorization = f"Bearer {settings.token}"
        config_store = ConfigStore(settings.runtime_home)
        store = SqliteRuntimeStore(settings.runtime_home / "state.db")
        if store.journal_contains_protected_values(config_store.protected_values()):
            raise ConfigError("configured credentials conflict with persisted Runtime data")
        recovery = store.recover_incomplete_runs()
        event_bus = EventBus(next_seq=store.latest_sequence() + 1)
        kernel = RuntimeKernel(store, event_bus.publish, config_store=config_store)
        kernel.start(recovery.queued_run_ids)

        def authenticate(connection: ServerConnection, request: Request) -> Response | None:
            authorization = request.headers.get("Authorization", "")
            if secrets.compare_digest(authorization, expected_authorization):
                return None
            return connection.respond(HTTPStatus.UNAUTHORIZED, "Unauthorized\n")

        async with serve(
            lambda connection: _handle_connection(connection, stop_event, kernel, event_bus),
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
            print(json_dumps(readiness, separators=(",", ":")), flush=True)
            parent_watcher = asyncio.create_task(_watch_parent(settings.parent_pid, stop_event))
            try:
                await stop_event.wait()
            finally:
                parent_watcher.cancel()
                await asyncio.gather(parent_watcher, return_exceptions=True)
    finally:
        if kernel is not None:
            await kernel.close()
        if store is not None:
            store.close()
        runtime_lock.release()
