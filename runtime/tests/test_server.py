from __future__ import annotations

import asyncio
import json
import os
import secrets
import signal
import sys
from asyncio.subprocess import Process
from pathlib import Path
from typing import Any, cast

import pytest
from websockets.asyncio.client import ClientConnection, connect
from websockets.asyncio.server import ServerConnection
from websockets.exceptions import InvalidStatus

from ikaros_runtime.domain import JournalEvent
from ikaros_runtime.kernel import RuntimeKernel
from ikaros_runtime.server import EventBus, _handle_connection, _parent_is_alive
from ikaros_runtime.storage import SqliteRuntimeStore


class AckFailingConnection:
    def __init__(self, messages: list[dict[str, Any]]) -> None:
        self._messages = iter(messages)
        self.sent: list[dict[str, Any]] = []

    def __aiter__(self) -> AckFailingConnection:
        return self

    async def __anext__(self) -> str:
        try:
            message = next(self._messages)
        except StopIteration as error:
            raise StopAsyncIteration from error
        return json.dumps(message)

    async def send(self, message: str) -> None:
        value = json.loads(message)
        if value.get("id") == 2:
            raise ConnectionError("simulated ACK disconnect")
        self.sent.append(value)

    async def close(self, *, code: int, reason: str) -> None:
        del code, reason


class ControlledAckConnection:
    def __init__(self, messages: list[dict[str, Any]], ack_gate: asyncio.Event) -> None:
        self._messages = iter(messages)
        self._ack_gate = ack_gate
        self.ack_started = asyncio.Event()
        self.ack_completed = asyncio.Event()
        self.sent: list[dict[str, Any]] = []

    def __aiter__(self) -> ControlledAckConnection:
        return self

    async def __anext__(self) -> str:
        try:
            message = next(self._messages)
        except StopIteration as error:
            raise StopAsyncIteration from error
        return json.dumps(message)

    async def send(self, message: str) -> None:
        value = json.loads(message)
        if value.get("id") == 2:
            self.ack_started.set()
            await self._ack_gate.wait()
            self.ack_completed.set()
        self.sent.append(value)

    async def close(self, *, code: int, reason: str) -> None:
        del code, reason


async def _start_runtime(token: str, runtime_home: Path) -> tuple[Process, dict[str, Any]]:
    process = await asyncio.create_subprocess_exec(
        sys.executable,
        "-m",
        "ikaros_runtime",
        "serve",
        "--host",
        "127.0.0.1",
        "--port",
        "0",
        f"--token={token}",
        "--parent-pid",
        str(os.getpid()),
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
        env={**os.environ, "IKAROS_HOME": str(runtime_home)},
    )
    try:
        assert process.stdout is not None
        line = await asyncio.wait_for(process.stdout.readline(), timeout=10)
        if not line:
            assert process.stderr is not None
            stderr = (await process.stderr.read()).decode(errors="replace")
            raise AssertionError(f"Runtime exited before readiness: {stderr}")
        return process, json.loads(line)
    except BaseException:
        await _stop_failed_process(process)
        raise


async def _stop_failed_process(process: Process) -> None:
    if process.returncode is not None:
        return
    process.terminate()
    try:
        await asyncio.wait_for(process.wait(), timeout=5)
    except TimeoutError:
        process.kill()
        await process.wait()


async def _rpc(
    connection: ClientConnection,
    request_id: int,
    method: str,
    params: dict[str, Any],
    notifications: list[dict[str, Any]] | None = None,
) -> dict[str, Any]:
    await connection.send(
        json.dumps({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params})
    )
    while True:
        response = json.loads(await connection.recv())
        assert response["jsonrpc"] == "2.0"
        if response.get("method") == "event":
            if notifications is not None:
                notifications.append(response["params"])
            continue
        assert response["id"] == request_id
        return cast(dict[str, Any], response)


async def _collect_run_events(
    connection: ClientConnection,
    run_id: str,
) -> list[dict[str, Any]]:
    events: list[dict[str, Any]] = []
    while True:
        message = json.loads(await asyncio.wait_for(connection.recv(), timeout=10))
        assert message["jsonrpc"] == "2.0"
        assert message["method"] == "event"
        event = message["params"]
        events.append(event)
        if event["type"] == "run.settled" and event["runId"] == run_id:
            return events


async def _replay_until_run_settled(
    connection: ClientConnection,
    run_id: str,
    *,
    request_id: int,
) -> list[dict[str, Any]]:
    cursor = 0
    events: list[dict[str, Any]] = []
    for offset in range(300):
        response = await _rpc(
            connection,
            request_id + offset,
            "event.replay",
            {"afterSeq": cursor, "limit": 1000},
            [],
        )
        page = cast(list[dict[str, Any]], response["result"]["events"])
        if page:
            assert [event["seq"] for event in page] == list(
                range(cursor + 1, cursor + len(page) + 1)
            )
            events.extend(page)
            cursor = int(response["result"]["nextAfterSeq"])
        if any(event["type"] == "run.settled" and event["runId"] == run_id for event in events):
            return events
        await asyncio.sleep(0.01)
    raise AssertionError(f"run {run_id} did not settle")


async def _initialize(uri: str, token: str) -> ClientConnection:
    connection = await connect(
        uri,
        additional_headers={"Authorization": f"Bearer {token}"},
    )
    initialized = await _rpc(
        connection,
        1,
        "initialize",
        {
            "protocolVersion": 1,
            "client": {"name": "runtime-test", "version": "0.1.0"},
        },
    )
    assert initialized["result"] == {
        "protocolVersion": 1,
        "server": {"name": "ikaros-runtime", "version": "0.1.0"},
        "capabilities": {
            "threads": True,
            "turns": True,
            "eventReplay": True,
            "streaming": True,
            "scriptedProvider": True,
            "runCancellation": True,
            "tools": ["process.run"],
            "executionPolicy": "full_access",
        },
    }
    return connection


