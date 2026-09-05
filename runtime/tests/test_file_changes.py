from __future__ import annotations

import asyncio
import codecs
import json
import threading
from collections.abc import AsyncIterator, Callable
from dataclasses import replace
from pathlib import Path
from typing import Any

import pytest

from ikaros_runtime.agent import AgentLoop
from ikaros_runtime.cancellation import CancellationToken
from ikaros_runtime.domain import JournalEvent, WorkspaceSummary
from ikaros_runtime.file_changes import (
    MAX_CAPTURE_BYTES,
    MAX_CHANGE_EVENT_BYTES,
    FileChangeCapture,
    build_file_change,
    capture_before,
    capture_bytes,
    perform_captured_write,
    validate_file_change_record,
)
from ikaros_runtime.providers.base import (
    ProviderEvent,
    ProviderRequest,
    ResponseCompleted,
    TextDelta,
    ToolCallCompleted,
)
from ikaros_runtime.storage import SqliteRuntimeStore
from ikaros_runtime.tools import edit as edit_module
from ikaros_runtime.tools import write as write_module
from ikaros_runtime.tools.core import ToolCall, ToolExecutor, ToolRegistry, ToolResult
from ikaros_runtime.tools.edit import EditTool
from ikaros_runtime.tools.file_common import FileToolError, atomic_write_bytes
from ikaros_runtime.tools.policy import FullAccessPolicy
from ikaros_runtime.tools.read import ReadTool
from ikaros_runtime.tools.write import WriteTool

from .helpers import prepare_turn


class _FileProvider:
    def __init__(self, call: ToolCall) -> None:
        self.call = call
        self.requests: list[ProviderRequest] = []

    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]:
        cancellation.raise_if_cancelled()
        self.requests.append(request)
        if len(self.requests) == 1:
            yield ToolCallCompleted(self.call)
        else:
            yield TextDelta("Done.")
        yield ResponseCompleted()


@pytest.mark.parametrize(
    "before,after,expected_diff,added,deleted",
    [
        (
            b"old",
            b"new",
            "--- before\n+++ after\n@@ -1 +1 @@\n-old\n\\ No newline at end of file\n"
            "+new\n\\ No newline at end of file\n",
            1,
            1,
        ),
        (b"old\n", b"old\nnew\n", "--- before\n+++ after\n@@ -1 +1,2 @@\n old\n+new\n", 1, 0),
        (b"++literal\n--literal\n", b"++replacement\n--replacement\n", None, 2, 2),
        (b"same\n", codecs.BOM_UTF8 + b"same\r\n", "", 0, 0),
        (b"", b"", "", 0, 0),
    ],
)
def test_diff_tracks_exact_bytes_and_handles_no_final_newline(
    tmp_path: Path,
    before: bytes,
    after: bytes,
    expected_diff: str | None,
    added: int,
    deleted: int,
) -> None:
    path = tmp_path / "text.txt"
    path.write_bytes(before)
    capture = build_file_change(path, "write", capture_before(path), after)
    assert capture.reason is None
    assert (capture.additions, capture.deletions) == (added, deleted)
    if expected_diff is not None:
        assert capture.diff == expected_diff
    record = capture.to_record(
        thread_id="thread_test",
        tool_call_item_id="item_test",
        recorded_at="2026-09-05T00:00:00.000Z",
    )
    assert record["before"]["byteCount"] == len(before)
    assert record["after"]["byteCount"] == len(after)
    assert record["after"]["bom"] == after.startswith(codecs.BOM_UTF8)
    assert record["before"]["revision"] != record["after"]["revision"] or before == after


@pytest.mark.parametrize(
    "raw,reason",
    [
        (b"x" * (MAX_CAPTURE_BYTES + 1), "too_large"),
        (b"x\n" * 5_001, "too_large"),
        (b"\0binary", "binary_file"),
        (b"\xff\xfe", "unsupported_encoding"),
    ],
)
def test_unrepresentable_input_produces_unavailable_capture(raw: bytes, reason: str) -> None:
    captured = capture_bytes(raw)
    assert captured.reason == reason
    assert captured.raw is None and captured.text is None
    assert captured.metadata["byteCount"] == len(raw)


def test_capture_line_count_uses_only_cr_lf_and_preserves_format_metadata() -> None:
    assert capture_bytes("a\u2028b\n".encode()).metadata["lineCount"] == 1
    assert capture_bytes(b"one\rtwo\r").metadata["lineCount"] == 2
    assert capture_bytes(b"one\rtwo\r").metadata["newline"] == "mixed"
    assert capture_bytes(b"one\r\ntwo\n").metadata["newline"] == "mixed"
    assert capture_bytes(b"x\n" * 5_000).reason is None
    assert capture_bytes(b"x" * MAX_CAPTURE_BYTES).reason is None


