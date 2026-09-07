from __future__ import annotations

import argparse
import asyncio
import json
import tempfile
import uuid
from collections.abc import AsyncIterator, Callable
from datetime import UTC, datetime, timedelta
from pathlib import Path
from types import SimpleNamespace
from typing import Any, cast
from unittest.mock import patch

import ikaros_runtime.agent.loop as agent_loop_module
import ikaros_runtime.memory.store as memory_store_module
import ikaros_runtime.storage.store as store_module
from ikaros_runtime import __version__
from ikaros_runtime.agent.loop import AgentLoop
from ikaros_runtime.cancellation import CancellationToken
from ikaros_runtime.domain import JournalEvent, ModelUsage
from ikaros_runtime.file_changes import FileChangeCapture, FileRevisionCapture, build_file_change
from ikaros_runtime.file_preview import preview_text_file
from ikaros_runtime.identity import load_identity_core
from ikaros_runtime.memory import MemoryRetrieverV1, SqliteMemoryStore
from ikaros_runtime.protocol.spec import (
    EVENT_NOTIFICATION_METHOD,
    INITIALIZE_METHOD,
    JSONRPC_VERSION,
    PROTOCOL_VERSION,
    SERVER_NAME,
    initialize_capabilities,
)
from ikaros_runtime.providers.base import (
    ProviderEvent,
    ProviderRequest,
    ResponseCompleted,
)
from ikaros_runtime.providers.scripted import ScriptedProvider
from ikaros_runtime.run_input import (
    ProviderExecutionSnapshot,
    RunConfigTemplate,
)
from ikaros_runtime.services.memories import MemoryService
from ikaros_runtime.storage import SqliteRuntimeStore
from ikaros_runtime.tools import ProcessManager
from ikaros_runtime.tools import process as process_tools
from ikaros_runtime.tools import process_manager as process_manager_module
from ikaros_runtime.tools.core import (
    ToolCall,
    ToolExecutor,
    ToolRegistry,
    ToolResult,
)
from ikaros_runtime.tools.policy import FullAccessPolicy
from ikaros_runtime.tools.process import (
    ProcessReadTool,
    ProcessStartTool,
    ProcessStopTool,
    ProcessWaitTool,
)
from ikaros_runtime.tools.process_platform import _SpawnedProcess
from ikaros_runtime.tools.write import WriteTool

_RUNTIME_ROOT = Path(__file__).resolve().parents[1]
_GOLDEN_TRACE_PATH = _RUNTIME_ROOT / "protocol" / "golden-trace.json"

type GoldenMessage = dict[str, Any]


class _DeterministicIds:
    def __init__(self) -> None:
        self._next = 1

    def __call__(self) -> uuid.UUID:
        value = uuid.UUID(int=self._next)
        self._next += 1
        return value


class _DeterministicUtc:
    def __init__(self) -> None:
        self._next = 0
        self._base = datetime(2026, 8, 15, 12, 0, tzinfo=UTC)

    def __call__(self) -> str:
        value = self._base + timedelta(milliseconds=self._next)
        self._next += 1
        return value.isoformat(timespec="milliseconds").replace("+00:00", "Z")


class _DeterministicMonotonic:
    def __init__(self) -> None:
        self._next = 0

    def __call__(self) -> float:
        value = self._next / 1_000
        self._next += 1
        return value


async def _golden_spawn(command: str, cwd: str | None) -> _SpawnedProcess:
    if command != "golden" or cwd != "C:/golden-workspace":
        raise AssertionError("unexpected golden command or workspace")
    stdout, stderr = asyncio.StreamReader(), asyncio.StreamReader()
    stdout.feed_data(b"golden-output\n")
    stdout.feed_eof()
    stderr.feed_eof()
    process = cast(
        asyncio.subprocess.Process,
        SimpleNamespace(
            pid=4242,
            returncode=0,
            stdout=stdout,
            stderr=stderr,
        ),
    )
    return _SpawnedProcess(process)


async def _golden_terminate(spawned: _SpawnedProcess) -> None:
    return None


