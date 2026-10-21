from __future__ import annotations

import asyncio
from collections.abc import AsyncIterator, Callable
from contextlib import suppress
from pathlib import Path

import pytest

from ikaros_runtime.agent.loop import AgentLoop
from ikaros_runtime.agent.scheduler import (
    AGENT_WORKERS_ENV,
    AgentScheduler,
    agent_worker_limit,
)
from ikaros_runtime.cancellation import CancellationToken, RunCancelled
from ikaros_runtime.domain import JournalEvent, PreparedTurn
from ikaros_runtime.providers.base import (
    ProviderEvent,
    ProviderRequest,
    ResponseCompleted,
    TextDelta,
)
from ikaros_runtime.storage import SqliteRuntimeStore

from .helpers import prepare_turn


class ControlledExecutor:
    def __init__(self, thread_ids: dict[str, str]) -> None:
        self._thread_ids = dict(thread_ids)
        self.calls: list[str] = []
        self.finished: list[str] = []
        self.cancelled_queued: list[str] = []
        self._started: dict[str, asyncio.Event] = {}
        self._release: dict[str, asyncio.Event] = {}
        self._active: set[str] = set()
        self._max_active = 0

    @property
    def active(self) -> frozenset[str]:
        return frozenset(self._active)

    @property
    def max_active(self) -> int:
        return self._max_active

    def run_thread_id(self, run_id: str) -> str:
        return self._thread_ids.get(run_id, run_id)

    async def run(self, run_id: str, cancellation: CancellationToken) -> None:
        self.calls.append(run_id)
        self._active.add(run_id)
        self._max_active = max(self._max_active, len(self._active))
        self._started.setdefault(run_id, asyncio.Event()).set()
        release_wait = asyncio.create_task(
            self._release.setdefault(run_id, asyncio.Event()).wait()
        )
        cancel_wait = asyncio.create_task(cancellation.wait())
        try:
            await asyncio.wait({release_wait, cancel_wait}, return_when=asyncio.FIRST_COMPLETED)
            try:
                cancellation.raise_if_cancelled()
            except RunCancelled:
                return
        finally:
            for task in (release_wait, cancel_wait):
                if not task.done():
                    task.cancel()
                with suppress(asyncio.CancelledError):
                    await task
            self._active.discard(run_id)
            self.finished.append(run_id)

    async def cancel(self, run_id: str) -> None:
        self.cancelled_queued.append(run_id)

    async def wait_started(self, run_id: str) -> None:
        await asyncio.wait_for(
            self._started.setdefault(run_id, asyncio.Event()).wait(),
            timeout=2,
        )

    def is_started(self, run_id: str) -> bool:
        event = self._started.get(run_id)
        return event is not None and event.is_set()

    def release_run(self, run_id: str) -> None:
        self._release.setdefault(run_id, asyncio.Event()).set()


class GatedProvider:
    id = "gated"
    model_id = "gated-v1"

    def __init__(self) -> None:
        self._started: dict[str, asyncio.Event] = {}
        self._release: dict[str, asyncio.Event] = {}
        self._finished: dict[str, asyncio.Event] = {}
        self._active: set[str] = set()
        self._max_active = 0

    @property
    def active_keys(self) -> frozenset[str]:
        return frozenset(self._active)

    @property
    def max_active(self) -> int:
        return self._max_active

    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]:
        key = _provider_key(request)
        self._active.add(key)
        self._max_active = max(self._max_active, len(self._active))
        self._started.setdefault(key, asyncio.Event()).set()
        release = self._release.setdefault(key, asyncio.Event())
        release_wait = asyncio.create_task(release.wait())
        cancel_wait = asyncio.create_task(cancellation.wait())
        try:
            await asyncio.wait({release_wait, cancel_wait}, return_when=asyncio.FIRST_COMPLETED)
            cancellation.raise_if_cancelled()
            yield TextDelta(f"answer:{key}")
            yield ResponseCompleted()
        finally:
            for task in (release_wait, cancel_wait):
                if not task.done():
                    task.cancel()
                with suppress(asyncio.CancelledError):
                    await task
            self._active.discard(key)
            self._finished.setdefault(key, asyncio.Event()).set()

    async def wait_started(self, key: str) -> None:
        await asyncio.wait_for(
            self._started.setdefault(key, asyncio.Event()).wait(),
            timeout=2,
        )

    async def wait_finished(self, key: str) -> None:
        await asyncio.wait_for(
            self._finished.setdefault(key, asyncio.Event()).wait(),
            timeout=2,
        )

    def is_started(self, key: str) -> bool:
        event = self._started.get(key)
        return event is not None and event.is_set()

    def release_run(self, key: str) -> None:
        self._release.setdefault(key, asyncio.Event()).set()


