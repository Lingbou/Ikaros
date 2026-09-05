"""Ordinary next Turns consume incomplete history through actual provider requests."""

from __future__ import annotations

import asyncio
import json
import os
import shlex
import sys
from copy import deepcopy
from pathlib import Path
from typing import Any
from unittest.mock import patch

import httpx
import pytest

from ikaros_runtime.agent import AgentLoop
from ikaros_runtime.cancellation import CancellationToken
from ikaros_runtime.domain import JournalEvent
from ikaros_runtime.errors import (
    ContextBudgetExceededError,
    ModelInputUnavailableError,
    RunCancelled,
)
from ikaros_runtime.history_status import FrozenHistoryStatusV1, HistoryRunStatusV1
from ikaros_runtime.memory import MemoryRetrieverV1, MemoryScope, SqliteMemoryStore
from ikaros_runtime.providers.base import ModelConfig, ProviderConfig
from ikaros_runtime.providers.openai_compatible.adapter import OpenAICompatibleAdapter
from ikaros_runtime.run_input import (
    MAXIMUM_INPUT_CHARACTERS_V1,
    RESERVED_CURRENT_RUN_CHARACTERS_V1,
    FrozenMemoryContextV1,
    canonical_json,
    frame_input_character_counts,
    parse_context_snapshot,
    parse_step_manifest,
)
from ikaros_runtime.storage import SqliteRuntimeStore
from ikaros_runtime.storage.context_history import load_context_for_snapshot
from ikaros_runtime.storage.projections import get_context_snapshot
from ikaros_runtime.tools.core import ToolCall, ToolExecutor, ToolRegistry, ToolResult
from ikaros_runtime.tools.policy import FullAccessPolicy
from ikaros_runtime.tools.process import ProcessRunTool
from ikaros_runtime.tools.read import ReadTool
from ikaros_runtime.tools.write import WriteTool

from .helpers import prepare_turn, submission_frame


def _provider() -> ProviderConfig:
    return ProviderConfig(
        id="custom",
        display_name="Custom",
        origin="custom",
        base_url="https://provider.invalid/v1",
        api_key="test-provider-key",
        headers=(),
        models=(ModelConfig("model", "Model", True, True),),
    )


def _response(call: ToolCall | None = None) -> httpx.Response:
    delta: dict[str, Any] = {"content": "Done."}
    if call is not None:
        delta = {
            "tool_calls": [
                {
                    "index": 0,
                    "id": call.id,
                    "type": "function",
                    "function": {"name": call.name, "arguments": json.dumps(call.arguments)},
                }
            ]
        }
    data = {
        "choices": [
            {
                "index": 0,
                "delta": delta,
                "finish_reason": "tool_calls" if call is not None else "stop",
            }
        ]
    }
    return httpx.Response(
        200,
        headers={"content-type": "text/event-stream"},
        content=f"data: {json.dumps(data)}\n\ndata: [DONE]\n\n".encode(),
    )


def _status_messages(body: dict[str, Any]) -> list[str]:
    return [
        message["content"]
        for message in body["messages"]
        if message["role"] == "system" and message["content"].startswith("Runtime history status")
    ]


def _assert_tool_pair(body: dict[str, Any], call_id: str) -> dict[str, Any]:
    calls = [
        call
        for message in body["messages"]
        for call in message.get("tool_calls", [])
        if call["id"] == call_id
    ]
    results = [message for message in body["messages"] if message.get("tool_call_id") == call_id]
    assert len(calls) == len(results) == 1
    return json.loads(results[0]["content"])  # type: ignore[no-any-return]


def _persisted_inputs(store: SqliteRuntimeStore) -> tuple[list[tuple[Any, ...]], ...]:
    return tuple(
        [tuple(row) for row in store._connection.execute(query).fetchall()]
        for query in (
            "SELECT run_id, context_snapshot_json FROM run_inputs ORDER BY run_id",
            "SELECT run_id, step_ordinal, step_manifest_json FROM model_steps "
            "ORDER BY run_id, step_ordinal",
        )
    )


