from __future__ import annotations

import asyncio
from collections.abc import AsyncIterator
from pathlib import Path

import pytest

from ikaros_runtime.agent import AgentLoop, AgentScheduler
from ikaros_runtime.cancellation import CancellationToken, RunCancelled
from ikaros_runtime.domain import JournalEvent
from ikaros_runtime.providers import ProviderRequest
from ikaros_runtime.storage import SqliteRuntimeStore


class FailingProvider:
    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[str]:
        cancellation.raise_if_cancelled()
        await asyncio.sleep(0)
        if request.messages:
            raise RuntimeError("expected provider failure")
        yield "unreachable"


class FailingOnceExecutor:
    def __init__(self) -> None:
        self.calls: list[str] = []
        self.second_completed = asyncio.Event()

    async def run(self, run_id: str, cancellation: CancellationToken) -> None:
        cancellation.raise_if_cancelled()
        self.calls.append(run_id)
        if len(self.calls) == 1:
            raise RuntimeError("expected executor failure")
        self.second_completed.set()

    async def cancel(self, run_id: str) -> None:
        del run_id


class PausingProvider:
    def __init__(self) -> None:
        self.first_delta_emitted = asyncio.Event()

    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[str]:
        del request
        yield "partial"
        self.first_delta_emitted.set()
        await cancellation.sleep(60)
        yield "late"


class BlockingExecutor:
    def __init__(self) -> None:
        self.calls: list[str] = []
        self.cancelled_queued: list[str] = []
        self.first_started = asyncio.Event()

    async def run(self, run_id: str, cancellation: CancellationToken) -> None:
        self.calls.append(run_id)
        self.first_started.set()
        try:
            await cancellation.sleep(60)
        except RunCancelled:
            return

    async def cancel(self, run_id: str) -> None:
        self.cancelled_queued.append(run_id)


@pytest.mark.asyncio
async def test_provider_failure_terminalizes_item_and_settles_run_once(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    events: list[JournalEvent] = []

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    try:
        thread, _ = store.create_thread("Provider failure")
        prepared = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="fail safely",
            provider_id="failing",
            model_id="failing-v1",
        )
        loop = AgentLoop(store, {"failing": FailingProvider()}, publish)

        await loop.run(prepared.run_id, CancellationToken())

        run_events = [event for event in events if event.run_id == prepared.run_id]
        settled = [event for event in run_events if event.type == "run.settled"]
        assistant_terminal = [
            event
            for event in run_events
            if event.type == "item.completed"
            and event.payload.get("item", {}).get("role") == "assistant"
        ]
        assert len(settled) == 1
        assert settled[0].payload["status"] == "failed"
        assert len(assistant_terminal) == 1
        assert assistant_terminal[0].payload["item"]["status"] == "failed"
    finally:
        store.close()


@pytest.mark.asyncio
async def test_running_provider_stops_after_cancellation_and_preserves_partial_text(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    events: list[JournalEvent] = []

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    try:
        thread, _ = store.create_thread("Cancellation")
        prepared = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="cancel me",
            provider_id="pausing",
            model_id="pausing-v1",
        )
        provider = PausingProvider()
        loop = AgentLoop(store, {"pausing": provider}, publish)
        cancellation = CancellationToken()
        task = asyncio.create_task(loop.run(prepared.run_id, cancellation))

        await asyncio.wait_for(provider.first_delta_emitted.wait(), timeout=1)
        cancellation.cancel()
        await asyncio.wait_for(task, timeout=1)

        deltas = [
            event.payload["delta"]
            for event in events
            if event.type == "item.delta" and event.run_id == prepared.run_id
        ]
        terminal = [
            event
            for event in events
            if event.run_id == prepared.run_id
            and event.type in {"item.completed", "run.settled"}
        ]
        assert deltas == ["partial"]
        assert [event.type for event in terminal] == ["item.completed", "run.settled"]
        assert terminal[0].payload["item"]["content"] == "partial"
        assert terminal[0].payload["item"]["status"] == "cancelled"
        assert terminal[1].payload["status"] == "cancelled"
    finally:
        store.close()


@pytest.mark.asyncio
async def test_queued_run_is_cancelled_without_calling_executor() -> None:
    executor = BlockingExecutor()
    scheduler = AgentScheduler(executor)
    scheduler.start()
    try:
        await scheduler.enqueue("run-first")
        await asyncio.wait_for(executor.first_started.wait(), timeout=1)
        await scheduler.enqueue("run-queued")

        assert await scheduler.cancel("run-queued") is True
        assert executor.calls == ["run-first"]
        assert executor.cancelled_queued == ["run-queued"]
        assert await scheduler.cancel("run-first") is True
    finally:
        await asyncio.wait_for(scheduler.close(), timeout=1)


@pytest.mark.asyncio
async def test_cancelling_reserved_head_wakes_next_activated_run() -> None:
    executor = BlockingExecutor()
    scheduler = AgentScheduler(executor)
    scheduler.start()
    try:
        scheduler.reserve("run-reserved")
        await scheduler.enqueue("run-next")
        await asyncio.sleep(0)
        assert executor.calls == []

        assert await scheduler.cancel("run-reserved") is True
        await asyncio.wait_for(executor.first_started.wait(), timeout=1)
        assert executor.calls == ["run-next"]
        assert executor.cancelled_queued == ["run-reserved"]
        assert await scheduler.cancel("run-next") is True
    finally:
        await asyncio.wait_for(scheduler.close(), timeout=1)


@pytest.mark.asyncio
async def test_scheduler_close_wakes_and_terminalizes_reserved_head() -> None:
    executor = BlockingExecutor()
    scheduler = AgentScheduler(executor)
    scheduler.start()
    scheduler.reserve("run-reserved")
    await asyncio.sleep(0)

    await asyncio.wait_for(scheduler.close(), timeout=1)

    assert executor.calls == []
    assert executor.cancelled_queued == ["run-reserved"]


@pytest.mark.asyncio
async def test_scheduler_continues_after_an_unhandled_run_failure() -> None:
    executor = FailingOnceExecutor()
    scheduler = AgentScheduler(executor)
    scheduler.start()
    try:
        await scheduler.enqueue("run-first")
        await scheduler.enqueue("run-second")
        await asyncio.wait_for(executor.second_completed.wait(), timeout=1)
        assert executor.calls == ["run-first", "run-second"]
    finally:
        await scheduler.close()