async def _shutdown(connection: ClientConnection, process: Process, request_id: int) -> None:
    shutdown = await _rpc(connection, request_id, "runtime.shutdown", {})
    assert shutdown["result"] == {"accepted": True}
    await connection.close()
    assert await asyncio.wait_for(process.wait(), timeout=5) == 0


def _journal_event(seq: int) -> JournalEvent:
    return JournalEvent(
        seq=seq,
        type="test.event",
        thread_id=None,
        branch_id=None,
        turn_id=None,
        run_id=None,
        item_id=None,
        timestamp="2026-08-11T12:00:00.000Z",
        payload={},
    )


def test_parent_process_probe_is_non_destructive() -> None:
    assert _parent_is_alive(os.getpid())
    assert not _parent_is_alive(2_147_483_647)


@pytest.mark.asyncio
async def test_event_bus_releases_notifications_in_sequence_order() -> None:
    observed: list[int] = []
    event_bus = EventBus(next_seq=5)
    event_bus.subscribe(lambda event: observed.append(event.seq))

    await event_bus.publish(_journal_event(6))
    assert observed == []

    await event_bus.publish(_journal_event(5))
    assert observed == [5, 6]


@pytest.mark.asyncio
async def test_event_bus_drops_a_slow_sink_without_blocking_other_subscribers() -> None:
    observed: list[int] = []
    event_bus = EventBus(next_seq=1)

    def full_sink(_event: JournalEvent) -> None:
        raise asyncio.QueueFull

    event_bus.subscribe(full_sink)
    event_bus.subscribe(lambda event: observed.append(event.seq))

    await event_bus.publish(_journal_event(1))
    await event_bus.publish(_journal_event(2))

    assert observed == [1, 2]


@pytest.mark.asyncio
async def test_accepted_turn_runs_even_when_the_ack_connection_disconnects(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    thread, _ = store.create_thread("ACK disconnect")
    event_bus = EventBus(next_seq=store.latest_sequence() + 1)
    kernel = RuntimeKernel(store, event_bus.publish)
    kernel.start()
    connection = AckFailingConnection(
        [
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {"protocolVersion": 1},
            },
            {
                "jsonrpc": "2.0",
                "id": 2,
                "method": "turn.start",
                "params": {
                    "threadId": thread.id,
                    "branchId": thread.default_branch_id,
                    "content": "continue after disconnect",
                    "providerId": "scripted",
                    "modelId": "scripted-v1",
                },
            },
        ]
    )

    try:
        with pytest.raises(ConnectionError, match="simulated ACK disconnect"):
            await _handle_connection(
                cast(ServerConnection, connection),
                asyncio.Event(),
                kernel,
                event_bus,
            )

        settled: list[JournalEvent] = []
        for _ in range(100):
            replayed, _latest_seq = store.replay_events(0, 1000)
            settled = [event for event in replayed if event.type == "run.settled"]
            if settled:
                break
            await asyncio.sleep(0.01)

        assert len(settled) == 1
        assert settled[0].payload["status"] == "completed"
    finally:
        await kernel.close()
        store.close()


