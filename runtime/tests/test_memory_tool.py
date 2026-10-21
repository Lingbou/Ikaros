from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any, cast

import pytest

from ikaros_runtime.cancellation import CancellationToken
from ikaros_runtime.domain import WorkspaceSummary
from ikaros_runtime.json_codec import loads as json_loads
from ikaros_runtime.memory import MemoryRetriever, MemoryScope, SqliteMemoryStore
from ikaros_runtime.run_input import MEMORY_SELECTION_MAX_CHARACTERS
from ikaros_runtime.security import RuntimeSecurity
from ikaros_runtime.services.memories import MemoryService, MemoryToolService
from ikaros_runtime.storage import SqliteRuntimeStore
from ikaros_runtime.tools import MemoryTool, ToolCall, ToolExecutionContext
from ikaros_runtime.tools.memory import memory_tool_definition

from .helpers import prepare_turn

_TOOL_ITEM_ID = "item_" + "a" * 32
_OTHER_TOOL_ITEM_ID = "item_" + "b" * 32
_THIRD_TOOL_ITEM_ID = "item_" + "c" * 32
_PROTECTED_VALUE = "sk-memory-tool-sentinel-0123456789"


@dataclass(frozen=True, slots=True)
class _Environment:
    store: SqliteRuntimeStore
    memory_store: SqliteMemoryStore
    tool: MemoryTool


@dataclass(frozen=True, slots=True)
class _Outcome:
    ok: bool
    payload: dict[str, Any]
    details: dict[str, Any]
    output: str


def _environment(
    tmp_path: Path,
    *,
    protected_values: tuple[str, ...] = (),
) -> _Environment:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    memory_store = SqliteMemoryStore(tmp_path / "memory.db")
    security = RuntimeSecurity(lambda: protected_values, lambda _values: False)
    return _Environment(
        store=store,
        memory_store=memory_store,
        tool=MemoryTool(
            MemoryToolService(
                MemoryService(memory_store, security.assert_request_safe, store),
                MemoryRetriever(memory_store),
                store.memory_tool_run_scope,
            )
        ),
    )


def _context(
    run_id: str,
    thread_id: str,
    item_id: str = _TOOL_ITEM_ID,
) -> ToolExecutionContext:
    return ToolExecutionContext(
        run_id=run_id,
        step_ordinal=1,
        item_id=item_id,
        thread_id=thread_id,
    )


async def _call(
    tool: MemoryTool,
    arguments: dict[str, Any],
    context: ToolExecutionContext,
) -> _Outcome:
    result = await tool.execute(
        ToolCall("call-1", "memory", arguments),
        cancellation=CancellationToken(),
        context=context,
    )
    return _Outcome(
        ok=result.ok,
        payload=cast(dict[str, Any], json_loads(result.output)) if result.ok else {},
        details=dict(result.details),
        output=result.output,
    )


def _count_memories(memory_store: SqliteMemoryStore) -> int:
    row = memory_store._connection.execute(
        "SELECT COUNT(*) AS total FROM memory_records"
    ).fetchone()
    return int(row["total"])


def _prepare_search_run(
    environment: _Environment,
    *,
    workspace: WorkspaceSummary | None = None,
) -> ToolExecutionContext:
    thread, _event = environment.store.create_thread(
        "Memory tool",
        workspace=workspace,
    )
    prepared = prepare_turn(
        environment.store,
        thread_id=thread.id,
        branch_id=thread.default_branch_id,
        content="Exercise the Memory tool.",
        provider_id="scripted",
        model_id="scripted-v1",
    )
    return _context(prepared.run_id, thread.id)


