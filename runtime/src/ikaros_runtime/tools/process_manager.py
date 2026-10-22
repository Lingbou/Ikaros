"""Thread-owned commands with durable facts, file-backed output, and supervision."""

from __future__ import annotations

import asyncio
import codecs
import hashlib
import time
from collections.abc import Callable, Sequence
from contextlib import suppress
from dataclasses import dataclass, field
from pathlib import Path
from typing import Literal, Protocol
from uuid import NAMESPACE_URL, uuid5

from ..cancellation import CancellationToken
from ..domain import JsonObject, utc_now
from ..errors import ProtectedValueError
from ..json_codec import dumps as json_dumps
from .core import ToolExecutionContext
from .process_log import PROCESS_LOG_LIMIT, ProcessLog, ProcessLogError
from .process_platform import (
    PTY_DEFAULT_COLUMNS,
    PTY_DEFAULT_ROWS,
    PTY_MAX_CELLS,
    PTY_MIN_CELLS,
    _await_uninterruptibly,
    _PtySession,
    _resume_spawned_process,
    _spawn_process,
    _spawn_pty,
    _SpawnedProcess,
    _terminate_process_tree,
    probe_custody,
    terminate_custody_tree,
)

_PROCESS_OUTPUT_LIMIT = PROCESS_LOG_LIMIT
_PAGE_LIMIT = 16 * 1024
_MAX_RUNNING_PER_THREAD = 64
_TERMINAL_RETENTION_SECONDS = 1800
_MAX_RETAINED_ENTRIES = 256
SUPERVISE_BACKOFF_DEFAULT_MS = 500
_SUPERVISE_BACKOFF_DEFAULT_MS = SUPERVISE_BACKOFF_DEFAULT_MS
_SUPERVISE_BACKOFF_MAX_MS = 30_000
_SUPERVISE_MAX_RESTARTS = 10
_SUPERVISE_WINDOW_SECONDS = 600
_LOG_POLL_SECONDS = 0.02
_RECOVERED_POLL_SECONDS = 0.5

type ProcessState = Literal["running", "exited", "terminated", "unknown"]

SUPERVISE_POLICIES = frozenset({"always", "on-failure"})
SUPERVISE_BACKOFF_MIN_MS = 100
SUPERVISE_BACKOFF_MAX_MS = 60_000



class ProcessError(ValueError):
    def __init__(self, code: str, message: str) -> None:
        super().__init__(message)
        self.code = code


def _trim_utf8_tail(value: bytes) -> bytes:
    """Drop a truncated multi-byte sequence at the end of ``value``."""
    for back in range(1, min(4, len(value)) + 1):
        byte = value[-back]
        if byte & 0xC0 != 0x80:
            lead = byte
            expected = 1
            if lead >= 0xF0:
                expected = 4
            elif lead >= 0xE0:
                expected = 3
            elif lead >= 0xC0:
                expected = 2
            return value if back >= expected else value[:-back]
    return value


def _drop_utf8_continuation(value: bytes, count: int) -> bytes:
    """Drop ``count`` bytes, then any continuation bytes of a split sequence."""
    if count <= 0:
        return value
    if count >= len(value):
        return b""
    boundary = count
    while boundary < len(value) and value[boundary] & 0xC0 == 0x80:
        boundary += 1
    return value[boundary:]


def _slice_utf8(value: bytes, offset: int) -> bytes:
    if offset <= 0:
        return value
    if offset >= len(value):
        return b""
    boundary = offset
    while boundary < len(value) and value[boundary] & 0xC0 == 0x80:
        boundary += 1
    return value[boundary:]