@pytest.mark.asyncio
@pytest.mark.parametrize("first_selection", ["bounded-history-v1", "bounded-history-v2"])
@pytest.mark.parametrize("next_prompt", ["继续", "Explain the weather on Mars."])
@pytest.mark.parametrize("fail_again", [False, True])
async def test_next_turn_sees_successful_write_after_provider_failure_without_replay(
    tmp_path: Path,
    first_selection: str,
    next_prompt: str,
    fail_again: bool,
) -> None:
    target = tmp_path / "written.txt"
    bodies: list[dict[str, Any]] = []
    events: list[JournalEvent] = []
    write_call = ToolCall("write-once", "write", {"filePath": str(target), "content": "first"})
    read_call = ToolCall("inspect-now", "read", {"filePath": str(target)})

    async def handler(request: httpx.Request) -> httpx.Response:
        bodies.append(json.loads(request.content))
        if len(bodies) == 1:
            return _response(write_call)
        if len(bodies) == 2 or (fail_again and len(bodies) == 3):
            return httpx.Response(503, json={"error": {"message": "unavailable"}})
        if len(bodies) == (4 if fail_again else 3):
            return _response(read_call)
        return _response()

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    store = SqliteRuntimeStore(tmp_path / "state.db")
    memory = SqliteMemoryStore(tmp_path / "memory.db")
    try:
        memory.create_memory_once(
            kind="preference",
            scope=MemoryScope("global", None),
            content="继续 Write the file. Explain the weather on Mars. Keep answers concise.",
            client_request_id="history-memory",
        )
        thread, _ = store.create_thread("Natural continuation")
        executor = ToolExecutor(ToolRegistry((WriteTool(), ReadTool())), FullAccessPolicy())
        with patch("ikaros_runtime.storage.store.CONTEXT_SELECTION_VERSION", first_selection):
            first = prepare_turn(
                store,
                thread_id=thread.id,
                branch_id=thread.default_branch_id,
                content="Write the file.",
                provider_id="custom",
                model_id="model",
                tools=executor.definitions,
            )
        # A previously queued V1 Run executes after the default has changed to V2.
        assert store.run_status(first.run_id) == "queued"
        async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
            loop = AgentLoop(
                store,
                {"custom": OpenAICompatibleAdapter(_provider(), client=client, max_retries=0)},
                publish,
                tool_executor=executor,
                memory_retriever=MemoryRetrieverV1(memory),
            )
            await loop.run(first.run_id, CancellationToken())
            assert target.read_text() == "first"
            assert store.run_status(first.run_id) == "failed"
            # Replaying the old write would destroy this later user edit.
            target.write_text("user edit")
            next_turn = prepare_turn(
                store,
                thread_id=thread.id,
                branch_id=thread.default_branch_id,
                content=next_prompt,
                provider_id="custom",
                model_id="model",
                tools=executor.definitions,
            )
            await loop.run(next_turn.run_id, CancellationToken())
            failed_runs = [first.run_id]
            if fail_again:
                assert store.run_status(next_turn.run_id) == "failed"
                failed_runs.append(next_turn.run_id)
                next_turn = prepare_turn(
                    store,
                    thread_id=thread.id,
                    branch_id=thread.default_branch_id,
                    content="继续",
                    provider_id="custom",
                    model_id="model",
                    tools=executor.definitions,
                )
                await loop.run(next_turn.run_id, CancellationToken())

        assert store.run_status(next_turn.run_id) == "completed"
        assert next_turn.run_id != first.run_id
        assert target.read_text() == "user edit"
        for body in bodies[2:]:
            result = _assert_tool_pair(body, "write-once")
            assert result["ok"] is True and result["bytesWritten"] == 5
            assert "Do not replay old Tool Calls" in _status_messages(body)[0]
        assert bodies[2]["messages"][-1] == {"role": "user", "content": next_prompt}
        assert "user edit" in _assert_tool_pair(bodies[-1], "inspect-now")["output"]
        latest_status = _status_messages(bodies[-1])[0]
        assert [run["runId"] for run in json.loads(latest_status.split("\n", 1)[1])["runs"]] == (
            failed_runs
        )
        assert _status_messages(bodies[-2]) == _status_messages(bodies[-1])
        input_events = [event for event in events if event.type == "model.input_prepared"]
        initial = input_events[0].payload["contextSnapshot"]
        assert initial["schemaVersion"] == (1 if first_selection.endswith("v1") else 2)
        assert ("historyStatus" in initial) is first_selection.endswith("v2")
        for event in input_events[2:]:
            snapshot = parse_context_snapshot(event.payload["contextSnapshot"])
            step = parse_step_manifest(event.payload["stepManifest"])
            assert step.history_status == snapshot.history_status
            assert snapshot.memory
            assert snapshot.budget.context_data_characters > snapshot.history_status.characters
            assert FrozenMemoryContextV1.from_snapshot(snapshot).memory == snapshot.memory
        before = _persisted_inputs(store)
        final_snapshot = get_context_snapshot(store._connection, next_turn.run_id)
        assert final_snapshot is not None
        records_before = load_context_for_snapshot(
            store._connection,
            run_id=next_turn.run_id,
            snapshot=final_snapshot,
        )
        store.rebuild_projections()
        assert _persisted_inputs(store) == before
        assert (
            load_context_for_snapshot(
                store._connection,
                run_id=next_turn.run_id,
                snapshot=final_snapshot,
            )
            == records_before
        )
        assert store._connection.execute("PRAGMA user_version").fetchone()[0] == 9
    finally:
        memory.close()
        store.close()


