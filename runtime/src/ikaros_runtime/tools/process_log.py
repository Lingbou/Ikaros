"""Durable per-process output: one append-only log file plus a metadata sidecar.

Every managed command owns ``<home>/processes/<threadId>/<processId>.log`` and a
``<processId>.json`` sidecar. Non-terminal commands write their stdout/stderr
straight into the log file, so output produced while the Runtime is down is
still readable after a restart; terminal (PTY) sessions append through the
Runtime. The Runtime keeps the file bounded by folding it back to the retained
head/tail window, which never rewrites the head prefix and therefore keeps
absolute byte cursors stable.
"""

from __future__ import annotations

import os
import re
from collections.abc import Mapping
from contextlib import suppress
from pathlib import Path
from typing import IO

from ..json_codec import dumps as json_dumps
from ..json_codec import loads as json_loads

PROCESS_LOG_LIMIT = 1024 * 1024
PROCESS_LOG_READ_CHUNK = 64 * 1024
PROCESS_LOG_METADATA_VERSION = 1

_THREAD_SEGMENT = re.compile(r"\A[A-Za-z0-9][A-Za-z0-9._-]{0,127}\Z")
_PROCESS_ID = re.compile(r"\Aprocess_[0-9a-f]{32}\Z")


class ProcessLogError(ValueError):
    """Raised when a process log path is unsafe or the log cannot be used."""


class ProcessLog:
    """One process's durable output file and its metadata sidecar."""

    def __init__(self, directory: Path, process_id: str, *, limit: int = PROCESS_LOG_LIMIT) -> None:
        self.directory = directory
        self.process_id = process_id
        self.limit = limit
        self.path = directory / f"{process_id}.log"
        self.metadata_path = directory / f"{process_id}.json"
        self._append: IO[bytes] | None = None
        self._tail: IO[bytes] | None = None
        self._tail_offset = 0

    @classmethod
    def open(
        cls,
        root: Path,
        thread_id: str,
        process_id: str,
        *,
        limit: int = PROCESS_LOG_LIMIT,
    ) -> ProcessLog:
        """Resolve one process log below ``root``; reject anything that escapes it."""
        if not _THREAD_SEGMENT.fullmatch(thread_id):
            raise ProcessLogError("threadId is not a safe path segment")
        if not _PROCESS_ID.fullmatch(process_id):
            raise ProcessLogError("processId is not a safe path segment")
        base = Path(root).expanduser().resolve()
        directory = (base / thread_id).resolve()
        if not directory.is_relative_to(base):
            raise ProcessLogError("process log directory escapes the process root")
        return cls(directory, process_id, limit=limit)

    # -- lifecycle ---------------------------------------------------------

    def prepare(self) -> None:
        self.directory.mkdir(parents=True, exist_ok=True)
        if not self.path.exists():
            with self.path.open("ab"):
                pass

    def child_handle(self) -> IO[bytes]:
        """An append-mode handle handed to the child as stdout/stderr."""
        if self._append is None:
            self.prepare()
            self._append = self.path.open("ab", buffering=0)
        return self._append

    def append(self, data: bytes) -> None:
        """Runtime-side append (terminal sessions own no child file handle)."""
        if data:
            self.child_handle().write(data)

    def close(self) -> None:
        for handle in (self._append, self._tail):
            if handle is not None:
                with suppress(OSError):
                    handle.close()
        self._append = None
        self._tail = None

    # -- reading -----------------------------------------------------------

    def size(self) -> int:
        try:
            return self.path.stat().st_size
        except OSError:
            return 0

    def read_at(self, offset: int, length: int) -> bytes:
        if length <= 0 or offset < 0:
            return b""
        try:
            with self.path.open("rb", buffering=0) as handle:
                handle.seek(offset)
                return handle.read(length)
        except OSError:
            return b""

    def start_tail(self, offset: int) -> None:
        """Follow the file from ``offset``; new bytes arrive as the child appends."""
        self.prepare()
        if self._tail is not None:
            self._tail.close()
        self._tail = self.path.open("rb", buffering=0)
        self._tail_offset = max(0, offset)
        self._tail.seek(self._tail_offset)

    def tail_chunk(self, maximum: int = PROCESS_LOG_READ_CHUNK) -> bytes:
        if self._tail is None:
            raise ProcessLogError("process log tail was not started")
        chunk = self._tail.read(maximum)
        self._tail_offset += len(chunk)
        return chunk

    @property
    def tail_offset(self) -> int:
        return self._tail_offset

    # -- bounding ----------------------------------------------------------

    def compact(self, *, head_bytes: int, tail_bytes: int) -> bool:
        """Fold the file back to its retained head plus its most recent tail.

        Only the bytes between the head prefix and the retained tail are dropped,
        so every previously published cursor still maps onto the same content.
        """
        size = self.size()
        head = max(0, min(head_bytes, size))
        tail = max(0, min(tail_bytes, size - head))
        if size <= head + tail:
            return False
        start = size - tail
        try:
            with self.path.open("r+b", buffering=0) as handle:
                handle.seek(start)
                retained = handle.read(tail)
                handle.seek(head)
                handle.write(retained)
                handle.truncate(head + len(retained))
        except OSError as error:
            raise ProcessLogError(f"process log could not be compacted: {error}") from error
        self._tail_offset = head + len(retained)
        if self._tail is not None:
            self._tail.seek(self._tail_offset)
        return True

    # -- metadata ----------------------------------------------------------

    def write_metadata(self, value: Mapping[str, object]) -> None:
        self.directory.mkdir(parents=True, exist_ok=True)
        payload = dict(value)
        payload.setdefault("metadataVersion", PROCESS_LOG_METADATA_VERSION)
        temporary = self.metadata_path.with_name(f"{self.metadata_path.name}.tmp")
        try:
            temporary.write_bytes(json_dumps(payload).encode("utf-8"))
            os.replace(temporary, self.metadata_path)
        except OSError:
            with suppress(OSError):
                temporary.unlink()

    def read_metadata(self) -> dict[str, object] | None:
        try:
            raw = self.metadata_path.read_bytes()
        except OSError:
            return None
        try:
            value = json_loads(raw.decode("utf-8"))
        except ValueError:
            return None
        return value if isinstance(value, dict) else None