@pytest.mark.asyncio
async def test_accepted_cancel_runs_even_when_the_ack_connection_disconnects(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    thread, _ = store.create_thread("Cancel ACK disconnect")
    event_bus = EventBus(next_seq=store.latest_sequence() + 1)
    kernel = RuntimeKernel(store, event_bus.publish)
    kernel.start()
    started = kernel.start_turn(
        {
            "threadId": thread.id,
            "branchId": thread.default_branch_id,
            "content": "x" * 4000,
            "providerId": "scripted",
            "modelId": "scripted-v1",
        }
    )
    run_id = cast(str, started.result["runId"])
    await kernel.finish_command(started)

    try:
        for _ in range(100):
            if store.run_status(run_id) == "running":
                break
            await asyncio.sleep(0.01)
        assert store.run_status(run_id) == "running"
        connection = AckFailingConnection(
            [
                {
                    "jsonrpc": "2.0",
                    "id": 1,
                    "method": "initialize",
                    "params": {"protocolVersion": 1},
                },
                {
                    "jsonrpc": "2.0",
                    "id": 2,
                    "method": "run.cancel",
                    "params": {"runId": run_id},
                },
            ]
        )

        with pytest.raises(ConnectionError, match="simulated ACK disconnect"):
            await _handle_connection(
                cast(ServerConnection, connection),
                asyncio.Event(),
                kernel,
                event_bus,
            )

        for _ in range(100):
            if store.run_status(run_id) == "cancelled":
                break
            await asyncio.sleep(0.01)
        assert store.run_status(run_id) == "cancelled"
        replayed, _ = store.replay_events(0, 1000)
        settled = [
            event for event in replayed if event.type == "run.settled" and event.run_id == run_id
        ]
        assert len(settled) == 1
        assert settled[0].payload["status"] == "cancelled"
    finally:
        await kernel.close()
        store.close()


@pytest.mark.asyncio
async def test_cancel_between_turn_prepare_and_activation_prevents_execution(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    thread, _ = store.create_thread("Cross-client cancellation")
    event_bus = EventBus(next_seq=store.latest_sequence() + 1)
    kernel = RuntimeKernel(store, event_bus.publish)
    kernel.start()

    try:
        started = kernel.start_turn(
            {
                "threadId": thread.id,
                "branchId": thread.default_branch_id,
                "content": "must never reach the provider",
                "providerId": "scripted",
                "modelId": "scripted-v1",
            }
        )
        run_id = cast(str, started.result["runId"])
        cancelled = kernel.cancel_run({"runId": run_id})
        assert cancelled.result == {
            "accepted": True,
            "runId": run_id,
            "status": "queued",
        }

        # Simulate two client handlers whose post-ACK actions interleave: the
        # cancel ACK completes before the original turn.start ACK completes.
        await kernel.finish_command(cancelled)
        await kernel.finish_command(started)
        await asyncio.sleep(0.02)

        assert store.run_status(run_id) == "cancelled"
        replayed, _ = store.replay_events(0, 1000)
        run_events = [event for event in replayed if event.run_id == run_id]
        assert not any(
            event.type in {"item.started", "item.delta"}
            or (event.type == "run.state_changed" and event.payload["status"] == "running")
            for event in run_events
        )
        settled = [event for event in run_events if event.type == "run.settled"]
        assert len(settled) == 1
        assert settled[0].payload["status"] == "cancelled"
    finally:
        await kernel.close()
        store.close()


@pytest.mark.asyncio
async def test_reverse_ack_order_preserves_persisted_turn_execution_order(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    thread, _ = store.create_thread("Reverse ACK order")
    event_bus = EventBus(next_seq=store.latest_sequence() + 1)
    kernel = RuntimeKernel(store, event_bus.publish)
    kernel.start()
    first_ack_gate = asyncio.Event()
    second_ack_gate = asyncio.Event()
    second_ack_gate.set()
    first_connection = ControlledAckConnection(
        [
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {"protocolVersion": 1},
            },
            {
                "jsonrpc": "2.0",
                "id": 2,
                "method": "turn.start",
                "params": {
                    "threadId": thread.id,
                    "branchId": thread.default_branch_id,
                    "content": "alpha",
                    "providerId": "scripted",
                    "modelId": "scripted-v1",
                },
            },
        ],
        first_ack_gate,
    )
    second_connection = ControlledAckConnection(
        [
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {"protocolVersion": 1},
            },
            {
                "jsonrpc": "2.0",
                "id": 2,
                "method": "turn.start",
                "params": {
                    "threadId": thread.id,
                    "branchId": thread.default_branch_id,
                    "content": "beta",
                    "providerId": "scripted",
                    "modelId": "scripted-v1",
                },
            },
        ],
        second_ack_gate,
    )
    tasks: list[asyncio.Task[None]] = []

    try:
        first_task = asyncio.create_task(
            _handle_connection(
                cast(ServerConnection, first_connection),
                asyncio.Event(),
                kernel,
                event_bus,
            )
        )
        tasks.append(first_task)
        await asyncio.wait_for(first_connection.ack_started.wait(), timeout=1)

        second_task = asyncio.create_task(
            _handle_connection(
                cast(ServerConnection, second_connection),
                asyncio.Event(),
                kernel,
                event_bus,
            )
        )
        tasks.append(second_task)
        await asyncio.wait_for(second_connection.ack_completed.wait(), timeout=1)
        await asyncio.wait_for(second_task, timeout=1)
        assert not first_connection.ack_completed.is_set()
        assert not first_task.done()

        second_response = next(value for value in second_connection.sent if value.get("id") == 2)
        second_run_id = cast(str, second_response["result"]["runId"])
        assert store.run_status(second_run_id) == "queued"
        replayed_before_first_ack, _ = store.replay_events(0, 1000)
        assert not any(
            event.type == "run.state_changed"
            and event.run_id == second_run_id
            and event.payload["status"] == "running"
            for event in replayed_before_first_ack
        )

        first_ack_gate.set()
        await asyncio.wait_for(first_task, timeout=1)
        first_response = next(value for value in first_connection.sent if value.get("id") == 2)
        first_run_id = cast(str, first_response["result"]["runId"])

        replayed: list[JournalEvent] = []
        for _ in range(500):
            replayed, _ = store.replay_events(0, 1000)
            settled_ids = [
                event.run_id
                for event in replayed
                if event.type == "run.settled" and event.run_id in {first_run_id, second_run_id}
            ]
            if len(settled_ids) == 2:
                break
            await asyncio.sleep(0.005)

        assert settled_ids == [first_run_id, second_run_id]
        second_answer = next(
            event.payload["item"]["content"]
            for event in replayed
            if event.type == "item.completed"
            and event.run_id == second_run_id
            and event.payload.get("item", {}).get("role") == "assistant"
        )
        assert second_answer == (
            "Previous user: alpha\n"
            "Previous assistant: Scripted response to: alpha\n"
            "Current user: beta"
        )
    finally:
        first_ack_gate.set()
        second_ack_gate.set()
        await asyncio.gather(*tasks, return_exceptions=True)
        await kernel.close()
        store.close()


@pytest.mark.asyncio
async def test_natural_completion_can_win_after_cancel_is_accepted(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    thread, _ = store.create_thread("Completion race")
    event_bus = EventBus(next_seq=store.latest_sequence() + 1)
    kernel = RuntimeKernel(store, event_bus.publish)
    kernel.start()

    try:
        started = kernel.start_turn(
            {
                "threadId": thread.id,
                "branchId": thread.default_branch_id,
                "content": "completion-wins-" + ("x" * 1000),
                "providerId": "scripted",
                "modelId": "scripted-v1",
            }
        )
        run_id = cast(str, started.result["runId"])
        await kernel.finish_command(started)
        for _ in range(100):
            if store.run_status(run_id) == "running":
                break
            await asyncio.sleep(0.001)
        assert store.run_status(run_id) == "running"

        # The response represents the ACK-time decision. Post-ACK cancellation
        # is intentionally delayed here so the natural terminal transaction wins.
        cancellation = kernel.cancel_run({"runId": run_id})
        assert cancellation.result == {
            "accepted": True,
            "runId": run_id,
            "status": "running",
        }
        for _ in range(1000):
            if store.run_status(run_id) == "completed":
                break
            await asyncio.sleep(0.001)
        assert store.run_status(run_id) == "completed"

        await kernel.finish_command(cancellation)
        assert store.run_status(run_id) == "completed"
        replayed, _ = store.replay_events(0, 2000)
        settled = [
            event for event in replayed if event.type == "run.settled" and event.run_id == run_id
        ]
        assert len(settled) == 1
        assert settled[0].payload["status"] == "completed"
    finally:
        await kernel.close()
        store.close()


@pytest.mark.asyncio
async def test_cli_server_requires_authentication_and_completes_handshake(
    tmp_path: Path,
) -> None:
    token = "-option-shaped-launch-token"
    process, ready = await _start_runtime(token, tmp_path)
    uri = f"ws://{ready['host']}:{ready['port']}"

    try:
        assert ready["type"] == "ikaros_runtime.ready"
        assert ready["protocolVersion"] == 1
        assert ready["host"] == "127.0.0.1"
        assert ready["port"] > 0
        assert ready["pid"] > 0
        assert (tmp_path / "state.db").is_file()
        assert not (tmp_path / "config.yaml").exists()

        with pytest.raises(InvalidStatus) as unauthorized:
            async with connect(uri):
                await asyncio.sleep(0)
        assert unauthorized.value.response.status_code == 401

        connection = await _initialize(uri, token)
        await _shutdown(connection, process, 2)
    finally:
        await _stop_failed_process(process)


@pytest.mark.asyncio
async def test_thread_and_event_journal_survive_runtime_restart(tmp_path: Path) -> None:
    first_token = secrets.token_urlsafe(32)
    first_process, first_ready = await _start_runtime(first_token, tmp_path)
    try:
        first = await _initialize(
            f"ws://{first_ready['host']}:{first_ready['port']}",
            first_token,
        )
        empty = await _rpc(first, 2, "thread.list", {})
        assert empty["result"] == {"threads": []}

        created = await _rpc(first, 3, "thread.create", {"title": "Persistent thread"})
        thread = created["result"]["thread"]
        event = created["result"]["event"]
        assert thread["title"] == "Persistent thread"
        assert thread["defaultBranchId"].startswith("branch_")
        assert event["seq"] == 1
        assert event["type"] == "thread.created"
        assert event["threadId"] == thread["id"]
        assert event["branchId"] == thread["defaultBranchId"]

        replayed = await _rpc(first, 4, "event.replay", {"afterSeq": 0})
        assert replayed["result"] == {
            "events": [event],
            "latestSeq": 1,
            "nextAfterSeq": 1,
            "hasMore": False,
        }
        await _shutdown(first, first_process, 5)
    finally:
        await _stop_failed_process(first_process)

    second_token = secrets.token_urlsafe(32)
    second_process, second_ready = await _start_runtime(second_token, tmp_path)
    try:
        second = await _initialize(
            f"ws://{second_ready['host']}:{second_ready['port']}",
            second_token,
        )
        listed = await _rpc(second, 2, "thread.list", {})
        assert listed["result"] == {"threads": [thread]}

        caught_up = await _rpc(second, 3, "event.replay", {"afterSeq": 1})
        assert caught_up["result"] == {
            "events": [],
            "latestSeq": 1,
            "nextAfterSeq": 1,
            "hasMore": False,
        }

        next_created = await _rpc(second, 4, "thread.create", {})
        assert next_created["result"]["event"]["seq"] == 2
        page = await _rpc(second, 5, "event.replay", {"afterSeq": 0, "limit": 1})
        assert [event["seq"] for event in page["result"]["events"]] == [1]
        assert page["result"]["latestSeq"] == 2
        assert page["result"]["nextAfterSeq"] == 1
        assert page["result"]["hasMore"] is True
        await _shutdown(second, second_process, 6)
    finally:
        await _stop_failed_process(second_process)


@pytest.mark.asyncio
async def test_client_request_ids_make_mutating_commands_exactly_once(tmp_path: Path) -> None:
    token = secrets.token_urlsafe(32)
    process, ready = await _start_runtime(token, tmp_path)
    try:
        connection = await _initialize(f"ws://{ready['host']}:{ready['port']}", token)
        create_params = {
            "title": "Idempotent commands",
            "clientRequestId": "desktop-create-request",
        }
        first_created = await _rpc(connection, 2, "thread.create", create_params)
        repeated_created = await _rpc(connection, 3, "thread.create", create_params, [])
        assert repeated_created["result"] == first_created["result"]
        thread = first_created["result"]["thread"]

        start_params = {
            "threadId": thread["id"],
            "branchId": thread["defaultBranchId"],
            "content": "execute exactly once",
            "providerId": "scripted",
            "modelId": "scripted-v1",
            "clientRequestId": "desktop-turn-request",
        }
        first_started = await _rpc(connection, 4, "turn.start", start_params, [])
        repeated_started = await _rpc(connection, 5, "turn.start", start_params, [])
        assert repeated_started["result"] == first_started["result"]

        run_id = first_started["result"]["runId"]
        replayed = await _replay_until_run_settled(
            connection,
            run_id,
            request_id=6,
        )
        run_events = [event for event in replayed if event["runId"] == run_id]
        assert sum(event["type"] == "run.settled" for event in run_events) == 1
        assert (
            sum(
                event["type"] == "item.completed"
                and event["payload"].get("item", {}).get("role") == "user"
                for event in run_events
            )
            == 1
        )
        user_event = next(
            event
            for event in run_events
            if event["type"] == "item.completed"
            and event["payload"].get("item", {}).get("role") == "user"
        )
        assert user_event["payload"]["clientRequestId"] == "desktop-turn-request"
        listed = await _rpc(connection, 100, "thread.list", {}, [])
        assert len(listed["result"]["threads"]) == 1
        await _shutdown(connection, process, 101)
    finally:
        await _stop_failed_process(process)


@pytest.mark.asyncio
async def test_scripted_provider_streams_two_contextual_turns_and_settles_once(
    tmp_path: Path,
) -> None:
    token = secrets.token_urlsafe(32)
    process, ready = await _start_runtime(token, tmp_path)
    try:
        connection = await _initialize(f"ws://{ready['host']}:{ready['port']}", token)
        created = await _rpc(connection, 2, "thread.create", {"title": "Two turns"})
        thread = created["result"]["thread"]

        first_started = await _rpc(
            connection,
            3,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": "alpha",
                "providerId": "scripted",
                "modelId": "scripted-v1",
            },
        )
        first_run_id = first_started["result"]["runId"]
        first_events = await _collect_run_events(connection, first_run_id)
        first_types = [event["type"] for event in first_events]
        assert first_types[:4] == [
            "item.completed",
            "run.state_changed",
            "run.state_changed",
            "item.started",
        ]
        assert first_types.count("item.delta") >= 2
        assert first_types[-2:] == ["item.completed", "run.settled"]
        assert (
            sum(
                event["type"] == "run.settled" and event["runId"] == first_run_id
                for event in first_events
            )
            == 1
        )
        first_answer = next(
            event["payload"]["item"]["content"]
            for event in first_events
            if event["type"] == "item.completed" and event["payload"]["item"]["role"] == "assistant"
        )
        assert first_answer == "Scripted response to: alpha"

        second_started = await _rpc(
            connection,
            4,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": "beta",
                "providerId": "scripted",
                "modelId": "scripted-v1",
            },
        )
        second_run_id = second_started["result"]["runId"]
        second_events = await _collect_run_events(connection, second_run_id)
        second_answer = next(
            event["payload"]["item"]["content"]
            for event in second_events
            if event["type"] == "item.completed" and event["payload"]["item"]["role"] == "assistant"
        )
        assert second_answer == (
            "Previous user: alpha\n"
            "Previous assistant: Scripted response to: alpha\n"
            "Current user: beta"
        )
        assert [event["seq"] for event in first_events + second_events] == sorted(
            event["seq"] for event in first_events + second_events
        )
        assert sum(event["type"] == "run.settled" for event in second_events) == 1
        await _shutdown(connection, process, 5)
    finally:
        await _stop_failed_process(process)


