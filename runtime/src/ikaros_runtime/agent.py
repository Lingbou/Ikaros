from __future__ import annotations

import asyncio
import logging
from collections.abc import Awaitable, Callable
from typing import Protocol

from .domain import JournalEvent
from .providers import ProviderAdapter, ProviderMessage, ProviderRequest
from .storage import SqliteRuntimeStore

EventPublisher = Callable[[JournalEvent], Awaitable[None]]
_LOGGER = logging.getLogger("ikaros_runtime.agent")


class RunExecutor(Protocol):
    async def run(self, run_id: str) -> None: ...


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

    async def run(self, run_id: str) -> None:
        assistant_item_id: str | None = None
        try:
            run = self._store.get_run(run_id)
            provider = self._providers.get(run.provider_id)
            if provider is None:
                raise ValueError(f"unknown provider: {run.provider_id}")

            await self._publish(self._store.mark_run_running(run_id))
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
            async for delta in provider.stream(request):
                if not delta:
                    continue
                await self._publish(self._store.append_text_delta(assistant_item_id, delta))

            await self._publish(self._store.complete_assistant_item(assistant_item_id))
            settled = self._store.settle_run(run_id, "completed")
            if settled is not None:
                await self._publish(settled)
        except Exception:
            _LOGGER.exception("Run %s failed", run_id)
            if assistant_item_id is not None:
                try:
                    await self._publish(
                        self._store.complete_assistant_item(assistant_item_id, failed=True)
                    )
                except Exception:
                    _LOGGER.exception("Failed to terminalize assistant item for run %s", run_id)
            settled = self._store.settle_run(run_id, "failed")
            if settled is not None:
                await self._publish(settled)


class AgentScheduler:
    def __init__(self, loop: RunExecutor) -> None:
        self._loop = loop
        self._queue: asyncio.Queue[str | None] = asyncio.Queue()
        self._worker: asyncio.Task[None] | None = None

    def start(self) -> None:
        if self._worker is not None:
            return
        self._worker = asyncio.create_task(self._work(), name="ikaros-agent-scheduler")

    async def enqueue(self, run_id: str) -> None:
        if self._worker is None:
            raise RuntimeError("Agent scheduler has not started")
        await self._queue.put(run_id)

    async def close(self) -> None:
        worker = self._worker
        if worker is None:
            return
        await self._queue.put(None)
        await worker
        self._worker = None

    async def _work(self) -> None:
        while True:
            run_id = await self._queue.get()
            try:
                if run_id is None:
                    return
                try:
                    await self._loop.run(run_id)
                except Exception:
                    _LOGGER.exception("Unhandled scheduler failure for run %s", run_id)
            finally:
                self._queue.task_done()