class _CancelAfterWrite(WriteTool):
    async def execute(
        self,
        call: ToolCall,
        *,
        cancellation: CancellationToken,
        default_cwd: str | None = None,
    ) -> ToolResult:
        await super().execute(call, cancellation=cancellation, default_cwd=default_cwd)
        cancellation.cancel()
        raise RunCancelled


@pytest.mark.asyncio
@pytest.mark.parametrize("interruption", ["cancelled", "restart"])
async def test_cancelled_or_recovered_write_is_unknown_in_next_provider_input(
    tmp_path: Path,
    interruption: str,
) -> None:
    target = tmp_path / "effect.txt"
    call = ToolCall("interrupted-write", "write", {"filePath": str(target), "content": "exists"})
    bodies: list[dict[str, Any]] = []

    async def handler(request: httpx.Request) -> httpx.Response:
        bodies.append(json.loads(request.content))
        return _response(call if len(bodies) == 1 and interruption == "cancelled" else None)

    async def publish(event: JournalEvent) -> None:
        del event

    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        executor = ToolExecutor(ToolRegistry((_CancelAfterWrite(),)), FullAccessPolicy())
        thread, _ = store.create_thread("Interrupted side effect")
        first = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="Write",
            provider_id="custom",
            model_id="model",
            tools=executor.definitions,
        )
        async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
            adapter = OpenAICompatibleAdapter(_provider(), client=client, max_retries=0)
            if interruption == "cancelled":
                await AgentLoop(store, {"custom": adapter}, publish, tool_executor=executor).run(
                    first.run_id,
                    CancellationToken(),
                )
            else:
                store.mark_run_running(first.run_id)
                store.prepare_model_step(first.run_id, step_ordinal=1)
                store.complete_provider_step(
                    first.run_id,
                    step_ordinal=1,
                    assistant_item_id=None,
                    tool_calls=(call,),
                    reasoning_content=None,
                    usage=None,
                    response_model_id=None,
                    request_id=None,
                )
                # Crash after an actual write, before its completion is committed.
                write_result = await WriteTool().execute(call, cancellation=CancellationToken())
                assert write_result.ok
                store.close()
                store = SqliteRuntimeStore(tmp_path / "state.db")
                store.recover_incomplete_runs()
            assert target.read_text() == "exists"
            next_turn = prepare_turn(
                store,
                thread_id=thread.id,
                branch_id=thread.default_branch_id,
                content="继续",
                provider_id="custom",
                model_id="model",
                tools=executor.definitions,
            )
            await AgentLoop(store, {"custom": adapter}, publish, tool_executor=executor).run(
                next_turn.run_id,
                CancellationToken(),
            )
        assert store.run_status(next_turn.run_id) == "completed"
        assert store.run_status(first.run_id) == (
            "cancelled" if interruption == "cancelled" else "failed"
        )
        result = _assert_tool_pair(bodies[-1], call.id)
        assert result["executionOutcome"] == "unknown"
        assert "not started" not in result["output"]
        assert "may already have changed files" in result["executionNotice"]
        assert "do not prove that an action was not executed" in _status_messages(bodies[-1])[0]
        snapshot = get_context_snapshot(store._connection, next_turn.run_id)
        assert snapshot is not None
        records = load_context_for_snapshot(
            store._connection, run_id=next_turn.run_id, snapshot=snapshot
        )
        normalized = next(record for record in records if record.kind == "tool_result")
        reference = next(ref for ref in snapshot.history_items if ref.item_id == normalized.item_id)
        assert reference.characters == len(canonical_json(result))
        stored = store._connection.execute(
            "SELECT content FROM items WHERE id = ?", (reference.item_id,)
        )
        assert "executionOutcome" not in stored.fetchone()[0]
        before = _persisted_inputs(store)
        store.rebuild_projections()
        assert _persisted_inputs(store) == before
        assert (
            load_context_for_snapshot(
                store._connection,
                run_id=next_turn.run_id,
                snapshot=snapshot,
            )
            == records
        )
    finally:
        store.close()


