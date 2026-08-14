"""Transaction-neutral helpers for Runtime query projections."""

from __future__ import annotations

import sqlite3
from collections.abc import Sequence
from typing import Any, cast

from ..domain import ContextItem, PreparedTurn, RunDescriptor, WorkspaceSummary
from ..json_codec import dumps as json_dumps
from ..json_codec import loads as json_loads
from ..security import (
    contains_protected_value,
    json_contains_protected_value,
    json_values_contain_protected_value,
)


def has_active_runs(connection: sqlite3.Connection) -> bool:
    return (
        connection.execute(
            "SELECT 1 FROM runs WHERE status IN ('queued', 'running') LIMIT 1"
        ).fetchone()
        is not None
    )


def contains_protected_projection_values(
    connection: sqlite3.Connection,
    protected_values: Sequence[str],
) -> bool:
    values = tuple(dict.fromkeys(value for value in protected_values if value))
    if not values:
        return False
    thread_rows = connection.execute(
        "SELECT title, client_request_id, workspace_json FROM threads"
    ).fetchall()
    if any(
        json_contains_protected_value(
            (
                row["title"],
                row["client_request_id"],
                (workspace.id, workspace.name, workspace.root_uri)
                if (workspace := workspace_from_json(row["workspace_json"])) is not None
                else None,
            ),
            values,
        )
        for row in thread_rows
    ):
        return True
    for row in connection.execute(
        "SELECT provider_id, model_id, client_request_id FROM runs"
    ).fetchall():
        references = [row["client_request_id"]]
        if row["provider_id"] != "scripted":
            references.extend((row["provider_id"], row["model_id"]))
        if json_contains_protected_value(references, values):
            return True
    for row in connection.execute("SELECT kind, content, data_json FROM items").fetchall():
        kind = str(row["kind"])
        content = str(row["content"])
        data = json_loads(row["data_json"])
        if kind == "message":
            if contains_protected_value(content, values) or json_values_contain_protected_value(
                data, values
            ):
                return True
        elif kind == "tool_call":
            dynamic = (
                data.get("callId"),
                data.get("toolName"),
                data.get("arguments"),
                data.get("reasoningContent"),
            )
            if json_contains_protected_value(dynamic, values):
                return True
        elif kind == "tool_result":
            if json_contains_protected_value(
                (data.get("callId"), data.get("toolName")), values
            ) or json_values_contain_protected_value(data.get("result"), values):
                return True
        elif contains_protected_value(content, values) or json_contains_protected_value(
            data, values
        ):
            return True
    return False


def find_turn_by_client_request_id(
    connection: sqlite3.Connection,
    *,
    thread_id: str,
    branch_id: str,
    content: str,
    provider_id: str,
    model_id: str,
    client_request_id: str,
) -> PreparedTurn | None:
    existing = connection.execute(
        """
        SELECT r.id AS run_id, r.turn_id, r.provider_id, r.model_id,
               t.thread_id, t.branch_id, i.content
        FROM runs r
        JOIN turns t ON t.id = r.turn_id
        JOIN items i ON i.run_id = r.id AND i.ordinal = 1
        WHERE r.client_request_id = ?
        """,
        (client_request_id,),
    ).fetchone()
    if existing is None:
        return None
    expected = (thread_id, branch_id, provider_id, model_id, content)
    actual = tuple(
        str(existing[name])
        for name in ("thread_id", "branch_id", "provider_id", "model_id", "content")
    )
    if actual != expected:
        raise LookupError("clientRequestId was already used with different turn.start parameters")
    return PreparedTurn(
        turn_id=str(existing["turn_id"]),
        run_id=str(existing["run_id"]),
        thread_id=str(existing["thread_id"]),
        branch_id=str(existing["branch_id"]),
        initial_events=(),
        newly_created=False,
    )


def get_run(connection: sqlite3.Connection, run_id: str) -> RunDescriptor:
    row = connection.execute(
        """
        SELECT r.id, r.turn_id, r.provider_id, r.model_id, r.execution_policy,
               t.thread_id, t.branch_id, th.workspace_json
        FROM runs r
        JOIN turns t ON t.id = r.turn_id
        JOIN threads th ON th.id = t.thread_id
        WHERE r.id = ?
        """,
        (run_id,),
    ).fetchone()
    if row is None:
        raise LookupError("run was not found")
    return RunDescriptor(
        id=row["id"],
        turn_id=row["turn_id"],
        thread_id=row["thread_id"],
        branch_id=row["branch_id"],
        provider_id=row["provider_id"],
        model_id=row["model_id"],
        execution_policy=row["execution_policy"],
        workspace=workspace_from_json(row["workspace_json"]),
    )


