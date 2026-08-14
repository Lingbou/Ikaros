from __future__ import annotations

import base64
import binascii
import re
import sqlite3
from dataclasses import dataclass
from datetime import datetime

from ..domain import JsonObject, ThreadSummary
from ..json_codec import dumps as json_dumps
from ..json_codec import loads as json_loads
from .journal import latest_sequence
from .projections import workspace_from_json

THREAD_CATALOG_DEFAULT_LIMIT = 50
THREAD_CATALOG_MAX_LIMIT = 100

_CURSOR_VERSION = 1
_MAX_CURSOR_LENGTH = 1024
_MAX_THREAD_ID_LENGTH = 200
_CURSOR_CHARACTERS = re.compile(r"^[A-Za-z0-9_-]+$")
_CANONICAL_TIMESTAMP = re.compile(
    r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$"
)
_INVALID_CURSOR_MESSAGE = "thread.list cursor is invalid"


@dataclass(frozen=True, slots=True)
class ThreadCatalogCursor:
    updated_at: str
    thread_id: str


@dataclass(frozen=True, slots=True)
class ThreadCatalogPage:
    threads: tuple[ThreadSummary, ...]
    next_cursor: str | None
    has_more: bool
    snapshot_seq: int

    def to_wire(self) -> JsonObject:
        return {
            "threads": [thread.to_wire() for thread in self.threads],
            "nextCursor": self.next_cursor,
            "hasMore": self.has_more,
            "snapshotSeq": self.snapshot_seq,
        }


def list_thread_page(
    connection: sqlite3.Connection,
    *,
    cursor: str | None,
    limit: int,
) -> ThreadCatalogPage:
    if isinstance(limit, bool) or not isinstance(limit, int):
        raise ValueError("thread.list limit must be an integer")
    if limit < 1 or limit > THREAD_CATALOG_MAX_LIMIT:
        raise ValueError(
            f"thread.list limit must be between 1 and {THREAD_CATALOG_MAX_LIMIT}"
        )

    boundary = decode_thread_catalog_cursor(cursor) if cursor is not None else None
    if connection.in_transaction:
        raise RuntimeError("thread catalog reads require an idle SQLite connection")
    connection.execute("BEGIN")
    try:
        snapshot_seq = latest_sequence(connection)
        if boundary is None:
            rows = connection.execute(
                """
                SELECT id, title, default_branch_id, workspace_json, created_at, updated_at
                FROM threads
                ORDER BY updated_at DESC, id ASC
                LIMIT ?
                """,
                (limit + 1,),
            ).fetchall()
        else:
            rows = connection.execute(
                """
                SELECT id, title, default_branch_id, workspace_json, created_at, updated_at
                FROM threads
                WHERE updated_at <= ? AND (updated_at < ? OR id > ?)
                ORDER BY updated_at DESC, id ASC
                LIMIT ?
                """,
                (
                    boundary.updated_at,
                    boundary.updated_at,
                    boundary.thread_id,
                    limit + 1,
                ),
            ).fetchall()
        connection.commit()
    except BaseException:
        connection.rollback()
        raise

    has_more = len(rows) > limit
    returned_rows = rows[:limit]
    threads = tuple(_thread_from_row(row) for row in returned_rows)
    next_cursor = None
    if has_more:
        last_row = returned_rows[-1]
        next_cursor = encode_thread_catalog_cursor(
            ThreadCatalogCursor(
                updated_at=str(last_row["updated_at"]),
                thread_id=str(last_row["id"]),
            )
        )
    return ThreadCatalogPage(
        threads=threads,
        next_cursor=next_cursor,
        has_more=has_more,
        snapshot_seq=snapshot_seq,
    )


def encode_thread_catalog_cursor(cursor: ThreadCatalogCursor) -> str:
    _validate_cursor_boundary(cursor.updated_at, cursor.thread_id)
    payload = json_dumps(
        {"v": _CURSOR_VERSION, "updatedAt": cursor.updated_at, "id": cursor.thread_id},
        ensure_ascii=False,
        separators=(",", ":"),
    ).encode("utf-8")
    return base64.urlsafe_b64encode(payload).rstrip(b"=").decode("ascii")


def decode_thread_catalog_cursor(value: str) -> ThreadCatalogCursor:
    if (
        not value
        or len(value) > _MAX_CURSOR_LENGTH
        or _CURSOR_CHARACTERS.fullmatch(value) is None
    ):
        raise ValueError(_INVALID_CURSOR_MESSAGE)
    try:
        padding = "=" * (-len(value) % 4)
        decoded = base64.b64decode(
            value + padding,
            altchars=b"-_",
            validate=True,
        )
        payload = json_loads(decoded)
    except (UnicodeDecodeError, ValueError, binascii.Error):
        raise ValueError(_INVALID_CURSOR_MESSAGE) from None
    if not isinstance(payload, dict) or set(payload) != {"v", "updatedAt", "id"}:
        raise ValueError(_INVALID_CURSOR_MESSAGE)
    version = payload["v"]
    updated_at = payload["updatedAt"]
    thread_id = payload["id"]
    if isinstance(version, bool) or not isinstance(version, int) or version != _CURSOR_VERSION:
        raise ValueError(_INVALID_CURSOR_MESSAGE)
    if not isinstance(updated_at, str) or not isinstance(thread_id, str):
        raise ValueError(_INVALID_CURSOR_MESSAGE)
    cursor = ThreadCatalogCursor(updated_at=updated_at, thread_id=thread_id)
    try:
        _validate_cursor_boundary(cursor.updated_at, cursor.thread_id)
    except ValueError:
        raise ValueError(_INVALID_CURSOR_MESSAGE) from None
    if encode_thread_catalog_cursor(cursor) != value:
        raise ValueError(_INVALID_CURSOR_MESSAGE)
    return cursor


def _validate_cursor_boundary(updated_at: str, thread_id: str) -> None:
    if _CANONICAL_TIMESTAMP.fullmatch(updated_at) is None:
        raise ValueError(_INVALID_CURSOR_MESSAGE)
    try:
        datetime.strptime(updated_at, "%Y-%m-%dT%H:%M:%S.%fZ")
    except ValueError:
        raise ValueError(_INVALID_CURSOR_MESSAGE) from None
    if (
        not thread_id
        or len(thread_id) > _MAX_THREAD_ID_LENGTH
        or thread_id != thread_id.strip()
        or any(ord(character) < 0x20 or ord(character) == 0x7F for character in thread_id)
    ):
        raise ValueError(_INVALID_CURSOR_MESSAGE)


def _thread_from_row(row: sqlite3.Row) -> ThreadSummary:
    return ThreadSummary(
        id=str(row["id"]),
        title=row["title"],
        default_branch_id=str(row["default_branch_id"]),
        workspace=workspace_from_json(row["workspace_json"]),
        created_at=str(row["created_at"]),
        updated_at=str(row["updated_at"]),
    )


__all__ = [
    "THREAD_CATALOG_DEFAULT_LIMIT",
    "THREAD_CATALOG_MAX_LIMIT",
    "ThreadCatalogPage",
    "list_thread_page",
]