class _RetainedOutput:
    """A stable-offset, byte-bounded head/tail view of one output stream."""

    def __init__(self, limit: int) -> None:
        if limit < 2:
            raise ValueError("retained output limit must be at least two bytes")
        self._head_budget = limit // 2
        self._tail_budget = limit - self._head_budget
        self._head = b""
        self._tail = b""
        self._tail_start = 0
        self._total = 0

    @classmethod
    def restored(
        cls, *, head: bytes, tail: bytes, tail_start: int, total: int, limit: int
    ) -> _RetainedOutput:
        value = cls(limit)
        value._head = _trim_utf8_tail(head)
        value._tail = tail
        value._tail_start = max(0, tail_start)
        value._total = max(total, len(value._head) + len(value._tail))
        return value

    @property
    def total(self) -> int:
        return self._total

    @property
    def head_bytes(self) -> int:
        return len(self._head)

    @property
    def tail_bytes(self) -> int:
        return len(self._tail)

    @property
    def tail_start(self) -> int:
        return self._tail_start

    @property
    def omitted(self) -> int:
        return max(0, self._tail_start - len(self._head))

    @property
    def truncated(self) -> bool:
        return self.omitted > 0

    @property
    def head(self) -> bytes:
        return self._head

    @property
    def tail(self) -> bytes:
        return self._tail

    @property
    def text(self) -> str:
        omitted = self.omitted
        body = self._head + self._tail
        if omitted == 0:
            return body.decode("utf-8", errors="replace")
        marker = self._marker(omitted)
        return (
            self._head.decode("utf-8", errors="replace")
            + marker
            + self._tail.decode("utf-8", errors="replace")
        )

    def append(self, value: bytes) -> int:
        """Append raw bytes; return how many leading tail bytes had to be dropped."""
        if not value:
            return 0
        self._total += len(value)
        remaining_head = self._head_budget - len(self._head)
        head_part = _trim_utf8_tail(value[:remaining_head])
        remainder = value[len(head_part) :]
        if head_part:
            self._head += head_part
        if not remainder:
            return 0
        if not self._tail:
            self._tail_start = self._total - len(remainder)
        self._tail += remainder
        overflow = len(self._tail) - self._tail_budget
        if overflow <= 0:
            return 0
        before = len(self._tail)
        self._tail = _drop_utf8_continuation(self._tail, overflow)
        dropped = before - len(self._tail)
        self._tail_start += dropped
        return dropped

    def invalid_cursor(self, cursor: int) -> bool:
        return cursor < 0 or cursor > self._total

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
    raw_command: str
    cwd: str
    raw_cwd: str
    started_at: str
    invocation_fingerprint: str = ""
    background: bool = False
    supervise: str | None = None
    backoff_ms: int = _SUPERVISE_BACKOFF_DEFAULT_MS
    restart_count: int = 0
    last_restart_at: str | None = None
    restart_times: list[float] = field(default_factory=list)
    pid_started_at: str = ""
    state: ProcessState = "unknown"
    error_code: str | None = "start_pending"
    finished_at: str | None = None
    exit_code: int | None = None
    pid: int | None = None
    pty_mode: bool = False
    columns: int = PTY_DEFAULT_COLUMNS
    rows: int = PTY_DEFAULT_ROWS
    output: _RetainedOutput = field(
        default_factory=lambda: _RetainedOutput(_PROCESS_OUTPUT_LIMIT)
    )
    truncated: bool = False
    file_bytes: int = 0
    log_offset: int = 0
    command_redacted: bool = False
    log: ProcessLog | None = None
    compacted: bool = False
    tail_file_offset: int = 0
    spawned: _SpawnedProcess | None = None
    pty: _PtySession | None = None
    monitor: asyncio.Task[None] | None = None
    watch: asyncio.Task[None] | None = None
    pumps: list[asyncio.Task[None]] = field(default_factory=list)
    done: asyncio.Event = field(default_factory=asyncio.Event)
    stop_requested: asyncio.Event = field(default_factory=asyncio.Event)
    stopping: bool = False
    recovered: bool = False
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
            "background": self.background,
            "supervise": self.supervise,
            "invocationRedacted": self.command_redacted,
            "restartCount": self.restart_count,
            "lastRestartAt": self.last_restart_at,
            "state": self.state,
            "exitCode": self.exit_code,
            "pid": self.pid,
            "pidStartedAt": self.pid_started_at or self.started_at,
            "startedAt": self.started_at,
            "finishedAt": self.finished_at,
            "output": self.output.text,
            "outputBytes": self.output.total,
            "headBytes": self.output.head_bytes,
            "tailStart": self.output.tail_start,
            # The retained window holds decoded text, so replacement decoding can
            # make it larger than the raw log file. The window came from that file
            # and cannot exceed it, so report a size that is consistent with it.
            "fileBytes": max(
                self.file_bytes,
                self.output.head_bytes,
                self.output.tail_start,
            ),
            "truncated": self.truncated,
            "errorCode": self.error_code,
        }


def _now() -> str:
    return utc_now()


def _child_returncode(entry: _Entry) -> int | None:
    if entry.pty is not None:
        return entry.pty.returncode
    if entry.spawned is not None:
        return entry.spawned.process.returncode
    return None


def _policy_or_none(value: object) -> str | None:
    return value if isinstance(value, str) and value in SUPERVISE_POLICIES else None


def _optional_str(value: object) -> str | None:
    return value if isinstance(value, str) and value else None


def _restore_output_from_fact(fact: JsonObject) -> _RetainedOutput:
    """Rebuild the retained head/tail view stored in one process fact."""
    text = str(fact.get("output", ""))
    raw = text.encode("utf-8")
    total = int(fact.get("outputBytes", len(raw)))
    if not bool(fact.get("truncated", False)):
        return _RetainedOutput.restored(
            head=raw,
            tail=b"",
            tail_start=0,
            total=max(total, len(raw)),
            limit=_PROCESS_OUTPUT_LIMIT,
        )
    head_bytes = max(0, int(fact.get("headBytes", 0)))
    tail_start = max(head_bytes, int(fact.get("tailStart", head_bytes)))
    marker = _RetainedOutput._marker(max(0, tail_start - head_bytes)).encode("utf-8")
    head = raw[:head_bytes]
    tail = raw[head_bytes + len(marker) :]
    return _RetainedOutput.restored(
        head=head,
        tail=tail,
        tail_start=tail_start,
        total=total,
        limit=_PROCESS_OUTPUT_LIMIT,
    )


def _supervise_should_restart(policy: str | None, exit_code: int | None) -> bool:
    if policy == "always":
        return True
    if policy == "on-failure":
        return exit_code != 0
    return False


