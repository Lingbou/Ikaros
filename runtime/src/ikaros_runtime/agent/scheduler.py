"""Bounded worker-pool scheduler for independent Agent Runs."""

from __future__ import annotations

import asyncio
import logging
import os
from collections import deque
from collections.abc import Awaitable, Callable, Mapping, Sequence
from contextlib import suppress
from dataclasses import dataclass
from typing import Protocol, cast

from ..attachments import AttachmentRef
from ..cancellation import CancellationToken

_LOGGER = logging.getLogger("ikaros_runtime.agent.scheduler")

AGENT_WORKERS_ENV = "IKAROS_AGENT_WORKERS"
DEFAULT_AGENT_WORKERS = 6
MIN_AGENT_WORKERS = 1
MAX_AGENT_WORKERS = 8


class RunExecutor(Protocol):
    async def run(self, run_id: str, cancellation: CancellationToken) -> None: ...

    async def cancel(self, run_id: str) -> None: ...

    def run_thread_id(self, run_id: str) -> str: ...

    # Implementations may opt into in-flight steering.


@dataclass(slots=True)
class _ScheduledRun:
    thread_id: str
    cancellation: CancellationToken
    state: str = "queued"


def agent_worker_limit(environment: Mapping[str, str] | None = None) -> int:
    """Read and validate the configured Run concurrency limit."""

    source = os.environ if environment is None else environment
    raw = source.get(AGENT_WORKERS_ENV)
    if raw is None:
        return DEFAULT_AGENT_WORKERS
    try:
        value = int(raw.strip())
    except ValueError:
        raise ValueError(
            f"{AGENT_WORKERS_ENV} must be an integer between "
            f"{MIN_AGENT_WORKERS} and {MAX_AGENT_WORKERS}"
        ) from None
    if not MIN_AGENT_WORKERS <= value <= MAX_AGENT_WORKERS:
        raise ValueError(
            f"{AGENT_WORKERS_ENV} must be an integer between "
            f"{MIN_AGENT_WORKERS} and {MAX_AGENT_WORKERS}"
        )
    return value