def test_memory_tool_definition_freezes_the_argument_surface() -> None:
    class _UnusedBackend:
        def save_memory(self, **_arguments: object) -> dict[str, Any]:
            raise AssertionError("the definition must not touch the backend")

        def search_memories(self, **_arguments: object) -> dict[str, Any]:
            raise AssertionError("the definition must not touch the backend")

        def update_memory(self, **_arguments: object) -> dict[str, Any]:
            raise AssertionError("the definition must not touch the backend")

        def forget_memory(self, **_arguments: object) -> dict[str, Any]:
            raise AssertionError("the definition must not touch the backend")

    assert MemoryTool(_UnusedBackend()).definition == memory_tool_definition()
    definition = memory_tool_definition()
    assert definition.name == "memory"
    schema = definition.input_schema
    assert schema["required"] == ["action"]
    assert schema["additionalProperties"] is False
    properties = cast(dict[str, dict[str, Any]], schema["properties"])
    assert properties["action"]["enum"] == ["save", "search", "update", "forget"]
    assert properties["kind"]["enum"] == [
        "fact",
        "preference",
        "project",
        "relationship",
    ]
    assert properties["scope"]["enum"] == ["global", "workspace"]
    assert properties["content"]["maxLength"] == 2_048
    assert properties["memoryId"]["pattern"] == "^memory_[0-9a-f]{32}$"
    assert properties["memoryId"]["minLength"] == 39
    assert properties["memoryId"]["maxLength"] == 39
    assert properties["limit"]["minimum"] == 1
    assert properties["limit"]["maximum"] == 8


@pytest.mark.asyncio
async def test_memory_tool_save_writes_once_and_replays_the_same_item(
    tmp_path: Path,
) -> None:
    environment = _environment(tmp_path)
    try:
        context = _prepare_search_run(environment)
        arguments = {
            "action": "save",
            "content": "The user prefers concise technical explanations.",
            "kind": "preference",
            "scope": "global",
        }

        first = await _call(environment.tool, arguments, context)

        assert first.ok is True
        assert first.payload["action"] == "save"
        assert first.payload["kind"] == "preference"
        assert first.payload["scope"] == "global"
        assert first.payload["revision"] == 1
        assert first.payload["created"] is True
        assert first.details == {
            "action": "save",
            "truncated": False,
            "memoryId": first.payload["memoryId"],
            "created": True,
        }
        memory_id = cast(str, first.payload["memoryId"])
        stored = environment.memory_store.get_memory(memory_id)
        assert stored.content == "The user prefers concise technical explanations."
        assert stored.scope.to_wire() == {"type": "global", "key": None}
        assert stored.provenance.source_kind == "session_item"
        config = environment.store.get_run_config(context.run_id)
        assert stored.provenance.thread_id == config.thread_id
        assert stored.provenance.turn_id == config.turn_id
        assert stored.provenance.item_id == config.user_item_id
        assert _count_memories(environment.memory_store) == 1

        replay = await _call(environment.tool, arguments, context)

        assert replay.payload["memoryId"] == memory_id
        assert replay.payload["created"] is False
        assert replay.details["created"] is False
        assert _count_memories(environment.memory_store) == 1

        second_item = await _call(
            environment.tool,
            arguments,
            _context(context.run_id, context.thread_id, _OTHER_TOOL_ITEM_ID),
        )

        assert second_item.payload["created"] is True
        assert second_item.payload["memoryId"] != memory_id
        assert _count_memories(environment.memory_store) == 2
    finally:
        environment.store.close()
        environment.memory_store.close()


@pytest.mark.asyncio
async def test_memory_tool_save_derives_the_workspace_scope_from_the_run(
    tmp_path: Path,
) -> None:
    environment = _environment(tmp_path)
    try:
        context = _prepare_search_run(
            environment,
            workspace=WorkspaceSummary(
                "workspace_test",
                "Test",
                "project",
                str(tmp_path),
            ),
        )

        saved = await _call(
            environment.tool,
            {
                "action": "save",
                "content": "This workspace ships on Linux first.",
                "kind": "project",
                "scope": "workspace",
            },
            context,
        )

        assert saved.ok is True
        stored = environment.memory_store.get_memory(
            cast(str, saved.payload["memoryId"])
        )
        assert stored.scope.to_wire() == {
            "type": "workspace",
            "key": "workspace_test",
        }

        unscoped = _prepare_search_run(environment)
        rejected = await _call(
            environment.tool,
            {
                "action": "save",
                "content": "This must not become workspace Memory.",
                "kind": "project",
                "scope": "workspace",
            },
            unscoped,
        )

        assert rejected.ok is False
        assert rejected.details["errorCode"] == "invalid_arguments"
        assert "Workspace" in rejected.output
        assert _count_memories(environment.memory_store) == 1
    finally:
        environment.store.close()
        environment.memory_store.close()


