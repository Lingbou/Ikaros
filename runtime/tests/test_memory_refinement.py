"""Background Memory refinement: cursors, budgets, isolation, and lifecycle."""

from __future__ import annotations

import asyncio
import json
import sqlite3
from collections.abc import AsyncIterator, Sequence
from pathlib import Path
from typing import cast

import pytest

from ikaros_runtime.agent.loop import AgentLoop
from ikaros_runtime.cancellation import CancellationToken
from ikaros_runtime.domain import JournalEvent, WorkspaceSummary
from ikaros_runtime.errors import MemorySchemaIncompatibleError, ProviderFailure
from ikaros_runtime.memory import (
    MemoryRefinementService,
    MemoryScope,
    MemorySourceSnapshot,
    SqliteMemoryStore,
)
from ikaros_runtime.memory.refinement import (
    MEMORY_REFINEMENT_MAX_INPUT_BYTES,
    MEMORY_REFINEMENT_MAX_ITEMS,
    MemoryCandidate,
)
from ikaros_runtime.memory.schema import MEMORY_SCHEMA_VERSION
from ikaros_runtime.providers.base import (
    ProviderAdapter,
    ProviderEvent,
    ProviderRequest,
    ResponseCompleted,
    TextDelta,
)
from ikaros_runtime.run_input import ProviderExecutionSnapshot, RunConfig
from ikaros_runtime.services.memories import MemoryService
from ikaros_runtime.storage import SqliteRuntimeStore

from .helpers import prepare_turn

_PROVIDER_ID = "scripted"
_MODEL_ID = "scripted-v1"


def _snapshot(provider_id: str, model_id: str) -> ProviderExecutionSnapshot:
    return ProviderExecutionSnapshot(
        provider_id=provider_id,
        origin="test",
        base_url=None,
        model_id=model_id,
        supports_tools=True,
    )



class _ExtractionProvider:
    """Replay canned extraction responses and record every request."""

    def __init__(self, responses: Sequence[str | Exception] = ()) -> None:
        self.responses = list(responses)
        self.requests: list[ProviderRequest] = []

    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]:
        cancellation.raise_if_cancelled()
        self.requests.append(request)
        response = self.responses.pop(0) if self.responses else '{"memories":[]}'
        if isinstance(response, Exception):
            raise response
        yield TextDelta(response)
        yield ResponseCompleted()


class _RunAndExtractProvider:
    """Answer an Agent Run, then reply to the background extraction call."""

    def __init__(self, extraction: str) -> None:
        self._extraction = extraction
        self.run_requests: list[ProviderRequest] = []
        self.extraction_requests: list[ProviderRequest] = []

    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]:
        cancellation.raise_if_cancelled()
        extraction = any(
            message.role == "system" and "durable long-term Memory" in message.content
            for message in request.messages
        )
        if extraction:
            self.extraction_requests.append(request)
            yield TextDelta(self._extraction)
            yield ResponseCompleted()
            return
        self.run_requests.append(request)
        yield TextDelta("Durable state lives in SQLite.")
        yield ResponseCompleted()


class _BlockingProvider:
    def __init__(self, response: str) -> None:
        self.response = response
        self.started = asyncio.Event()
        self.release = asyncio.Event()
        self.cancelled = 0
        self.requests: list[ProviderRequest] = []

    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]:
        self.requests.append(request)
        self.started.set()
        try:
            await self.release.wait()
        except asyncio.CancelledError:
            self.cancelled += 1
            raise
        yield TextDelta(self.response)
        yield ResponseCompleted()


def _service(
    *,
    memory_store: SqliteMemoryStore,
    runtime_store: SqliteRuntimeStore,
    provider: ProviderAdapter | None,
    enabled: bool = True,
    protected_values: Sequence[str] = (),
) -> MemoryRefinementService:
    return MemoryRefinementService(
        memory_store=memory_store,
        runtime_store=runtime_store,
        resolve_provider=lambda provider_id: (
            provider if provider_id == _PROVIDER_ID else None
        ),
        provider_snapshot=_snapshot,
        protected_values=lambda: tuple(protected_values),
        enabled=enabled,
    )


async def _drain(service: MemoryRefinementService) -> None:
    tasks = tuple(service._tasks.values())
    if tasks:
        await asyncio.gather(*tasks)