@pytest.mark.asyncio
async def test_oversized_nearest_failed_turn_keeps_a_status_notice(tmp_path: Path) -> None:
    bodies: list[dict[str, Any]] = []

    async def handler(request: httpx.Request) -> httpx.Response:
        bodies.append(json.loads(request.content))
        return _response()

    async def publish(event: JournalEvent) -> None:
        del event

    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Oversized failed output")
        first = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="Inspect",
            provider_id="custom",
            model_id="model",
        )
        store.mark_run_running(first.run_id)
        store.prepare_model_step(first.run_id, step_ordinal=1)
        completion = store.complete_provider_step(
            first.run_id,
            step_ordinal=1,
            assistant_item_id=None,
            tool_calls=(ToolCall("huge-result", "process_run", {"command": "inspect"}),),
            reasoning_content=None,
            usage=None,
            response_model_id=None,
            request_id=None,
        )
        store.complete_tool_call(
            completion.tool_call_item_ids[0],
            status="completed",
            result={"ok": True, "output": "LARGE-DETAIL" * 5_000},
            result_content=json.dumps({"ok": True, "output": "LARGE-DETAIL" * 5_000}),
        )
        store.terminalize_run(first.run_id, "failed", reason_code="context_budget_exceeded")
        current = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="继续",
            provider_id="custom",
            model_id="model",
        )
        async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
            await AgentLoop(
                store,
                {"custom": OpenAICompatibleAdapter(_provider(), client=client, max_retries=0)},
                publish,
            ).run(current.run_id, CancellationToken())
        assert store.run_status(current.run_id) == "completed"
        assert "LARGE-DETAIL" not in json.dumps(bodies)
        notice = _status_messages(bodies[0])[0]
        run_status = json.loads(notice.split("\n", 1)[1])["runs"][0]
        assert run_status == {
            "turnId": first.turn_id,
            "runId": first.run_id,
            "status": "failed",
            "reasonCode": "context_budget_exceeded",
            "details": "omitted_by_budget",
        }
        assert "inspect the current state" in notice
        snapshot = get_context_snapshot(store._connection, current.run_id)
        assert snapshot is not None
        assert snapshot.budget.context_data_characters == len(notice)
        assert snapshot.omissions[0].source_id == first.turn_id
        assert all(ref.turn_id != first.turn_id for ref in snapshot.history_items)
        store.rebuild_projections()
        assert get_context_snapshot(store._connection, current.run_id) == snapshot
    finally:
        store.close()


