from __future__ import annotations

import asyncio
import ctypes
import errno
import logging
import math
import os
import secrets
import sys
from collections.abc import Awaitable, Callable
from contextlib import suppress
from ctypes import wintypes
from dataclasses import dataclass
from http import HTTPStatus
from importlib import import_module
from pathlib import Path
from typing import Any, BinaryIO, Self, cast

from websockets.asyncio.server import ServerConnection, serve
from websockets.http11 import Request, Response

from ..errors import RuntimeHomeLockError
from ..json_codec import dumps as json_dumps
from ..protocol.jsonrpc import PROTOCOL_VERSION

_LOGGER = logging.getLogger("ikaros_runtime")
_LOOPBACK_HOSTS = frozenset({"127.0.0.1", "::1"})
_LOCK_FILENAME = "runtime.lock"
_DEFAULT_LOCK_TIMEOUT_SECONDS = 10.0
_LOCK_POLL_INTERVAL_SECONDS = 0.05
_BUSY_ERRNOS = frozenset(
    {
        errno.EACCES,
        errno.EAGAIN,
        getattr(errno, "EDEADLK", errno.EAGAIN),
    }
)
_BUSY_WINDOWS_ERRORS = frozenset({32, 33, 36})
_LOCKING: Any = import_module("msvcrt" if os.name == "nt" else "fcntl")

type ConnectionHandler = Callable[[ServerConnection, asyncio.Event], Awaitable[None]]
type ParentProbe = Callable[[int], bool]


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


class RuntimeHomeLock:
    """A process-lifetime, cross-platform lock for one Ikaros Runtime home."""

    __slots__ = ("_handle", "_path")

    def __init__(self, path: Path, handle: BinaryIO) -> None:
        self._path = path
        self._handle: BinaryIO | None = handle

    @property
    def path(self) -> Path:
        return self._path

    @property
    def released(self) -> bool:
        return self._handle is None

    @classmethod
    async def acquire(
        cls,
        runtime_home: Path,
        *,
        timeout_seconds: float = _DEFAULT_LOCK_TIMEOUT_SECONDS,
        poll_interval_seconds: float = _LOCK_POLL_INTERVAL_SECONDS,
    ) -> Self:
        if not math.isfinite(timeout_seconds) or timeout_seconds < 0:
            raise ValueError("lock timeout must be a finite non-negative number")
        if not math.isfinite(poll_interval_seconds) or poll_interval_seconds <= 0:
            raise ValueError("lock poll interval must be a finite positive number")

        try:
            path, handle = _open_lock_file(runtime_home)
        except OSError:
            raise RuntimeHomeLockError("Runtime home lock could not be opened") from None

        loop = asyncio.get_running_loop()
        deadline = loop.time() + timeout_seconds
        try:
            while True:
                try:
                    _try_lock(handle)
                except OSError as error:
                    if not _is_lock_busy(error):
                        raise RuntimeHomeLockError(
                            "Runtime home lock could not be acquired"
                        ) from None
                    remaining = deadline - loop.time()
                    if remaining <= 0:
                        raise RuntimeHomeLockError("Runtime home is already in use") from None
                    await asyncio.sleep(min(poll_interval_seconds, remaining))
                    continue
                return cls(path, handle)
        except BaseException:
            with suppress(OSError):
                handle.close()
            raise

    def release(self) -> None:
        handle = self._handle
        if handle is None:
            return
        self._handle = None
        with suppress(OSError):
            _unlock(handle)
        with suppress(OSError):
            handle.close()

    def __enter__(self) -> Self:
        if self.released:
            raise RuntimeError("Runtime home lock has already been released")
        return self

    def __exit__(self, *_exc_info: object) -> None:
        self.release()


