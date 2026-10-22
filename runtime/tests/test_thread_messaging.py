"""Cross-session Thread management: create, rename, archive, and send."""

from __future__ import annotations

import asyncio
import json
from collections.abc import AsyncIterator
from pathlib import Path
from typing import Any

import pytest

from ikaros_runtime.agent import AgentLoop
from ikaros_runtime.cancellation import CancellationToken
from ikaros_runtime.domain import JournalEvent, JsonObject, WorkspaceSummary
from ikaros_runtime.errors import RunCancelled
from ikaros_runtime.protocol.spec import PROVIDER_TOOL_IDS
from ikaros_runtime.providers.base import (
    ProviderEvent,
    ProviderRequest,
    ResponseCompleted,
    TextDelta,
)
from ikaros_runtime.security import RuntimeSecurity
from ikaros_runtime.services.thread_messaging import (
    THREAD_SEND_MAX_HOPS,
    ThreadMessagingError,
    ThreadMessagingService,
)
from ikaros_runtime.services.threads import ThreadService
from ikaros_runtime.storage import SqliteRuntimeStore
from ikaros_runtime.storage.thread_messaging import THREAD_SEND_DATA_KEY
from ikaros_runtime.tools import (
    FullAccessPolicy,
    ThreadArchiveTool,
    ThreadCreateTool,
    ThreadRenameTool,
    ThreadSendTool,
    ToolCall,
    ToolDefinition,
    ToolExecutionContext,
    ToolExecutor,
    ToolRegistry,
)
from ikaros_runtime.workspaces import provision_managed_workspace

from .fakes.scripted_provider import ScriptedProvider
from .helpers import run_config_template


class _UnusedBackend:
    """Stand-in backend for the child Run's frozen Tool definitions."""

    async def create(self, **_: object) -> JsonObject:  # pragma: no cover - unused
        raise AssertionError("backend must not be called")

    async def rename(self, **_: object) -> JsonObject:  # pragma: no cover - unused
        raise AssertionError("backend must not be called")

    async def set_archived(self, **_: object) -> JsonObject:  # pragma: no cover - unused
        raise AssertionError("backend must not be called")

    async def send(self, **_: object) -> JsonObject:  # pragma: no cover - unused
        raise AssertionError("backend must not be called")


class _SchedulerStub:
    """Record delivered Runs instead of running them on a worker pool."""

    def __init__(self) -> None:
        self.enqueued: list[str] = []

    async def enqueue(self, run_id: str) -> None:
        self.enqueued.append(run_id)


class _SlowProvider:
    id = "scripted"
    model_id = "scripted-v1"

    def __init__(self, delay: float = 30.0) -> None:
        self.delay = delay

    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]:
        del request
        await cancellation.sleep(self.delay)
        yield TextDelta("slow child finished")
        yield ResponseCompleted()


def _environment(
    tmp_path: Path,
    *,
    protected_values: tuple[str, ...] = (),
    provider: Any = None,
) -> tuple[
    SqliteRuntimeStore,
    ThreadMessagingService,
    _SchedulerStub,
    list[JournalEvent],
    tuple[ToolDefinition, ...],
]:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    events: list[JournalEvent] = []

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    executor = ToolExecutor(
        ToolRegistry([ThreadSendTool(_UnusedBackend())]),
        FullAccessPolicy(),
    )
    active_provider = provider if provider is not None else ScriptedProvider()
    loop = AgentLoop(
        store,
        {ScriptedProvider.id: active_provider},
        publish,
        executor,
    )
    security = RuntimeSecurity(lambda: protected_values, lambda _values: False)
    threads = ThreadService(
        store,
        security.assert_request_safe,
        managed_workspace_factory=provision_managed_workspace,
    )
    scheduler = _SchedulerStub()
    service = ThreadMessagingService(
        store=store,
        threads=threads,
        assert_request_safe=security.assert_request_safe,
        publish=publish,
        enqueue_run=scheduler.enqueue,
        run_inline=loop.run,
    )
    return store, service, scheduler, events, executor.definitions