class AgentScheduler:
    """Run a bounded number of independent Threads concurrently."""

    def __init__(self, loop: RunExecutor, *, max_workers: int | None = None) -> None:
        workers = agent_worker_limit() if max_workers is None else max_workers
        if (
            isinstance(workers, bool)
            or not isinstance(workers, int)
            or not MIN_AGENT_WORKERS <= workers <= MAX_AGENT_WORKERS
        ):
            raise ValueError(
                f"max_workers must be an integer between "
                f"{MIN_AGENT_WORKERS} and {MAX_AGENT_WORKERS}"
            )
        self._loop = loop
        self._max_workers = workers
        self._pending: deque[str] = deque()
        self._workers: list[asyncio.Task[None]] = []
        self._entries: dict[str, _ScheduledRun] = {}
        self._active_threads: set[str] = set()
        self._running_count = 0
        self._wakeup = asyncio.Event()
        self._accepting = False
        self._closing = False

    @property
    def max_workers(self) -> int:
        return self._max_workers

    def start(self, initial_run_ids: Sequence[str] = ()) -> None:
        if self._workers:
            return
        self._accepting = True
        self._closing = False
        self._wakeup.clear()
        for run_id in initial_run_ids:
            self._register(run_id, state="queued")
            self._pending.append(run_id)
        self._workers = [
            asyncio.create_task(
                self._work(index),
                name=f"ikaros-agent-scheduler-{index}",
            )
            for index in range(self._max_workers)
        ]
        self._wakeup.set()

    def reserve(self, run_id: str) -> None:
        """Make a persisted Run cancellable before its start ACK is attempted."""

        if not self._workers or not self._accepting:
            raise RuntimeError("Agent scheduler has not started")
        self._register(run_id, state="reserved")
        self._pending.append(run_id)
        self._wakeup.set()

    async def activate(self, run_id: str) -> bool:
        """Queue a reserved Run after the command ACK attempt has finished."""

        entry = self._entries.get(run_id)
        if entry is None:
            return False
        if entry.state != "reserved":
            raise RuntimeError("run is not reserved")
        if not self._accepting:
            return False
        entry.state = "queued"
        self._wakeup.set()
        return True

    async def enqueue(self, run_id: str) -> None:
        if not self._workers or not self._accepting:
            raise RuntimeError("Agent scheduler has not started")
        self._register(run_id, state="queued")
        self._pending.append(run_id)
        self._wakeup.set()

    async def cancel(self, run_id: str) -> bool:
        entry = self._entries.get(run_id)
        if entry is None:
            return False
        entry.cancellation.cancel()
        if entry.state in {"reserved", "queued"}:
            entry.state = "cancelled"
            self._entries.pop(run_id, None)
            self._discard_pending(run_id)
            self._wakeup.set()
            await self._loop.cancel(run_id)
        return True

    async def steer(
        self,
        run_id: str,
        content: str,
        request_id: str,
        attachments: Sequence[AttachmentRef] = (),
    ) -> bool:
        entry = self._entries.get(run_id)
        if entry is None or entry.state != "running":
            return False
        steer = getattr(self._loop, "steer", None)
        if steer is None:
            return False
        callback = cast(Callable[[str, str, str, Sequence[AttachmentRef]], Awaitable[bool]], steer)
        return await callback(run_id, content, request_id, attachments)

    async def close(self) -> None:
        workers = tuple(self._workers)
        if not workers:
            return
        self._accepting = False
        self._closing = True
        queued_run_ids: list[str] = []
        for run_id, entry in tuple(self._entries.items()):
            entry.cancellation.cancel()
            if entry.state in {"reserved", "queued"}:
                entry.state = "cancelled"
                self._entries.pop(run_id, None)
                queued_run_ids.append(run_id)
        self._pending.clear()
        self._wakeup.set()
        for run_id in queued_run_ids:
            await self._loop.cancel(run_id)
        await asyncio.gather(*workers)
        self._workers = []
        self._entries.clear()
        self._pending.clear()
        self._active_threads.clear()
        self._running_count = 0
        self._closing = False

    def _register(self, run_id: str, *, state: str) -> _ScheduledRun:
        if run_id in self._entries:
            raise RuntimeError("run is already scheduled")
        thread_id = self._loop.run_thread_id(run_id)
        if not isinstance(thread_id, str) or not thread_id:
            raise RuntimeError("run thread id is unavailable")
        entry = _ScheduledRun(
            thread_id=thread_id,
            cancellation=CancellationToken(),
            state=state,
        )
        self._entries[run_id] = entry
        return entry

    def _discard_pending(self, run_id: str) -> None:
        with suppress(ValueError):
            self._pending.remove(run_id)

    def _take_next(self) -> str | None:
        index = 0
        while index < len(self._pending):
            run_id = self._pending[index]
            entry = self._entries.get(run_id)
            if entry is None or entry.state == "cancelled":
                del self._pending[index]
                continue
            if entry.state == "reserved":
                # Preserve the existing ACK barrier: a reserved Run must be
                # activated or cancelled before later Runs may start.
                return None
            if entry.state != "queued":
                del self._pending[index]
                continue
            if entry.thread_id in self._active_threads:
                index += 1
                continue
            del self._pending[index]
            entry.state = "running"
            self._active_threads.add(entry.thread_id)
            self._running_count += 1
            return run_id
        return None

    async def _wait_for_run(self) -> str | None:
        while True:
            run_id = self._take_next()
            if run_id is not None:
                return run_id
            if self._closing and self._running_count == 0:
                return None
            self._wakeup.clear()
            await self._wakeup.wait()

    async def _work(self, index: int) -> None:
        del index
        while True:
            run_id = await self._wait_for_run()
            if run_id is None:
                return
            entry = self._entries.get(run_id)
            if entry is None:
                continue
            try:
                await self._loop.run(run_id, entry.cancellation)
            except Exception:
                _LOGGER.error("Unhandled scheduler failure for run %s", run_id)
            finally:
                self._entries.pop(run_id, None)
                self._active_threads.discard(entry.thread_id)
                self._running_count -= 1
                self._wakeup.set()


__all__ = [
    "AGENT_WORKERS_ENV",
    "DEFAULT_AGENT_WORKERS",
    "MAX_AGENT_WORKERS",
    "MIN_AGENT_WORKERS",
    "AgentScheduler",
    "RunExecutor",
    "agent_worker_limit",
]
