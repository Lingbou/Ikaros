"""Immutable file-change queries bound to persisted Tool Calls."""

from __future__ import annotations

import sqlite3
from pathlib import Path, PurePosixPath, PureWindowsPath

from ..domain import JsonObject
from ..file_changes import validate_file_change_record
from ..json_codec import loads as json_loads
from ..tools.file_common import FileToolError, validate_file_path


def file_tool_call(
    connection: sqlite3.Connection,
    thread_id: str,
    tool_call_item_id: str,
    *,
    allowed: frozenset[str] = frozenset({"read", "write", "edit"}),
) -> tuple[sqlite3.Row, JsonObject]:
    row = connection.execute(
        """SELECT i.*, t.thread_id, t.branch_id, ri.submission_frame_json
        FROM items i JOIN turns t ON t.id = i.turn_id
        JOIN run_inputs ri ON ri.run_id = i.run_id
        WHERE i.id = ? AND t.thread_id = ? AND i.kind = 'tool_call'""",
        (tool_call_item_id, thread_id),
    ).fetchone()
    if row is None:
        raise LookupError("file Tool Call was not found in this Thread")
    data = json_loads(row["data_json"])
    if not isinstance(data, dict) or data.get("toolName") not in allowed:
        raise ValueError("Tool Call is not a supported file operation")
    return row, data


def resolve_file_tool_path(
    connection: sqlite3.Connection,
    thread_id: str,
    tool_call_item_id: str,
) -> str:
    row, data = file_tool_call(connection, thread_id, tool_call_item_id)
    # The tool result freezes its resolved absolute path, even if the workspace
    # moves or the original relative path later resolves through another symlink.
    for result_row in connection.execute(
        "SELECT data_json FROM items WHERE run_id = ? AND kind = 'tool_result' ORDER BY ordinal",
        (row["run_id"],),
    ):
        result_data = json_loads(result_row["data_json"])
        if result_data.get("toolCallItemId") != tool_call_item_id:
            continue
        result = result_data.get("result")
        path = result.get("path") if isinstance(result, dict) else None
        if isinstance(path, str) and (
            PurePosixPath(path).is_absolute() or PureWindowsPath(path).is_absolute()
        ):
            return path
    arguments = data.get("arguments")
    path = arguments.get("filePath") if isinstance(arguments, dict) else None
    frame = json_loads(row["submission_frame_json"])
    workspace = frame.get("workspace")
    cwd = workspace.get("rootUri") if isinstance(workspace, dict) else None
    if not isinstance(path, str) or (not Path(path).is_absolute() and not isinstance(cwd, str)):
        raise ValueError("file Tool Call has no stable path binding")
    return str(validate_file_path(path, default_cwd=cwd))


def get_file_change(
    connection: sqlite3.Connection,
    thread_id: str,
    tool_call_item_id: str,
) -> JsonObject:
    _, data = file_tool_call(
        connection,
        thread_id,
        tool_call_item_id,
        allowed=frozenset({"write", "edit"}),
    )
    row = connection.execute(
        "SELECT record_json FROM file_changes WHERE tool_call_item_id = ?",
        (tool_call_item_id,),
    ).fetchone()
    if row is not None:
        return validate_file_change_record(json_loads(row["record_json"]))
    try:
        path: str | None = resolve_file_tool_path(connection, thread_id, tool_call_item_id)
    except (ValueError, OSError, FileToolError):
        path = None
    return validate_file_change_record(
        {
            "threadId": thread_id,
            "toolCallItemId": tool_call_item_id,
            "path": path,
            "operation": data["toolName"],
            "recordedAt": None,
            "before": None,
            "after": None,
            "status": "unavailable",
            "reason": "not_recorded",
        },
        allow_not_recorded=True,
    )