@pytest.mark.asyncio
async def test_scripted_provider_runs_a_real_command_and_continues_the_conversation(
    tmp_path: Path,
) -> None:
    token = secrets.token_urlsafe(32)
    process, ready = await _start_runtime(token, tmp_path)
    try:
        connection = await _initialize(f"ws://{ready['host']}:{ready['port']}", token)
        created = await _rpc(connection, 2, "thread.create", {"title": "Tool turn"})
        thread = created["result"]["thread"]
        command = (
            "Write-Output 'gate5-tool-output'"
            if os.name == "nt"
            else "printf 'gate5-tool-output\\n'"
        )
        prompt = f"/process.run {command}"
        started = await _rpc(
            connection,
            3,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": prompt,
                "providerId": "scripted",
                "modelId": "scripted-v1",
            },
        )
        run_id = started["result"]["runId"]
        events = await _collect_run_events(connection, run_id)
        run_events = [event for event in events if event["runId"] == run_id]
        tool_call_events = [
            event
            for event in run_events
            if event["type"] in {"item.started", "item.completed"}
            and event["payload"].get("item", {}).get("kind") == "tool_call"
        ]
        tool_results = [
            event
            for event in run_events
            if event["type"] == "item.completed"
            and event["payload"].get("item", {}).get("kind") == "tool_result"
        ]
        assistant = next(
            event["payload"]["item"]
            for event in run_events
            if event["type"] == "item.completed"
            and event["payload"].get("item", {}).get("kind") == "message"
            and event["payload"]["item"].get("role") == "assistant"
        )

        assert [event["payload"]["item"]["status"] for event in tool_call_events] == [
            "running",
            "completed",
        ]
        assert len(tool_results) == 1
        result = tool_results[0]["payload"]["item"]["data"]["result"]
        assert result["toolName"] == "process_run"
        assert result["exitCode"] == 0
        assert result["ok"] is True
        assert "gate5-tool-output" in result["stdout"]
        assert "gate5-tool-output" in assistant["content"]
        assert not any(event["type"].startswith("permission.") for event in run_events)
        assert sum(event["type"] == "run.settled" for event in run_events) == 1

        replayed = await _replay_until_run_settled(
            connection,
            run_id,
            request_id=100,
        )
        replayed_run = [event for event in replayed if event["runId"] == run_id]
        assert [event["seq"] for event in replayed_run] == [event["seq"] for event in run_events]

        follow_up = await _rpc(
            connection,
            200,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": "what happened",
                "providerId": "scripted",
                "modelId": "scripted-v1",
            },
        )
        follow_up_events = await _collect_run_events(
            connection,
            follow_up["result"]["runId"],
        )
        follow_up_answer = next(
            event["payload"]["item"]["content"]
            for event in follow_up_events
            if event["type"] == "item.completed"
            and event["payload"].get("item", {}).get("role") == "assistant"
        )
        assert f"Previous user: {prompt}" in follow_up_answer
        assert "Previous assistant: Command exited with code 0." in follow_up_answer
        assert "Current user: what happened" in follow_up_answer
        await _shutdown(connection, process, 201)
    finally:
        await _stop_failed_process(process)