def _context(run_id: str, *, item_id: str = "item_caller") -> ToolExecutionContext:
    return ToolExecutionContext(run_id, 1, item_id, "thread_caller")


def _running_run(
    store: SqliteRuntimeStore,
    tools: tuple[ToolDefinition, ...],
    *,
    title: str,
    hop: int | None = None,
) -> tuple[str, str]:
    """Create a Thread with one running Run, optionally at a hop position."""

    thread, _event = store.create_thread(title)
    data = (
        None
        if hop is None
        else {
            THREAD_SEND_DATA_KEY: {
                "hop": hop,
                "sourceThreadId": "thread_source",
                "sourceRunId": "run_source",
            }
        }
    )
    prepared = store.prepare_turn(
        thread_id=thread.id,
        branch_id=thread.default_branch_id,
        content="parent task",
        run_config_template=run_config_template("scripted", "scripted-v1", tools=tools),
        user_item_data=data,
    )
    store.mark_run_running(prepared.run_id)
    return thread.id, prepared.run_id


def _turns(store: SqliteRuntimeStore, thread_id: str, branch_id: str) -> int:
    page = store.list_turn_page(
        thread_id=thread_id,
        branch_id=branch_id,
        cursor=None,
        limit=50,
    )
    return len(page.turns)


@pytest.mark.asyncio
async def test_thread_create_tool_creates_a_thread_and_replays_idempotently(
    tmp_path: Path,
) -> None:
    store, service, _scheduler, events, tools = _environment(tmp_path)
    try:
        tool = ThreadCreateTool(service)
        call = ToolCall("call_create", "thread_create", {"title": "Research"})
        result = await tool.execute(
            call,
            cancellation=CancellationToken(),
            context=_context("run_parent"),
        )
        assert result.ok
        payload = json.loads(result.output)
        thread_id = payload["threadId"]
        assert payload["title"] == "Research"
        assert payload["created"] is True
        assert payload["workspace"]["kind"] == "managed"
        assert result.details == {"threadId": thread_id, "created": True}
        page = store.list_thread_page(cursor=None, limit=50, archived=False)
        # Tool-created Threads belong to the agent, not the conversation list.
        assert thread_id not in {thread.id for thread in page.threads}
        assert store.get_thread(thread_id).thread.id == thread_id
        assert any(event.type == "thread.created" for event in events)

        replay = await tool.execute(
            call,
            cancellation=CancellationToken(),
            context=_context("run_parent"),
        )
        assert replay.ok
        replayed = json.loads(replay.output)
        assert replayed["threadId"] == thread_id
        assert replayed["created"] is False
        assert replay.details == {"threadId": thread_id, "created": False}
    finally:
        store.close()


@pytest.mark.asyncio
async def test_thread_create_tool_binds_an_existing_project_directory(tmp_path: Path) -> None:
    store, service, _scheduler, _events, tools = _environment(tmp_path)
    project = tmp_path / "project"
    project.mkdir()
    try:
        tool = ThreadCreateTool(service)
        first = await tool.execute(
            ToolCall("call_first", "thread_create", {"workspace": str(project)}),
            cancellation=CancellationToken(),
            context=_context("run_parent", item_id="item_first"),
        )
        second = await tool.execute(
            ToolCall("call_second", "thread_create", {"workspace": str(project)}),
            cancellation=CancellationToken(),
            context=_context("run_parent", item_id="item_second"),
        )
        assert first.ok and second.ok
        first_payload = json.loads(first.output)
        second_payload = json.loads(second.output)
        assert first_payload["workspace"]["kind"] == "project"
        assert Path(first_payload["workspace"]["rootUri"]) == project.resolve()
        assert (
            first_payload["workspace"]["id"] == second_payload["workspace"]["id"]
        )
        assert first_payload["threadId"] != second_payload["threadId"]

        relative = await tool.execute(
            ToolCall("call_relative", "thread_create", {"workspace": "relative/path"}),
            cancellation=CancellationToken(),
            context=_context("run_parent", item_id="item_relative"),
        )
        assert not relative.ok
        assert relative.details["errorCode"] == "invalid_arguments"

        missing = await tool.execute(
            ToolCall(
                "call_missing",
                "thread_create",
                {"workspace": str(tmp_path / "absent")},
            ),
            cancellation=CancellationToken(),
            context=_context("run_parent", item_id="item_missing"),
        )
        assert not missing.ok
        assert missing.details["errorCode"] == "invalid_arguments"
    finally:
        store.close()


