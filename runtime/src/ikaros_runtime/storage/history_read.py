"""Bounded, read-only recovery of persisted conversation Items.

This is deliberately separate from context selection: a model may need to recover
one omitted Item, while the next provider request must continue using its frozen
``ContextRevision``.  The reader never appends journal Events or changes the
active revision.
"""

from __future__ import annotations

import sqlite3
from typing import Any

from ..json_codec import dumps as json_dumps
from ..json_codec import loads as json_loads

MAX_HISTORY_READ_ITEMS = 41
MAX_HISTORY_READ_CHARS = 24_000


def read_history_slice(
    connection: sqlite3.Connection,
    *,
    thread_id: str,
    item_id: str,
    before: int = 10,
    after: int = 10,
    max_chars: int = MAX_HISTORY_READ_CHARS,
) -> dict[str, Any]:
    """Return a bounded chronological slice around *item_id* in one thread.

    The thread and branch are resolved from the target Item, so callers cannot
    use a valid Item ID to read another thread.  Only settled/context-eligible
    records are exposed; an in-flight Tool Result is never presented as a final
    fact.  This function is read-only and intentionally returns a JSON-shaped
    value suitable for a Runtime ToolResult.
    """

    _validate_limit("before", before, 0, MAX_HISTORY_READ_ITEMS)
    _validate_limit("after", after, 0, MAX_HISTORY_READ_ITEMS)
    _validate_limit("max_chars", max_chars, 256, MAX_HISTORY_READ_CHARS)
    target = connection.execute(
        """
        SELECT i.id, i.turn_id, i.run_id, i.kind, i.role, i.status, i.content,
               i.data_json, i.ordinal AS item_ordinal, t.branch_id, t.ordinal AS turn_ordinal,
               r.created_at AS run_created_at
        FROM items i
        JOIN turns t ON t.id = i.turn_id
        JOIN runs r ON r.id = i.run_id
        WHERE i.id = ? AND t.thread_id = ?
        """,
        (item_id, thread_id),
    ).fetchone()
    if target is None:
        raise LookupError("history Item was not found in this thread")

    rows = connection.execute(
        """
        SELECT i.id, i.turn_id, i.run_id, i.kind, i.role, i.status, i.content,
               i.data_json, i.ordinal AS item_ordinal, t.ordinal AS turn_ordinal,
               r.created_at AS run_created_at
        FROM items i
        JOIN turns t ON t.id = i.turn_id
        JOIN runs r ON r.id = i.run_id
        WHERE t.thread_id = ? AND t.branch_id = ?
          AND i.status IN ('completed', 'failed', 'cancelled')
          AND (
              (i.kind = 'message' AND i.role IN ('user', 'assistant'))
              OR i.kind IN ('tool_call', 'tool_result')
          )
        ORDER BY t.ordinal ASC, r.created_at ASC, r.id ASC, i.ordinal ASC, i.id ASC
        LIMIT ?
        """,
        (thread_id, target["branch_id"], MAX_HISTORY_READ_ITEMS * 32),
    ).fetchall()
    records = [_record(row) for row in rows]
    target_index = next((index for index, row in enumerate(records) if row["id"] == item_id), None)
    if target_index is None:
        # A target can be active while the surrounding query intentionally only
        # exposes settled records.  Returning it would violate that invariant.
        raise LookupError("history Item is not settled")
    selected = records[
        max(0, target_index - before) : min(len(records), target_index + after + 1)
    ]
    payload: dict[str, Any] = {
        "threadId": thread_id,
        "targetItemId": item_id,
        "items": selected,
        "truncated": False,
    }
    encoded = json_dumps(payload, ensure_ascii=False, separators=(",", ":"))
    if len(encoded) <= max_chars:
        return payload
    # Keep the target and trim neighbouring records from the farthest edges;
    # never return a partial JSON value or silently claim the complete slice.
    while len(selected) > 1:
        if selected[0]["id"] != item_id:
            selected.pop(0)
        elif selected[-1]["id"] != item_id:
            selected.pop()
        else:
            break
        payload["items"] = selected
        payload["truncated"] = True
        if len(json_dumps(payload, ensure_ascii=False, separators=(",", ":"))) <= max_chars:
            return payload
    # A single large record is still bounded, preserving metadata and a clear
    # marker rather than returning an over-budget Tool result.
    target_record = dict(selected[0])
    target_record["content"] = str(target_record["content"])[: max_chars // 2] + " …[truncated]"
    target_record["data"] = {"truncated": True}
    payload["items"] = [target_record]
    payload["truncated"] = True
    return payload


def _record(row: sqlite3.Row) -> dict[str, Any]:
    return {
        "id": str(row["id"]),
        "turnId": str(row["turn_id"]),
        "runId": str(row["run_id"]),
        "kind": str(row["kind"]),
        "role": str(row["role"]) if row["role"] is not None else None,
        "status": str(row["status"]),
        "content": str(row["content"]),
        "data": json_loads(str(row["data_json"])),
    }


def _validate_limit(name: str, value: int, minimum: int, maximum: int) -> None:
    if not isinstance(value, int) or isinstance(value, bool) or not minimum <= value <= maximum:
        raise ValueError(f"{name} must be an integer between {minimum} and {maximum}")


__all__ = ["MAX_HISTORY_READ_CHARS", "MAX_HISTORY_READ_ITEMS", "read_history_slice"]