@pytest.mark.asyncio
async def test_cancelling_process_run_preserves_partial_output_and_clears_running_tool(
    tmp_path: Path,
) -> None:
    token = secrets.token_urlsafe(32)
    process, ready = await _start_runtime(token, tmp_path)
    try:
        connection = await _initialize(f"ws://{ready['host']}:{ready['port']}", token)
        created = await _rpc(connection, 2, "thread.create", {"title": "Cancel tool"})
        thread = created["result"]["thread"]
        command = (
            "Write-Output 'before-stop'; Start-Sleep -Seconds 60"
            if os.name == "nt"
            else "printf 'before-stop\\n'; sleep 60"
        )
        started = await _rpc(
            connection,
            3,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": f"/process.run {command}",
                "providerId": "scripted",
                "modelId": "scripted-v1",
            },
        )
        run_id = started["result"]["runId"]
        events: list[dict[str, Any]] = []
        while True:
            message = json.loads(await asyncio.wait_for(connection.recv(), timeout=10))
            assert message["method"] == "event"
            event = message["params"]
            events.append(event)
            if (
                event["runId"] == run_id
                and event["type"] == "item.started"
                and event["payload"].get("item", {}).get("kind") == "tool_call"
            ):
                break
        await asyncio.sleep(0.75)
        notifications: list[dict[str, Any]] = []
        cancelled = await _rpc(
            connection,
            4,
            "run.cancel",
            {"runId": run_id},
            notifications,
        )
        assert cancelled["result"] == {
            "accepted": True,
            "runId": run_id,
            "status": "running",
        }
        events.extend(notifications)
        events.extend(await _collect_run_events(connection, run_id))
        run_events = [event for event in events if event["runId"] == run_id]
        tool_call_terminal = next(
            event["payload"]["item"]
            for event in run_events
            if event["type"] == "item.completed"
            and event["payload"].get("item", {}).get("kind") == "tool_call"
        )
        tool_result = next(
            event["payload"]["item"]
            for event in run_events
            if event["type"] == "item.completed"
            and event["payload"].get("item", {}).get("kind") == "tool_result"
        )
        settled = [event for event in run_events if event["type"] == "run.settled"]

        assert tool_call_terminal["status"] == "cancelled"
        assert tool_result["status"] == "cancelled"
        assert tool_result["data"]["result"]["cancelled"] is True
        assert "before-stop" in tool_result["data"]["result"]["stdout"]
        assert len(settled) == 1
        assert settled[0]["payload"]["status"] == "cancelled"
        assert not any(
            event["type"] == "item.completed"
            and event["payload"].get("item", {}).get("role") == "assistant"
            and event["payload"].get("item", {}).get("kind") == "message"
            for event in run_events
        )
        await _shutdown(connection, process, 5)
    finally:
        await _stop_failed_process(process)