@pytest.mark.asyncio
async def test_thread_rename_tool_renames_and_reports_idempotent_replays(
    tmp_path: Path,
) -> None:
    store, service, _scheduler, events, tools = _environment(tmp_path)
    try:
        thread, _event = store.create_thread("Draft")
        tool = ThreadRenameTool(service)
        call = ToolCall(
            "call_rename",
            "thread_rename",
            {"threadId": thread.id, "title": "  Final title  "},
        )
        result = await tool.execute(
            call,
            cancellation=CancellationToken(),
            context=_context("run_parent"),
        )
        assert result.ok
        payload = json.loads(result.output)
        assert payload == {"threadId": thread.id, "title": "Final title", "changed": True}
        assert result.details == {"threadId": thread.id, "changed": True}
        assert store.get_thread(thread.id).thread.title == "Final title"
        assert any(event.type == "thread.renamed" for event in events)

        replay = await tool.execute(
            call,
            cancellation=CancellationToken(),
            context=_context("run_parent"),
        )
        assert replay.ok
        assert json.loads(replay.output)["changed"] is False
        assert replay.details == {"threadId": thread.id, "changed": False}

        absent = await tool.execute(
            ToolCall(
                "call_absent",
                "thread_rename",
                {"threadId": "thread_missing", "title": "Nope"},
            ),
            cancellation=CancellationToken(),
            context=_context("run_parent"),
        )
        assert not absent.ok
        assert absent.details["errorCode"] == "thread_not_found"
    finally:
        store.close()


@pytest.mark.asyncio
async def test_thread_archive_tool_toggles_archived_state(tmp_path: Path) -> None:
    store, service, _scheduler, events, tools = _environment(tmp_path)
    try:
        thread, _event = store.create_thread("Archive me")
        tool = ThreadArchiveTool(service)
        archived = await tool.execute(
            ToolCall("call_archive", "thread_archive", {"threadId": thread.id}),
            cancellation=CancellationToken(),
            context=_context("run_parent"),
        )
        assert archived.ok
        payload = json.loads(archived.output)
        assert payload == {"threadId": thread.id, "archived": True, "changed": True}
        assert archived.details == {
            "threadId": thread.id,
            "archived": True,
            "changed": True,
        }
        assert store.get_thread(thread.id).thread.archived_at is not None
        assert any(event.type == "thread.archived" for event in events)

        repeated = await tool.execute(
            ToolCall("call_archive", "thread_archive", {"threadId": thread.id}),
            cancellation=CancellationToken(),
            context=_context("run_parent"),
        )
        assert repeated.ok
        assert json.loads(repeated.output)["changed"] is False

        restored = await tool.execute(
            ToolCall(
                "call_unarchive",
                "thread_archive",
                {"threadId": thread.id, "archived": False},
            ),
            cancellation=CancellationToken(),
            context=_context("run_parent"),
        )
        assert restored.ok
        assert json.loads(restored.output) == {
            "threadId": thread.id,
            "archived": False,
            "changed": True,
        }
        assert store.get_thread(thread.id).thread.archived_at is None

        busy_thread, _busy_run = _running_run(store, tools, title="Busy")
        busy = await tool.execute(
            ToolCall("call_busy", "thread_archive", {"threadId": busy_thread}),
            cancellation=CancellationToken(),
            context=_context("run_parent"),
        )
        assert not busy.ok
        assert busy.details["errorCode"] == "thread_busy"
    finally:
        store.close()


