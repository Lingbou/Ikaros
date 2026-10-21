"""Read-only, cross-Thread recovery of settled conversation Items.

The slice itself comes from :func:`read_history_slice`, which owns the settled
Item predicate and the bounded ``max_chars`` truncation.  This module only adds
Thread existence checks, the optional newest-Item anchor, and Thread metadata so
the model can discover and inspect another conversation without a second
history implementation.
"""

from __future__ import annotations

import sqlite3
from typing import Any

from ..tools.thread_read import (
    MAX_THREAD_READ_CHARS,
    ItemNotFoundError,
    ItemNotSettledError,
    ThreadNotFoundError,
)
from .history_read import read_history_slice

_SETTLED_ITEM_PREDICATE = """
          i.status IN ('completed', 'failed', 'cancelled')
          AND (
              (i.kind = 'message' AND i.role IN ('user', 'assistant'))
              OR i.kind IN ('tool_call', 'tool_result')
          )
"""


def read_thread_slice(
    connection: sqlite3.Connection,
    *,
    thread_id: str,
    item_id: str | None = None,
    before: int = 10,
    after: int = 10,
    max_chars: int = MAX_THREAD_READ_CHARS,
) -> dict[str, Any]:
    """Return a bounded slice of one Thread's settled Items.

    Without *item_id* the slice is anchored on the newest settled Item of the
    Thread's default branch; an empty Thread returns an empty slice instead of
    an error.  Unknown Thread IDs and Item IDs raise distinct errors so the
    model can recover without guessing.  The function never appends journal
    Events or changes the frozen context revision.
    """

    thread = connection.execute(
        """
        SELECT id, title, default_branch_id, updated_at, archived_at
        FROM threads
        WHERE id = ?
        """,
        (thread_id,),
    ).fetchone()
    if thread is None:
        raise ThreadNotFoundError("Thread was not found")

    target_item_id = item_id
    if target_item_id is None:
        target_item_id = _newest_settled_item_id(
            connection,
            thread_id=thread_id,
            branch_id=str(thread["default_branch_id"]),
        )
    if target_item_id is None:
        return {
            "thread": _thread_payload(thread),
            "targetItemId": None,
            "items": [],
            "truncated": False,
        }

    try:
        payload = read_history_slice(
            connection,
            thread_id=thread_id,
            item_id=target_item_id,
            before=before,
            after=after,
            max_chars=max_chars,
            envelope={"thread": _thread_payload(thread)},
        )
    except LookupError as error:
        raise _classify_missing_item(
            connection,
            thread_id=thread_id,
            item_id=target_item_id,
        ) from error

    return payload


def _newest_settled_item_id(
    connection: sqlite3.Connection,
    *,
    thread_id: str,
    branch_id: str,
) -> str | None:
    row = connection.execute(
        f"""
        SELECT i.id
        FROM items i
        JOIN turns t ON t.id = i.turn_id
        JOIN runs r ON r.id = i.run_id
        WHERE t.thread_id = ? AND t.branch_id = ?
          AND {_SETTLED_ITEM_PREDICATE}
        ORDER BY t.ordinal DESC, r.created_at DESC, r.id DESC, i.ordinal DESC, i.id DESC
        LIMIT 1
        """,
        (thread_id, branch_id),
    ).fetchone()
    return None if row is None else str(row["id"])


def _classify_missing_item(
    connection: sqlite3.Connection,
    *,
    thread_id: str,
    item_id: str,
) -> LookupError:
    """Distinguish an unknown Item from an Item that is still in flight.

    ``read_history_slice`` deliberately rejects both cases with ``LookupError``;
    the model receives a stable error code either way, but naming the exact
    cause keeps retries unambiguous.
    """

    row = connection.execute(
        """
        SELECT i.status
        FROM items i
        JOIN turns t ON t.id = i.turn_id
        WHERE i.id = ? AND t.thread_id = ?
        """,
        (item_id, thread_id),
    ).fetchone()
    if row is None:
        return ItemNotFoundError("Thread Item was not found")
    return ItemNotSettledError("Thread Item is not settled")


def _thread_payload(row: sqlite3.Row) -> dict[str, Any]:
    return {
        "id": str(row["id"]),
        "title": row["title"],
        "updatedAt": str(row["updated_at"]),
        "archived": row["archived_at"] is not None,
    }


__all__ = ["read_thread_slice"]