def _page_from(
    out: _RetainedOutput,
    cursor: int,
    *,
    log: ProcessLog | None,
    tail_file_offset: int,
) -> tuple[str, int, bool]:
    """Map an absolute byte cursor onto the retained head/tail window."""
    if out.invalid_cursor(cursor):
        raise IndexError("cursor")
    if cursor == out.total:
        return "", cursor, False
    head_bytes = out.head_bytes
    tail_start = out.tail_start
    tail_bytes = out.tail_bytes
    omitted = out.omitted
    if cursor < head_bytes:
        if log is not None:
            data = log.read_at(cursor, min(head_bytes - cursor, _PAGE_LIMIT))
        else:
            data = out.head[cursor : cursor + _PAGE_LIMIT]
        page = _trim_utf8_tail(data)
        next_cursor = cursor + len(page)
        return page.decode("utf-8", errors="replace"), next_cursor, next_cursor < out.total

    remaining = _PAGE_LIMIT
    parts: list[bytes] = []
    next_cursor = cursor
    if cursor < tail_start and omitted > 0:
        marker = _RetainedOutput._marker(omitted).encode("utf-8")
        marker_page = marker[:remaining]
        parts.append(marker_page)
        remaining -= len(marker_page)
        next_cursor = tail_start
    if remaining > 0 and cursor < out.total:
        if cursor >= tail_start:
            offset = tail_file_offset + (cursor - tail_start)
            length = min(tail_bytes - (cursor - tail_start), remaining)
            if log is not None:
                data = log.read_at(offset, length)
            else:
                data = out.tail[cursor - tail_start : cursor - tail_start + length]
            page = _trim_utf8_tail(data)
            parts.append(page)
            next_cursor = cursor + len(page)
        elif tail_start < out.total:
            if log is not None:
                data = log.read_at(tail_file_offset, min(tail_bytes, remaining))
            else:
                data = out.tail[:remaining]
            page = _trim_utf8_tail(data)
            parts.append(page)
            next_cursor = tail_start + len(page)
    return (
        b"".join(parts).decode("utf-8", errors="replace"),
        next_cursor,
        next_cursor < out.total,
    )


def read_fact_page(
    fact: JsonObject, cursor: int, *, log: ProcessLog | None = None
) -> tuple[str, int, bool]:
    """Page a persisted process fact using the same mapping as the live manager."""
    out = _restore_output_from_fact(fact)
    tail_bytes = out.tail_bytes
    file_bytes = max(0, int(fact.get("fileBytes", 0)))
    tail_file_offset = (
        out.head_bytes if tail_bytes == 0 else max(out.head_bytes, file_bytes - tail_bytes)
    )
    return _page_from(out, cursor, log=log, tail_file_offset=tail_file_offset)