def run_status(connection: sqlite3.Connection, run_id: str) -> str:
    row = connection.execute("SELECT status FROM runs WHERE id = ?", (run_id,)).fetchone()
    if row is None:
        raise LookupError("run was not found")
    return str(row["status"])


def context_messages(
    connection: sqlite3.Connection,
    branch_id: str,
    *,
    through_turn_id: str,
) -> list[tuple[str, str]]:
    boundary = connection.execute(
        "SELECT ordinal FROM turns WHERE id = ? AND branch_id = ?",
        (through_turn_id, branch_id),
    ).fetchone()
    if boundary is None:
        raise LookupError("context boundary turn was not found")
    rows = connection.execute(
        """
        SELECT i.role, i.content
        FROM items i JOIN turns t ON t.id = i.turn_id
        WHERE t.branch_id = ? AND t.ordinal <= ? AND i.kind = 'message'
          AND i.status = 'completed' AND i.role IN ('user', 'assistant')
        ORDER BY t.ordinal, i.ordinal
        """,
        (branch_id, int(boundary["ordinal"])),
    ).fetchall()
    return [(row["role"], row["content"]) for row in rows]


def context_items(
    connection: sqlite3.Connection,
    branch_id: str,
    *,
    through_turn_id: str,
) -> list[ContextItem]:
    boundary = connection.execute(
        "SELECT ordinal FROM turns WHERE id = ? AND branch_id = ?",
        (through_turn_id, branch_id),
    ).fetchone()
    if boundary is None:
        raise LookupError("context boundary turn was not found")
    rows = connection.execute(
        """
        SELECT i.kind, i.role, i.content, i.data_json
        FROM items i JOIN turns t ON t.id = i.turn_id JOIN runs r ON r.id = i.run_id
        WHERE t.branch_id = ? AND t.ordinal <= ? AND (
          (i.kind = 'message' AND i.status = 'completed' AND i.role IN ('user', 'assistant'))
          OR (i.kind IN ('tool_call', 'tool_result') AND i.status IN ('completed', 'failed')
              AND (t.id = ? OR r.status = 'completed'))
        ) ORDER BY t.ordinal, i.ordinal
        """,
        (branch_id, int(boundary["ordinal"]), through_turn_id),
    ).fetchall()
    return [
        ContextItem(
            kind=str(row["kind"]),
            role=str(row["role"]) if row["role"] is not None else None,
            content=str(row["content"]),
            data=json_loads(row["data_json"]),
        )
        for row in rows
    ]


def workspace_to_json(workspace: WorkspaceSummary | None) -> str | None:
    if workspace is None:
        return None
    return json_dumps(workspace.to_wire(), separators=(",", ":"), ensure_ascii=False)


def workspace_from_json(value: object) -> WorkspaceSummary | None:
    if value is None:
        return None
    if not isinstance(value, str):
        raise RuntimeError("thread workspace projection is invalid")
    try:
        return workspace_from_wire(json_loads(value))
    except (TypeError, ValueError):
        raise RuntimeError("thread workspace projection is invalid") from None


def workspace_from_wire(value: object) -> WorkspaceSummary | None:
    if value is None:
        return None
    if not isinstance(value, dict) or set(value) != {"id", "name", "rootUri"}:
        raise ValueError("thread workspace projection is invalid")
    workspace_id = value["id"]
    name = value["name"]
    root_uri = value["rootUri"]
    if (
        not isinstance(workspace_id, str)
        or not isinstance(name, str)
        or (root_uri is not None and not isinstance(root_uri, str))
    ):
        raise ValueError("thread workspace projection is invalid")
    return WorkspaceSummary(workspace_id, name, root_uri)