@pytest.mark.parametrize("exists", [False, True])
def test_write_capture_rejects_external_change_before_atomic_replace(
    tmp_path: Path, exists: bool
) -> None:
    path = tmp_path / "race.txt"
    if exists:
        path.write_text("before")

    def raced_writer(target: Path, payload: bytes, **kwargs: Any) -> bool:
        target.write_text("external")
        return atomic_write_bytes(target, payload, **kwargs)

    with pytest.raises(FileToolError, match="changed since"):
        perform_captured_write(raced_writer, path, b"ours", operation="write")
    assert path.read_text() == "external"


@pytest.mark.asyncio
@pytest.mark.parametrize("operation", ["write", "edit"])
@pytest.mark.parametrize("cancellation_mode", ["none", "token", "task"])
async def test_agent_persists_captured_change_atomically_and_rebuilds_without_reading_disk(
    tmp_path: Path,
    monkeypatch: pytest.MonkeyPatch,
    operation: str,
    cancellation_mode: str,
) -> None:
    path = tmp_path / "target.txt"
    path.write_bytes(codecs.BOM_UTF8 + b"before\r\n")
    arguments = (
        {"filePath": str(path), "content": "after\n"}
        if operation == "write"
        else {"filePath": str(path), "oldString": "before", "newString": "after"}
    )
    call = ToolCall("file-once", operation, arguments)
    provider = _FileProvider(call)
    executor = ToolExecutor(ToolRegistry((WriteTool(), EditTool(), ReadTool())), FullAccessPolicy())
    cancellation = CancellationToken()
    events: list[JournalEvent] = []
    entered = threading.Event()
    release = threading.Event()
    cancel_after_write = cancellation_mode != "none"

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    if cancel_after_write:
        module = write_module if operation == "write" else edit_module
        original: Callable[..., bool] = module.atomic_write_bytes

        def cancelled_writer(target: Path, payload: bytes, **kwargs: Any) -> bool:
            if cancellation_mode == "task":
                entered.set()
                if not release.wait(timeout=5):
                    raise TimeoutError("test did not release the file worker")
            result = original(target, payload, **kwargs)
            if cancellation_mode == "token":
                cancellation.cancel()
            return result

        monkeypatch.setattr(module, "atomic_write_bytes", cancelled_writer)

    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread(
            "File history", workspace=WorkspaceSummary("workspace_test", "Test", str(tmp_path))
        )
        prepared = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="Update target",
            provider_id="custom",
            model_id="model",
            tools=executor.definitions,
        )
        task = asyncio.create_task(
            AgentLoop(store, {"custom": provider}, publish, tool_executor=executor).run(
                prepared.run_id,
                cancellation,
            )
        )
        if cancellation_mode == "task":
            assert await asyncio.to_thread(entered.wait, 2)
            task.cancel()
            release.set()
        await task
        assert store.run_status(prepared.run_id) == (
            "cancelled" if cancel_after_write else "completed"
        )
        assert path.read_bytes() == codecs.BOM_UTF8 + b"after\r\n"
        changes = [event for event in events if event.type == "file.change_recorded"]
        assert len(changes) == 1
        event = changes[0]
        assert event.item_id is not None and event.schema_version == 6
        record = store.get_file_change(thread.id, event.item_id)
        assert record["status"] == "recorded"
        assert "-before\n+after\n" in record["diff"]
        assert record["before"]["bom"] is record["after"]["bom"] is True
        assert record["before"]["newline"] == record["after"]["newline"] == "crlf"
        assert store.resolve_file_tool_path(thread.id, event.item_id) == str(path)
        outcome = next(
            event
            for event in events
            if event.type == "item.completed" and event.payload["item"]["kind"] == "tool_result"
        )
        assert "file_change" not in json.dumps(outcome.to_wire())
        assert "diff" not in json.dumps(outcome.to_wire())
        assert outcome.seq == event.seq - 1
        if cancel_after_write:
            assert outcome.payload["item"]["data"]["result"]["ok"] is True
        else:
            assert all(
                "--- before" not in message.content
                for request in provider.requests
                for message in request.messages
            )
        path.unlink()
        store.rebuild_projections()
        assert store.get_file_change(thread.id, event.item_id) == record
        store.check_state()
        other, _ = store.create_thread("Other")
        with pytest.raises(LookupError):
            store.get_file_change(other.id, event.item_id)
        assert store.journal_contains_protected_values(("before",)) is True
        assert store.journal_contains_protected_values(("utf-8",)) is False
    finally:
        store.close()