class _GoldenScriptedProvider:
    """Run the production ScriptedProvider and add deterministic response metadata."""

    id = ScriptedProvider.id
    model_id = ScriptedProvider.model_id

    def __init__(self) -> None:
        self._provider = ScriptedProvider()
        self._response_ordinal = 0

    async def stream(
        self,
        request: ProviderRequest,
        *,
        cancellation: CancellationToken,
    ) -> AsyncIterator[ProviderEvent]:
        async for event in self._provider.stream(request, cancellation=cancellation):
            if not isinstance(event, ResponseCompleted):
                yield event
                continue
            self._response_ordinal += 1
            yield ResponseCompleted(
                usage=ModelUsage(
                    input_tokens=10 + self._response_ordinal,
                    output_tokens=4 + self._response_ordinal,
                    total_tokens=14 + (2 * self._response_ordinal),
                ),
                model_id=self.model_id,
                request_id=f"request_golden_{self._response_ordinal}",
            )


async def _discard_event(_event: JournalEvent) -> None:
    return None


def _provider_snapshot() -> ProviderExecutionSnapshot:
    return ProviderExecutionSnapshot(
        provider_id=ScriptedProvider.id,
        origin="scripted",
        base_url=None,
        model_id=ScriptedProvider.model_id,
        supports_tools=True,
    )


def _snapshot_resolver(
    snapshot: ProviderExecutionSnapshot,
) -> Callable[[str, str], ProviderExecutionSnapshot]:
    def resolve(provider_id: str, model_id: str) -> ProviderExecutionSnapshot:
        if (provider_id, model_id) != (snapshot.provider_id, snapshot.model_id):
            raise ValueError("Golden Run requested an unexpected Provider or Model")
        return snapshot

    return resolve


def _notification_name(event: JournalEvent, occurrence: int) -> str:
    payload = event.payload
    if event.type == "thread.created":
        return "thread-created"
    if event.type == "thread.renamed":
        return "thread-renamed"
    if event.type == "thread.archived":
        return "thread-archived"
    if event.type == "thread.unarchived":
        return "thread-unarchived"
    if event.type == "run.state_changed":
        return f"run-{payload['status']}"
    if event.type == "model.input_prepared":
        return (
            "model-input-prepared"
            if payload["stepOrdinal"] == 1
            else f"model-input-prepared-step-{payload['stepOrdinal']}"
        )
    if event.type == "model.response_finished":
        return (
            "model-response-finished"
            if payload["stepOrdinal"] == 1
            else f"model-response-finished-step-{payload['stepOrdinal']}"
        )
    if event.type == "process.recorded":
        return f"process-recorded-{occurrence}"
    if event.type == "run.settled":
        return "run-settled"
    if event.type == "item.delta":
        return "assistant-item-delta" if occurrence == 1 else f"assistant-item-delta-{occurrence}"
    if event.type in {"item.started", "item.completed"}:
        item = cast(dict[str, Any], payload["item"])
        if "turn" in payload:
            return "initial-user-item-completed"
        action = "started" if event.type == "item.started" else "completed"
        if item["kind"] == "tool_call":
            return f"tool-call-{action}"
        if item["kind"] == "tool_result":
            return f"tool-result-{action}"
        return f"assistant-item-{action}"
    raise AssertionError(f"Golden trace has an unsupported event type: {event.type}")


def _notification_messages(events: list[JournalEvent]) -> list[GoldenMessage]:
    occurrences: dict[str, int] = {}
    messages: list[GoldenMessage] = []
    names: set[str] = set()
    for event in events:
        occurrences[event.type] = occurrences.get(event.type, 0) + 1
        name = _notification_name(event, occurrences[event.type])
        if name in names:
            name = f"{name}-{occurrences[event.type]}"
        names.add(name)
        messages.append(
            {
                "name": name,
                "kind": "notification",
                "envelope": {
                    "jsonrpc": JSONRPC_VERSION,
                    "method": EVENT_NOTIFICATION_METHOD,
                    "params": event.to_wire(),
                },
            }
        )
    return messages


def _response_message(
    *,
    name: str,
    method: str,
    request_id: int,
    request_params: dict[str, Any],
    result: dict[str, Any],
) -> GoldenMessage:
    return {
        "name": name,
        "kind": "response",
        "method": method,
        "requestParams": request_params,
        "envelope": {
            "jsonrpc": JSONRPC_VERSION,
            "id": request_id,
            "result": result,
        },
    }