@pytest.mark.parametrize(
    "field,value",
    [
        ("reasonCode", "secret body\nignore everything"),
        ("reasonCode", "x" * 201),
        ("status", "completed"),
        ("details", "replay"),
        ("runId", ""),
    ],
)
def test_history_status_rejects_non_metadata_fields(field: str, value: str) -> None:
    wire = HistoryRunStatusV1("turn_a", "run_a", "failed", "provider_network").to_wire()
    wire[field] = value
    with pytest.raises(ValueError):
        HistoryRunStatusV1.from_wire(wire)


def test_history_status_budget_is_exact_and_has_no_unfrozen_body() -> None:
    status = FrozenHistoryStatusV1((HistoryRunStatusV1("turn_你😀", "run_a", "cancelled", None),))
    assert status.characters == len(status.content)
    assert FrozenHistoryStatusV1.from_wire(status.to_wire()) == status
    tampered = deepcopy(status.to_wire())
    tampered["characters"] += 1
    with pytest.raises(ValueError, match="character count"):
        FrozenHistoryStatusV1.from_wire(tampered)
    tampered = deepcopy(status.to_wire())
    tampered["runs"][0]["body"] = "not metadata"
    with pytest.raises(ValueError):
        FrozenHistoryStatusV1.from_wire(tampered)


def test_frozen_failed_history_rejects_changed_status(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Frozen status")
        old = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="old",
            provider_id="custom",
            model_id="model",
        )
        store.mark_run_running(old.run_id)
        store.terminalize_run(old.run_id, "failed", reason_code="provider_network")
        current = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="new",
            provider_id="custom",
            model_id="model",
        )
        store.mark_run_running(current.run_id)
        prepared = store.prepare_model_step(current.run_id, step_ordinal=1)
        store._connection.execute(
            "UPDATE runs SET reason_code = 'provider_timeout' WHERE id = ?",
            (old.run_id,),
        )
        with pytest.raises(ModelInputUnavailableError):
            load_context_for_snapshot(
                store._connection,
                run_id=current.run_id,
                snapshot=prepared.context_snapshot,
            )
    finally:
        store.close()


@pytest.mark.parametrize("boundary", ["included", "omitted", "omitted_overflow"])
def test_latest_failed_status_charges_actual_included_or_omitted_budget(
    tmp_path: Path,
    boundary: str,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Exact history status budget")
        old = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="x" if boundary == "included" else "x" * 36_000,
            provider_id="custom",
            model_id="model",
        )
        store.mark_run_running(old.run_id)
        store.terminalize_run(old.run_id, "failed", reason_code="agent_error")
        status = FrozenHistoryStatusV1(
            (
                HistoryRunStatusV1(
                    old.turn_id,
                    old.run_id,
                    "failed",
                    "agent_error",
                    "included" if boundary == "included" else "omitted_by_budget",
                ),
            )
        )
        instructions, tools = frame_input_character_counts(submission_frame("custom", "model"))
        prompt_length = (
            MAXIMUM_INPUT_CHARACTERS_V1
            - RESERVED_CURRENT_RUN_CHARACTERS_V1
            - instructions
            - tools
            - status.characters
            - (1 if boundary == "included" else 0)
            + (1 if boundary == "omitted_overflow" else 0)
        )
        current = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="n" * prompt_length,
            provider_id="custom",
            model_id="model",
        )
        store.mark_run_running(current.run_id)
        if boundary == "omitted_overflow":
            with pytest.raises(ContextBudgetExceededError):
                store.prepare_model_step(current.run_id, step_ordinal=1)
            assert get_context_snapshot(store._connection, current.run_id) is None
        else:
            prepared = store.prepare_model_step(current.run_id, step_ordinal=1)
            snapshot = prepared.context_snapshot
            assert snapshot.history_status == status
            assert (
                snapshot.budget.total_characters + snapshot.budget.reserved_current_run_characters
                == snapshot.budget.maximum_characters
            )
            store.rebuild_projections()
            assert get_context_snapshot(store._connection, current.run_id) == snapshot
    finally:
        store.close()