@pytest.mark.asyncio
async def test_thread_send_hands_the_run_to_the_scheduler_and_replays_idempotently(
    tmp_path: Path,
) -> None:
    store, service, scheduler, events, tools = _environment(tmp_path)
    try:
        target, _event = store.create_thread("Target")
        parent_thread, parent_run = _running_run(store, tools, title="Parent")
        tool = ThreadSendTool(service)
        call = ToolCall(
            "call_send",
            "thread_send",
            {"threadId": target.id, "content": "do the thing"},
        )
        result = await tool.execute(
            call,
            cancellation=CancellationToken(),
            context=_context(parent_run),
        )
        assert result.ok
        payload = json.loads(result.output)
        run_id = payload["runId"]
        assert payload["threadId"] == target.id
        assert payload["status"] == "queued"
        assert payload["waited"] is False
        assert result.details == {
            "threadId": target.id,
            "runId": run_id,
            "status": "queued",
            "waited": False,
        }
        assert scheduler.enqueued == [run_id]
        assert store.run_status(run_id) == "queued"
        delivered = store.get_submission_user_content(run_id)
        assert delivered == (
            f"[Cross-session message from thread {parent_thread}]\n\ndo the thing"
        )
        assert store.thread_send_hop(run_id) == 1
        assert any(event.run_id == run_id for event in events)
        assert _turns(store, target.id, target.default_branch_id) == 1

        replay = await tool.execute(
            call,
            cancellation=CancellationToken(),
            context=_context(parent_run),
        )
        assert replay.ok
        assert json.loads(replay.output)["runId"] == run_id
        assert scheduler.enqueued == [run_id]
        assert _turns(store, target.id, target.default_branch_id) == 1
    finally:
        store.close()


@pytest.mark.asyncio
async def test_thread_send_wait_returns_the_final_answer(tmp_path: Path) -> None:
    store, service, scheduler, _events, tools = _environment(tmp_path)
    try:
        target, _event = store.create_thread("Target")
        _parent_thread, parent_run = _running_run(store, tools, title="Parent")
        tool = ThreadSendTool(service)
        result = await tool.execute(
            ToolCall(
                "call_send",
                "thread_send",
                {"threadId": target.id, "content": "summarise the plan", "wait": True},
            ),
            cancellation=CancellationToken(),
            context=_context(parent_run),
        )
        assert result.ok
        assert result.details["waited"] is True
        assert result.details["status"] == "completed"
        assert "summarise the plan" in result.output
        assert result.output.startswith("Scripted response to:")
        assert scheduler.enqueued == []
        assert store.run_status(str(result.details["runId"])) == "completed"
    finally:
        store.close()


@pytest.mark.asyncio
async def test_thread_send_rejects_a_busy_target_thread(tmp_path: Path) -> None:
    store, service, scheduler, _events, tools = _environment(tmp_path)
    try:
        target, _event = store.create_thread("Busy target")
        busy_run = store.prepare_turn(
            thread_id=target.id,
            branch_id=target.default_branch_id,
            content="already running",
            run_config_template=run_config_template("scripted", "scripted-v1"),
        )
        _parent_thread, parent_run = _running_run(store, tools, title="Parent")
        tool = ThreadSendTool(service)
        result = await tool.execute(
            ToolCall(
                "call_send",
                "thread_send",
                {"threadId": target.id, "content": "second message"},
            ),
            cancellation=CancellationToken(),
            context=_context(parent_run),
        )
        assert not result.ok
        assert result.details["errorCode"] == "thread_busy"
        assert scheduler.enqueued == []
        assert store.run_status(busy_run.run_id) == "queued"
        assert _turns(store, target.id, target.default_branch_id) == 1
    finally:
        store.close()