def apply_event(
    connection: sqlite3.Connection,
    event_type: str,
    payload: dict[str, Any],
    *,
    timestamp: str,
) -> None:
    if event_type == "thread.created":
        thread = payload["thread"]
        branch = payload["branch"]
        connection.execute(
            """
            INSERT INTO threads(
                id, title, default_branch_id, workspace_json,
                created_at, updated_at, client_request_id
            ) VALUES (?, ?, ?, ?, ?, ?, ?)
            """,
            (
                thread["id"],
                thread["title"],
                thread["defaultBranchId"],
                workspace_to_json(workspace_from_wire(thread.get("workspace"))),
                thread["createdAt"],
                thread["updatedAt"],
                payload.get("clientRequestId"),
            ),
        )
        connection.execute(
            "INSERT INTO branches(id, thread_id, created_at, is_default) VALUES (?, ?, ?, ?)",
            (
                branch["id"],
                branch["threadId"],
                branch["createdAt"],
                int(branch["isDefault"]),
            ),
        )
    elif event_type == "item.completed" and "turn" in payload:
        turn = payload["turn"]
        run = payload["run"]
        item = payload["item"]
        connection.execute(
            """
            INSERT INTO turns(id, thread_id, branch_id, ordinal, status, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            """,
            (
                turn["id"],
                turn["threadId"],
                turn["branchId"],
                turn["ordinal"],
                turn["status"],
                turn["createdAt"],
                turn["updatedAt"],
            ),
        )
        connection.execute(
            """
            INSERT INTO runs(
                id, turn_id, provider_id, model_id, execution_policy, status,
                created_at, settled_at, client_request_id
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                run["id"],
                run["turnId"],
                run["providerId"],
                run["modelId"],
                run.get("executionPolicy", "full_access"),
                run["status"],
                run["createdAt"],
                run["settledAt"],
                run.get("clientRequestId"),
            ),
        )
        insert_item(connection, item)
        connection.execute(
            "UPDATE threads SET updated_at = ? WHERE id = ?",
            (turn["updatedAt"], turn["threadId"]),
        )
    elif event_type == "run.state_changed":
        connection.execute(
            "UPDATE runs SET status = ? WHERE id = ?",
            (payload["status"], payload.get("runId")),
        )
        connection.execute(
            "UPDATE turns SET status = ?, updated_at = ? WHERE id = ?",
            (payload["status"], timestamp, payload.get("turnId")),
        )
    elif event_type == "item.started":
        insert_item(connection, payload["item"])
    elif event_type == "item.delta":
        connection.execute(
            "UPDATE items SET content = content || ?, updated_at = ? WHERE id = ?",
            (payload["delta"], timestamp, payload.get("itemId")),
        )
    elif event_type == "item.completed":
        item = payload["item"]
        existing = connection.execute(
            "SELECT 1 FROM items WHERE id = ?",
            (item["id"],),
        ).fetchone()
        if existing is None:
            insert_item(connection, item)
        else:
            connection.execute(
                """
                UPDATE items
                SET status = ?, content = ?, updated_at = ?, data_json = ?
                WHERE id = ?
                """,
                (
                    item["status"],
                    item["content"],
                    item["updatedAt"],
                    json_dumps(item.get("data", {}), separators=(",", ":"), ensure_ascii=False),
                    item["id"],
                ),
            )
    elif event_type == "run.settled":
        connection.execute(
            "UPDATE runs SET status = ?, settled_at = ? WHERE id = ?",
            (payload["status"], payload.get("settledAt"), payload.get("runId")),
        )
        connection.execute(
            "UPDATE turns SET status = ?, updated_at = ? WHERE id = ?",
            (payload["status"], payload.get("settledAt"), payload.get("turnId")),
        )


def insert_item(connection: sqlite3.Connection, item: dict[str, Any]) -> None:
    connection.execute(
        """
        INSERT INTO items(
            id, turn_id, run_id, ordinal, kind, role, status, content,
            created_at, updated_at, data_json
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            item["id"],
            item["turnId"],
            item["runId"],
            item["ordinal"],
            item["kind"],
            item["role"],
            item["status"],
            item["content"],
            item["createdAt"],
            item["updatedAt"],
            json_dumps(item.get("data", {}), separators=(",", ":"), ensure_ascii=False),
        ),
    )


def item_location(connection: sqlite3.Connection, item_id: str) -> sqlite3.Row:
    row = connection.execute(
        """
        SELECT i.turn_id, i.run_id, t.thread_id, t.branch_id
        FROM items i JOIN turns t ON t.id = i.turn_id
        WHERE i.id = ?
        """,
        (item_id,),
    ).fetchone()
    if row is None:
        raise LookupError("item was not found")
    return cast(sqlite3.Row, row)


def item_row(connection: sqlite3.Connection, item_id: str) -> sqlite3.Row:
    row = connection.execute(
        """
        SELECT i.id, i.turn_id, i.run_id, i.ordinal, i.kind, i.role, i.status,
               i.content, i.created_at, i.updated_at, i.data_json,
               t.thread_id, t.branch_id
        FROM items i JOIN turns t ON t.id = i.turn_id
        WHERE i.id = ?
        """,
        (item_id,),
    ).fetchone()
    if row is None:
        raise LookupError("item was not found")
    return cast(sqlite3.Row, row)


def next_item_ordinal(connection: sqlite3.Connection, run_id: str) -> int:
    return int(
        connection.execute(
            "SELECT COALESCE(MAX(ordinal), 0) + 1 FROM items WHERE run_id = ?",
            (run_id,),
        ).fetchone()[0]
    )
