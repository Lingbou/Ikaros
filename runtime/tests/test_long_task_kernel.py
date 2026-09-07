"""Durable process facts, restart boundaries, and manual Run cancellation."""

from __future__ import annotations

import asyncio
import os
import shlex
import sqlite3
import time
from pathlib import Path

import pytest

from ikaros_runtime.agent import AgentLoop
from ikaros_runtime.cancellation import CancellationToken
from ikaros_runtime.domain import JournalEvent, JsonObject
from ikaros_runtime.providers.scripted import ScriptedProvider
from ikaros_runtime.storage import SqliteRuntimeStore
from ikaros_runtime.tools import (
    FullAccessPolicy,
    ProcessManager,
    ProcessReadTool,
    ProcessStartTool,
    ProcessStopTool,
    ProcessWaitTool,
    ToolCall,
    ToolExecutionContext,
    ToolExecutor,
    ToolRegistry,
)
from ikaros_runtime.tools import process_manager as process_module
from ikaros_runtime.tools.process_manager import ProcessError
from ikaros_runtime.tools.process_platform import _SpawnedProcess

from .helpers import prepare_turn
from .test_process_manager import _process_is_alive, _wait_for_process_exit


def _executor(manager: ProcessManager) -> ToolExecutor:
    return ToolExecutor(
        ToolRegistry(
            [
                ProcessStartTool(manager),
                ProcessReadTool(manager),
                ProcessWaitTool(manager),
                ProcessStopTool(manager),
            ]
        ),
        FullAccessPolicy(),
    )


def _manager(store: SqliteRuntimeStore) -> ProcessManager:
    def record(fact: JsonObject) -> None:
        store.record_process(fact)

    return ProcessManager(record=record)


async def _start_durable_command(
    store: SqliteRuntimeStore,
    manager: ProcessManager,
    command: str,
    cwd: Path,
) -> tuple[ToolExecutionContext, str]:
    executor = _executor(manager)
    thread, _ = store.create_thread("Process recovery")
    turn = prepare_turn(
        store,
        thread_id=thread.id,
        branch_id=thread.default_branch_id,
        content="Run a command",
        provider_id="scripted",
        model_id="scripted-v1",
        tools=executor.definitions,
    )
    store.mark_run_running(turn.run_id)
    store.prepare_model_step(turn.run_id, step_ordinal=1)
    call = ToolCall("start_command", "process_start", {"command": command, "cwd": str(cwd)})
    step = store.complete_provider_step(
        turn.run_id,
        step_ordinal=1,
        assistant_item_id=None,
        tool_calls=(call,),
        reasoning_content=None,
        usage=None,
        response_model_id=None,
        request_id=None,
    )
    context = ToolExecutionContext(turn.run_id, 1, step.tool_call_item_ids[0], thread.id, str(cwd))
    result = await executor.execute(call, cancellation=CancellationToken(), context=context)
    assert result.ok
    store.complete_tool_call(
        context.item_id,
        status="completed",
        result=result.to_wire(),
        result_content=result.to_model_content(),
    )
    return context, str(result.details["processId"])