@pytest.mark.asyncio
async def test_thread_send_requires_an_unarchived_target(tmp_path: Path) -> None:
    store, service, _scheduler, _events, tools = _environment(tmp_path)
    try:
        target, _event = store.create_thread("Archived target")
        store.set_thread_archived(target.id, archived=True)
        _parent_thread, parent_run = _running_run(store, tools, title="Parent")
        send = ThreadSendTool(service)
        archived = await send.execute(
            ToolCall(
                "call_send",
                "thread_send",
                {"threadId": target.id, "content": "hello"},
            ),
            cancellation=CancellationToken(),
            context=_context(parent_run),
        )
        assert not archived.ok
        assert archived.details["errorCode"] == "thread_archived"
        assert _turns(store, target.id, target.default_branch_id) == 0

        restore = ThreadArchiveTool(service)
        unarchived = await restore.execute(
            ToolCall(
                "call_unarchive",
                "thread_archive",
                {"threadId": target.id, "archived": False},
            ),
            cancellation=CancellationToken(),
            context=_context(parent_run),
        )
        assert unarchived.ok
        delivered = await send.execute(
            ToolCall(
                "call_send",
                "thread_send",
                {"threadId": target.id, "content": "hello"},
            ),
            cancellation=CancellationToken(),
            context=_context(parent_run),
        )
        assert delivered.ok
        assert _turns(store, target.id, target.default_branch_id) == 1
    finally:
        store.close()


@pytest.mark.asyncio
async def test_thread_send_stops_the_cross_session_chain_at_the_hop_limit(
    tmp_path: Path,
) -> None:
    store, service, _scheduler, _events, tools = _environment(tmp_path)
    try:
        target, _event = store.create_thread("Target")
        tool = ThreadSendTool(service)
        _thread, run_at_limit = _running_run(
            store,
            tools,
            title="At limit",
            hop=THREAD_SEND_MAX_HOPS,
        )
        blocked = await tool.execute(
            ToolCall(
                "call_send",
                "thread_send",
                {"threadId": target.id, "content": "loop again"},
            ),
            cancellation=CancellationToken(),
            context=_context(run_at_limit),
        )
        assert not blocked.ok
        assert blocked.details["errorCode"] == "thread_hop_limit"
        assert _turns(store, target.id, target.default_branch_id) == 0

        _thread, run_below_limit = _running_run(
            store,
            tools,
            title="Below limit",
            hop=THREAD_SEND_MAX_HOPS - 1,
        )
        allowed = await tool.execute(
            ToolCall(
                "call_send",
                "thread_send",
                {"threadId": target.id, "content": "one more hop"},
            ),
            cancellation=CancellationToken(),
            context=_context(run_below_limit),
        )
        assert allowed.ok
        run_id = str(json.loads(allowed.output)["runId"])
        assert store.thread_send_hop(run_id) == THREAD_SEND_MAX_HOPS
    finally:
        store.close()


@pytest.mark.asyncio
async def test_thread_send_rejects_protected_credential_values(tmp_path: Path) -> None:
    store, service, scheduler, _events, tools = _environment(
        tmp_path,
        protected_values=("sk-live-secret",),
    )
    try:
        target, _event = store.create_thread("Target")
        _parent_thread, parent_run = _running_run(store, tools, title="Parent")
        tool = ThreadSendTool(service)
        result = await tool.execute(
            ToolCall(
                "call_send",
                "thread_send",
                {"threadId": target.id, "content": "the key is sk-live-secret"},
            ),
            cancellation=CancellationToken(),
            context=_context(parent_run),
        )
        assert not result.ok
        assert result.details["errorCode"] == "invalid_arguments"
        assert "protected configuration data" in result.output
        assert scheduler.enqueued == []
        assert _turns(store, target.id, target.default_branch_id) == 0
    finally:
        store.close()


@pytest.mark.asyncio
async def test_thread_send_parent_cancellation_cancels_the_delivered_run(
    tmp_path: Path,
) -> None:
    store, service, _scheduler, _events, tools = _environment(
        tmp_path,
        provider=_SlowProvider(),
    )
    try:
        target, _event = store.create_thread("Target")
        _parent_thread, parent_run = _running_run(store, tools, title="Parent")
        tool = ThreadSendTool(service)
        token = CancellationToken()
        task = asyncio.create_task(
            tool.execute(
                ToolCall(
                    "call_send",
                    "thread_send",
                    {"threadId": target.id, "content": "slow child", "wait": True},
                ),
                cancellation=token,
                context=_context(parent_run),
            )
        )
        await asyncio.sleep(0.05)
        token.cancel()
        with pytest.raises(RunCancelled):
            await asyncio.wait_for(task, timeout=10)
        delivered = [
            event.run_id
            for event in _events
            if event.type == "item.completed" and event.run_id is not None
        ]
        assert delivered
        assert store.run_status(delivered[-1]) == "cancelled"
    finally:
        store.close()