@pytest.mark.asyncio
async def test_queued_turns_stream_events_in_seq_order_and_run_serially(tmp_path: Path) -> None:
    token = secrets.token_urlsafe(32)
    process, ready = await _start_runtime(token, tmp_path)
    try:
        connection = await _initialize(f"ws://{ready['host']}:{ready['port']}", token)
        notifications: list[dict[str, Any]] = []
        created = await _rpc(
            connection,
            2,
            "thread.create",
            {"title": "Queued turns"},
            notifications,
        )
        thread = created["result"]["thread"]
        first_content = f"alpha-{'x' * 1200}"
        first = await _rpc(
            connection,
            3,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": first_content,
                "providerId": "scripted",
                "modelId": "scripted-v1",
            },
            notifications,
        )
        second = await _rpc(
            connection,
            4,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": "beta",
                "providerId": "scripted",
                "modelId": "scripted-v1",
            },
            notifications,
        )
        run_ids = {first["result"]["runId"], second["result"]["runId"]}
        settled_ids = {
            event["runId"]
            for event in notifications
            if event["type"] == "run.settled" and event["runId"] in run_ids
        }
        while settled_ids != run_ids:
            message = json.loads(await asyncio.wait_for(connection.recv(), timeout=10))
            assert message["method"] == "event"
            event = message["params"]
            notifications.append(event)
            if event["type"] == "run.settled" and event["runId"] in run_ids:
                settled_ids.add(event["runId"])

        sequences = [event["seq"] for event in notifications]
        assert sequences == sorted(sequences)
        assert len(sequences) == len(set(sequences))
        first_settled_seq = next(
            event["seq"]
            for event in notifications
            if event["type"] == "run.settled" and event["runId"] == first["result"]["runId"]
        )
        second_running_seq = next(
            event["seq"]
            for event in notifications
            if event["type"] == "run.state_changed"
            and event["runId"] == second["result"]["runId"]
            and event["payload"]["status"] == "running"
        )
        assert second_running_seq > first_settled_seq
        second_answer = next(
            event["payload"]["item"]["content"]
            for event in notifications
            if event["type"] == "item.completed"
            and event["runId"] == second["result"]["runId"]
            and event["payload"]["item"]["role"] == "assistant"
        )
        assert second_answer.startswith(f"Previous user: {first_content}")
        assert second_answer.endswith("Current user: beta")
        await _shutdown(connection, process, 5)
    finally:
        await _stop_failed_process(process)


