from __future__ import annotations

import asyncio
import json
import os
import secrets
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
        json.dumps(
            {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}
        )
    )
    while True:
        response = json.loads(await connection.recv())
        assert response["jsonrpc"] == "2.0"
        if response.get("method") == "event":
            if notifications is not None:
                notifications.append(response["params"])
            continue
        assert response["id"] == request_id
        return response


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
        assert sum(
            event["type"] == "run.settled" and event["runId"] == first_run_id
            for event in first_events
        ) == 1
        first_answer = next(
            event["payload"]["item"]["content"]
            for event in first_events
            if event["type"] == "item.completed"
            and event["payload"]["item"]["role"] == "assistant"
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
            if event["type"] == "item.completed"
            and event["payload"]["item"]["role"] == "assistant"
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