@pytest.mark.asyncio
async def test_restart_marks_running_process_unknown_without_reattachment_or_replay(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    original = SqliteRuntimeStore(tmp_path / "original.db")
    manager = _manager(original)
    command = "Start-Sleep 60" if os.name == "nt" else "sleep 60"
    checkpoint = tmp_path / "crash-state.db"
    try:
        context, process_id = await _start_durable_command(original, manager, command, tmp_path)
        assert original.process_records()[-1]["state"] == "running"
        # Capture exactly the durable boundary a crash would leave, then clean the test's
        # real process through its original owner to avoid leaking children into the host.
        target = sqlite3.connect(checkpoint)
        try:
            original._connection.backup(target)
        finally:
            target.close()
    finally:
        await manager.close()
        original.close()

    spawn_calls = 0

    async def forbidden(command: str, cwd: str | None) -> _SpawnedProcess:
        nonlocal spawn_calls
        spawn_calls += 1
        raise AssertionError("recovery must not spawn or attach a process")

    monkeypatch.setattr(process_module, "_spawn_process", forbidden)
    restored = SqliteRuntimeStore(checkpoint)
    recovery_manager = _manager(restored)
    try:
        recovery_manager.restore(restored.process_records())
        restored.recover_incomplete_runs()
        fact = restored.process_records()[0]
        assert fact["processId"] == process_id
        assert fact["state"] == "unknown" and fact["exitCode"] is None
        assert fact["errorCode"] == "runtime_interrupted"
        assert restored.run_status(context.run_id) == "failed"
        with pytest.raises(ProcessError, match="ended"):
            await recovery_manager.start(
                command, str(tmp_path), context=context, cancellation=CancellationToken()
            )
        assert spawn_calls == 0
        before = restored.process_records()
        restored.rebuild_projections()
        assert restored.process_records() == before
    finally:
        await recovery_manager.close()
        restored.close()


@pytest.mark.asyncio
async def test_process_journal_rebuilds_lost_projection_and_survives_reopen(tmp_path: Path) -> None:
    path = tmp_path / "state.db"
    store = SqliteRuntimeStore(path)
    manager = _manager(store)
    try:
        context, process_id = await _start_durable_command(store, manager, "echo durable", tmp_path)
        final = await manager.wait(
            process_id, context=context, cancellation=CancellationToken(), timeout_ms=5000
        )
        assert final["state"] == "exited" and final["exitCode"] == 0
        await manager.close_run(context.run_id)
        store.terminalize_run(context.run_id, "completed")
        facts = store.process_records()
        assert len(facts) == 1 and "durable" in facts[0]["stdout"]
        events, _ = store.replay_events(0, 1000)
        assert sum(event.type == "process.recorded" for event in events) >= 3
        with store._connection:
            store._connection.execute("DELETE FROM process_sessions")
        assert not store.process_records()
        store.rebuild_projections()
        assert store.process_records() == facts
    finally:
        await manager.close()
        store.close()
    reopened = SqliteRuntimeStore(path)
    try:
        assert reopened.process_records() == facts
    finally:
        reopened.close()


@pytest.mark.asyncio
@pytest.mark.skipif(os.name != "posix", reason="POSIX descendant PID and process-group assertions")
async def test_manual_cancellation_stops_wait_and_process_tree_but_preserves_partial_output(
    tmp_path: Path,
) -> None:
    root_pid_file, child_pid_file = tmp_path / "root.pid", tmp_path / "child.pid"
    command = (
        f"printf '%s' $$ > {shlex.quote(str(root_pid_file))}; "
        'sh -c \'printf "%s" "$$" > "$1"; sleep 60\' sh '
        f"{shlex.quote(str(child_pid_file))} & "
        f"while [ ! -s {shlex.quote(str(child_pid_file))} ]; do sleep 0.01; done; "
        "printf 'cancel-partial\\n'; wait"
    )
    store = SqliteRuntimeStore(tmp_path / "state.db")
    output_ready = asyncio.Event()
    wait_started = asyncio.Event()

    def record(fact: JsonObject) -> None:
        store.record_process(fact)
        if "cancel-partial" in fact["stdout"]:
            output_ready.set()

    manager = ProcessManager(record=record)
    executor = _executor(manager)
    events: list[JournalEvent] = []
    cancellation = CancellationToken()
    task: asyncio.Task[None] | None = None

    async def publish(event: JournalEvent) -> None:
        events.append(event)
        if (
            event.type == "item.started"
            and event.payload["item"]["kind"] == "tool_call"
            and event.payload["item"]["data"]["toolName"] == "process_wait"
        ):
            wait_started.set()

    try:
        thread, _ = store.create_thread("Cancel running process")
        turn = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content=f"/process.run {command}",
            provider_id="scripted",
            model_id="scripted-v1",
            tools=executor.definitions,
        )
        loop = AgentLoop(
            store, {"scripted": ScriptedProvider()}, publish, executor, process_manager=manager
        )
        task = asyncio.create_task(loop.run(turn.run_id, cancellation))
        await asyncio.wait_for(
            asyncio.gather(output_ready.wait(), wait_started.wait()), timeout=5
        )
        assert store.run_status(turn.run_id) == "running"
        started = time.monotonic()
        cancellation.cancel()
        await asyncio.wait_for(task, timeout=10)
        assert time.monotonic() - started < 5
        assert store.run_status(turn.run_id) == "cancelled"
        settled = [event for event in events if event.type == "run.settled"]
        assert len(settled) == 1 and settled[0].payload["reasonCode"] == "cancelled"
        facts = store.process_records()
        assert len(facts) == 1 and facts[0]["state"] == "terminated"
        assert "cancel-partial" in facts[0]["stdout"]
        wait_results = [
            event.payload["item"]["data"]["result"]
            for event in events
            if event.type == "item.completed"
            and event.payload.get("item", {}).get("kind") == "tool_result"
            and event.payload["item"]["data"]["toolName"] == "process_wait"
        ]
        assert wait_results and "cancel-partial" in wait_results[-1]["output"]
        assert wait_results[-1]["cancelled"]
        root_pid, child_pid = int(root_pid_file.read_text()), int(child_pid_file.read_text())
        await _wait_for_process_exit(root_pid)
        await _wait_for_process_exit(child_pid)
        assert not _process_is_alive(root_pid) and not _process_is_alive(child_pid)
        before = store.process_records()
        store.rebuild_projections()
        assert store.process_records() == before
    finally:
        cancellation.cancel()
        if task is not None:
            await asyncio.wait_for(task, timeout=10)
        await manager.close()
        store.close()