@pytest.mark.asyncio
async def test_jsonrpc_cancel_queued_run_never_executes_provider_and_replays_once(
    tmp_path: Path,
) -> None:
    token = secrets.token_urlsafe(32)
    process, ready = await _start_runtime(token, tmp_path)
    try:
        connection = await _initialize(f"ws://{ready['host']}:{ready['port']}", token)
        created = await _rpc(connection, 2, "thread.create", {"title": "Queued cancel"})
        thread = created["result"]["thread"]
        notifications: list[dict[str, Any]] = []
        active = await _rpc(
            connection,
            3,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": "hold-active-" + ("x" * 12000),
                "providerId": "scripted",
                "modelId": "scripted-v1",
            },
            notifications,
        )
        active_run_id = active["result"]["runId"]
        while not any(
            event["type"] == "item.delta" and event["runId"] == active_run_id
            for event in notifications
        ):
            message = json.loads(await asyncio.wait_for(connection.recv(), timeout=5))
            assert message["method"] == "event"
            notifications.append(message["params"])

        queued = await _rpc(
            connection,
            4,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": "queued-provider-must-not-run",
                "providerId": "scripted",
                "modelId": "scripted-v1",
            },
            notifications,
        )
        queued_run_id = queued["result"]["runId"]
        before_ack: list[dict[str, Any]] = []
        cancelled = await _rpc(
            connection,
            5,
            "run.cancel",
            {"runId": queued_run_id},
            before_ack,
        )
        assert cancelled["result"] == {
            "accepted": True,
            "runId": queued_run_id,
            "status": "queued",
        }
        assert not any(
            event["type"] == "run.settled" and event["runId"] == queued_run_id
            for event in before_ack
        )

        replayed = await _replay_until_run_settled(
            connection,
            queued_run_id,
            request_id=6,
        )
        run_events = [event for event in replayed if event["runId"] == queued_run_id]
        assert not any(
            event["type"] in {"item.started", "item.delta"}
            or (event["type"] == "run.state_changed" and event["payload"]["status"] == "running")
            for event in run_events
        )
        settled = [event for event in run_events if event["type"] == "run.settled"]
        assert len(settled) == 1
        assert settled[0]["payload"]["status"] == "cancelled"

        inspection = SqliteRuntimeStore(tmp_path / "state.db")
        try:
            assert inspection.run_status(queued_run_id) == "cancelled"
        finally:
            inspection.close()
        await _shutdown(connection, process, 400)
    finally:
        await _stop_failed_process(process)


@pytest.mark.asyncio
async def test_runtime_shutdown_settles_active_and_queued_runs_without_recovery(
    tmp_path: Path,
) -> None:
    first_token = secrets.token_urlsafe(32)
    first_process, first_ready = await _start_runtime(first_token, tmp_path)
    try:
        first = await _initialize(
            f"ws://{first_ready['host']}:{first_ready['port']}",
            first_token,
        )
        created = await _rpc(first, 2, "thread.create", {"title": "Clean shutdown"})
        thread = created["result"]["thread"]
        notifications: list[dict[str, Any]] = []
        active = await _rpc(
            first,
            3,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": "shutdown-active-" + ("x" * 12000),
                "providerId": "scripted",
                "modelId": "scripted-v1",
            },
            notifications,
        )
        active_run_id = active["result"]["runId"]
        while not any(
            event["type"] == "item.delta" and event["runId"] == active_run_id
            for event in notifications
        ):
            message = json.loads(await asyncio.wait_for(first.recv(), timeout=5))
            assert message["method"] == "event"
            notifications.append(message["params"])

        queued = await _rpc(
            first,
            4,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": "shutdown-queued",
                "providerId": "scripted",
                "modelId": "scripted-v1",
            },
            notifications,
        )
        queued_run_id = queued["result"]["runId"]
        await _shutdown(first, first_process, 5)
    finally:
        await _stop_failed_process(first_process)

    inspection = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        assert inspection.run_status(active_run_id) == "cancelled"
        assert inspection.run_status(queued_run_id) == "cancelled"
        before_restart, before_restart_latest = inspection.replay_events(0, 5000)
        for run_id in (active_run_id, queued_run_id):
            settled = [
                event
                for event in before_restart
                if event.type == "run.settled" and event.run_id == run_id
            ]
            assert len(settled) == 1
            assert settled[0].payload["status"] == "cancelled"
        assert not any(
            event.type == "run.state_changed"
            and event.run_id == queued_run_id
            and event.payload["status"] == "running"
            for event in before_restart
        )
    finally:
        inspection.close()

    second_token = secrets.token_urlsafe(32)
    second_process, second_ready = await _start_runtime(second_token, tmp_path)
    try:
        second = await _initialize(
            f"ws://{second_ready['host']}:{second_ready['port']}",
            second_token,
        )
        await asyncio.sleep(0.05)
        replayed = await _rpc(second, 2, "event.replay", {"afterSeq": 0, "limit": 1000})
        assert replayed["result"]["latestSeq"] == before_restart_latest
        wire_events = cast(list[dict[str, Any]], replayed["result"]["events"])
        for run_id in (active_run_id, queued_run_id):
            wire_settled = [
                event
                for event in wire_events
                if event["type"] == "run.settled" and event["runId"] == run_id
            ]
            assert len(wire_settled) == 1
            assert wire_settled[0]["payload"]["status"] == "cancelled"
        await _shutdown(second, second_process, 3)
    finally:
        await _stop_failed_process(second_process)


@pytest.mark.asyncio
async def test_run_cancel_ack_precedes_canonical_cancelled_events(tmp_path: Path) -> None:
    token = secrets.token_urlsafe(32)
    process, ready = await _start_runtime(token, tmp_path)
    try:
        connection = await _initialize(f"ws://{ready['host']}:{ready['port']}", token)
        created = await _rpc(connection, 2, "thread.create", {"title": "Stop"})
        thread = created["result"]["thread"]
        notifications: list[dict[str, Any]] = []
        started = await _rpc(
            connection,
            3,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": "cancel-" + ("x" * 5000),
                "providerId": "scripted",
                "modelId": "scripted-v1",
            },
            notifications,
        )
        run_id = started["result"]["runId"]
        while not any(
            event["type"] == "item.delta" and event["runId"] == run_id for event in notifications
        ):
            message = json.loads(await asyncio.wait_for(connection.recv(), timeout=5))
            assert message["method"] == "event"
            notifications.append(message["params"])

        before_ack: list[dict[str, Any]] = []
        cancelled = await _rpc(
            connection,
            4,
            "run.cancel",
            {"runId": run_id},
            before_ack,
        )
        assert cancelled["result"] == {
            "accepted": True,
            "runId": run_id,
            "status": "running",
        }
        assert not any(
            event["type"] == "run.settled" and event["runId"] == run_id for event in before_ack
        )

        terminal_events = await _collect_run_events(connection, run_id)
        terminal = [
            event
            for event in terminal_events
            if event["runId"] == run_id
            and event["type"] in {"item.completed", "run.settled"}
            and (
                event["type"] == "run.settled"
                or event["payload"].get("item", {}).get("role") == "assistant"
            )
        ]
        assert [event["type"] for event in terminal] == ["item.completed", "run.settled"]
        assert terminal[0]["payload"]["item"]["status"] == "cancelled"
        assert terminal[0]["payload"]["item"]["content"]
        assert terminal[1]["payload"]["status"] == "cancelled"

        repeated = await _rpc(connection, 5, "run.cancel", {"runId": run_id})
        assert repeated["result"] == {
            "accepted": False,
            "runId": run_id,
            "status": "cancelled",
        }
        unknown = await _rpc(connection, 6, "run.cancel", {"runId": "run_missing"})
        assert unknown["error"]["code"] == -32602
        await _shutdown(connection, process, 7)
    finally:
        await _stop_failed_process(process)