@pytest.mark.asyncio
async def test_memory_tool_save_rejects_out_of_contract_arguments(
    tmp_path: Path,
) -> None:
    environment = _environment(tmp_path)
    try:
        context = _prepare_search_run(environment)
        base: dict[str, Any] = {
            "action": "save",
            "content": "A valid body.",
            "kind": "fact",
            "scope": "global",
        }
        cases: list[dict[str, Any]] = [
            {"action": "forget", "content": "A valid body."},
            {"action": "save", "content": "A valid body.", "kind": "fact"},
            {**base, "extra": True},
            {**base, "content": 12},
            {**base, "content": "   "},
            {**base, "content": "x" * 2_049},
            {**base, "kind": "opinion"},
            {**base, "scope": "team"},
        ]

        for arguments in cases:
            outcome = await _call(environment.tool, arguments, context)
            assert outcome.ok is False, arguments
            assert outcome.details["errorCode"] == "invalid_arguments", arguments
            assert _count_memories(environment.memory_store) == 0

        unknown_run = await _call(
            environment.tool,
            base,
            _context("run_" + "0" * 32, context.thread_id),
        )

        assert unknown_run.ok is False
        assert unknown_run.details["errorCode"] == "memory_unavailable"
        assert _count_memories(environment.memory_store) == 0
    finally:
        environment.store.close()
        environment.memory_store.close()


@pytest.mark.asyncio
async def test_memory_tool_save_rejects_protected_credential_values(
    tmp_path: Path,
) -> None:
    environment = _environment(tmp_path, protected_values=(_PROTECTED_VALUE,))
    try:
        context = _prepare_search_run(environment)

        outcome = await _call(
            environment.tool,
            {
                "action": "save",
                "content": f"the deployment token is {_PROTECTED_VALUE}",
                "kind": "fact",
                "scope": "global",
            },
            context,
        )

        assert outcome.ok is False
        assert outcome.details["errorCode"] == "invalid_arguments"
        assert "protected configuration data" in outcome.output
        assert _PROTECTED_VALUE not in json.dumps(outcome.details) + outcome.output
        assert _count_memories(environment.memory_store) == 0
    finally:
        environment.store.close()
        environment.memory_store.close()