@pytest.mark.asyncio
async def test_thread_tools_reject_invalid_arguments(tmp_path: Path) -> None:
    store, service, _scheduler, _events, tools = _environment(tmp_path)
    try:
        context = _context("run_parent")
        cases: tuple[tuple[Any, ToolCall], ...] = (
            (ThreadCreateTool(service), ToolCall("c", "thread_create", {"extra": True})),
            (
                ThreadCreateTool(service),
                ToolCall("c", "thread_create", {"title": "   "}),
            ),
            (ThreadRenameTool(service), ToolCall("c", "thread_rename", {"threadId": "t"})),
            (
                ThreadRenameTool(service),
                ToolCall("c", "thread_rename", {"threadId": "t", "title": ""}),
            ),
            (ThreadArchiveTool(service), ToolCall("c", "thread_archive", {})),
            (
                ThreadArchiveTool(service),
                ToolCall("c", "thread_archive", {"threadId": "t", "archived": "yes"}),
            ),
            (ThreadSendTool(service), ToolCall("c", "thread_send", {"threadId": "t"})),
            (
                ThreadSendTool(service),
                ToolCall("c", "thread_send", {"threadId": "t", "content": "  "}),
            ),
            (
                ThreadSendTool(service),
                ToolCall(
                    "c",
                    "thread_send",
                    {"threadId": "t", "content": "x", "wait": "yes"},
                ),
            ),
            (
                ThreadSendTool(service),
                ToolCall(
                    "c",
                    "thread_send",
                    {"threadId": "t", "content": "x", "extra": 1},
                ),
            ),
        )
        for tool, call in cases:
            result = await tool.execute(
                call,
                cancellation=CancellationToken(),
                context=context,
            )
            assert not result.ok, call.arguments
            assert result.details["errorCode"] == "invalid_arguments", call.arguments
    finally:
        store.close()


def test_thread_tools_are_mutating_and_declare_protocol_tool_ids() -> None:
    definitions = (
        ThreadCreateTool.definition,
        ThreadRenameTool.definition,
        ThreadArchiveTool.definition,
        ThreadSendTool.definition,
    )
    names = {definition.name for definition in definitions}
    assert names <= set(PROVIDER_TOOL_IDS)
    assert all(definition.mutating for definition in definitions)
    assert len(names) == 4


def test_thread_messaging_errors_carry_model_visible_codes() -> None:
    error = ThreadMessagingError("thread_busy", "busy")
    assert error.code == "thread_busy"
    assert str(error) == "busy"


@pytest.mark.asyncio
async def test_thread_create_inherits_the_calling_thread_project(tmp_path: Path) -> None:
    store, service, _scheduler, _events, tools = _environment(tmp_path)
    project = tmp_path / "project"
    project.mkdir()
    try:
        thread, _event = store.create_thread(
            "Caller",
            workspace=WorkspaceSummary(
                "workspace_project",
                "Project",
                "project",
                str(project),
            ),
        )
        prepared = store.prepare_turn(
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="parent task",
            run_config_template=run_config_template("scripted", "scripted-v1", tools=tools),
        )
        store.mark_run_running(prepared.run_id)

        tool = ThreadCreateTool(service)
        result = await tool.execute(
            ToolCall("call_inherit", "thread_create", {"title": "Child"}),
            cancellation=CancellationToken(),
            context=_context(prepared.run_id),
        )

        assert result.ok
        payload = json.loads(result.output)
        assert payload["workspace"]["kind"] == "project"
        assert payload["workspace"]["id"] == "workspace_project"
        assert Path(payload["workspace"]["rootUri"]) == project.resolve()
    finally:
        store.close()