class RuntimeServerHost:
    """Own the authenticated WebSocket listener and its stop lifecycle."""

    def __init__(
        self,
        settings: ServerSettings,
        connection_handler: ConnectionHandler,
        *,
        parent_probe: ParentProbe = lambda parent_pid: _parent_is_alive(parent_pid),
    ) -> None:
        self._settings = settings
        self._connection_handler = connection_handler
        self._parent_probe = parent_probe
        self._stop_event: asyncio.Event | None = None

    @property
    def stop_event(self) -> asyncio.Event | None:
        return self._stop_event

    def request_shutdown(self) -> None:
        if self._stop_event is not None:
            self._stop_event.set()

    async def run(self) -> None:
        self._settings.validate()
        if self._stop_event is not None:
            raise RuntimeError("Runtime server host can only be run once")

        stop_event = asyncio.Event()
        self._stop_event = stop_event
        expected_authorization = f"Bearer {self._settings.token}"

        def authenticate(connection: ServerConnection, request: Request) -> Response | None:
            authorization = request.headers.get("Authorization", "")
            if secrets.compare_digest(authorization, expected_authorization):
                return None
            return connection.respond(HTTPStatus.UNAUTHORIZED, "Unauthorized\n")

        async def handle_connection(connection: ServerConnection) -> None:
            await self._connection_handler(connection, stop_event)

        async with serve(
            handle_connection,
            self._settings.host,
            self._settings.port,
            process_request=authenticate,
        ) as server:
            socket = next(iter(server.sockets))
            selected_port = int(socket.getsockname()[1])
            readiness = {
                "type": "ikaros_runtime.ready",
                "protocolVersion": PROTOCOL_VERSION,
                "host": self._settings.host,
                "port": selected_port,
                "pid": os.getpid(),
            }
            print(json_dumps(readiness, separators=(",", ":")), flush=True)

            parent_watcher = asyncio.create_task(
                _watch_parent(
                    self._settings.parent_pid,
                    stop_event,
                    parent_probe=self._parent_probe,
                ),
                name="ikaros-runtime-parent-watcher",
            )
            try:
                await stop_event.wait()
            finally:
                parent_watcher.cancel()
                await asyncio.gather(parent_watcher, return_exceptions=True)


async def run_host(settings: ServerSettings, connection_handler: ConnectionHandler) -> None:
    await RuntimeServerHost(settings, connection_handler).run()


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


async def _watch_parent(
    parent_pid: int,
    stop_event: asyncio.Event,
    *,
    parent_probe: ParentProbe = lambda process_id: _parent_is_alive(process_id),
) -> None:
    while not stop_event.is_set():
        await asyncio.sleep(1)
        if not parent_probe(parent_pid):
            _LOGGER.info("Desktop parent process exited; stopping Runtime.")
            stop_event.set()


def _open_lock_file(runtime_home: Path) -> tuple[Path, BinaryIO]:
    runtime_home.mkdir(parents=True, exist_ok=True)
    path = runtime_home / _LOCK_FILENAME
    handle = path.open("a+b", buffering=0)
    try:
        _prepare_lock_file(handle)
    except BaseException:
        handle.close()
        raise
    return path, handle


def _prepare_lock_file(handle: BinaryIO) -> None:
    handle.seek(0, os.SEEK_END)
    if handle.tell() == 0:
        handle.write(b"\0")
        handle.flush()
    handle.seek(0)


def _try_lock(handle: BinaryIO) -> None:
    handle.seek(0)
    if os.name == "nt":
        _LOCKING.locking(handle.fileno(), _LOCKING.LK_NBLCK, 1)
        return
    _LOCKING.flock(handle.fileno(), _LOCKING.LOCK_EX | _LOCKING.LOCK_NB)


def _unlock(handle: BinaryIO) -> None:
    handle.seek(0)
    if os.name == "nt":
        _LOCKING.locking(handle.fileno(), _LOCKING.LK_UNLCK, 1)
        return
    _LOCKING.flock(handle.fileno(), _LOCKING.LOCK_UN)


def _is_lock_busy(error: OSError) -> bool:
    return error.errno in _BUSY_ERRNOS or getattr(error, "winerror", None) in _BUSY_WINDOWS_ERRORS
