from __future__ import annotations

import argparse
import asyncio
import json
import tempfile
import uuid
from collections.abc import AsyncIterator, Callable
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Any, cast
from unittest.mock import patch

import ikaros_runtime.agent.loop as agent_loop_module
import ikaros_runtime.memory.store as memory_store_module
import ikaros_runtime.storage.store as store_module
from ikaros_runtime.agent.loop import AgentLoop
from ikaros_runtime.cancellation import CancellationToken
from ikaros_runtime.domain import JournalEvent, ModelUsage
from ikaros_runtime.identity import load_identity_core
from ikaros_runtime.memory import SqliteMemoryStore
from ikaros_runtime.protocol.spec import EVENT_NOTIFICATION_METHOD, JSONRPC_VERSION
from ikaros_runtime.providers.base import (
    ProviderEvent,
    ProviderRequest,
    ResponseCompleted,
)
from ikaros_runtime.providers.scripted import ScriptedProvider
from ikaros_runtime.run_input import (
    ProviderExecutionSnapshotV1,
    SubmissionFrameTemplateV1,
)
from ikaros_runtime.services.memories import MemoryService
from ikaros_runtime.storage import SqliteRuntimeStore
from ikaros_runtime.tools.core import (
    ToolCall,
    ToolExecutor,
    ToolRegistry,
    ToolResult,
)
from ikaros_runtime.tools.policy import FullAccessPolicy
from ikaros_runtime.tools.process import ProcessRunTool

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


class _GoldenProcessTool:
    """A deterministic executor for the real built-in process_run definition."""

    definition = ProcessRunTool.definition

    async def execute(
        self,
        call: ToolCall,
        *,
        cancellation: CancellationToken,
        default_cwd: str | None = None,
    ) -> ToolResult:
        cancellation.raise_if_cancelled()
        if call.name != "process_run" or call.arguments != {"command": "golden"}:
            raise AssertionError("Golden ScriptedProvider emitted an unexpected Tool call")
        if default_cwd is not None:
            raise AssertionError("Golden Tool unexpectedly received a Thread workspace")
        return ToolResult(
            tool_call_id=call.id,
            tool_name=call.name,
            ok=True,
            output="golden-output",
            details={
                "stdout": "golden-output\n",
                "stderr": "",
                "cwd": "C:/golden-workspace",
                "exitCode": 0,
                "durationMs": 3,
                "timedOut": False,
                "truncated": False,
            },
        )


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


def _provider_snapshot() -> ProviderExecutionSnapshotV1:
    return ProviderExecutionSnapshotV1(
        provider_id=ScriptedProvider.id,
        origin="scripted",
        base_url=None,
        model_id=ScriptedProvider.model_id,
        supports_tools=True,
    )


def _snapshot_resolver(
    snapshot: ProviderExecutionSnapshotV1,
) -> Callable[[str, str], ProviderExecutionSnapshotV1]:
    def resolve(provider_id: str, model_id: str) -> ProviderExecutionSnapshotV1:
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
            raise AssertionError(f"Golden notification name is duplicated: {name}")
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
    tool = _GoldenProcessTool()
    executor = ToolExecutor(ToolRegistry((tool,)), FullAccessPolicy())

    with (
        patch.object(uuid, "uuid4", new=ids),
        patch.object(store_module, "utc_now", new=utc_now),
        patch.object(memory_store_module, "utc_now", new=utc_now),
        patch.object(store_module, "local_activity_date", return_value="2026-08-15"),
        patch.object(agent_loop_module, "monotonic", new=monotonic),
    ):
        store = SqliteRuntimeStore(database_path)
        memory_store = SqliteMemoryStore(database_path.with_name("memory.db"))
        try:
            thread, _created = store.create_thread("Golden trace draft")
            renamed, _renamed = store.rename_thread(thread.id, "Golden trace")
            store.set_thread_archived(thread.id, archived=True)
            store.set_thread_archived(thread.id, archived=False)
            frame = SubmissionFrameTemplateV1.create(
                provider=snapshot,
                execution_policy=executor.policy_name,
                skills=(),
                tools=executor.definitions,
                identity_core=identity_core,
                max_steps=2,
            )
            prepared = store.prepare_turn(
                thread_id=thread.id,
                branch_id=thread.default_branch_id,
                content="/process.run golden",
                frame_template=frame,
            )
            loop = AgentLoop(
                store,
                {provider.id: provider},
                _discard_event,
                executor,
                provider_snapshot_resolver=_snapshot_resolver(snapshot),
                identity_core=identity_core,
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
            memory_service = MemoryService(memory_store, lambda _value: None)
            memory_create_params = {
                "kind": "preference",
                "scope": {"type": "global", "key": None},
                "content": "The user prefers concise technical explanations.",
                "clientRequestId": "golden-memory-create",
            }
            memory_created = memory_service.create(memory_create_params)
            memory_id = cast(str, memory_created["memoryId"])
            memory_list_params = {
                "limit": 25,
                "scope": {"type": "global", "key": None},
                "state": "active",
            }
            memory_get_params = {"memoryId": memory_id}
            return [
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
                    request_params=memory_list_params,
                    result=memory_service.list(memory_list_params),
                ),
                _response_message(
                    name="memory-record",
                    method="memory.get",
                    request_id=6,
                    request_params=memory_get_params,
                    result=memory_service.get(memory_get_params),
                ),
                *_notification_messages(events),
            ]
        finally:
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
        message["name"]: message
        for message in production_messages
        if message["kind"] == "response"
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
        if message["kind"] == "response"
        and message["name"] not in committed_response_names
    )
    return {
        "fixtureVersion": committed.get("fixtureVersion"),
        "messages": [
            *responses,
            *(
                message
                for message in production_messages
                if message["kind"] == "notification"
            ),
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