def _provider_key(request: ProviderRequest) -> str:
    for message in reversed(request.messages):
        if message.role == "user":
            return message.content
    raise ValueError("gated provider requires a user message")


async def _wait_until(predicate: Callable[[], bool]) -> None:
    async with asyncio.timeout(2):
        while not predicate():  # noqa: ASYNC110
            await asyncio.sleep(0.005)


async def _wait_status(store: SqliteRuntimeStore, run_id: str, status: str) -> None:
    await _wait_until(lambda: store.run_status(run_id) == status)


def _prepare(
    store: SqliteRuntimeStore,
    *,
    thread_id: str,
    branch_id: str,
    content: str,
) -> PreparedTurn:
    return prepare_turn(
        store,
        thread_id=thread_id,
        branch_id=branch_id,
        content=content,
        provider_id=GatedProvider.id,
        model_id=GatedProvider.model_id,
    )


def test_agent_worker_limit_reads_and_validates_environment(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    assert agent_worker_limit({}) == 2
    assert agent_worker_limit({AGENT_WORKERS_ENV: "1"}) == 1
    assert agent_worker_limit({AGENT_WORKERS_ENV: " 8 "}) == 8
    for invalid in ("0", "9", "two", ""):
        with pytest.raises(ValueError):
            agent_worker_limit({AGENT_WORKERS_ENV: invalid})
    with pytest.raises(ValueError):
        AgentScheduler(ControlledExecutor({}), max_workers=9)

    monkeypatch.setenv(AGENT_WORKERS_ENV, "3")
    assert AgentScheduler(ControlledExecutor({})).max_workers == 3
    monkeypatch.setenv(AGENT_WORKERS_ENV, "9")
    with pytest.raises(ValueError):
        AgentScheduler(ControlledExecutor({}))


@pytest.mark.asyncio
async def test_scheduler_runs_two_threads_concurrently() -> None:
    executor = ControlledExecutor({"run-a": "thread-a", "run-b": "thread-b"})
    scheduler = AgentScheduler(executor, max_workers=2)
    scheduler.start()
    try:
        await scheduler.enqueue("run-a")
        await scheduler.enqueue("run-b")
        await executor.wait_started("run-a")
        await executor.wait_started("run-b")
        assert executor.max_active == 2
        executor.release_run("run-a")
        executor.release_run("run-b")
        await _wait_until(lambda: set(executor.finished) == {"run-a", "run-b"})
    finally:
        await scheduler.close()


@pytest.mark.asyncio
async def test_scheduler_pulls_next_fifo_run_when_capacity_opens() -> None:
    executor = ControlledExecutor({"run-a": "thread-a", "run-b": "thread-b"})
    scheduler = AgentScheduler(executor, max_workers=1)
    scheduler.start()
    try:
        await scheduler.enqueue("run-a")
        await scheduler.enqueue("run-b")
        await executor.wait_started("run-a")
        assert not executor.is_started("run-b")

        executor.release_run("run-a")
        await executor.wait_started("run-b")
        executor.release_run("run-b")
        await _wait_until(lambda: executor.finished == ["run-a", "run-b"])
    finally:
        await scheduler.close()

@pytest.mark.asyncio
async def test_scheduler_serializes_same_thread_and_skips_blocked_head() -> None:
    executor = ControlledExecutor(
        {"run-a": "thread-a", "run-b": "thread-a", "run-c": "thread-b"}
    )
    scheduler = AgentScheduler(executor, max_workers=2)
    scheduler.start()
    try:
        await scheduler.enqueue("run-a")
        await scheduler.enqueue("run-b")
        await scheduler.enqueue("run-c")
        await executor.wait_started("run-a")
        await executor.wait_started("run-c")
        await asyncio.sleep(0.05)
        assert not executor.is_started("run-b")
        assert executor.max_active == 2

        executor.release_run("run-a")
        await executor.wait_started("run-b")
        assert executor.max_active == 2

        executor.release_run("run-b")
        executor.release_run("run-c")
        await _wait_until(lambda: set(executor.finished) == {"run-a", "run-b", "run-c"})
    finally:
        await scheduler.close()


@pytest.mark.asyncio
async def test_scheduler_reserved_head_blocks_later_runs_until_activated() -> None:
    executor = ControlledExecutor({"run-a": "thread-a", "run-b": "thread-b"})
    scheduler = AgentScheduler(executor, max_workers=2)
    scheduler.start()
    try:
        scheduler.reserve("run-a")
        await scheduler.enqueue("run-b")
        await asyncio.sleep(0.05)
        assert executor.calls == []

        assert await scheduler.activate("run-a") is True
        await executor.wait_started("run-a")
        await executor.wait_started("run-b")
        executor.release_run("run-a")
        executor.release_run("run-b")
        await _wait_until(lambda: set(executor.finished) == {"run-a", "run-b"})
    finally:
        await scheduler.close()


@pytest.mark.asyncio
async def test_scheduler_close_cancels_running_workers_cleanly() -> None:
    executor = ControlledExecutor({"run-a": "thread-a", "run-b": "thread-b"})
    scheduler = AgentScheduler(executor, max_workers=2)
    scheduler.start()
    await scheduler.enqueue("run-a")
    await scheduler.enqueue("run-b")
    await executor.wait_started("run-a")
    await executor.wait_started("run-b")

    await scheduler.close()

    assert set(executor.finished) == {"run-a", "run-b"}
    assert executor.active == frozenset()
    assert not any(
        task.get_name().startswith("ikaros-agent-scheduler")
        for task in asyncio.all_tasks()
    )


@pytest.mark.asyncio
async def test_two_threads_advance_concurrently_with_sequenced_events(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    provider = GatedProvider()
    events: list[JournalEvent] = []

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    loop = AgentLoop(store, {GatedProvider.id: provider}, publish)
    scheduler = AgentScheduler(loop, max_workers=2)
    try:
        first_thread, _ = store.create_thread("Concurrent one")
        second_thread, _ = store.create_thread("Concurrent two")
        first = _prepare(
            store,
            thread_id=first_thread.id,
            branch_id=first_thread.default_branch_id,
            content="first",
        )
        second = _prepare(
            store,
            thread_id=second_thread.id,
            branch_id=second_thread.default_branch_id,
            content="second",
        )

        scheduler.start([first.run_id, second.run_id])
        await provider.wait_started("first")
        await provider.wait_started("second")
        assert provider.max_active == 2
        assert store.run_status(first.run_id) == "running"
        assert store.run_status(second.run_id) == "running"

        provider.release_run("first")
        await _wait_status(store, first.run_id, "completed")
        assert store.run_status(second.run_id) == "running"

        provider.release_run("second")
        await _wait_status(store, second.run_id, "completed")
        await provider.wait_finished("first")
        await provider.wait_finished("second")

        sequences = [event.seq for event in events]
        assert sequences == sorted(sequences)
        assert len(sequences) == len(set(sequences))
        assert all(event.seq > 0 for event in events)
    finally:
        await scheduler.close()
        store.close()


@pytest.mark.asyncio
async def test_same_thread_runs_stay_serial(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    provider = GatedProvider()
    events: list[JournalEvent] = []

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    loop = AgentLoop(store, {GatedProvider.id: provider}, publish)
    scheduler = AgentScheduler(loop, max_workers=2)
    try:
        thread, _ = store.create_thread("Serial thread")
        first = _prepare(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="first",
        )
        second = _prepare(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="second",
        )

        scheduler.start([first.run_id, second.run_id])
        await provider.wait_started("first")
        await asyncio.sleep(0.05)
        assert not provider.is_started("second")
        assert store.run_status(second.run_id) == "queued"
        assert provider.max_active == 1

        provider.release_run("first")
        await provider.wait_started("second")
        assert store.run_status(first.run_id) == "completed"
        provider.release_run("second")
        await _wait_status(store, second.run_id, "completed")
    finally:
        await scheduler.close()
        store.close()


@pytest.mark.asyncio
async def test_cancelling_one_run_does_not_disturb_another(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    provider = GatedProvider()
    events: list[JournalEvent] = []

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    loop = AgentLoop(store, {GatedProvider.id: provider}, publish)
    scheduler = AgentScheduler(loop, max_workers=2)
    try:
        first_thread, _ = store.create_thread("Cancel one")
        second_thread, _ = store.create_thread("Keep running")
        first = _prepare(
            store,
            thread_id=first_thread.id,
            branch_id=first_thread.default_branch_id,
            content="first",
        )
        second = _prepare(
            store,
            thread_id=second_thread.id,
            branch_id=second_thread.default_branch_id,
            content="second",
        )

        scheduler.start([first.run_id, second.run_id])
        await provider.wait_started("first")
        await provider.wait_started("second")

        assert await scheduler.cancel(first.run_id) is True
        await _wait_status(store, first.run_id, "cancelled")
        assert store.run_status(second.run_id) == "running"
        assert provider.is_started("second")
        assert "second" in provider.active_keys

        provider.release_run("second")
        await _wait_status(store, second.run_id, "completed")
        assert any(event.run_id == second.run_id for event in events)
    finally:
        await scheduler.close()
        store.close()


@pytest.mark.asyncio
async def test_shutdown_stops_all_workers_and_running_runs(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    provider = GatedProvider()
    events: list[JournalEvent] = []

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    loop = AgentLoop(store, {GatedProvider.id: provider}, publish)
    scheduler = AgentScheduler(loop, max_workers=2)
    try:
        first_thread, _ = store.create_thread("Shutdown one")
        second_thread, _ = store.create_thread("Shutdown two")
        first = _prepare(
            store,
            thread_id=first_thread.id,
            branch_id=first_thread.default_branch_id,
            content="first",
        )
        second = _prepare(
            store,
            thread_id=second_thread.id,
            branch_id=second_thread.default_branch_id,
            content="second",
        )

        scheduler.start([first.run_id, second.run_id])
        await provider.wait_started("first")
        await provider.wait_started("second")

        await scheduler.close()

        assert store.run_status(first.run_id) == "cancelled"
        assert store.run_status(second.run_id) == "cancelled"
        assert provider.active_keys == frozenset()
        settled = {
            event.run_id: event.payload.get("reasonCode")
            for event in events
            if event.type == "run.settled"
        }
        assert settled[first.run_id] == "cancelled"
        assert settled[second.run_id] == "cancelled"
        assert not any(
            task.get_name().startswith("ikaros-agent-scheduler")
            for task in asyncio.all_tasks()
        )
    finally:
        await scheduler.close()
        store.close()


@pytest.mark.asyncio
async def test_concurrent_crash_recovery_keeps_running_and_queued_semantics(
    tmp_path: Path,
) -> None:
    database_path = tmp_path / "state.db"
    store = SqliteRuntimeStore(database_path)
    thread, _ = store.create_thread("Crash recovery")
    other_thread, _ = store.create_thread("Crash recovery other")
    running_first = _prepare(
        store,
        thread_id=thread.id,
        branch_id=thread.default_branch_id,
        content="running first",
    )
    running_second = _prepare(
        store,
        thread_id=other_thread.id,
        branch_id=other_thread.default_branch_id,
        content="running second",
    )
    queued = _prepare(
        store,
        thread_id=thread.id,
        branch_id=thread.default_branch_id,
        content="queued after crash",
    )
    store.mark_run_running(running_first.run_id)
    store.mark_run_running(running_second.run_id)
    store.close()

    store = SqliteRuntimeStore(database_path)
    try:
        recovery = store.recover_incomplete_runs()
        assert recovery.queued_run_ids == (queued.run_id,)
        assert store.run_status(running_first.run_id) == "failed"
        assert store.run_status(running_second.run_id) == "failed"
        events, _ = store.replay_events(0, 500)
        reasons = {
            event.run_id: event.payload.get("reasonCode")
            for event in events
            if event.type == "run.settled"
        }
        assert reasons[running_first.run_id] == "runtime_interrupted"
        assert reasons[running_second.run_id] == "runtime_interrupted"

        executor = ControlledExecutor({queued.run_id: "thread-a"})
        scheduler = AgentScheduler(executor, max_workers=2)
        scheduler.start(recovery.queued_run_ids)
        try:
            await executor.wait_started(queued.run_id)
            executor.release_run(queued.run_id)
            await _wait_until(lambda: queued.run_id in executor.finished)
        finally:
            await scheduler.close()
    finally:
        store.close()