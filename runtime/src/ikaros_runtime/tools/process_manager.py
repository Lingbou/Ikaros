"""Run-owned commands with durable facts and bounded, continuously drained output."""

from __future__ import annotations

import asyncio
import codecs
import hashlib
import time
from collections.abc import Callable, Sequence
from contextlib import suppress
from dataclasses import dataclass, field
from typing import Literal, Protocol
from uuid import NAMESPACE_URL, uuid5

from ..cancellation import CancellationToken
from ..domain import JsonObject, utc_now
from ..errors import ProtectedValueError
from ..json_codec import dumps as json_dumps
from .core import ToolExecutionContext
from .process_platform import (
    _await_uninterruptibly,
    _resume_spawned_process,
    _spawn_process,
    _SpawnedProcess,
    _terminate_process_tree,
)

_PROCESS_OUTPUT_LIMIT = 1024 * 1024
_STREAM_OUTPUT_LIMIT = 256 * 1024
_PAGE_LIMIT = 16 * 1024
_MAX_RUNNING_PER_RUN = 64

type ProcessState = Literal["running", "exited", "terminated", "unknown"]


class ProcessError(ValueError):
    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code


def _byte_length(value: str) -> int:
    return len(value.encode("utf-8"))


def _prefix_bytes(value: str, maximum: int) -> tuple[str, int]:
    if maximum <= 0 or not value:
        return "", 0
    encoded = value.encode("utf-8")
    if len(encoded) <= maximum:
        return value, len(encoded)
    prefix = encoded[:maximum].decode("utf-8", errors="ignore")
    return prefix, len(prefix.encode("utf-8"))


def _drop_prefix_bytes(value: str, count: int) -> str:
    if count <= 0 or not value:
        return value
    encoded = value.encode("utf-8")
    if count >= len(encoded):
        return ""
    boundary = count
    while boundary < len(encoded) and encoded[boundary] & 0xC0 == 0x80:
        boundary += 1
    return encoded[boundary:].decode("utf-8")


def _slice_from_byte_offset(value: str, offset: int) -> str:
    if offset <= 0:
        return value
    encoded = value.encode("utf-8")
    if offset >= len(encoded):
        return ""
    boundary = offset
    while boundary < len(encoded) and encoded[boundary] & 0xC0 == 0x80:
        boundary += 1
    return encoded[boundary:].decode("utf-8")


class _RetainedOutput:
    """A stable-offset, byte-bounded head/tail output buffer."""

    def __init__(self, limit: int) -> None:
        if limit < 2:
            raise ValueError("retained output limit must be at least two bytes")
        self._limit = limit
        self._head_budget = limit // 2
        self._tail_budget = limit - self._head_budget
        self._head = ""
        self._tail = ""
        self._tail_start = 0
        self._total = 0

    @property
    def total(self) -> int:
        return self._total

    @property
    def omitted(self) -> int:
        return max(0, self._tail_start - _byte_length(self._head))

    @property
    def truncated(self) -> bool:
        return self.omitted > 0

    @property
    def text(self) -> str:
        omitted = self.omitted
        if omitted == 0:
            return self._head + self._tail
        return self._head + self._marker(omitted) + self._tail

    def append(self, value: str) -> bool:
        if not value:
            return False
        value_bytes = _byte_length(value)
        self._total += value_bytes

        remaining_head = self._head_budget - _byte_length(self._head)
        head_part, consumed = _prefix_bytes(value, remaining_head)
        remainder = value[len(head_part) :]
        if head_part:
            self._head += head_part
        if remainder:
            if not self._tail:
                self._tail_start = self._total - _byte_length(remainder)
            self._tail += remainder
            overflow = _byte_length(self._tail) - self._tail_budget
            if overflow > 0:
                dropped = _prefix_bytes(self._tail, overflow)[0]
                self._tail = _drop_prefix_bytes(self._tail, overflow)
                self._tail_start += _byte_length(dropped)
        return True

    def invalid_cursor(self, cursor: int) -> bool:
        return cursor < 0 or cursor > self._total

    def read(self, cursor: int) -> tuple[str, int, bool]:
        if self.invalid_cursor(cursor):
            raise IndexError("cursor")
        if cursor == self._total:
            return "", cursor, False

        head_bytes = _byte_length(self._head)
        tail_start = self._tail_start
        omitted = self.omitted
        if cursor < head_bytes:
            text = _slice_from_byte_offset(self._head, cursor)
            page, consumed = _prefix_bytes(text, _PAGE_LIMIT)
            next_cursor = cursor + consumed
            return page, next_cursor, next_cursor < self._total

        remaining = _PAGE_LIMIT
        parts: list[str] = []
        next_cursor = cursor
        if cursor < tail_start and omitted > 0:
            marker = self._marker(omitted)
            marker_page, marker_bytes = _prefix_bytes(marker, remaining)
            parts.append(marker_page)
            remaining -= marker_bytes
            next_cursor = tail_start
        if remaining > 0 and cursor < self._total:
            if cursor >= tail_start:
                text = _slice_from_byte_offset(self._tail, cursor - tail_start)
                page, consumed = _prefix_bytes(text, remaining)
                parts.append(page)
                next_cursor = cursor + consumed
            elif tail_start < self._total:
                text = self._tail
                page, consumed = _prefix_bytes(text, remaining)
                parts.append(page)
                next_cursor = tail_start + consumed
        return "".join(parts), next_cursor, next_cursor < self._total

    @staticmethod
    def _marker(omitted: int) -> str:
        return f"\n... {omitted} bytes omitted ...\n"


