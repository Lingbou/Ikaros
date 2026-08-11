from __future__ import annotations

import asyncio
import logging
from collections.abc import Awaitable, Callable, Sequence
from dataclasses import dataclass
from typing import Protocol

from .cancellation import CancellationToken, RunCancelled
from .domain import JournalEvent
from .providers import ProviderAdapter, ProviderMessage, ProviderRequest
from .storage import SqliteRuntimeStore

EventPublisher = Callable[[JournalEvent], Awaitable[None]]
_LOGGER = logging.getLogger("ikaros_runtime.agent")


class RunExecutor(Protocol):
    async def run(self, run_id: str, cancellation: CancellationToken) -> None: ...

    async def cancel(self, run_id: str) -> None: ...


class AgentLoop:
    def __init__(
        self,
        store: SqliteRuntimeStore,
        providers: dict[str, ProviderAdapter],
        publish: EventPublisher,
    ) -> None:
        self._store = store
        self._providers = providers
        self._publish = publish

    async def run(self, run_id: str, cancellation: CancellationToken) -> None:
        try:
            cancellation.raise_if_cancelled()
            run = self._store.get_run(run_id)
            provider = self._providers.get(run.provider_id)
            if provider is None:
                raise ValueError(f"unknown provider: {run.provider_id}")

            await self._publish(self._store.mark_run_running(run_id))
            cancellation.raise_if_cancelled()
            messages = [
                ProviderMessage(role=role, content=content)
                for role, content in self._store.context_messages(
                    run.branch_id,
                    through_turn_id=run.turn_id,
                )
            ]
            assistant_item_id, started = self._store.create_assistant_item(run_id)
            await self._publish(started)

            request = ProviderRequest(model_id=run.model_id, messages=messages)
            async for delta in provider.stream(request, cancellation=cancellation):
                cancellation.raise_if_cancelled()
                if not delta:
                    continue
                await self._publish(self._store.append_text_delta(assistant_item_id, delta))

            cancellation.raise_if_cancelled()
            await self._publish_terminal_events(
                self._store.terminalize_run(run_id, "completed")
            )
        except RunCancelled:
            await self._publish_terminal_events(
                self._store.terminalize_run(run_id, "cancelled")
            )
        except Exception:
            _LOGGER.exception("Run %s failed", run_id)
            await self._publish_terminal_events(self._store.terminalize_run(run_id, "failed"))

    async def cancel(self, run_id: str) -> None:
        await self._publish_terminal_events(self._store.terminalize_run(run_id, "cancelled"))

    async def _publish_terminal_events(self, events: Sequence[JournalEvent]) -> None:
        for event in events:
            await self._publish(event)


@dataclass(slots=True)
class _ScheduledRun:
    cancellation: CancellationToken
    activation: asyncio.Event
    state: str = "queued"


class AgentScheduler:
    def __init__(self, loop: RunExecutor) -> None:
        self._loop = loop
        self._queue: asyncio.Queue[str | None] = asyncio.Queue()
        self._worker: asyncio.Task[None] | None = None
        self._entries: dict[str, _ScheduledRun] = {}
        self._accepting = False

    def start(self, initial_run_ids: Sequence[str] = ()) -> None:
        if self._worker is not None:
            return
        self._accepting = True
        for run_id in initial_run_ids:
            entry = self._register(run_id, state="queued")
            entry.activation.set()
            self._queue.put_nowait(run_id)
        self._worker = asyncio.create_task(self._work(), name="ikaros-agent-scheduler")

    def reserve(self, run_id: str) -> None:
        """Make a persisted Run cancellable before its start ACK is attempted."""
        if self._worker is None or not self._accepting:
            raise RuntimeError("Agent scheduler has not started")
        self._register(run_id, state="reserved")
        self._queue.put_nowait(run_id)

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
        entry.activation.set()
        return True

    async def enqueue(self, run_id: str) -> None:
        if self._worker is None or not self._accepting:
            raise RuntimeError("Agent scheduler has not started")
        entry = self._register(run_id, state="queued")
        entry.activation.set()
        self._queue.put_nowait(run_id)

    async def cancel(self, run_id: str) -> bool:
        entry = self._entries.get(run_id)
        if entry is None:
            return False
        entry.cancellation.cancel()
        if entry.state in {"reserved", "queued"}:
            self._entries.pop(run_id, None)
            entry.activation.set()
            await self._loop.cancel(run_id)
        return True

    async def close(self) -> None:
        worker = self._worker
        if worker is None:
            return
        self._accepting = False
        queued_run_ids: list[str] = []
        for run_id, entry in tuple(self._entries.items()):
            entry.cancellation.cancel()
            if entry.state in {"reserved", "queued"}:
                self._entries.pop(run_id, None)
                entry.activation.set()
                queued_run_ids.append(run_id)
        for run_id in queued_run_ids:
            await self._loop.cancel(run_id)
        await self._queue.put(None)
        await worker
        self._worker = None
        self._entries.clear()

    def _register(self, run_id: str, *, state: str) -> _ScheduledRun:
        if run_id in self._entries:
            raise RuntimeError("run is already scheduled")
        entry = _ScheduledRun(CancellationToken(), asyncio.Event(), state=state)
        self._entries[run_id] = entry
        return entry

    async def _work(self) -> None:
        while True:
            run_id = await self._queue.get()
            try:
                if run_id is None:
                    return
                entry = self._entries.get(run_id)
                if entry is None:
                    continue
                if entry.state == "reserved":
                    await entry.activation.wait()
                    entry = self._entries.get(run_id)
                    if entry is None:
                        continue
                if entry.state != "queued":
                    continue
                entry.state = "running"
                try:
                    await self._loop.run(run_id, entry.cancellation)
                except Exception:
                    _LOGGER.exception("Unhandled scheduler failure for run %s", run_id)
                finally:
                    self._entries.pop(run_id, None)
            finally:
                self._queue.task_done()