@pytest.mark.asyncio
async def test_memory_tool_update_appends_revision_and_preserves_scope(
    tmp_path: Path,
) -> None:
    environment = _environment(tmp_path)
    try:
        context = _prepare_search_run(environment)
        saved = await _call(
            environment.tool,
            {
                "action": "save",
                "content": "The user prefers concise technical explanations.",
                "kind": "preference",
                "scope": "global",
            },
            context,
        )
        memory_id = cast(str, saved.payload["memoryId"])
        update_context = _context(
            context.run_id,
            context.thread_id,
            _OTHER_TOOL_ITEM_ID,
        )
        arguments = {
            "action": "update",
            "memoryId": memory_id,
            "content": "The user prefers light mode.",
        }

        updated = await _call(environment.tool, arguments, update_context)

        assert updated.ok is True
        assert updated.payload == {
            "action": "update",
            "memoryId": memory_id,
            "revision": 2,
            "created": True,
            "truncated": False,
        }
        assert updated.details == {
            "action": "update",
            "truncated": False,
            "memoryId": memory_id,
            "created": True,
            "revision": 2,
        }
        stored = environment.memory_store.get_memory(memory_id)
        assert stored.content == "The user prefers light mode."
        assert stored.kind == "preference"
        assert stored.scope.to_wire() == {"type": "global", "key": None}
        assert stored.revision == 2
        config = environment.store.get_run_config(context.run_id)
        assert stored.provenance.source_kind == "session_item"
        assert stored.provenance.item_id == config.user_item_id
        old_revision = environment.memory_store._connection.execute(
            """
            SELECT content, source_kind
            FROM memory_revisions
            WHERE memory_id = ? AND revision = 1
            """,
            (memory_id,),
        ).fetchone()
        assert old_revision is not None
        assert tuple(old_revision) == (
            "The user prefers concise technical explanations.",
            "session_item",
        )
        assert _count_memories(environment.memory_store) == 1

        replay = await _call(environment.tool, arguments, update_context)

        assert replay.payload["revision"] == 2
        assert replay.payload["created"] is False
        assert _count_memories(environment.memory_store) == 1

        found = await _call(
            environment.tool,
            {"action": "search", "query": "light mode"},
            context,
        )
        memories = cast(list[dict[str, Any]], found.payload["memories"])
        assert [(memory["id"], memory["revision"], memory["content"]) for memory in memories] == [
            (memory_id, 2, "The user prefers light mode.")
        ]
        old = await _call(
            environment.tool,
            {"action": "search", "query": "concise technical explanations"},
            context,
        )
        assert old.payload["count"] == 0
    finally:
        environment.store.close()
        environment.memory_store.close()


@pytest.mark.asyncio
async def test_memory_tool_forget_is_idempotent_and_removes_from_search(
    tmp_path: Path,
) -> None:
    environment = _environment(tmp_path)
    try:
        context = _prepare_search_run(environment)
        saved = await _call(
            environment.tool,
            {
                "action": "save",
                "content": "The release cadence is weekly.",
                "kind": "project",
                "scope": "global",
            },
            context,
        )
        memory_id = cast(str, saved.payload["memoryId"])
        forget_context = _context(
            context.run_id,
            context.thread_id,
            _THIRD_TOOL_ITEM_ID,
        )
        arguments = {"action": "forget", "memoryId": memory_id}

        forgotten = await _call(environment.tool, arguments, forget_context)

        assert forgotten.ok is True
        assert forgotten.payload == {
            "action": "forget",
            "memoryId": memory_id,
            "revision": 2,
            "created": True,
            "truncated": False,
        }
        stored = environment.memory_store.get_memory(memory_id)
        assert stored.state == "forgotten"
        assert stored.revision == 2
        assert stored.content is None
        assert stored.forgotten_at is not None
        config = environment.store.get_run_config(context.run_id)
        assert stored.provenance.source_kind == "session_item"
        assert stored.provenance.item_id == config.user_item_id
        found = await _call(
            environment.tool,
            {"action": "search", "query": "release cadence"},
            context,
        )
        assert found.payload["count"] == 0

        replay = await _call(environment.tool, arguments, forget_context)

        assert replay.payload["revision"] == 2
        assert replay.payload["created"] is False
        assert environment.memory_store.get_memory(memory_id).revision == 2
    finally:
        environment.store.close()
        environment.memory_store.close()


@pytest.mark.asyncio
async def test_memory_tool_update_rejects_mutation_and_protected_content(
    tmp_path: Path,
) -> None:
    protected_value = "sk-memory-update-sentinel-0123456789"
    environment = _environment(tmp_path, protected_values=(protected_value,))
    try:
        context = _prepare_search_run(environment)
        saved = await _call(
            environment.tool,
            {
                "action": "save",
                "content": "A safe durable fact.",
                "kind": "fact",
                "scope": "global",
            },
            context,
        )
        memory_id = cast(str, saved.payload["memoryId"])
        update_context = _context(
            context.run_id,
            context.thread_id,
            _OTHER_TOOL_ITEM_ID,
        )
        contract_violation = await _call(
            environment.tool,
            {
                "action": "update",
                "memoryId": memory_id,
                "content": "Attempted change.",
                "kind": "preference",
            },
            update_context,
        )
        assert contract_violation.details["errorCode"] == "invalid_arguments"

        protected = await _call(
            environment.tool,
            {
                "action": "update",
                "memoryId": memory_id,
                "content": f"The token is {protected_value}.",
            },
            update_context,
        )

        assert protected.ok is False
        assert protected.details["errorCode"] == "invalid_arguments"
        assert protected_value not in protected.output
        stored = environment.memory_store.get_memory(memory_id)
        assert stored.content == "A safe durable fact."
        assert stored.revision == 1
        assert stored.kind == "fact"
        assert stored.scope.to_wire() == {"type": "global", "key": None}
    finally:
        environment.store.close()
        environment.memory_store.close()