@pytest.mark.asyncio
async def test_cancelled_real_command_keeps_partial_output_in_next_turn_without_replay(
    tmp_path: Path,
) -> None:
    marker = tmp_path / "started.pid"
    script = tmp_path / "wait_for_cancellation.py"
    script.write_text(
        "import os, pathlib, sys, time\n"
        "print('partial-command-output', flush=True)\n"
        "with pathlib.Path(sys.argv[1]).open('a', encoding='utf-8') as ready:\n"
        "    ready.write(str(os.getpid()) + '\\n')\n"
        "    ready.flush()\n"
        "    os.fsync(ready.fileno())\n"
        "time.sleep(60)\n",
        encoding="utf-8",
    )
    arguments = [sys.executable, "-u", str(script), str(marker)]
    command = (
        "& " + " ".join("'" + argument.replace("'", "''") + "'" for argument in arguments)
        if os.name == "nt" else shlex.join(arguments)
    )
    call = ToolCall("cancelled-command", "process_run", {"command": command})
    bodies: list[dict[str, Any]] = []

    async def handler(request: httpx.Request) -> httpx.Response:
        bodies.append(json.loads(request.content))
        return _response(call if len(bodies) == 1 else None)

    async def publish(event: JournalEvent) -> None:
        del event

    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread("Cancel command and continue")
        executor = ToolExecutor(ToolRegistry((ProcessRunTool(),)), FullAccessPolicy())
        first = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="Run the command",
            provider_id="custom",
            model_id="model",
            tools=executor.definitions,
        )
        async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as client:
            loop = AgentLoop(
                store,
                {"custom": OpenAICompatibleAdapter(_provider(), client=client, max_retries=0)},
                publish,
                tool_executor=executor,
            )
            cancellation = CancellationToken()
            running = asyncio.create_task(loop.run(first.run_id, cancellation))
            try:
                for _ in range(500):
                    if marker.exists() and marker.read_text().strip():
                        break
                    await asyncio.sleep(0.01)
                else:
                    raise AssertionError("command did not publish its ready PID")
            finally:
                cancellation.cancel()
                await asyncio.wait_for(running, timeout=10)
            assert store.run_status(first.run_id) == "cancelled"
            assert len(marker.read_text().splitlines()) == 1
            current = prepare_turn(
                store,
                thread_id=thread.id,
                branch_id=thread.default_branch_id,
                content="继续",
                provider_id="custom",
                model_id="model",
                tools=executor.definitions,
            )
            await loop.run(current.run_id, CancellationToken())
        assert store.run_status(current.run_id) == "completed"
        assert current.run_id != first.run_id
        assert len(bodies) == 2
        result = _assert_tool_pair(bodies[-1], call.id)
        assert "partial-command-output" in result["stdout"]
        assert "partial-command-output" in result["output"]
        assert result["executionOutcome"] == "unknown"
        assert "may already have changed files" in result["executionNotice"]
        assert "Do not replay old Tool Calls" in _status_messages(bodies[-1])[0]
        assert len(marker.read_text().splitlines()) == 1
        assert (
            store._connection.execute(
                "SELECT COUNT(*) FROM items WHERE run_id = ? AND kind = 'tool_call'",
                (current.run_id,),
            ).fetchone()[0]
            == 0
        )
        store.rebuild_projections()
        assert store.run_status(first.run_id) == "cancelled"
    finally:
        store.close()