@pytest.mark.asyncio
async def test_websocket_disconnect_does_not_cancel_run_and_replay_has_no_gaps(
    tmp_path: Path,
) -> None:
    token = secrets.token_urlsafe(32)
    process, ready = await _start_runtime(token, tmp_path)
    uri = f"ws://{ready['host']}:{ready['port']}"
    try:
        first = await _initialize(uri, token)
        created = await _rpc(first, 2, "thread.create", {"title": "Reconnect"})
        thread = created["result"]["thread"]
        started = await _rpc(
            first,
            3,
            "turn.start",
            {
                "threadId": thread["id"],
                "branchId": thread["defaultBranchId"],
                "content": "reconnect-" + ("x" * 3000),
                "providerId": "scripted",
                "modelId": "scripted-v1",
            },
        )
        run_id = started["result"]["runId"]
        while True:
            message = json.loads(await asyncio.wait_for(first.recv(), timeout=5))
            if message["params"]["type"] == "item.delta":
                break
        await first.close()

        second = await _initialize(uri, token)
        replayed = await _replay_until_run_settled(second, run_id, request_id=2)
        run_events = [event for event in replayed if event["runId"] == run_id]
        assert process.returncode is None
        assert [event["seq"] for event in replayed] == list(range(1, len(replayed) + 1))
        assert len({event["seq"] for event in replayed}) == len(replayed)
        assert sum(event["type"] == "run.settled" for event in run_events) == 1
        assert (
            next(event for event in run_events if event["type"] == "run.settled")["payload"][
                "status"
            ]
            == "completed"
        )
        await _shutdown(second, process, 400)
    finally:
        await _stop_failed_process(process)


@pytest.mark.asyncio
async def test_runtime_restart_fails_running_run_and_resumes_queued_run(
    tmp_path: Path,
) -> None:
    first_token = secrets.token_urlsafe(32)
    first_process, first_ready = await _start_runtime(first_token, tmp_path)
    try:
        first = await _initialize(
            f"ws://{first_ready['host']}:{first_ready['port']}",
            first_token,
        )
        try:
            created = await _rpc(first, 2, "thread.create", {"title": "Crash recovery"})
            thread = created["result"]["thread"]
            notifications: list[dict[str, Any]] = []
            running = await _rpc(
                first,
                3,
                "turn.start",
                {
                    "threadId": thread["id"],
                    "branchId": thread["defaultBranchId"],
                    "content": "crash-" + ("x" * 1200),
                    "providerId": "scripted",
                    "modelId": "scripted-v1",
                },
                notifications,
            )
            queued = await _rpc(
                first,
                4,
                "turn.start",
                {
                    "threadId": thread["id"],
                    "branchId": thread["defaultBranchId"],
                    "content": "resume queued",
                    "providerId": "scripted",
                    "modelId": "scripted-v1",
                },
                notifications,
            )
            running_id = running["result"]["runId"]
            queued_id = queued["result"]["runId"]
            while not any(
                event["type"] == "item.delta" and event["runId"] == running_id
                for event in notifications
            ):
                message = json.loads(await asyncio.wait_for(first.recv(), timeout=5))
                notifications.append(message["params"])

            os.kill(int(first_ready["pid"]), signal.SIGTERM)
            await asyncio.wait_for(first_process.wait(), timeout=5)
        finally:
            await first.close()
    finally:
        await _stop_failed_process(first_process)

    second_token = secrets.token_urlsafe(32)
    second_process, second_ready = await _start_runtime(second_token, tmp_path)
    try:
        second = await _initialize(
            f"ws://{second_ready['host']}:{second_ready['port']}",
            second_token,
        )
        replayed = await _replay_until_run_settled(second, queued_id, request_id=2)
        running_settled = [
            event
            for event in replayed
            if event["type"] == "run.settled" and event["runId"] == running_id
        ]
        queued_settled = [
            event
            for event in replayed
            if event["type"] == "run.settled" and event["runId"] == queued_id
        ]
        partial = next(
            event
            for event in replayed
            if event["type"] == "item.completed"
            and event["runId"] == running_id
            and event["payload"].get("item", {}).get("role") == "assistant"
        )
        assert len(running_settled) == 1
        assert running_settled[0]["payload"]["status"] == "failed"
        assert running_settled[0]["payload"]["reasonCode"] == "runtime_interrupted"
        assert partial["payload"]["item"]["status"] == "failed"
        assert partial["payload"]["item"]["content"]
        assert len(queued_settled) == 1
        assert queued_settled[0]["payload"]["status"] == "completed"
        await _shutdown(second, second_process, 400)
    finally:
        await _stop_failed_process(second_process)