class _StreamGuard(Protocol):
    def feed(self, value: str) -> str: ...
    def finish(self) -> str: ...


@dataclass(slots=True)
class _Entry:
    process_id: str
    context: ToolExecutionContext
    command: str
    cwd: str
    started_at: str
    invocation_fingerprint: str = ""
    state: ProcessState = "unknown"
    error_code: str | None = "start_pending"
    finished_at: str | None = None
    exit_code: int | None = None
    pid: int | None = None
    stdout: _RetainedOutput = field(
        default_factory=lambda: _RetainedOutput(_STREAM_OUTPUT_LIMIT)
    )
    stderr: _RetainedOutput = field(
        default_factory=lambda: _RetainedOutput(_STREAM_OUTPUT_LIMIT)
    )
    output: _RetainedOutput = field(
        default_factory=lambda: _RetainedOutput(_PROCESS_OUTPUT_LIMIT)
    )
    truncated: bool = False
    spawned: _SpawnedProcess | None = None
    monitor: asyncio.Task[None] | None = None
    pumps: list[asyncio.Task[None]] = field(default_factory=list)
    done: asyncio.Event = field(default_factory=asyncio.Event)
    stopping: bool = False
    dirty: bool = False
    last_record: float = 0.0
    output_guard: _StreamGuard | None = None
    output_blocked: bool = False

    def fact(self) -> JsonObject:
        return {
            "processId": self.process_id,
            "threadId": self.context.thread_id,
            "runId": self.context.run_id,
            "stepOrdinal": self.context.step_ordinal,
            "itemId": self.context.item_id,
            "command": self.command,
            "cwd": self.cwd,
            "state": self.state,
            "exitCode": self.exit_code,
            "pid": self.pid,
            "startedAt": self.started_at,
            "finishedAt": self.finished_at,
            "stdout": self.stdout.text,
            "stderr": self.stderr.text,
            "output": self.output.text,
            "truncated": self.truncated,
            "errorCode": self.error_code,
        }


def _now() -> str:
    return utc_now()