@pytest.mark.asyncio
@pytest.mark.parametrize("tamper_scope", ["first", "all"])
@pytest.mark.parametrize("false_step", [999, 2])
async def test_process_step_tampering_rebuild_is_rejected_without_losing_projections(
    tmp_path: Path,
    tamper_scope: str,
    false_step: int,
) -> None:
    from ikaros_runtime.json_codec import dumps, loads

    store = SqliteRuntimeStore(tmp_path / "state.db")
    manager = _manager(store)
    try:
        context, process_id = await _start_durable_command(store, manager, "echo owned", tmp_path)
        result = await manager.wait(
            process_id, context=context, cancellation=CancellationToken(), timeout_ms=5000
        )
        assert result["state"] == "exited"
        await manager.close_run(context.run_id)
        # Step 2 really exists and completed; matching an arbitrary model_calls row
        # must not let a command claim an unrelated response as its origin.
        store.prepare_model_step(context.run_id, step_ordinal=2)
        store.complete_provider_step(
            context.run_id,
            step_ordinal=2,
            assistant_item_id=None,
            tool_calls=(),
            reasoning_content=None,
            usage=None,
            response_model_id=None,
            request_id=None,
        )
        store.terminalize_run(context.run_id, "completed")
        before = store.process_records()
        rows = store._connection.execute(
            "SELECT seq, payload_json FROM events WHERE event_type = 'process.recorded' "
            "ORDER BY seq"
        ).fetchall()
        assert len(rows) >= 3
        with store._connection:
            for row in rows[:1] if tamper_scope == "first" else rows:
                payload = loads(row["payload_json"])
                payload["process"]["stepOrdinal"] = false_step
                store._connection.execute(
                    "UPDATE events SET payload_json = ? WHERE seq = ?", (dumps(payload), row["seq"])
                )
        with pytest.raises(ValueError, match="process Step does not match"):
            store.rebuild_projections()
        assert store.process_records() == before
        assert store.run_status(context.run_id) == "completed"
        assert store._connection.execute("SELECT COUNT(*) FROM model_calls").fetchone()[0] == 2
    finally:
        await manager.close()
        store.close()


@pytest.mark.asyncio
async def test_terminal_process_cannot_become_running_during_append_or_rebuild(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    manager = _manager(store)
    try:
        context, process_id = await _start_durable_command(store, manager, "echo final", tmp_path)
        result = await manager.wait(
            process_id, context=context, cancellation=CancellationToken(), timeout_ms=5000
        )
        assert result["state"] == "exited"
        await manager.close_run(context.run_id)
        store.terminalize_run(context.run_id, "completed")
        before = store.process_records()
        backwards = {**before[0], "state": "running", "finishedAt": None, "exitCode": None}
        with pytest.raises(ValueError, match="terminal process record changed"):
            store.record_process(backwards)
        assert store.process_records() == before
        run = store.get_run(context.run_id)
        with store._connection:
            store._append_event(
                event_type="process.recorded",
                thread_id=run.thread_id,
                branch_id=run.branch_id,
                turn_id=run.turn_id,
                run_id=run.id,
                item_id=context.item_id,
                timestamp="2026-09-07T00:00:00.000Z",
                payload={"process": backwards},
            )
        with pytest.raises(ValueError, match="terminal process record changed"):
            store.rebuild_projections()
        assert store.process_records() == before
        assert store.run_status(context.run_id) == "completed"
    finally:
        await manager.close()
        store.close()
