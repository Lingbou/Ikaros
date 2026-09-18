from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path
from typing import Any, cast

import pytest

from ikaros_runtime.protocol.spec import (
    EVENT_NOTIFICATION_METHOD,
    INITIALIZE_METHOD,
    JOURNAL_EVENT_SCHEMA_VERSION,
    JOURNAL_EVENT_TYPE_SET,
    JSONRPC_VERSION,
    MEMORY_OPERATION_ERROR_CODE,
    MEMORY_OPERATION_ERROR_MESSAGE,
    MEMORY_OPERATION_REASON_CODES,
    PROTOCOL_VERSION,
    PROVIDER_TOOL_IDS,
    RPC_METHOD_SET,
    RPC_METHODS,
    SERVER_NAME,
    protocol_manifest,
)
from ikaros_runtime.storage import SqliteRuntimeStore

from .golden_trace import build_production_messages

_RUNTIME_ROOT = Path(__file__).resolve().parents[1]
_MANIFEST_PATH = _RUNTIME_ROOT / "protocol" / "runtime-protocol.json"
_GOLDEN_TRACE_PATH = _RUNTIME_ROOT / "protocol" / "golden-trace.json"


def _load_object(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text(encoding="utf-8"))
    assert isinstance(value, dict)
    return cast(dict[str, Any], value)


def test_committed_manifest_is_the_deterministic_python_spec() -> None:
    assert _load_object(_MANIFEST_PATH) == protocol_manifest()
    completed = subprocess.run(
        [sys.executable, "-m", "ikaros_runtime.protocol.generate", "--check"],
        cwd=_RUNTIME_ROOT,
        check=False,
        capture_output=True,
        text=True,
    )
    assert completed.returncode == 0, completed.stdout + completed.stderr


def test_golden_trace_envelopes_match_the_python_protocol_spec() -> None:
    trace = _load_object(_GOLDEN_TRACE_PATH)
    assert trace["fixtureVersion"] == 1
    messages = trace["messages"]
    assert isinstance(messages, list)
    observed_methods: set[str] = set()
    observed_event_types: set[str] = set()

    for raw_message in messages:
        assert isinstance(raw_message, dict)
        message = cast(dict[str, Any], raw_message)
        envelope = message["envelope"]
        assert isinstance(envelope, dict)
        assert envelope["jsonrpc"] == JSONRPC_VERSION
        if message["kind"] == "response":
            method = message["method"]
            assert isinstance(method, str)
            assert method == INITIALIZE_METHOD or method in RPC_METHOD_SET
            assert isinstance(message["requestParams"], dict)
            assert isinstance(envelope["id"], int) and not isinstance(envelope["id"], bool)
            result = envelope["result"]
            assert isinstance(result, dict)
            observed_methods.add(method)
            if method == INITIALIZE_METHOD:
                assert result["protocolVersion"] == PROTOCOL_VERSION
                assert result["server"]["name"] == SERVER_NAME
                assert set(result) == {"protocolVersion", "server"}
            elif method == "thread.list":
                assert isinstance(result["threads"], list)
                assert isinstance(result["snapshotSeq"], int)
            elif method == "turn.list":
                assert isinstance(result["turns"], list)
                assert isinstance(result["snapshotSeq"], int)
            elif method == "memory.create":
                assert isinstance(result["memoryId"], str)
                assert result["resultingRevision"] == 1
            elif method == "memory.list":
                assert isinstance(result["memories"], list)
                assert result["hasMore"] is False
            elif method == "memory.get":
                assert isinstance(result["memory"], dict)
        else:
            assert message["kind"] == "notification"
            assert envelope["method"] == EVENT_NOTIFICATION_METHOD
            event = envelope["params"]
            assert isinstance(event, dict)
            assert event["schemaVersion"] == JOURNAL_EVENT_SCHEMA_VERSION
            assert event["type"] in JOURNAL_EVENT_TYPE_SET
            assert isinstance(event["payload"], dict)
            for wire_key in ("turnId", "runId", "itemId"):
                scope_value = event[wire_key]
                if scope_value is None:
                    assert wire_key not in event["payload"]
                else:
                    assert event["payload"][wire_key] == scope_value
            observed_event_types.add(event["type"])

    assert observed_methods == {
        INITIALIZE_METHOD,
        "memory.create",
        "memory.correct",
        "memory.forget",
        "memory.get",
        "memory.list",
        "skill.list",
        "skill.set_enabled",
        "thread.list",
        "turn.list",
        "file.preview",
        "file.change.get",
    }
    assert observed_event_types <= JOURNAL_EVENT_TYPE_SET
    assert {"thread.created", "run.settled", "run.steered"} <= observed_event_types
    assert len(RPC_METHODS) == 32