@pytest.mark.asyncio
async def test_memory_tool_update_and_forget_report_stable_reason_codes(
    tmp_path: Path,
) -> None:
    environment = _environment(tmp_path)
    try:
        context = _prepare_search_run(environment)
        missing = await _call(
            environment.tool,
            {
                "action": "update",
                "memoryId": "memory_" + "0" * 32,
                "content": "A replacement.",
            },
            _context(context.run_id, context.thread_id, _OTHER_TOOL_ITEM_ID),
        )
        assert missing.ok is False
        assert missing.details["errorCode"] == "memory_not_found"

        saved = await _call(
            environment.tool,
            {
                "action": "save",
                "content": "A fact that will be forgotten.",
                "kind": "fact",
                "scope": "global",
            },
            context,
        )
        memory_id = cast(str, saved.payload["memoryId"])
        forgot = await _call(
            environment.tool,
            {"action": "forget", "memoryId": memory_id},
            _context(context.run_id, context.thread_id, _THIRD_TOOL_ITEM_ID),
        )
        assert forgot.ok is True

        stale_update = await _call(
            environment.tool,
            {
                "action": "update",
                "memoryId": memory_id,
                "content": "Cannot revive a tombstone.",
            },
            _context(context.run_id, context.thread_id, _TOOL_ITEM_ID),
        )
        assert stale_update.ok is False
        assert stale_update.details["errorCode"] == "memory_forgotten"

        conflict = await _call(
            environment.tool,
            {
                "action": "update",
                "memoryId": memory_id,
                "content": "A different body for the same request id.",
            },
            _context(context.run_id, context.thread_id, _THIRD_TOOL_ITEM_ID),
        )
        assert conflict.ok is False
        assert conflict.details["errorCode"] == "memory_idempotency_conflict"
    finally:
        environment.store.close()
        environment.memory_store.close()