async def build_production_messages(database_path: Path) -> list[GoldenMessage]:
    """Generate deterministic Session responses and notifications through production code."""

    ids = _DeterministicIds()
    utc_now = _DeterministicUtc()
    monotonic = _DeterministicMonotonic()
    snapshot = _provider_snapshot()
    identity_core = load_identity_core()
    provider = _GoldenScriptedProvider()

    with (
        patch.object(process_manager_module, "_spawn_process", new=_golden_spawn),
        patch.object(process_manager_module, "_terminate_process_tree", new=_golden_terminate),
        patch.object(process_manager_module, "_resume_spawned_process", new=lambda spawned: None),
        patch.object(process_manager_module, "_now", new=utc_now),
        patch.object(process_tools, "_resolve_cwd", return_value=Path("C:/golden-workspace")),
        patch.object(uuid, "uuid4", new=ids),
        patch.object(store_module, "utc_now", new=utc_now),
        patch.object(memory_store_module, "utc_now", new=utc_now),
        patch.object(store_module, "local_activity_date", return_value="2026-08-15"),
        patch.object(agent_loop_module, "monotonic", new=monotonic),
    ):
        store = SqliteRuntimeStore(database_path)

        def record_process(fact: dict[str, Any]) -> None:
            store.record_process(fact)

        manager = ProcessManager(record=record_process)
        executor = ToolExecutor(
            ToolRegistry(
                (
                    ProcessStartTool(manager),
                    ProcessReadTool(manager),
                    ProcessWaitTool(manager),
                    ProcessStopTool(manager),
                )
            ),
            FullAccessPolicy(),
        )
        memory_store = SqliteMemoryStore(database_path.with_name("memory.db"))
        try:
            thread, _created = store.create_thread("Golden trace draft")
            renamed, _renamed = store.rename_thread(thread.id, "Golden trace")
            store.set_thread_archived(thread.id, archived=True)
            store.set_thread_archived(thread.id, archived=False)
            frame = RunConfigTemplate.create(
                provider=snapshot,
                execution_policy=executor.policy_name,
                skills=(),
                tools=executor.definitions,
                identity_core=identity_core,
            )
            prepared = store.prepare_turn(
                thread_id=thread.id,
                branch_id=thread.default_branch_id,
                content="/process.run golden",
                run_config_template=frame,
            )
            memory_service = MemoryService(memory_store, lambda _value: None, store)
            memory_create_params = {
                "kind": "preference",
                "scope": {"type": "global", "key": None},
                "content": "For golden commands, the user prefers concise technical explanations.",
                "clientRequestId": "golden-memory-create",
                "source": {
                    "type": "session_item",
                    "itemId": prepared.initial_events[0].item_id,
                },
            }
            memory_created = memory_service.create(memory_create_params)
            memory_id = cast(str, memory_created["memoryId"])
            loop = AgentLoop(
                store,
                {provider.id: provider},
                _discard_event,
                executor,
                process_manager=manager,
                provider_snapshot_resolver=_snapshot_resolver(snapshot),
                identity_core=identity_core,
                memory_retriever=MemoryRetrieverV1(memory_store),
            )
            await loop.run(prepared.run_id, CancellationToken())
            if store.run_status(prepared.run_id) != "completed":
                raise AssertionError("Golden production Run did not complete")
            events, latest_seq = store.replay_events(0, 1_000)
            if latest_seq != len(events):
                raise AssertionError("Golden Journal replay is incomplete")
            if renamed.title != "Golden trace":
                raise AssertionError("Golden Thread lifecycle did not complete")
            thread_params = {"limit": 25}
            turn_params = {
                "threadId": thread.id,
                "branchId": thread.default_branch_id,
                "limit": 25,
            }
            memory_active_list_params = {
                "limit": 25,
                "scope": {"type": "global", "key": None},
                "state": "active",
            }
            memory_get_params = {"memoryId": memory_id}
            memory_active_list = memory_service.list(memory_active_list_params)
            memory_source_record = memory_service.get(memory_get_params)
            memory_correct_params = {
                "memoryId": memory_id,
                "expectedRevision": 1,
                "content": "The user prefers precise, concise technical explanations.",
                "clientRequestId": "golden-memory-correct",
            }
            memory_corrected = memory_service.correct(memory_correct_params)
            memory_corrected_record = memory_service.get(memory_get_params)
            memory_forget_params = {
                "memoryId": memory_id,
                "expectedRevision": 2,
                "clientRequestId": "golden-memory-forget",
            }
            memory_forgotten = memory_service.forget(memory_forget_params)
            memory_forgotten_list_params = {
                "limit": 25,
                "scope": {"type": "global", "key": None},
                "state": "forgotten",
            }
            memory_forgotten_list = memory_service.list(memory_forgotten_list_params)
            memory_tombstone = memory_service.get(memory_get_params)
            session_messages = [
                _response_message(
                    name="thread-list-page",
                    method="thread.list",
                    request_id=2,
                    request_params=thread_params,
                    result=store.list_thread_page(cursor=None, limit=25).to_wire(),
                ),
                _response_message(
                    name="turn-list-page",
                    method="turn.list",
                    request_id=3,
                    request_params=turn_params,
                    result=store.list_turn_page(
                        thread_id=thread.id,
                        branch_id=thread.default_branch_id,
                        cursor=None,
                        limit=25,
                    ).to_wire(),
                ),
                _response_message(
                    name="memory-created",
                    method="memory.create",
                    request_id=4,
                    request_params=memory_create_params,
                    result=memory_created,
                ),
                _response_message(
                    name="memory-list-page",
                    method="memory.list",
                    request_id=5,
                    request_params=memory_active_list_params,
                    result=memory_active_list,
                ),
                _response_message(
                    name="memory-record",
                    method="memory.get",
                    request_id=6,
                    request_params=memory_get_params,
                    result=memory_source_record,
                ),
                _response_message(
                    name="memory-corrected",
                    method="memory.correct",
                    request_id=7,
                    request_params=memory_correct_params,
                    result=memory_corrected,
                ),
                _response_message(
                    name="memory-corrected-record",
                    method="memory.get",
                    request_id=8,
                    request_params=memory_get_params,
                    result=memory_corrected_record,
                ),
                _response_message(
                    name="memory-forgotten",
                    method="memory.forget",
                    request_id=9,
                    request_params=memory_forget_params,
                    result=memory_forgotten,
                ),
                _response_message(
                    name="memory-forgotten-list-page",
                    method="memory.list",
                    request_id=10,
                    request_params=memory_forgotten_list_params,
                    result=memory_forgotten_list,
                ),
                _response_message(
                    name="memory-tombstone",
                    method="memory.get",
                    request_id=11,
                    request_params=memory_get_params,
                    result=memory_tombstone,
                ),
                *_notification_messages(events),
            ]
            file_path = "C:/golden-workspace/hello.txt"
            file_frame = RunConfigTemplate.create(
                provider=snapshot,
                execution_policy="full_access",
                skills=(),
                tools=(WriteTool.definition,),
                identity_core=identity_core,
            )
            file_turn = store.prepare_turn(
                thread_id=thread.id,
                branch_id=thread.default_branch_id,
                content="Write hello.txt",
                run_config_template=file_frame,
            )
            store.mark_run_running(file_turn.run_id)
            store.prepare_model_step(file_turn.run_id, step_ordinal=1)
            call = ToolCall("golden-write", "write", {"filePath": file_path, "content": "你好\n"})
            completed = store.complete_provider_step(
                file_turn.run_id,
                step_ordinal=1,
                assistant_item_id=None,
                tool_calls=(call,),
                reasoning_content=None,
                usage=None,
                response_model_id=None,
                request_id=None,
            )
            item_id = completed.tool_call_item_ids[0]
            before = FileRevisionCapture(
                {
                    "exists": False,
                    "byteCount": 0,
                    "revision": None,
                    "encoding": None,
                    "bom": None,
                    "newline": None,
                    "lineCount": 0,
                },
                text="",
            )
            capture = build_file_change(Path(file_path), "write", before, "你好\n".encode())
            # Path is a fixture identity, independent of the host path syntax.
            capture = FileChangeCapture(
                file_path,
                capture.operation,
                capture.before,
                capture.after,
                capture.diff,
                capture.additions,
                capture.deletions,
                capture.reason,
            )
            result = ToolResult(
                call.id,
                call.name,
                True,
                "Created file successfully",
                {
                    "path": file_path,
                    "created": True,
                    "bytesWritten": 7,
                    "verified": True,
                    "bom": False,
                    "newline": "lf",
                    "truncated": False,
                },
            )
            store.complete_tool_call(
                item_id,
                status="completed",
                result=result.to_wire(),
                result_content=result.to_model_content(),
                file_change=capture,
            )
            store.terminalize_run(file_turn.run_id, "completed")
            file_events, _ = store.replay_events(latest_seq, 1000)
            file_messages = []
            for index, event in enumerate(file_events):
                file_messages.append(
                    {
                        "name": "file-change-recorded"
                        if event.type == "file.change_recorded"
                        else f"file-flow-event-{index}",
                        "kind": "notification",
                        "envelope": {
                            "jsonrpc": JSONRPC_VERSION,
                            "method": EVENT_NOTIFICATION_METHOD,
                            "params": event.to_wire(),
                        },
                    }
                )
            preview_path = database_path.with_name("preview.txt")
            preview_path.write_bytes("你好\n".encode())
            preview = preview_text_file(thread_id=thread.id, path=preview_path)
            preview["path"] = file_path
            preview["revision"] = "a" * 64  # Opaque, host-specific metadata fingerprint.
            return [
                *session_messages[: -len(events)],
                _response_message(
                    name="file-change",
                    method="file.change.get",
                    request_id=12,
                    request_params={"threadId": thread.id, "toolCallItemId": item_id},
                    result=store.get_file_change(thread.id, item_id),
                ),
                _response_message(
                    name="file-preview",
                    method="file.preview",
                    request_id=13,
                    request_params={"threadId": thread.id, "path": file_path},
                    result=preview,
                ),
                *session_messages[-len(events) :],
                *file_messages,
            ]
        finally:
            await manager.close()
            memory_store.close()
            store.close()


