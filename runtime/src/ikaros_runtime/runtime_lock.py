from __future__ import annotations

import asyncio
import errno
import math
import os
from contextlib import suppress
from importlib import import_module
from pathlib import Path
from typing import Any, BinaryIO, Self

_LOCK_FILENAME = "runtime.lock"
_DEFAULT_TIMEOUT_SECONDS = 10.0
_POLL_INTERVAL_SECONDS = 0.05
_BUSY_ERRNOS = frozenset(
    {
        errno.EACCES,
        errno.EAGAIN,
        getattr(errno, "EDEADLK", errno.EAGAIN),
    }
)
_BUSY_WINDOWS_ERRORS = frozenset({32, 33, 36})
_LOCKING: Any = import_module("msvcrt" if os.name == "nt" else "fcntl")


class RuntimeHomeLockError(RuntimeError):
    """The Runtime home could not be exclusively owned by this process."""


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
        timeout_seconds: float = _DEFAULT_TIMEOUT_SECONDS,
        poll_interval_seconds: float = _POLL_INTERVAL_SECONDS,
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