@pytest.mark.asyncio
async def test_committed_golden_session_trace_matches_the_production_agent_trace(
    tmp_path: Path,
) -> None:
    trace = _load_object(_GOLDEN_TRACE_PATH)
    committed = [
        message
        for message in cast(list[dict[str, Any]], trace["messages"])
        if message["kind"] == "notification"
        or message["name"]
        in {
            "memory-created",
            "memory-corrected",
            "memory-corrected-record",
            "memory-forgotten",
            "memory-forgotten-list-page",
            "memory-list-page",
            "memory-record",
            "memory-tombstone",
            "thread-list-page",
            "turn-list-page",
            "file-change",
            "file-preview",
        }
    ]

    generated = await build_production_messages(tmp_path / "state.db")

    assert committed == generated


def test_golden_trace_notifications_rebuild_the_production_projections(
    tmp_path: Path,
) -> None:
    trace = _load_object(_GOLDEN_TRACE_PATH)
    store = SqliteRuntimeStore(tmp_path / "state.db")
    try:
        with store._connection:
            for raw_message in trace["messages"]:
                message = cast(dict[str, Any], raw_message)
                if message["kind"] != "notification":
                    continue
                envelope = cast(dict[str, Any], message["envelope"])
                event = cast(dict[str, Any], envelope["params"])
                payload = dict(cast(dict[str, Any], event["payload"]))
                for scope_key in ("turnId", "runId", "itemId"):
                    payload.pop(scope_key, None)
                store._append_event(
                    event_type=cast(str, event["type"]),
                    thread_id=cast(str | None, event["threadId"]),
                    branch_id=cast(str | None, event["branchId"]),
                    turn_id=cast(str | None, event["turnId"]),
                    run_id=cast(str | None, event["runId"]),
                    item_id=cast(str | None, event["itemId"]),
                    timestamp=cast(str, event["timestamp"]),
                    payload=payload,
                )

        before, latest = store.replay_events(0, 1000)
        store.rebuild_projections()
        after, rebuilt_latest = store.replay_events(0, 1000)
        assert after == before
        assert rebuilt_latest == latest == len(before)
        initial = next(
            event for event in after if event.type == "item.completed" and "turn" in event.payload
        )
        assert initial.run_id is not None
        assert store.run_status(initial.run_id) == "completed"
    finally:
        store.close()


def test_protocol_registries_are_unique_and_do_not_use_display_tool_ids() -> None:
    assert PROTOCOL_VERSION == 6
    assert JOURNAL_EVENT_SCHEMA_VERSION == 9
    assert protocol_manifest()["errors"] == {
        "memoryOperation": {
            "code": MEMORY_OPERATION_ERROR_CODE,
            "message": MEMORY_OPERATION_ERROR_MESSAGE,
            "reasonCodes": list(MEMORY_OPERATION_REASON_CODES),
        }
    }
    assert len(MEMORY_OPERATION_REASON_CODES) == len(set(MEMORY_OPERATION_REASON_CODES))
    assert len(RPC_METHODS) == len(RPC_METHOD_SET)
    assert {"memory.correct", "memory.forget"} <= RPC_METHOD_SET
    assert {"process_start", "process_read", "process_wait", "process_stop"} <= set(
        PROVIDER_TOOL_IDS
    )
    assert "process_run" not in PROVIDER_TOOL_IDS
    assert "process.run" not in PROVIDER_TOOL_IDS