@pytest.mark.asyncio
async def test_private_capture_protects_existing_secret_even_if_tool_output_does_not_contain_it(
    tmp_path: Path,
) -> None:
    path = tmp_path / "private.txt"
    path.write_text("secret-value-9380\n")
    provider = _FileProvider(
        ToolCall("replace-secret", "write", {"filePath": str(path), "content": "public\n"})
    )
    executor = ToolExecutor(ToolRegistry((WriteTool(),)), FullAccessPolicy())
    store = SqliteRuntimeStore(tmp_path / "state.db")
    events: list[JournalEvent] = []

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    try:
        thread, _ = store.create_thread("Protected capture")
        prepared = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="Replace",
            provider_id="custom",
            model_id="model",
            tools=executor.definitions,
        )
        await AgentLoop(
            store,
            {"custom": provider},
            publish,
            tool_executor=executor,
            protected_values=lambda: ("secret-value-9380",),
        ).run(prepared.run_id, CancellationToken())
        assert store.run_status(prepared.run_id) == "completed"
        change = next(event for event in events if event.type == "file.change_recorded")
        assert change.payload["status"] == "unavailable"
        assert change.payload["reason"] == "protected_content"
        assert change.payload["before"] is change.payload["after"] is None
        assert "secret-value-9380" not in json.dumps([event.to_wire() for event in events])
        assert not store.journal_contains_protected_values(("secret-value-9380",))
        store.rebuild_projections()
    finally:
        store.close()


def _running_call(
    store: SqliteRuntimeStore, path: Path, *, operation: str = "write"
) -> tuple[str, str, str]:
    thread, _ = store.create_thread("Stored capture")
    prepared = prepare_turn(
        store,
        thread_id=thread.id,
        branch_id=thread.default_branch_id,
        content="Change",
        provider_id="custom",
        model_id="model",
    )
    store.mark_run_running(prepared.run_id)
    store.prepare_model_step(prepared.run_id, step_ordinal=1)
    completed = store.complete_provider_step(
        prepared.run_id,
        step_ordinal=1,
        assistant_item_id=None,
        tool_calls=(
            ToolCall("stored-write", operation, {"filePath": str(path), "content": "new"}),
        ),
        reasoning_content=None,
        usage=None,
        response_model_id=None,
        request_id=None,
    )
    return thread.id, prepared.run_id, completed.tool_call_item_ids[0]


def test_total_event_byte_cap_downgrades_whole_diff(tmp_path: Path) -> None:
    path = tmp_path / "large.txt"
    path.write_bytes(b"a" * 150_000)
    capture = build_file_change(path, "write", capture_before(path), b"b" * 150_000)
    assert capture.reason is None
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread_id, _, item_id = _running_call(store, path)
        events = store.complete_tool_call(
            item_id,
            status="completed",
            result={"path": str(path)},
            result_content="{}",
            file_change=capture,
        )
        assert len(events) == 3
        assert (
            len(
                json.dumps(events[-1].to_wire(), ensure_ascii=False, separators=(",", ":")).encode()
            )
            <= MAX_CHANGE_EVENT_BYTES
        )
        record = store.get_file_change(thread_id, item_id)
        assert record["status"] == "unavailable" and record["reason"] == "too_large"
        assert "diff" not in record
        store.rebuild_projections()
    finally:
        store.close()