def _thread_with_turns(
    store: SqliteRuntimeStore,
    contents: Sequence[str],
    *,
    workspace: WorkspaceSummary | None = None,
) -> tuple[str, str, list[tuple[str, str, str]]]:
    thread, _event = store.create_thread("Memory refinement", workspace=workspace)
    created: list[tuple[str, str, str]] = []
    for content in contents:
        prepared = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content=content,
            provider_id=_PROVIDER_ID,
            model_id=_MODEL_ID,
        )
        item_id = prepared.initial_events[0].item_id
        assert item_id is not None
        created.append((prepared.turn_id, prepared.run_id, item_id))
    return thread.id, thread.default_branch_id, created


def _active_memories(memory_store: SqliteMemoryStore) -> list[str]:
    page = memory_store.list_memories(
        cursor=None,
        limit=100,
        scope=None,
        kind=None,
        state="active",
    )
    return [memory.id for memory in page.memories]


def _payload(request: ProviderRequest) -> dict[str, object]:
    content = request.messages[-1].content
    start = content.index("{")
    parsed = json.loads(content[start:])
    assert isinstance(parsed, dict)
    return cast(dict[str, object], parsed)


def _payload_records(request: ProviderRequest) -> list[dict[str, object]]:
    parsed = _payload(request)["records"]
    assert isinstance(parsed, list)
    records: list[dict[str, object]] = []
    for entry in parsed:
        assert isinstance(entry, dict)
        records.append(cast(dict[str, object], entry))
    return records


def _payload_existing_memories(request: ProviderRequest) -> list[dict[str, object]]:
    parsed = _payload(request)["existingMemories"]
    assert isinstance(parsed, list)
    records: list[dict[str, object]] = []
    for entry in parsed:
        assert isinstance(entry, dict)
        records.append(cast(dict[str, object], entry))
    return records


def _candidate(
    item_id: str,
    *,
    content: str = "Durable state lives in SQLite.",
    kind: str = "fact",
    scope: str = "global",
    memory_id: str | None = None,
) -> dict[str, object]:
    candidate: dict[str, object] = {
        "kind": kind,
        "scope": scope,
        "content": content,
        "sourceItemId": item_id,
    }
    if memory_id is not None:
        candidate["memoryId"] = memory_id
    return candidate


def _response(*candidates: dict[str, object]) -> str:
    return json.dumps({"memories": list(candidates)})