class ProcessManager:
    def __init__(
        self,
        *,
        logs_root: Path,
        record: Callable[[JsonObject], None] | None = None,
        protected_values: Callable[[], Sequence[str]] | None = None,
        protect: Callable[[str], str] | None = None,
    ) -> None:
        self._logs_root = Path(logs_root)
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
        fact = entry.fact()
        if self._record is not None:
            self._record(fact)
        if entry.log is not None:
            metadata = dict(fact)
            metadata["tailFileOffset"] = entry.tail_file_offset
            metadata["updatedAt"] = _now()
            with suppress(Exception):
                entry.log.write_metadata(metadata)
        entry.dirty = False
        entry.last_record = time.monotonic()

    # -- restore -----------------------------------------------------------

    def restore(self, records: Sequence[JsonObject]) -> None:
        """Reconcile persisted commands after a Runtime restart.

        Output files keep growing while the Runtime is down. A live command is
        re-adopted by custody and its log is tailed again; a supervised command
        that died in the meantime is restarted; everything else settles to
        ``unknown`` after folding any output written during downtime into the
        retained window.
        """
        for fact in records:
            run_id = str(fact["runId"])
            if any(entry.context.run_id == run_id for entry in self._entries.values()):
                raise ValueError("cannot restore processes for a Run active in this Runtime")
            self._closed_runs.add(run_id)
            if fact["state"] != "running" and fact.get("errorCode") != "start_pending":
                continue
            entry = self._entry_from_fact(fact)
            if fact["state"] == "running" and self._adopt(entry):
                self._start_log_pump(entry)
                self._start_recovered_watch(entry)
                continue
            self._drain_log_once(entry)
            self._maybe_compact(entry)
            self._restart_or_settle_recovered(entry)

    def _entry_from_fact(self, fact: JsonObject) -> _Entry:
        entry = _Entry(
            process_id=str(fact["processId"]),
            context=ToolExecutionContext(
                run_id=str(fact["runId"]),
                step_ordinal=int(fact["stepOrdinal"]),
                item_id=str(fact["itemId"]),
                thread_id=str(fact["threadId"]),
            ),
            command=str(fact["command"]),
            raw_command=str(fact["command"]),
            cwd=str(fact["cwd"]),
            raw_cwd=str(fact["cwd"]),
            started_at=str(fact["startedAt"]),
            background=bool(fact.get("background", False)),
            supervise=_policy_or_none(fact.get("supervise")),
            restart_count=max(0, int(fact.get("restartCount", 0) or 0)),
            last_restart_at=_optional_str(fact.get("lastRestartAt")),
            pid_started_at=_optional_str(fact.get("pidStartedAt"))
            or str(fact.get("startedAt", "")),
            command_redacted=bool(fact.get("invocationRedacted", False)),
        )
        entry.output = _restore_output_from_fact(fact)
        entry.truncated = bool(fact.get("truncated", False))
        entry.file_bytes = max(0, int(fact.get("fileBytes", 0)))
        entry.log_offset = entry.file_bytes
        if fact["state"] == "running":
            entry.state = "running"
            entry.error_code = _optional_str(fact.get("errorCode"))
            pid = fact.get("pid")
            entry.pid = pid if isinstance(pid, int) and not isinstance(pid, bool) else None
        entry.log = self._open_log(entry)
        entry.compacted = bool(fact.get("truncated", False))
        entry.tail_file_offset = self._tail_file_offset(entry)
        return entry

    def _open_log(self, entry: _Entry) -> ProcessLog | None:
        try:
            log = ProcessLog.open(self._logs_root, entry.context.thread_id, entry.process_id)
            log.prepare()
            return log
        except (ProcessLogError, OSError):
            return None

    @staticmethod
    def _tail_file_offset(entry: _Entry) -> int:
        head_bytes = entry.output.head_bytes
        tail_bytes = entry.output.tail_bytes
        if tail_bytes == 0:
            return head_bytes
        return max(head_bytes, entry.file_bytes - tail_bytes)

    def _adopt(self, entry: _Entry) -> bool:
        pid = entry.pid
        if not isinstance(pid, int) or isinstance(pid, bool) or pid <= 0:
            return False
        if not entry.pid_started_at:
            return False
        if probe_custody(pid, entry.pid_started_at) != "alive":
            return False
        entry.recovered = True
        entry.error_code = "runtime_recovered"
        self._entries[entry.process_id] = entry
        self._persist(entry, force=True)
        return True

    def _drain_log_once(self, entry: _Entry) -> None:
        """Fold output written while the Runtime was down into the retained window."""
        if entry.log is None:
            return
        try:
            entry.log.start_tail(entry.log_offset)
        except (ProcessLogError, OSError):
            return
        guard = self._new_guard()
        decoder = codecs.getincrementaldecoder("utf-8")(errors="replace")
        while True:
            try:
                chunk = entry.log.tail_chunk()
            except (ProcessLogError, OSError):
                return
            if not chunk:
                break
            entry.file_bytes = entry.log.tail_offset
            try:
                text = guard.feed(decoder.decode(chunk))
            except ProtectedValueError:
                self._block_output(entry)
                return
            if text:
                with suppress(Exception):
                    self._ingest(entry, text)

    def _restart_or_settle_recovered(self, entry: _Entry) -> None:
        """Handle a supervised command that died while the Runtime was down."""
        if (
            entry.supervise is not None
            and entry.pid is not None
            and not entry.command_redacted
            and not self._closed
            and self._within_restart_budget(entry)
        ):
            entry.error_code = None
            entry.state = "running"
            entry.recovered = False
            entry.restart_count += 1
            entry.restart_times.append(time.monotonic())
            entry.last_restart_at = _now()
            entry.log_offset = entry.file_bytes
            entry.done.clear()
            self._entries[entry.process_id] = entry
            entry.monitor = asyncio.create_task(self._restart_recovered(entry))
            return
        reason = "runtime_interrupted"
        if entry.supervise is not None:
            reason = (
                "supervise_storm"
                if not self._within_restart_budget(entry)
                else "supervise_unavailable"
            )
        recovered = entry.fact()
        recovered.update(
            {
                "state": "unknown",
                "errorCode": reason,
                "exitCode": None,
                "finishedAt": _now(),
            }
        )
        if self._record is not None:
            self._record(recovered)

    async def _restart_recovered(self, entry: _Entry) -> None:
        if entry.stopping:
            self._settle_supervised(entry, state="terminated")
            return
        try:
            await self._spawn_incarnation(entry, restart=True)
            self._start_pumps(entry)
        except (Exception, asyncio.CancelledError):
            entry.state = "unknown"
            entry.error_code = "supervise_restart_failed"
            entry.finished_at = _now()
            with suppress(Exception):
                self._persist(entry, force=True)
            entry.done.set()
            return
        if entry.stopping:
            # process_stop arrived while the replacement was spawning.
            await _await_uninterruptibly(self._kill(entry))
            self._settle_supervised(entry, state="terminated")
            return
        entry.monitor = asyncio.create_task(self._monitor(entry))

    def _settle_supervised(self, entry: _Entry, *, state: ProcessState) -> None:
        entry.state = state
        entry.error_code = None if state == "terminated" else entry.error_code
        entry.finished_at = _now()
        for task in entry.pumps:
            task.cancel()
        entry.done.set()
        with suppress(Exception):
            self._persist(entry, force=True)
    # -- start -------------------------------------------------------------

    async def start(
        self,
        command: str,
        cwd: str,
        *,
        context: ToolExecutionContext,
        cancellation: CancellationToken,
        background: bool = False,
        pty: bool = False,
        columns: int = PTY_DEFAULT_COLUMNS,
        rows: int = PTY_DEFAULT_ROWS,
        supervise: str | None = None,
        backoff_ms: int = _SUPERVISE_BACKOFF_DEFAULT_MS,
    ) -> JsonObject:
        from ..security import ProtectedStreamGuard

        if supervise is not None and supervise not in SUPERVISE_POLICIES:
            raise ProcessError("invalid_arguments", "supervise policy is invalid.")
        if pty and supervise is not None:
            raise ProcessError(
                "invalid_arguments", "supervise is unavailable for terminal commands."
            )
        # The trusted call Item is the idempotency key, even across restarted Runtime facts.
        process_id = (
            "process_" + uuid5(NAMESPACE_URL, f"ikaros:{context.run_id}:{context.item_id}").hex
        )
        if pty and not (
            PTY_MIN_CELLS <= columns <= PTY_MAX_CELLS
            and PTY_MIN_CELLS <= rows <= PTY_MAX_CELLS
        ):
            raise ProcessError("invalid_arguments", "terminal size is out of range.")
        fingerprint = hashlib.sha256(
            json_dumps(
                [command, cwd, background, pty, columns, rows, supervise, backoff_ms]
            ).encode()
        ).hexdigest()
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
            self._prune()
            owned = [e for e in self._entries.values() if e.context.thread_id == context.thread_id]
            if sum(not e.done.is_set() for e in owned) >= _MAX_RUNNING_PER_THREAD:
                raise ProcessError(
                    "process_limit",
                    f"A thread may own at most {_MAX_RUNNING_PER_THREAD} active commands.",
                )
            cancellation.raise_if_cancelled()
            try:
                log = ProcessLog.open(self._logs_root, context.thread_id, process_id)
                log.prepare()
            except (ProcessLogError, OSError) as error:
                raise ProcessError("process_log_unavailable", self._safe(str(error))) from error
            safe_command = self._safe(command)
            safe_cwd = self._safe(cwd)
            entry = _Entry(
                process_id=process_id,
                context=context,
                command=safe_command,
                raw_command=command,
                cwd=safe_cwd,
                raw_cwd=cwd,
                started_at=_now(),
                invocation_fingerprint=fingerprint,
                background=background,
                supervise=supervise,
                backoff_ms=backoff_ms,
                pty_mode=pty,
                columns=columns,
                rows=rows,
                log=log,
                command_redacted=safe_command != command or safe_cwd != cwd,
            )
            entry.output_guard = ProtectedStreamGuard(self._protected_values())
            # Record intent before spawning: a crash in the following gap has an unknown result.
            self._persist(entry, force=True)
            self._entries[process_id] = entry
            try:
                await self._spawn_incarnation(entry, restart=False)
                entry.state = "running"
                entry.error_code = None
                self._persist(entry, force=True)
                self._start_pumps(entry)
                entry.monitor = asyncio.create_task(self._monitor(entry))
            except BaseException as error:
                await _await_uninterruptibly(self._kill(entry))
                if entry.pty is not None:
                    await _await_uninterruptibly(entry.pty.finish())
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

    async def _spawn_incarnation(self, entry: _Entry, *, restart: bool) -> None:
        assert entry.log is not None
        entry.pid_started_at = _now()
        entry.stopping = False
        if entry.pty_mode:
            pty_task = asyncio.create_task(
                _spawn_pty(entry.raw_command, entry.raw_cwd, columns=entry.columns, rows=entry.rows)
            )
            try:
                entry.pty = await asyncio.shield(pty_task)
            except BaseException:
                with suppress(BaseException):
                    entry.pty = await _await_uninterruptibly(pty_task)
                    entry.pid = entry.pty.pid
                raise
            entry.pid = entry.pty.pid
        else:
            spawn_task = asyncio.create_task(
                _spawn_process(entry.raw_command, entry.raw_cwd, log=entry.log.child_handle())
            )
            try:
                spawned = await asyncio.shield(spawn_task)
            except BaseException:
                # A cancelled start still owns whatever the spawn already created.
                with suppress(BaseException):
                    entry.spawned = await _await_uninterruptibly(spawn_task)
                    entry.pid = entry.spawned.process.pid
                raise
            entry.spawned = spawned
            entry.pid = spawned.process.pid
            _resume_spawned_process(spawned)
        if restart:
            entry.error_code = None
            self._persist(entry, force=True)

    def _start_pumps(self, entry: _Entry) -> None:
        entry.pumps = [asyncio.create_task(self._pump(entry))]

    def _start_log_pump(self, entry: _Entry) -> None:
        entry.pumps = [asyncio.create_task(self._pump(entry))]

    def _start_recovered_watch(self, entry: _Entry) -> None:
        entry.watch = asyncio.create_task(self._watch_recovered(entry))

    async def _pump(self, entry: _Entry) -> None:
        if entry.pty is not None:
            await self._drain_terminal(entry)
        else:
            await self._drain_log(entry)

    def _new_guard(self) -> _StreamGuard:
        from ..security import ProtectedStreamGuard

        return ProtectedStreamGuard(self._protected_values())

    async def _drain_log(self, entry: _Entry) -> None:
        log = entry.log
        if log is None:
            return
        guard = entry.output_guard or self._new_guard()
        entry.output_guard = guard
        decoder = codecs.getincrementaldecoder("utf-8")(errors="replace")
        try:
            log.start_tail(entry.log_offset)
        except (ProcessLogError, OSError):
            entry.error_code = entry.error_code or "process_output_unavailable"
            return
        while True:
            try:
                chunk = log.tail_chunk()
            except (ProcessLogError, OSError):
                entry.error_code = entry.error_code or "process_output_unavailable"
                return
            if not chunk:
                if entry.stopping or _child_returncode(entry) is not None:
                    break
                self._maybe_compact(entry)
                await asyncio.sleep(_LOG_POLL_SECONDS)
                continue
            entry.file_bytes = log.tail_offset
            if entry.output_blocked:
                continue
            try:
                safe = guard.feed(decoder.decode(chunk))
            except ProtectedValueError:
                self._block_output(entry)
                continue
            if safe:
                self._ingest(entry, safe)
        if entry.output_blocked:
            return
        try:
            remainder = guard.feed(decoder.decode(b"", final=True)) + guard.finish()
        except ProtectedValueError:
            self._block_output(entry)
            return
        if remainder:
            self._ingest(entry, remainder)
        self._maybe_compact(entry)

    async def _drain_terminal(self, entry: _Entry) -> None:
        assert entry.pty is not None
        guard = entry.output_guard or self._new_guard()
        entry.output_guard = guard
        decoder = codecs.getincrementaldecoder("utf-8")(errors="replace")
        while True:
            chunk = await entry.pty.output.read(16 * 1024)
            if not chunk:
                break
            if entry.output_blocked:
                continue
            try:
                safe = guard.feed(decoder.decode(chunk))
            except ProtectedValueError:
                self._block_output(entry)
                continue
            if safe:
                self._ingest(entry, safe)
        if entry.output_blocked:
            return
        try:
            remainder = guard.feed(decoder.decode(b"", final=True)) + guard.finish()
        except ProtectedValueError:
            self._block_output(entry)
            return
        if remainder:
            self._ingest(entry, remainder)
        self._maybe_compact(entry)

    def _ingest(self, entry: _Entry, text: str) -> None:
        if not text:
            return
        raw = text.encode("utf-8")
        if entry.pty is not None and entry.log is not None:
            with suppress(Exception):
                entry.log.append(raw)
                entry.file_bytes = entry.log.tail_offset
        out = entry.output
        was_empty = out.tail_bytes == 0
        dropped = out.append(raw)
        if entry.compacted:
            entry.tail_file_offset += dropped
        elif was_empty and out.tail_bytes > 0:
            entry.tail_file_offset = out.tail_start
        entry.truncated = entry.truncated or out.truncated
        entry.dirty = True
        self._persist(entry)

    def _block_output(self, entry: _Entry) -> None:
        entry.output_blocked = True
        entry.error_code = "protected_output"
        entry.truncated = True
        entry.dirty = True
        with suppress(Exception):
            self._persist(entry)

    def _maybe_compact(self, entry: _Entry) -> None:
        if entry.log is None:
            return
        if entry.log.size() <= _PROCESS_OUTPUT_LIMIT:
            return
        out = entry.output
        try:
            compacted = entry.log.compact(head_bytes=out.head_bytes, tail_bytes=out.tail_bytes)
        except ProcessLogError:
            return
        if compacted:
            entry.compacted = True
            entry.tail_file_offset = out.head_bytes
            entry.file_bytes = entry.log.size()
            entry.log_offset = entry.file_bytes
    # -- tool operations ---------------------------------------------------

    def _owned(self, process_id: str, context: ToolExecutionContext) -> _Entry:
        entry = self._entries.get(process_id)
        if entry is None or entry.context.thread_id != context.thread_id:
            raise ProcessError(
                "process_not_found", "No command with that processId belongs to this thread."
            )
        return entry

    def read(
        self, process_id: str, *, context: ToolExecutionContext, cursor: int = 0
    ) -> JsonObject:
        entry = self._owned(process_id, context)
        try:
            page, next_cursor, has_more = self._page(entry, cursor)
        except IndexError:
            raise ProcessError(
                "invalid_cursor", "cursor is outside the retained command output."
            ) from None
        return {
            "processId": process_id,
            "background": entry.background,
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
            "supervise": entry.supervise,
            "restartCount": entry.restart_count,
            "lastRestartAt": entry.last_restart_at,
        }

    def read_for_thread(self, process_id: str, *, thread_id: str, cursor: int = 0) -> JsonObject:
        """Read a process from the desktop control plane using thread ownership."""
        entry = self._entries.get(process_id)
        if entry is None or entry.context.thread_id != thread_id:
            raise ProcessError(
                "process_not_found", "No command with that processId belongs to this thread."
            )
        return self.read(process_id, context=entry.context, cursor=cursor)

    def _page(self, entry: _Entry, cursor: int) -> tuple[str, int, bool]:
        return _page_from(
            entry.output,
            cursor,
            log=entry.log,
            tail_file_offset=entry.tail_file_offset,
        )

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

    async def write(
        self,
        process_id: str,
        *,
        context: ToolExecutionContext,
        data: str,
        cursor: int = 0,
    ) -> JsonObject:
        entry = self._owned(process_id, context)
        self.read(process_id, context=context, cursor=cursor)
        if entry.recovered:
            raise ProcessError(
                "process_recovered",
                "The command survived a Runtime restart; live input is unavailable.",
            )
        if entry.pty is None:
            raise ProcessError(
                "process_not_terminal", "Only a terminal command accepts input."
            )
        if entry.done.is_set():
            raise ProcessError("process_closed", "The command has already finished.")
        try:
            await entry.pty.write(data.encode("utf-8"))
        except OSError as error:
            raise ProcessError("process_input_failed", self._safe(str(error))) from error
        return self.read(process_id, context=context, cursor=cursor)

    async def resize(
        self,
        process_id: str,
        *,
        context: ToolExecutionContext,
        columns: int,
        rows: int,
        cursor: int = 0,
    ) -> JsonObject:
        entry = self._owned(process_id, context)
        self.read(process_id, context=context, cursor=cursor)
        if entry.recovered:
            raise ProcessError(
                "process_recovered",
                "The command survived a Runtime restart; live window control is unavailable.",
            )
        if entry.pty is None:
            raise ProcessError(
                "process_not_terminal", "Only a terminal command has a window size."
            )
        if entry.done.is_set():
            raise ProcessError("process_closed", "The command has already finished.")
        try:
            await entry.pty.resize(columns, rows)
        except OSError as error:
            raise ProcessError("process_resize_failed", self._safe(str(error))) from error
        return self.read(process_id, context=context, cursor=cursor)

    async def _stop(self, entry: _Entry) -> None:
        entry.stopping = True
        entry.stop_requested.set()
        if entry.recovered and entry.spawned is None and entry.pty is None:
            if entry.state != "running":
                return
            await self._settle_recovered(entry)
            return
        if entry.done.is_set():
            return
        await _await_uninterruptibly(self._kill(entry))
        if entry.monitor is not None:
            await _await_uninterruptibly(entry.monitor)

    async def _settle_recovered(self, entry: _Entry) -> None:
        """Stop a command that was re-adopted after a Runtime restart."""
        assert entry.pid is not None
        outcome = await terminate_custody_tree(entry.pid, entry.pid_started_at)
        if outcome == "terminated":
            entry.state = "terminated"
            entry.error_code = None
        else:
            entry.state = "unknown"
            entry.error_code = "runtime_interrupted"
        entry.finished_at = _now()
        for task in entry.pumps:
            task.cancel()
        await asyncio.gather(*entry.pumps, return_exceptions=True)
        entry.done.set()
        with suppress(Exception):
            self._persist(entry, force=True)

    async def close_run(self, run_id: str) -> None:
        """Mark a Run closed without touching its commands (they are thread-owned)."""
        async with self._start_lock:
            self._closed_runs.add(run_id)

    async def close(self) -> None:
        """Persist facts, release log handles, and leave commands running.

        The child keeps its inherited output handle, so a restart re-adopts the
        command and continues reading the same log file.
        """
        self._closed = True
        for entry in list(self._entries.values()):
            with suppress(Exception):
                self._persist(entry, force=True)
        for entry in list(self._entries.values()):
            for task in (*entry.pumps, entry.watch):
                if task is not None:
                    task.cancel()
            pending = [task for task in (*entry.pumps, entry.watch) if task is not None]
            if pending:
                await asyncio.gather(*pending, return_exceptions=True)
            self._close_log(entry)
    # -- monitoring --------------------------------------------------------

    async def _kill(self, entry: _Entry) -> None:
        if entry.pty is not None:
            try:
                await asyncio.wait_for(entry.pty.terminate_tree(), timeout=6)
            except (OSError, TimeoutError):
                entry.error_code = "process_cleanup_failed"
                entry.pty.release()
            return
        if entry.spawned is None:
            return
        try:
            await asyncio.wait_for(_terminate_process_tree(entry.spawned), timeout=6)
        except (OSError, TimeoutError):
            entry.error_code = "process_cleanup_failed"
            # Even a detached descendant holding the output handle cannot block shutdown.
            transport = getattr(entry.spawned.process, "_transport", None)
            if transport is not None:
                transport.close()

    async def _monitor(self, entry: _Entry) -> None:
        try:
            while True:
                await self._await_incarnation(entry)
                await self._kill(entry)
                if entry.pty is not None:
                    # Closing the terminal delivers the last bytes and ends the stream.
                    await entry.pty.finish()
                await self._join_pumps(entry)
                entry.exit_code = _child_returncode(entry)
                if entry.error_code == "process_cleanup_failed":
                    entry.state = "unknown"
                    break
                if not self._should_restart(entry, entry.exit_code):
                    entry.state = "terminated" if entry.stopping else "exited"
                    if entry.exit_code not in (0, None) and entry.error_code is None:
                        entry.error_code = "terminated" if entry.stopping else "non_zero_exit"
                    break
                delay = self._restart_delay(entry)
                if delay is None:
                    entry.state = "exited"
                    entry.error_code = "supervise_storm"
                    break
                entry.exit_code = None
                entry.restart_times.append(time.monotonic())
                await self._sleep_restart(entry, delay)
                if entry.stopping or self._closed:
                    entry.state = "terminated" if entry.stopping else "exited"
                    break
                entry.restart_count += 1
                entry.last_restart_at = _now()
                entry.error_code = None
                entry.spawned = None
                entry.log_offset = entry.file_bytes
                try:
                    await self._spawn_incarnation(entry, restart=True)
                except (Exception, asyncio.CancelledError):
                    entry.state = "exited"
                    entry.error_code = "supervise_restart_failed"
                    break
                entry.pumps = [asyncio.create_task(self._pump(entry))]
        except (Exception, asyncio.CancelledError):
            await _await_uninterruptibly(self._kill(entry))
            entry.state = "unknown"
            entry.error_code = entry.error_code or "process_monitor_failed"
        finally:
            for task in entry.pumps:
                task.cancel()
            await asyncio.gather(*entry.pumps, return_exceptions=True)
            if entry.pty is not None:
                with suppress(Exception):
                    await _await_uninterruptibly(entry.pty.finish())
            entry.finished_at = _now()
            try:
                self._persist(entry, force=True)
            except Exception:
                entry.state = "unknown"
                entry.error_code = "process_record_failed"
            entry.done.set()

    async def _await_incarnation(self, entry: _Entry) -> None:
        # Process.wait may wait for inherited handles after root exit. Watch the
        # root's exit status directly and terminate remaining children first.
        while _child_returncode(entry) is None:
            if entry.error_code == "process_cleanup_failed":
                raise RuntimeError("process tree could not be confirmed stopped")
            if any(
                task.done() and not task.cancelled() and task.exception()
                for task in entry.pumps
            ):
                raise RuntimeError("command output persistence failed")
            self._persist(entry)
            await asyncio.sleep(0.02)

    async def _join_pumps(self, entry: _Entry) -> None:
        if not entry.pumps:
            return
        with suppress(TimeoutError):
            await asyncio.wait_for(asyncio.gather(*entry.pumps), timeout=2)

    def _should_restart(self, entry: _Entry, exit_code: int | None) -> bool:
        if entry.stopping or self._closed or entry.supervise is None:
            return False
        if entry.command_redacted:
            return False
        return _supervise_should_restart(entry.supervise, exit_code)

    def _restart_delay(self, entry: _Entry) -> float | None:
        """Exponential backoff, or ``None`` when the storm budget is exhausted."""
        now = time.monotonic()
        window_start = now - _SUPERVISE_WINDOW_SECONDS
        entry.restart_times = [value for value in entry.restart_times if value >= window_start]
        if len(entry.restart_times) >= _SUPERVISE_MAX_RESTARTS:
            return None
        exponent = min(entry.restart_count, 20)
        base = max(1, entry.backoff_ms) / 1000
        return min(base * (2**exponent), _SUPERVISE_BACKOFF_MAX_MS / 1000)

    def _within_restart_budget(self, entry: _Entry) -> bool:
        now = time.monotonic()
        window_start = now - _SUPERVISE_WINDOW_SECONDS
        recent = [value for value in entry.restart_times if value >= window_start]
        return len(recent) < _SUPERVISE_MAX_RESTARTS

    async def _sleep_restart(self, entry: _Entry, delay: float) -> None:
        with suppress(TimeoutError):
            await asyncio.wait_for(entry.stop_requested.wait(), timeout=delay)

    async def _watch_recovered(self, entry: _Entry) -> None:
        """Watch an adopted command; supervise it if it dies under this Runtime."""
        while entry.state == "running" and not entry.stopping:
            await asyncio.sleep(_RECOVERED_POLL_SECONDS)
            if entry.stopping or entry.state != "running":
                return
            if entry.pid is None or probe_custody(entry.pid, entry.pid_started_at) == "alive":
                continue
            if (
                entry.supervise is None
                or entry.command_redacted
                or self._closed
                or self._restart_delay(entry) is None
            ):
                entry.state = "unknown"
                entry.exit_code = None
                entry.finished_at = _now()
                entry.error_code = (
                    "supervise_storm"
                    if entry.supervise is not None and not self._within_restart_budget(entry)
                    else "runtime_recovered"
                )
                entry.done.set()
                for task in entry.pumps:
                    task.cancel()
                await asyncio.gather(*entry.pumps, return_exceptions=True)
                with suppress(Exception):
                    self._persist(entry, force=True)
                return
            delay = self._restart_delay(entry) or 0.0
            entry.restart_times.append(time.monotonic())
            await self._sleep_restart(entry, delay)
            if entry.stopping:
                return
            entry.restart_count += 1
            entry.last_restart_at = _now()
            entry.log_offset = entry.file_bytes
            entry.recovered = False
            try:
                await self._spawn_incarnation(entry, restart=True)
            except (Exception, asyncio.CancelledError):
                entry.state = "unknown"
                entry.error_code = "supervise_restart_failed"
                entry.finished_at = _now()
                entry.done.set()
                with suppress(Exception):
                    self._persist(entry, force=True)
                return
            if entry.stopping:
                await _await_uninterruptibly(self._kill(entry))
                self._settle_supervised(entry, state="terminated")
                return
            entry.monitor = asyncio.create_task(self._monitor(entry))
            return

    # -- pruning -----------------------------------------------------------

    def _prune(self) -> None:
        """Drop old terminal entries; running and recovered entries are kept."""
        now = time.monotonic()
        for process_id, entry in list(self._entries.items()):
            if (
                entry.done.is_set()
                and entry.state != "running"
                and now - entry.last_record > _TERMINAL_RETENTION_SECONDS
            ):
                self._entries.pop(process_id, None)
                self._close_log(entry)
        overflow = len(self._entries) - _MAX_RETAINED_ENTRIES
        if overflow <= 0:
            return
        candidates = sorted(
            (
                entry
                for entry in self._entries.values()
                if entry.done.is_set() and entry.state != "running"
            ),
            key=lambda entry: entry.last_record,
        )
        for entry in candidates[:overflow]:
            self._entries.pop(entry.process_id, None)
            self._close_log(entry)

    @staticmethod
    def _close_log(entry: _Entry) -> None:
        if entry.log is not None:
            entry.log.close()
