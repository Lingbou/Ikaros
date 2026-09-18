"""Current process facts; replay restores observations and never starts commands."""

from __future__ import annotations

import sqlite3
from typing import Any

from ..domain import JsonObject
from ..json_codec import loads as json_loads
from ..run_input import canonical_json


def project_process(connection: sqlite3.Connection, value: JsonObject) -> None:
    required = {
        "processId",
        "threadId",
        "runId",
        "stepOrdinal",
        "itemId",
        "command",
        "cwd",
        "state",
        "exitCode",
        "pid",
        "startedAt",
        "finishedAt",
        "stdout",
        "stderr",
        "output",
        "truncated",
        "errorCode",
    }
    if set(value) != required:
        raise ValueError("process record fields are invalid")
    for key in ("processId", "threadId", "runId", "itemId", "command", "cwd", "startedAt"):
        if not isinstance(value[key], str) or not value[key]:
            raise ValueError("process record identity is invalid")
    if value["state"] not in {"running", "exited", "terminated", "unknown"}:
        raise ValueError("process state is invalid")
    if type(value["stepOrdinal"]) is not int or value["stepOrdinal"] < 1:
        raise ValueError("process Step is invalid")
    if type(value["truncated"]) is not bool:
        raise ValueError("process truncation is invalid")
    output_limits = {
        "stdout": 256 * 1024 + 128,
        "stderr": 256 * 1024 + 128,
        "output": 1024 * 1024 + 128,
    }
    for key, limit in output_limits.items():
        if not isinstance(value[key], str) or len(value[key].encode("utf-8")) > limit:
            raise ValueError("process output is invalid")
    for key in ("exitCode", "pid"):
        if value[key] is not None and type(value[key]) is not int:
            raise ValueError("process result is invalid")
    for key in ("finishedAt", "errorCode"):
        if value[key] is not None and not isinstance(value[key], str):
            raise ValueError("process result is invalid")
    owner = connection.execute(
        "SELECT i.run_id, i.kind, i.data_json, t.thread_id FROM items i "
        "JOIN turns t ON t.id = i.turn_id WHERE i.id = ?",
        (value["itemId"],),
    ).fetchone()
    if (
        owner is None
        or owner["run_id"] != value["runId"]
        or owner["thread_id"] != value["threadId"]
        or owner["kind"] != "tool_call"
        or json_loads(owner["data_json"]).get("toolName") != "process_start"
    ):
        raise ValueError("process owner is invalid")
    _require_completed_owner_step(connection, value)
    previous = connection.execute(
        "SELECT record_json FROM process_sessions WHERE process_id = ?", (value["processId"],)
    ).fetchone()
    if previous is not None:
        before: dict[str, Any] = json_loads(previous["record_json"])
        for key in ("threadId", "runId", "stepOrdinal", "itemId", "command", "cwd", "startedAt"):
            if before[key] != value[key]:
                raise ValueError("process ownership or invocation changed")
        terminal = before["state"] in {"exited", "terminated"} or (
            before["state"] == "unknown" and before["errorCode"] != "start_pending"
        )
        if terminal and before != value:
            raise ValueError("terminal process record changed")
        if before["state"] == "running" and (
            value["state"] == "unknown" and value["errorCode"] == "start_pending"
        ):
            raise ValueError("running process cannot return to start pending")
    connection.execute(
        "INSERT INTO process_sessions(process_id,run_id,item_id,record_json) VALUES (?,?,?,?) "
        "ON CONFLICT(process_id) DO UPDATE SET record_json=excluded.record_json",
        (value["processId"], value["runId"], value["itemId"], canonical_json(value)),
    )


def _require_completed_owner_step(connection: sqlite3.Connection, value: JsonObject) -> None:
    # Tool Call Items are appended before their originating response finishes, in
    # that response's transaction. The next finished response in the same Run is
    # therefore their source model call, even when later calls have also completed.
    response = connection.execute(
        "SELECT finished.payload_json FROM events started JOIN events finished "
        "ON finished.run_id = started.run_id AND finished.seq > started.seq "
        "WHERE started.run_id = ? AND started.item_id = ? "
        "AND started.event_type = 'item.started' "
        "AND finished.event_type = 'model.response_finished' "
        "ORDER BY finished.seq LIMIT 1",
        (value["runId"], value["itemId"]),
    ).fetchone()
    if response is None:
        raise ValueError("process owner has no completed model Step")
    payload = json_loads(response["payload_json"])
    if (
        not isinstance(payload, dict)
        or payload.get("stepOrdinal") != value["stepOrdinal"]
        or payload.get("outcome") != "completed"
    ):
        raise ValueError("process Step does not match its owning Tool Call")
    # During replay the complete Journal is visible; require the source response
    # to have actually been projected already, not merely exist later in the log.
    step = connection.execute(
        "SELECT outcome FROM model_calls WHERE run_id = ? AND step_ordinal = ?",
        (value["runId"], value["stepOrdinal"]),
    ).fetchone()
    if step is None or step["outcome"] != "completed":
        raise ValueError("process owner model Step is not completed")