@pytest.mark.asyncio
async def test_completed_run_refines_memory_and_advances_cursor(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    memory_store = SqliteMemoryStore(tmp_path / "memory.db")
    try:
        workspace = WorkspaceSummary(
            id="ws_memory",
            name="Ikaros",
            kind="project",
            root_uri=None,
        )
        thread, branch_id, turns = _thread_with_turns(
            store,
            ["We keep durable state in SQLite."],
            workspace=workspace,
        )
        turn_id, run_id, item_id = turns[0]
        provider = _RunAndExtractProvider(
            _response(_candidate(item_id, kind="project", scope="workspace"))
        )
        service = _service(
            memory_store=memory_store,
            runtime_store=store,
            provider=provider,
        )
        events: list[JournalEvent] = []

        async def publish(event: JournalEvent) -> None:
            events.append(event)

        loop = AgentLoop(
            store,
            {_PROVIDER_ID: provider},
            publish,
            on_run_terminal=service.notify_run_terminal,
        )
        await loop.run(run_id, CancellationToken())
        await _drain(service)

        assert store.run_status(run_id) == "completed"
        assert len(provider.extraction_requests) == 1
        extraction = provider.extraction_requests[0]
        assert extraction.model_id == _MODEL_ID
        assert extraction.tools == ()
        assert item_id in extraction.messages[-1].content
        assert "JSON" in extraction.messages[0].content

        memory_ids = _active_memories(memory_store)
        assert len(memory_ids) == 1
        record = memory_store.get_memory(memory_ids[0])
        assert record.content == "Durable state lives in SQLite."
        assert record.kind == "project"
        assert record.scope == MemoryScope("workspace", "ws_memory")
        assert record.provenance.source_kind == "auto_extracted"
        assert record.provenance.thread_id == thread
        assert record.provenance.turn_id == turn_id
        assert record.provenance.item_id == item_id
        memories = MemoryService(memory_store, lambda value: None, store)
        detail = cast(dict[str, object], memories.get({"memoryId": memory_ids[0]}))
        wire = cast(dict[str, object], detail["memory"])
        assert wire["provenance"] == {
            "sourceKind": "auto_extracted",
            "threadId": thread,
            "turnId": turn_id,
            "itemId": item_id,
            "status": "available",
        }

        settled = store.read_settled_thread_items_after(
            thread_id=thread,
            after=None,
            limit=MEMORY_REFINEMENT_MAX_ITEMS,
        )
        cursor = memory_store.memory_extraction_cursor(thread)
        assert cursor is not None
        assert cursor.item_id == settled[-1].item_id
        assert cursor.item_ordinal == settled[-1].item_ordinal
        assert cursor.run_id == settled[-1].run_id
    finally:
        memory_store.close()
        store.close()


@pytest.mark.asyncio
async def test_refinement_ignores_disabled_and_unsettled_runs(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    memory_store = SqliteMemoryStore(tmp_path / "memory.db")
    try:
        _thread_id, _branch_id, turns = _thread_with_turns(store, ["A preference."])
        _turn_id, run_id, _item_id = turns[0]
        config = store.get_run_config(run_id)
        provider = _ExtractionProvider()
        disabled = _service(
            memory_store=memory_store,
            runtime_store=store,
            provider=provider,
            enabled=False,
        )
        disabled.notify_run_terminal(config, "completed")
        assert disabled._tasks == {}
        assert disabled.enabled is False

        enabled = _service(
            memory_store=memory_store,
            runtime_store=store,
            provider=provider,
        )
        enabled.notify_run_terminal(config, "failed")
        enabled.notify_run_terminal(config, "cancelled")
        assert enabled._tasks == {}
        assert provider.requests == []
        assert _active_memories(memory_store) == []
    finally:
        memory_store.close()
        store.close()


@pytest.mark.asyncio
async def test_refinement_runs_for_every_completed_run(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    memory_store = SqliteMemoryStore(tmp_path / "memory.db")
    try:
        thread, branch_id, turns = _thread_with_turns(store, ["First preference."])
        _turn_id, run_id, first_item = turns[0]
        provider = _ExtractionProvider([_response(_candidate(first_item))])
        service = _service(
            memory_store=memory_store,
            runtime_store=store,
            provider=provider,
        )
        config = store.get_run_config(run_id)

        service.notify_run_terminal(config, "completed")
        service.notify_run_terminal(config, "completed")
        assert len(service._tasks) == 1
        await _drain(service)
        assert len(provider.requests) == 1
        assert len(_active_memories(memory_store)) == 1

        # No new Items: the pass returns before spending a provider call.
        service.notify_run_terminal(config, "completed")
        await _drain(service)
        assert len(provider.requests) == 1

        # A new Run refines immediately: no hourly budget gates the next pass.
        prepared = prepare_turn(
            store,
            thread_id=thread,
            branch_id=branch_id,
            content="Second preference.",
            provider_id=_PROVIDER_ID,
            model_id=_MODEL_ID,
        )
        second_item = prepared.initial_events[0].item_id
        assert second_item is not None
        provider.responses.append(
            _response(_candidate(second_item, content="The team deploys on Fridays."))
        )
        service.notify_run_terminal(store.get_run_config(prepared.run_id), "completed")
        await _drain(service)
        assert len(provider.requests) == 2
        assert [record["itemId"] for record in _payload_records(provider.requests[1])] == [
            second_item
        ]
        assert len(_active_memories(memory_store)) == 2
    finally:
        memory_store.close()
        store.close()


class _QueuedBlockingProvider:
    """Block the first extraction call until the test releases it."""

    def __init__(self, responses: Sequence[str]) -> None:
        self.responses = list(responses)
        self.started = asyncio.Event()
        self.release = asyncio.Event()
        self.requests: list[ProviderRequest] = []

    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]:
        cancellation.raise_if_cancelled()
        self.requests.append(request)
        response = self.responses.pop(0)
        if len(self.requests) == 1:
            self.started.set()
            await self.release.wait()
        yield TextDelta(response)
        yield ResponseCompleted()


@pytest.mark.asyncio
async def test_refinement_notifications_during_a_pass_queue_another_pass(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    memory_store = SqliteMemoryStore(tmp_path / "memory.db")
    try:
        thread, branch_id, turns = _thread_with_turns(store, ["First preference."])
        _turn_id, run_id, first_item = turns[0]
        provider = _QueuedBlockingProvider([_response(_candidate(first_item))])
        service = _service(
            memory_store=memory_store,
            runtime_store=store,
            provider=provider,
        )
        config = store.get_run_config(run_id)

        service.notify_run_terminal(config, "completed")
        await provider.started.wait()
        prepared = prepare_turn(
            store,
            thread_id=thread,
            branch_id=branch_id,
            content="Second preference.",
            provider_id=_PROVIDER_ID,
            model_id=_MODEL_ID,
        )
        second_item = prepared.initial_events[0].item_id
        assert second_item is not None
        provider.responses.append(
            _response(_candidate(second_item, content="The team deploys on Fridays."))
        )
        service.notify_run_terminal(store.get_run_config(prepared.run_id), "completed")
        provider.release.set()
        await _drain(service)

        assert len(provider.requests) == 2
        assert [record["itemId"] for record in _payload_records(provider.requests[1])] == [
            second_item
        ]
        assert len(_active_memories(memory_store)) == 2
    finally:
        memory_store.close()
        store.close()

@pytest.mark.asyncio
async def test_refinement_bounds_item_and_candidate_budgets(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    memory_store = SqliteMemoryStore(tmp_path / "memory.db")
    try:
        contents = [f"Item number {index}." for index in range(MEMORY_REFINEMENT_MAX_ITEMS + 5)]
        thread, _branch_id, turns = _thread_with_turns(store, contents)
        _turn_id, run_id, _item_id = turns[0]
        candidates = [
            _candidate(turns[0][2], content=f"Fact number {index}.")
            for index in range(8)
        ]
        provider = _ExtractionProvider([_response(*candidates)])
        service = _service(
            memory_store=memory_store,
            runtime_store=store,
            provider=provider,
        )
        service.notify_run_terminal(store.get_run_config(run_id), "completed")
        await _drain(service)

        records = _payload_records(provider.requests[0])
        assert len(records) == MEMORY_REFINEMENT_MAX_ITEMS
        assert len(_active_memories(memory_store)) == 4
        cursor = memory_store.memory_extraction_cursor(thread)
        assert cursor is not None
        assert cursor.item_id == records[-1]["itemId"]
    finally:
        memory_store.close()
        store.close()


@pytest.mark.asyncio
async def test_refinement_input_budget_defers_oversized_items(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    memory_store = SqliteMemoryStore(tmp_path / "memory.db")
    try:
        contents = [f"{index}" + ("x" * 8_000) for index in range(6)]
        thread, _branch_id, turns = _thread_with_turns(store, contents)
        _turn_id, run_id, _item_id = turns[0]
        provider = _ExtractionProvider(['{"memories":[]}', '{"memories":[]}'])
        service = _service(
            memory_store=memory_store,
            runtime_store=store,
            provider=provider,
            )
        service.notify_run_terminal(store.get_run_config(run_id), "completed")
        await _drain(service)

        request = provider.requests[0]
        content = request.messages[-1].content
        start = content.index("{")
        assert len(content[start:].encode("utf-8")) <= MEMORY_REFINEMENT_MAX_INPUT_BYTES
        first_records = _payload_records(request)
        assert 1 <= len(first_records) < len(contents)
        cursor = memory_store.memory_extraction_cursor(thread)
        assert cursor is not None
        assert cursor.item_id == first_records[-1]["itemId"]

        service.notify_run_terminal(store.get_run_config(run_id), "completed")
        await _drain(service)
        rest = _payload_records(provider.requests[1])
        assert len(rest) == len(contents) - len(first_records)
        assert rest[0]["itemId"] != first_records[0]["itemId"]
    finally:
        memory_store.close()
        store.close()


def test_auto_extracted_writes_are_idempotent_and_deduplicated(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    memory_store = SqliteMemoryStore(tmp_path / "memory.db")
    try:
        _thread_id, _branch_id, turns = _thread_with_turns(
            store, ["Durable state lives in SQLite."]
        )
        _turn_id, _run_id, item_id = turns[0]
        captured = store.capture_session_item_provenance(item_id)
        snapshot = MemorySourceSnapshot(
            thread_id=captured.thread_id,
            turn_id=captured.turn_id,
            item_id=captured.item_id,
            item_digest=captured.item_digest,
        )
        request_id = "refine_" + "a" * 32
        first = memory_store.create_memory_once(
            kind="fact",
            scope=MemoryScope("global", None),
            content="Durable state lives in SQLite.",
            client_request_id=request_id,
            source=snapshot,
            source_kind="auto_extracted",
        )
        second = memory_store.create_memory_once(
            kind="fact",
            scope=MemoryScope("global", None),
            content="Durable state lives in SQLite.",
            client_request_id=request_id,
            source=snapshot,
            source_kind="auto_extracted",
        )
        assert first.created is True
        assert second.created is False
        assert second.memory_id == first.memory_id
        assert _active_memories(memory_store) == [first.memory_id]
        assert memory_store.active_memory_content_exists(
            kind="fact",
            scope=MemoryScope("global", None),
            content="Durable state lives in SQLite.",
        )

        memory_store.forget_memory_once(
            memory_id=first.memory_id,
            expected_revision=1,
            client_request_id="forget-refined-memory",
        )
        assert not memory_store.active_memory_content_exists(
            kind="fact",
            scope=MemoryScope("global", None),
            content="Durable state lives in SQLite.",
        )
    finally:
        memory_store.close()
        store.close()


@pytest.mark.asyncio
async def test_refinement_skips_canonical_duplicates_and_logs_counts(
    tmp_path: Path,
    caplog: pytest.LogCaptureFixture,
) -> None:
    caplog.set_level("INFO", logger="ikaros_runtime.memory.refinement")
    store = SqliteRuntimeStore(tmp_path / "state.db")
    memory_store = SqliteMemoryStore(tmp_path / "memory.db")
    try:
        thread, branch_id, turns = _thread_with_turns(
            store,
            ["The user prefers dark mode."],
        )
        _turn_id, run_id, first_item = turns[0]
        provider = _ExtractionProvider(
            [_response(_candidate(first_item, content="The user prefers dark mode."))]
        )
        service = _service(
            memory_store=memory_store,
            runtime_store=store,
            provider=provider,
        )
        service.notify_run_terminal(store.get_run_config(run_id), "completed")
        await _drain(service)

        memory_id = _active_memories(memory_store)[0]
        assert "written=1" in caplog.records[-1].message

        prepared = prepare_turn(
            store,
            thread_id=thread,
            branch_id=branch_id,
            content="Another turn restates the same preference.",
            provider_id=_PROVIDER_ID,
            model_id=_MODEL_ID,
        )
        second_item = prepared.initial_events[0].item_id
        assert second_item is not None
        provider.responses.append(
            _response(
                _candidate(
                    second_item,
                    content="  THE USER PREFERS DARK MODE!  ",
                )
            )
        )
        service.notify_run_terminal(store.get_run_config(prepared.run_id), "completed")
        await _drain(service)

        assert _active_memories(memory_store) == [memory_id]
        record = memory_store.get_memory(memory_id)
        assert record.revision == 1
        assert record.content == "The user prefers dark mode."
        assert "skipped_duplicate=1" in caplog.records[-1].message
    finally:
        memory_store.close()
        store.close()


@pytest.mark.asyncio
async def test_refinement_merges_explicit_memory_id_as_new_revision(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    memory_store = SqliteMemoryStore(tmp_path / "memory.db")
    try:
        thread, branch_id, turns = _thread_with_turns(
            store,
            ["The user prefers dark mode."],
        )
        _turn_id, run_id, first_item = turns[0]
        provider = _ExtractionProvider(
            [_response(_candidate(first_item, content="The user prefers dark mode."))]
        )
        service = _service(
            memory_store=memory_store,
            runtime_store=store,
            provider=provider,
        )
        service.notify_run_terminal(store.get_run_config(run_id), "completed")
        await _drain(service)
        memory_id = _active_memories(memory_store)[0]

        prepared = prepare_turn(
            store,
            thread_id=thread,
            branch_id=branch_id,
            content="The user corrects the theme preference.",
            provider_id=_PROVIDER_ID,
            model_id=_MODEL_ID,
        )
        second_item = prepared.initial_events[0].item_id
        assert second_item is not None
        provider.responses.append(
            _response(
                _candidate(
                    second_item,
                    content="Theme preference: light.",
                    memory_id=memory_id,
                )
            )
        )
        service.notify_run_terminal(store.get_run_config(prepared.run_id), "completed")
        await _drain(service)

        catalog = _payload_existing_memories(provider.requests[1])
        assert [(entry["id"], entry["revision"]) for entry in catalog] == [
            (memory_id, 1)
        ]
        record = memory_store.get_memory(memory_id)
        assert _active_memories(memory_store) == [memory_id]
        assert record.revision == 2
        assert record.content == "Theme preference: light."
        assert record.provenance.source_kind == "auto_extracted"
        assert record.provenance.item_id == second_item
        old_revision = memory_store._connection.execute(
            """
            SELECT content, source_kind
            FROM memory_revisions
            WHERE memory_id = ? AND revision = 1
            """,
            (memory_id,),
        ).fetchone()
        assert old_revision is not None
        assert tuple(old_revision) == ("The user prefers dark mode.", "auto_extracted")
    finally:
        memory_store.close()
        store.close()


@pytest.mark.asyncio
async def test_refinement_auto_merges_a_supplement_without_memory_id(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    memory_store = SqliteMemoryStore(tmp_path / "memory.db")
    try:
        thread, branch_id, turns = _thread_with_turns(
            store,
            ["The user prefers dark mode."],
        )
        _turn_id, run_id, first_item = turns[0]
        provider = _ExtractionProvider(
            [_response(_candidate(first_item, content="The user prefers dark mode."))]
        )
        service = _service(
            memory_store=memory_store,
            runtime_store=store,
            provider=provider,
        )
        service.notify_run_terminal(store.get_run_config(run_id), "completed")
        await _drain(service)
        memory_id = _active_memories(memory_store)[0]

        prepared = prepare_turn(
            store,
            thread_id=thread,
            branch_id=branch_id,
            content="The preference gains a detail.",
            provider_id=_PROVIDER_ID,
            model_id=_MODEL_ID,
        )
        second_item = prepared.initial_events[0].item_id
        assert second_item is not None
        provider.responses.append(
            _response(
                _candidate(
                    second_item,
                    content="The user prefers dark mode at night.",
                )
            )
        )
        service.notify_run_terminal(store.get_run_config(prepared.run_id), "completed")
        await _drain(service)

        record = memory_store.get_memory(memory_id)
        assert record.revision == 2
        assert record.content == "The user prefers dark mode at night."
        assert _active_memories(memory_store) == [memory_id]
        old_content = memory_store._connection.execute(
            "SELECT content FROM memory_revisions WHERE memory_id = ? AND revision = 1",
            (memory_id,),
        ).fetchone()
        assert old_content is not None
        assert old_content["content"] == "The user prefers dark mode."
    finally:
        memory_store.close()
        store.close()


@pytest.mark.asyncio
async def test_refinement_rejects_explicit_target_with_changed_kind(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    memory_store = SqliteMemoryStore(tmp_path / "memory.db")
    try:
        thread, branch_id, turns = _thread_with_turns(
            store,
            ["The user prefers dark mode."],
        )
        _turn_id, run_id, first_item = turns[0]
        provider = _ExtractionProvider(
            [
                _response(
                    _candidate(
                        first_item,
                        kind="preference",
                        content="The user prefers dark mode.",
                    )
                )
            ]
        )
        service = _service(
            memory_store=memory_store,
            runtime_store=store,
            provider=provider,
        )
        service.notify_run_terminal(store.get_run_config(run_id), "completed")
        await _drain(service)
        memory_id = _active_memories(memory_store)[0]

        prepared = prepare_turn(
            store,
            thread_id=thread,
            branch_id=branch_id,
            content="A different kind must not retarget the same Memory.",
            provider_id=_PROVIDER_ID,
            model_id=_MODEL_ID,
        )
        second_item = prepared.initial_events[0].item_id
        assert second_item is not None
        provider.responses.append(
            _response(
                _candidate(
                    second_item,
                    kind="fact",
                    content="The user prefers light mode.",
                    memory_id=memory_id,
                )
            )
        )
        service.notify_run_terminal(store.get_run_config(prepared.run_id), "completed")
        await _drain(service)

        record = memory_store.get_memory(memory_id)
        assert record.kind == "preference"
        assert record.revision == 1
        assert record.content == "The user prefers dark mode."
        assert _active_memories(memory_store) == [memory_id]
    finally:
        memory_store.close()
        store.close()


@pytest.mark.asyncio
async def test_refinement_failures_are_isolated(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    memory_store = SqliteMemoryStore(tmp_path / "memory.db")
    try:
        thread, _branch_id, turns = _thread_with_turns(store, ["We deploy on Fridays."])
        _turn_id, run_id, item_id = turns[0]
        failing = _ExtractionProvider(
            [ProviderFailure("server", "provider is unavailable", retryable=True)]
        )
        service = _service(
            memory_store=memory_store,
            runtime_store=store,
            provider=failing,
            )
        service.notify_run_terminal(store.get_run_config(run_id), "completed")
        await _drain(service)

        assert len(failing.requests) == 1
        assert _active_memories(memory_store) == []
        assert memory_store.memory_extraction_cursor(thread) is None
        assert memory_store.memory_extraction_attempted_at(thread) is not None

        malformed = _ExtractionProvider(["not a json document"])
        retry = _service(
            memory_store=memory_store,
            runtime_store=store,
            provider=malformed,
            )
        retry.notify_run_terminal(store.get_run_config(run_id), "completed")
        await _drain(retry)
        assert _active_memories(memory_store) == []
        assert memory_store.memory_extraction_cursor(thread) is None

        unknown_source = _ExtractionProvider(
            [_response(_candidate("item_" + "0" * 32))]
        )
        tolerant = _service(
            memory_store=memory_store,
            runtime_store=store,
            provider=unknown_source,
            )
        tolerant.notify_run_terminal(store.get_run_config(run_id), "completed")
        await _drain(tolerant)
        assert _active_memories(memory_store) == []
        cursor = memory_store.memory_extraction_cursor(thread)
        assert cursor is not None
        assert cursor.item_id == item_id
    finally:
        memory_store.close()
        store.close()


@pytest.mark.asyncio
async def test_run_terminal_observer_failure_does_not_break_settlement(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        _thread_id, _branch_id, turns = _thread_with_turns(
            store, ["Keep the repository tidy."]
        )
        _turn_id, run_id, _item_id = turns[0]
        provider = _ExtractionProvider()
        observed: list[tuple[str, str]] = []

        def observer(config: RunConfig, status: str) -> None:
            observed.append((config.run_id, status))
            raise RuntimeError("observer exploded")

        events: list[JournalEvent] = []

        async def publish(event: JournalEvent) -> None:
            events.append(event)

        loop = AgentLoop(
            store,
            {_PROVIDER_ID: provider},
            publish,
            on_run_terminal=observer,
        )
        await loop.run(run_id, CancellationToken())

        assert observed == [(run_id, "completed")]
        assert store.run_status(run_id) == "completed"
        settled = [event for event in events if event.type == "run.settled"]
        assert len(settled) == 1
    finally:
        store.close()


@pytest.mark.asyncio
async def test_refinement_never_writes_protected_values(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    memory_store = SqliteMemoryStore(tmp_path / "memory.db")
    secret = "sk-live-secret-value"
    try:
        thread, _branch_id, turns = _thread_with_turns(
            store, ["Our key is sk-live-secret-value."]
        )
        _turn_id, run_id, item_id = turns[0]
        provider = _ExtractionProvider(
            [_response(_candidate(item_id, content=f"The key is {secret}."))]
        )
        service = _service(
            memory_store=memory_store,
            runtime_store=store,
            provider=provider,
            protected_values=(secret,),
        )
        config = store.get_run_config(run_id)
        service.notify_run_terminal(config, "completed")
        await _drain(service)

        assert _active_memories(memory_store) == []
        assert memory_store.memory_extraction_cursor(thread) is None
        assert (
            service._write_candidate(
                config=config,
                candidate=MemoryCandidate(
                    kind="fact",
                    scope="global",
                    content=f"The key is {secret}.",
                    source_item_id=item_id,
                ),
            )
            == "dropped"
        )
        assert _active_memories(memory_store) == []
    finally:
        memory_store.close()
        store.close()


@pytest.mark.asyncio
async def test_workspace_scope_without_workspace_is_dropped(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    memory_store = SqliteMemoryStore(tmp_path / "memory.db")
    try:
        thread, _branch_id, turns = _thread_with_turns(store, ["A workspace fact."])
        _turn_id, run_id, item_id = turns[0]
        provider = _ExtractionProvider(
            [_response(_candidate(item_id, scope="workspace"))]
        )
        service = _service(
            memory_store=memory_store,
            runtime_store=store,
            provider=provider,
        )
        service.notify_run_terminal(store.get_run_config(run_id), "completed")
        await _drain(service)

        assert _active_memories(memory_store) == []
        cursor = memory_store.memory_extraction_cursor(thread)
        assert cursor is not None
        assert cursor.item_id == item_id
    finally:
        memory_store.close()
        store.close()


@pytest.mark.asyncio
async def test_close_cancels_inflight_refinement_and_stops_scheduling(
    tmp_path: Path,
) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    memory_store = SqliteMemoryStore(tmp_path / "memory.db")
    try:
        _thread_id, _branch_id, turns = _thread_with_turns(store, ["A preference."])
        _turn_id, run_id, _item_id = turns[0]
        provider = _BlockingProvider('{"memories":[]}')
        service = _service(
            memory_store=memory_store,
            runtime_store=store,
            provider=provider,
        )
        config = store.get_run_config(run_id)
        service.notify_run_terminal(config, "completed")
        await asyncio.wait_for(provider.started.wait(), timeout=1)
        assert len(service._tasks) == 1

        await service.close()
        assert service._tasks == {}
        assert provider.cancelled == 1

        service.notify_run_terminal(config, "completed")
        assert service._tasks == {}
        assert len(provider.requests) == 1
    finally:
        memory_store.close()
        store.close()


def test_memory_schema_version_and_cursor_drift(tmp_path: Path) -> None:
    assert MEMORY_SCHEMA_VERSION == 3
    path = tmp_path / "memory.db"
    store = SqliteMemoryStore(path)
    try:
        columns = {
            str(row[1])
            for row in store._connection.execute(
                "PRAGMA table_info(memory_extraction_cursors)"
            )
        }
        assert columns == {
            "thread_id",
            "last_turn_ordinal",
            "last_run_created_at",
            "last_run_id",
            "last_item_ordinal",
            "last_item_id",
            "attempted_at",
            "extracted_at",
        }
        assert store._connection.execute("PRAGMA user_version").fetchone()[0] == 3
    finally:
        store.close()

    connection = sqlite3.connect(path)
    connection.execute("DROP TABLE memory_extraction_cursors")
    connection.commit()
    connection.close()
    before = path.read_bytes()
    with pytest.raises(MemorySchemaIncompatibleError, match="incompatible"):
        SqliteMemoryStore(path)
    assert path.read_bytes() == before


def test_settled_reader_orders_filters_and_resumes(tmp_path: Path) -> None:
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _branch_id, turns = _thread_with_turns(
            store, ["First message.", "Second message."]
        )
        _turn_id, run_id, _item_id = turns[0]
        store.mark_run_running(run_id)
        assistant_id, _event = store.create_assistant_item(run_id)
        store.append_text_delta(assistant_id, "Partial answer.")

        items = store.read_settled_thread_items_after(
            thread_id=thread,
            after=None,
            limit=40,
        )
        assert [item.content for item in items] == ["First message.", "Second message."]
        assert [item.role for item in items] == ["user", "user"]

        store.complete_assistant_item(assistant_id)
        items = store.read_settled_thread_items_after(
            thread_id=thread,
            after=None,
            limit=40,
        )
        assert assistant_id in [item.item_id for item in items]
        assert items[0].content == "First message."
        assert items[1].item_id == assistant_id

        rest = store.read_settled_thread_items_after(
            thread_id=thread,
            after=items[0].to_cursor(),
            limit=1,
        )
        assert [item.item_id for item in rest] == [assistant_id]
    finally:
        store.close()