@pytest.mark.asyncio
async def test_memory_tool_search_ranks_scoped_memories_and_bounds_the_limit(
    tmp_path: Path,
) -> None:
    environment = _environment(tmp_path)
    try:
        context = _prepare_search_run(
            environment,
            workspace=WorkspaceSummary(
                "workspace_test",
                "Test",
                "project",
                str(tmp_path),
            ),
        )
        memory_store = environment.memory_store
        relevant = memory_store.create_memory_once(
            kind="project",
            scope=MemoryScope("global", None),
            content="The starlattice project runs on zephyr workers.",
            client_request_id="search-a",
        ).memory_id
        memory_store.create_memory_once(
            kind="fact",
            scope=MemoryScope("global", None),
            content="Zephyr is the codename of the retired scheduler.",
            client_request_id="search-b",
        )
        memory_store.create_memory_once(
            kind="preference",
            scope=MemoryScope("workspace", "workspace_test"),
            content="The user prefers zephyr summaries in this workspace.",
            client_request_id="search-c",
        )
        memory_store.create_memory_once(
            kind="fact",
            scope=MemoryScope("workspace", "workspace_other"),
            content="Zephyr is hidden in another workspace.",
            client_request_id="search-d",
        )
        memory_store.create_memory_once(
            kind="relationship",
            scope=MemoryScope("global", None),
            content="Alice mentors Bob on documentation.",
            client_request_id="search-e",
        )

        found = await _call(
            environment.tool,
            {"action": "search", "query": "starlattice zephyr"},
            context,
        )

        assert found.ok is True
        memories = cast(list[dict[str, Any]], found.payload["memories"])
        assert memories[0]["id"] == relevant
        assert found.payload["count"] == len(memories) == 3
        assert found.payload["truncated"] is False
        assert found.payload["omittedCount"] == 0
        assert found.payload["characters"] == sum(
            len(cast(str, memory["content"])) for memory in memories
        )
        assert found.details == {"action": "search", "truncated": False, "count": 3}
        assert {memory["scope"] for memory in memories} == {"global", "workspace"}
        assert {memory["kind"] for memory in memories} == {
            "fact",
            "preference",
            "project",
        }
        assert all(memory["revision"] == 1 for memory in memories)
        assert "hidden in another workspace" not in found.output

        limited = await _call(
            environment.tool,
            {"action": "search", "query": "starlattice zephyr", "limit": 2},
            context,
        )

        limited_memories = cast(list[dict[str, Any]], limited.payload["memories"])
        assert limited_memories[0]["id"] == relevant
        assert limited.payload["count"] == 2
        assert limited.payload["truncated"] is True
        assert limited.payload["omittedCount"] == 1
        assert limited.details["truncated"] is True

        single = await _call(
            environment.tool,
            {"action": "search", "query": "alice mentors"},
            context,
        )

        assert single.ok is True
        assert single.payload["count"] == 1
        assert single.payload["truncated"] is False
    finally:
        environment.store.close()
        environment.memory_store.close()


@pytest.mark.asyncio
async def test_memory_tool_search_is_read_only_and_character_bounded(
    tmp_path: Path,
) -> None:
    environment = _environment(tmp_path)
    try:
        context = _prepare_search_run(environment)
        memory_store = environment.memory_store
        for index in range(4):
            prefix = f"truncationword {index} "
            memory_store.create_memory_once(
                kind="fact",
                scope=MemoryScope("global", None),
                content=prefix + "z" * (2_048 - len(prefix)),
                client_request_id=f"bounded-{index}",
            )
        sequence_before = environment.store.latest_sequence()

        found = await _call(
            environment.tool,
            {"action": "search", "query": "truncationword", "limit": 8},
            context,
        )

        assert found.ok is True
        memories = cast(list[dict[str, Any]], found.payload["memories"])
        assert found.payload["count"] == len(memories) == 2
        assert found.payload["characters"] == 4_096
        assert found.payload["characters"] <= MEMORY_SELECTION_MAX_CHARACTERS
        assert found.payload["truncated"] is True
        assert found.payload["omittedCount"] == 2
        assert found.details["count"] == 2
        assert _count_memories(memory_store) == 4
        assert environment.store.latest_sequence() == sequence_before
    finally:
        environment.store.close()
        environment.memory_store.close()


@pytest.mark.asyncio
async def test_memory_tool_search_rejects_out_of_contract_arguments(
    tmp_path: Path,
) -> None:
    environment = _environment(tmp_path)
    try:
        context = _prepare_search_run(environment)
        cases: list[dict[str, Any]] = [
            {"action": "search"},
            {"action": "search", "query": ""},
            {"action": "search", "query": "   "},
            {"action": "search", "query": 7},
            {"action": "search", "query": "zephyr", "limit": 0},
            {"action": "search", "query": "zephyr", "limit": 9},
            {"action": "search", "query": "zephyr", "limit": "3"},
            {"action": "search", "query": "zephyr", "limit": True},
            {"action": "search", "query": "zephyr", "kind": "fact"},
        ]

        for arguments in cases:
            outcome = await _call(environment.tool, arguments, context)
            assert outcome.ok is False, arguments
            assert outcome.details["errorCode"] == "invalid_arguments", arguments
    finally:
        environment.store.close()
        environment.memory_store.close()