class ProcessManager:
    def __init__(
        self,
        *,
        record: Callable[[JsonObject], None] | None = None,
        protected_values: Callable[[], Sequence[str]] | None = None,
        protect: Callable[[str], str] | None = None,
    ) -> None:
        self._record = record
        self._protected_values = protected_values or (lambda: ())
        self._protect = protect
        self._entries: dict[str, _Entry] = {}
        self._start_lock = asyncio.Lock()
        self._closed_runs: set[str] = set()
        self._closed = False

    def _safe(self, text: str) -> str:
        if self._protect is not None:
            text = self._protect(text)
        for value in self._protected_values():
            if value:
                text = text.replace(value, "[redacted]")
        return text

    def _persist(self, entry: _Entry, *, force: bool = False) -> None:
        # At most four output snapshots/second; terminal facts are always immediate.
        if not force and (not entry.dirty or time.monotonic() - entry.last_record < 0.25):
            return
        if self._record is not None:
            self._record(entry.fact())
        entry.dirty = False
        entry.last_record = time.monotonic()

    def restore(self, records: Sequence[JsonObject]) -> None:
        """Set interrupted persisted commands to unknown; never reattach or replay them."""
        for fact in records:
            run_id = str(fact["runId"])
            if any(entry.context.run_id == run_id for entry in self._entries.values()):
                raise ValueError("cannot restore processes for a Run active in this Runtime")
            self._closed_runs.add(run_id)
            if fact["state"] != "running" and fact.get("errorCode") != "start_pending":
                continue
            recovered = dict(fact)
            recovered.update(
                {
                    "state": "unknown",
                    "errorCode": "runtime_interrupted",
                    "exitCode": None,
                    "finishedAt": _now(),
                }
            )
            if self._record is not None:
                self._record(recovered)

    async def start(
        self,
        command: str,
        cwd: str,
        *,
        context: ToolExecutionContext,
        cancellation: CancellationToken,
    ) -> JsonObject:
        from ..security import ProtectedStreamGuard

        # The trusted call Item is the idempotency key, even across restarted Runtime facts.
        process_id = (
            "process_" + uuid5(NAMESPACE_URL, f"ikaros:{context.run_id}:{context.item_id}").hex
        )
        fingerprint = hashlib.sha256(json_dumps([command, cwd]).encode()).hexdigest()
        async with self._start_lock:
            if process_id in self._entries:
                if self._entries[process_id].invocation_fingerprint != fingerprint:
                    raise ProcessError(
                        "process_invocation_conflict",
                        "This tool call Item already started a different command or cwd.",
                    )
                return self.read(process_id, context=context)
            if self._closed or context.run_id in self._closed_runs:
                raise ProcessError(
                    "run_closed", "The owning Run has ended; cannot start a command."
                )
            owned = [e for e in self._entries.values() if e.context.run_id == context.run_id]
            if sum(not e.done.is_set() for e in owned) >= _MAX_RUNNING_PER_RUN:
                raise ProcessError(
                    "process_limit",
                    f"A Run may own at most {_MAX_RUNNING_PER_RUN} active commands.",
                )
            cancellation.raise_if_cancelled()
            entry = _Entry(
                process_id,
                context,
                self._safe(command),
                self._safe(cwd),
                _now(),
                invocation_fingerprint=fingerprint,
            )
            # Record intent before spawning: a crash in the following gap has an unknown result.
            self._persist(entry, force=True)
            self._entries[process_id] = entry
            entry.output_guard = ProtectedStreamGuard(self._protected_values())
            spawn_task = asyncio.create_task(_spawn_process(command, cwd))
            try:
                entry.spawned = await asyncio.shield(spawn_task)
                entry.pid = entry.spawned.process.pid
                cancellation.raise_if_cancelled()
                _resume_spawned_process(entry.spawned)
                entry.state = "running"
                entry.error_code = None
                self._persist(entry, force=True)
                process = entry.spawned.process
                if process.stdout is None or process.stderr is None:
                    raise RuntimeError("process output pipes were not created")
                entry.pumps = [
                    asyncio.create_task(self._drain(entry, process.stdout, "stdout")),
                    asyncio.create_task(self._drain(entry, process.stderr, "stderr")),
                ]
                entry.monitor = asyncio.create_task(self._monitor(entry))
            except BaseException as error:
                if entry.spawned is None:
                    with suppress(BaseException):
                        entry.spawned = await _await_uninterruptibly(spawn_task)
                if entry.spawned is not None:
                    await _await_uninterruptibly(self._kill(entry))
                entry.state = "unknown"
                entry.error_code = (
                    "spawn_failed" if isinstance(error, OSError) else "start_interrupted"
                )
                entry.finished_at = _now()
                entry.done.set()
                with suppress(Exception):
                    self._persist(entry, force=True)
                if isinstance(error, (OSError, ValueError)):
                    raise ProcessError("spawn_failed", self._safe(str(error))) from error
                raise
            return self.read(process_id, context=context)

    def _owned(self, process_id: str, context: ToolExecutionContext) -> _Entry:
        entry = self._entries.get(process_id)
        if entry is None or (
            entry.context.run_id != context.run_id or entry.context.thread_id != context.thread_id
        ):
            raise ProcessError(
                "process_not_found", "No command with that processId belongs to this Run."
            )
        return entry

    def read(
        self, process_id: str, *, context: ToolExecutionContext, cursor: int = 0
    ) -> JsonObject:
        entry = self._owned(process_id, context)
        try:
            page, next_cursor, has_more = entry.output.read(cursor)
        except IndexError:
            raise ProcessError(
                "invalid_cursor", "cursor is outside the retained command output."
            ) from None
        return {
            "processId": process_id,
            "state": entry.state,
            "exitCode": entry.exit_code,
            "cwd": entry.cwd,
            "pid": entry.pid,
            "startedAt": entry.started_at,
            "finishedAt": entry.finished_at,
            "output": page,
            "cursor": cursor,
            "nextCursor": next_cursor,
            "hasMore": has_more,
            "truncated": entry.truncated,
            "errorCode": entry.error_code,
        }

    def read_for_thread(self, process_id: str, *, thread_id: str, cursor: int = 0) -> JsonObject:
        """Read a process from the desktop control plane using thread ownership."""
        entry = self._entries.get(process_id)
        if entry is None or entry.context.thread_id != thread_id:
            raise ProcessError(
                "process_not_found", "No command with that processId belongs to this thread."
            )
        return self.read(process_id, context=entry.context, cursor=cursor)

    async def wait(
        self,
        process_id: str,
        *,
        context: ToolExecutionContext,
        cancellation: CancellationToken,
        timeout_ms: int = 1000,
        cursor: int = 0,
    ) -> JsonObject:
        entry = self._owned(process_id, context)
        self.read(process_id, context=context, cursor=cursor)
        cancellation.raise_if_cancelled()
        done_task = asyncio.create_task(entry.done.wait())
        cancel_task = asyncio.create_task(cancellation.wait())
        try:
            await asyncio.wait(
                {done_task, cancel_task},
                timeout=timeout_ms / 1000,
                return_when=asyncio.FIRST_COMPLETED,
            )
            cancellation.raise_if_cancelled()
        finally:
            done_task.cancel()
            cancel_task.cancel()
            await asyncio.gather(done_task, cancel_task, return_exceptions=True)
        return self.read(process_id, context=context, cursor=cursor)

    async def stop(
        self, process_id: str, *, context: ToolExecutionContext, cursor: int = 0
    ) -> JsonObject:
        entry = self._owned(process_id, context)
        self.read(process_id, context=context, cursor=cursor)
        await self._stop(entry)
        return self.read(process_id, context=context, cursor=cursor)

    async def stop_for_thread(
        self, process_id: str, *, thread_id: str, cursor: int = 0
    ) -> JsonObject:
        entry = self._entries.get(process_id)
        if entry is None or entry.context.thread_id != thread_id:
            raise ProcessError(
                "process_not_found", "No command with that processId belongs to this thread."
            )
        return await self.stop(process_id, context=entry.context, cursor=cursor)

    async def _stop(self, entry: _Entry) -> None:
        if entry.done.is_set():
            return
        entry.stopping = True
        await _await_uninterruptibly(self._kill(entry))
        if entry.monitor is not None:
            await _await_uninterruptibly(entry.monitor)

    async def close_run(self, run_id: str) -> None:
        async with self._start_lock:
            self._closed_runs.add(run_id)
            entries = [entry for entry in self._entries.values() if entry.context.run_id == run_id]
        await asyncio.gather(*(self._stop(entry) for entry in entries))
        # Durable history is in the repository; retaining every Run's buffers leaks memory.
        for entry in entries:
            self._entries.pop(entry.process_id, None)

    async def close(self) -> None:
        self._closed = True
        await asyncio.gather(
            *(
                self.close_run(run_id)
                for run_id in {entry.context.run_id for entry in self._entries.values()}
            )
        )

    async def _kill(self, entry: _Entry) -> None:
        if entry.spawned is None:
            return
        try:
            await asyncio.wait_for(_terminate_process_tree(entry.spawned), timeout=6)
        except (OSError, TimeoutError):
            entry.error_code = "process_cleanup_failed"
            # Even a detached descendant holding the output pipes cannot block shutdown.
            transport = getattr(entry.spawned.process, "_transport", None)
            if transport is not None:
                transport.close()

    async def _monitor(self, entry: _Entry) -> None:
        assert entry.spawned is not None
        try:
            # Process.wait may wait for inherited pipes after root exit. Watch the root's
            # exit status directly and terminate all remaining children before joining pipes.
            while entry.spawned.process.returncode is None:
                if entry.error_code == "process_cleanup_failed":
                    raise RuntimeError("process tree could not be confirmed stopped")
                if any(
                    task.done() and not task.cancelled() and task.exception()
                    for task in entry.pumps
                ):
                    raise RuntimeError("command output persistence failed")
                self._persist(entry)
                await asyncio.sleep(0.02)
            await self._kill(entry)
            await asyncio.wait_for(asyncio.gather(*entry.pumps), timeout=2)
            entry.exit_code = entry.spawned.process.returncode
            entry.state = "terminated" if entry.stopping else "exited"
            if entry.error_code == "process_cleanup_failed":
                entry.state = "unknown"
            elif entry.exit_code != 0 and entry.error_code is None:
                entry.error_code = "terminated" if entry.stopping else "non_zero_exit"
        except (Exception, asyncio.CancelledError):
            await _await_uninterruptibly(self._kill(entry))
            entry.state = "unknown"
            entry.error_code = entry.error_code or "process_monitor_failed"
        finally:
            for task in entry.pumps:
                task.cancel()
            await asyncio.gather(*entry.pumps, return_exceptions=True)
            if entry.output_guard is not None and not entry.output_blocked:
                entry.output.append(entry.output_guard.finish())
            entry.finished_at = _now()
            try:
                self._persist(entry, force=True)
            except Exception:
                entry.state = "unknown"
                entry.error_code = "process_record_failed"
            entry.done.set()

    async def _drain(
        self, entry: _Entry, stream: asyncio.StreamReader, name: Literal["stdout", "stderr"]
    ) -> None:
        # Local import avoids security -> run_input -> tools package initialization cycle.
        from ..security import ProtectedStreamGuard

        guard: _StreamGuard = ProtectedStreamGuard(self._protected_values())
        decoder = codecs.getincrementaldecoder("utf-8")(errors="replace")
        blocked = False
        while True:
            chunk = await stream.read(16 * 1024)
            if not chunk:
                break
            if blocked:
                continue
            try:
                safe = guard.feed(decoder.decode(chunk))
            except ProtectedValueError:
                blocked = True
                entry.error_code = "protected_output"
                entry.truncated = True
                entry.dirty = True
                self._persist(entry)
                continue
            self._append(entry, name, safe)
        if not blocked:
            try:
                self._append(entry, name, guard.feed(decoder.decode(b"", final=True)))
                self._append(entry, name, guard.finish())
            except ProtectedValueError:
                entry.error_code = "protected_output"
                entry.truncated = True
                entry.dirty = True

    def _append(self, entry: _Entry, name: Literal["stdout", "stderr"], text: str) -> None:
        changed = getattr(entry, name).append(text)
        if entry.output_guard is not None and not entry.output_blocked:
            try:
                changed = entry.output.append(entry.output_guard.feed(text)) or changed
            except ProtectedValueError:
                entry.output_blocked = True
                entry.error_code = "protected_output"
                changed = True
        else:
            changed = entry.output.append(text) or changed
        entry.truncated = (
            entry.truncated
            or entry.output.truncated
            or entry.stdout.truncated
            or entry.stderr.truncated
        )
        if changed:
            entry.dirty = True
            self._persist(entry)