async def build_production_notification_messages(database_path: Path) -> list[GoldenMessage]:
    messages = await build_production_messages(database_path)
    return [message for message in messages if message["kind"] == "notification"]


def _load_trace() -> dict[str, Any]:
    value = json.loads(_GOLDEN_TRACE_PATH.read_text(encoding="utf-8"))
    if not isinstance(value, dict) or not isinstance(value.get("messages"), list):
        raise ValueError("Golden trace has an invalid structure")
    return cast(dict[str, Any], value)


async def _expected_trace(database_path: Path) -> dict[str, Any]:
    committed = _load_trace()
    production_messages = await build_production_messages(database_path)
    production_responses = {
        message["name"]: message for message in production_messages if message["kind"] == "response"
    }
    responses = [
        production_responses.get(message["name"], message)
        for message in cast(list[GoldenMessage], committed["messages"])
        if message.get("kind") == "response"
    ]
    committed_response_names = {message["name"] for message in responses}
    responses.extend(
        message
        for message in production_messages
        if message["kind"] == "response" and message["name"] not in committed_response_names
    )
    for message in responses:
        if message.get("method") != INITIALIZE_METHOD:
            continue
        message["requestParams"] = {
            "protocolVersion": PROTOCOL_VERSION,
            "client": {"name": "golden-client", "version": "1.0.0"},
        }
        message["envelope"] = {
            "jsonrpc": JSONRPC_VERSION,
            "id": 1,
            "result": {
                "protocolVersion": PROTOCOL_VERSION,
                "server": {"name": SERVER_NAME, "version": __version__},
                "capabilities": initialize_capabilities(),
            },
        }
    return {
        "fixtureVersion": committed.get("fixtureVersion"),
        "messages": [
            *responses,
            *(message for message in production_messages if message["kind"] == "notification"),
        ],
    }


def _render(value: object) -> str:
    return json.dumps(value, ensure_ascii=False, indent=2) + "\n"


async def _run(*, check: bool) -> int:
    with tempfile.TemporaryDirectory(prefix="ikaros-golden-trace-") as temporary:
        expected = await _expected_trace(Path(temporary) / "state.db")
    expected_text = _render(expected)
    if check:
        if _GOLDEN_TRACE_PATH.read_text(encoding="utf-8") != expected_text:
            print("protocol Golden Session trace is stale")
            return 1
        return 0
    _GOLDEN_TRACE_PATH.write_text(expected_text, encoding="utf-8", newline="\n")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Generate the production-backed Ikaros Runtime Golden Session trace"
    )
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check", action="store_true", help="fail when notifications drift")
    mode.add_argument(
        "--write",
        action="store_true",
        help="replace production-backed Session response and notification fixtures",
    )
    args = parser.parse_args()
    return asyncio.run(_run(check=bool(args.check)))


if __name__ == "__main__":
    raise SystemExit(main())


__all__ = ["build_production_messages", "build_production_notification_messages"]