def test_file_capture_inserts_roll_back_with_tool_completion(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    path = tmp_path / "transaction.txt"
    capture = build_file_change(path, "write", capture_before(path), b"new")
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread_id, _, item_id = _running_call(store, path)
        sequence = store.latest_sequence()
        store._connection.execute(
            "CREATE TRIGGER reject_capture BEFORE INSERT ON file_changes "
            "BEGIN SELECT RAISE(ABORT, 'capture failed'); END"
        )
        with pytest.raises(Exception, match="capture failed"):
            store.complete_tool_call(
                item_id,
                status="completed",
                result={"path": str(path)},
                result_content="{}",
                file_change=capture,
            )
        assert store.latest_sequence() == sequence
        assert (
            store._connection.execute(
                "SELECT status FROM items WHERE id = ?", (item_id,)
            ).fetchone()[0]
            == "running"
        )
        assert store.get_file_change(thread_id, item_id)["reason"] == "not_recorded"
    finally:
        store.close()


def test_old_or_crashed_mutation_has_no_reconstructed_diff(tmp_path: Path) -> None:
    path = tmp_path / "crashed.txt"
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread_id, _, item_id = _running_call(store, path)
        path.write_text("side effect before process died")
        store.close()
        store = SqliteRuntimeStore(tmp_path / "state.db")
        store.recover_incomplete_runs()
        assert store.get_file_change(thread_id, item_id)["reason"] == "not_recorded"
        assert store._connection.execute("SELECT COUNT(*) FROM file_changes").fetchone()[0] == 0
        store.rebuild_projections()
        assert store.get_file_change(thread_id, item_id)["reason"] == "not_recorded"
    finally:
        store.close()


@pytest.mark.parametrize("tamper", ["scope", "path", "operation", "timestamp", "schema"])
def test_rebuild_rejects_file_change_binding_tampering(tmp_path: Path, tamper: str) -> None:
    path = tmp_path / "bound.txt"
    capture = build_file_change(path, "write", capture_before(path), b"new")
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        _, _, item_id = _running_call(store, path)
        event = store.complete_tool_call(
            item_id,
            status="completed",
            result={"path": str(path)},
            result_content="{}",
            file_change=capture,
        )[-1]
        payload = dict(event.payload)
        if tamper == "schema":
            store._connection.execute(
                "UPDATE events SET schema_version = 5 WHERE seq = ?", (event.seq,)
            )
        else:
            key, value = {
                "scope": ("threadId", "thread_other"),
                "path": ("path", str(tmp_path / "other")),
                "operation": ("operation", "edit"),
                "timestamp": ("recordedAt", "wrong"),
            }[tamper]
            payload[key] = value
            store._connection.execute(
                "UPDATE events SET payload_json = ? WHERE seq = ?", (json.dumps(payload), event.seq)
            )
        store._connection.commit()
        with pytest.raises(RuntimeError):
            store.rebuild_projections()
    finally:
        store.close()


def test_protected_capture_removes_sensitive_path(tmp_path: Path) -> None:
    path = tmp_path / "credential-secret.txt"
    capture = build_file_change(path, "write", capture_before(path), b"new")
    protected = capture.protected(("credential-secret",))
    assert protected.path is None and protected.reason == "protected_content"
    assert protected.before is protected.after is None


def test_record_validator_rejects_extra_model_body() -> None:
    capture = FileChangeCapture(None, "write", None, None, reason="result_unknown")
    record = capture.to_record(
        thread_id="thread_a", tool_call_item_id="item_a", recorded_at="2026-09-05T00:00:00.000Z"
    )
    record["modelBody"] = "not metadata"
    with pytest.raises(ValueError):
        validate_file_change_record(record)


@pytest.mark.asyncio
async def test_redacted_tool_output_keeps_safe_capture_path_for_rebuild(tmp_path: Path) -> None:
    class SecretOutputWrite(WriteTool):
        async def execute(
            self,
            call: ToolCall,
            *,
            cancellation: CancellationToken,
            default_cwd: str | None = None,
        ) -> ToolResult:
            result = await super().execute(call, cancellation=cancellation, default_cwd=default_cwd)
            return replace(result, output="secret-output-9380")

    workspace = tmp_path / "workspace"
    workspace.mkdir()
    executor = ToolExecutor(ToolRegistry((SecretOutputWrite(),)), FullAccessPolicy())
    provider = _FileProvider(
        ToolCall("redacted-write", "write", {"filePath": "new.txt", "content": "new"})
    )
    events: list[JournalEvent] = []

    async def publish(event: JournalEvent) -> None:
        events.append(event)

    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        thread, _ = store.create_thread(
            "Safe path", workspace=WorkspaceSummary("workspace_test", "Test", str(workspace))
        )
        prepared = prepare_turn(
            store,
            thread_id=thread.id,
            branch_id=thread.default_branch_id,
            content="Write",
            provider_id="custom",
            model_id="model",
            tools=executor.definitions,
        )
        await AgentLoop(
            store,
            {"custom": provider},
            publish,
            tool_executor=executor,
            protected_values=lambda: ("secret-output-9380",),
        ).run(prepared.run_id, CancellationToken())
        change = next(event for event in events if event.type == "file.change_recorded")
        assert change.item_id is not None
        record = store.get_file_change(thread.id, change.item_id)
        assert record["status"] == "recorded"
        assert not store.journal_contains_protected_values(("secret-output-9380",))
        workspace.rename(tmp_path / "moved-workspace")
        store.rebuild_projections()
        assert store.get_file_change(thread.id, change.item_id) == record
    finally:
        store.close()
