from __future__ import annotations

import asyncio
from collections.abc import AsyncIterator
from pathlib import Path

import pytest

from ikaros_runtime.agent import AgentLoop, AgentScheduler
from ikaros_runtime.domain import JournalEvent
from ikaros_runtime.providers import ProviderRequest
from ikaros_runtime.storage import SqliteRuntimeStore


class FailingProvider:
    async def stream(self, request: ProviderRequest) -> AsyncIterator[str]:
        await asyncio.sleep(0)
        if request.messages:
            raise RuntimeError("expected provider failure")
        yield "unreachable"


class FailingOnceExecutor:
    def __init__(self) -> None:
        self.calls: list[str] = []
        self.second_completed = asyncio.Event()

    async def run(self, run_id: str) -> None:
        self.calls.append(run_id)
        if len(self.calls) == 1:
            raise RuntimeError("expected executor failure")
        self.second_completed.set()


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

        await loop.run(prepared.run_id)

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
