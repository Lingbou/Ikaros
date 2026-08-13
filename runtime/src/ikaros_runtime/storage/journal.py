"""Transaction-neutral access to the append-only Runtime event journal."""

from __future__ import annotations

import sqlite3
from typing import Any

from ..domain import JournalEvent
from ..json_codec import dumps as json_dumps
from ..json_codec import loads as json_loads


def append_event(
    connection: sqlite3.Connection,
    *,
    event_type: str,
    thread_id: str | None = None,
    branch_id: str | None = None,
    turn_id: str | None = None,
    run_id: str | None = None,
    item_id: str | None = None,
    payload: dict[str, Any],
    timestamp: str,
) -> JournalEvent:
    event_payload = {
        **payload,
        **({"turnId": turn_id} if turn_id is not None else {}),
        **({"runId": run_id} if run_id is not None else {}),
        **({"itemId": item_id} if item_id is not None else {}),
    }
    cursor = connection.execute(
        """
        INSERT INTO events(
            event_type, thread_id, branch_id, turn_id, run_id, item_id,
            created_at, payload_json
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            event_type,
            thread_id,
            branch_id,
            turn_id,
            run_id,
            item_id,
            timestamp,
            json_dumps(event_payload, separators=(",", ":"), ensure_ascii=False),
        ),
    )
    if cursor.lastrowid is None:
        raise RuntimeError("SQLite did not return an event sequence")
    return JournalEvent(
        seq=cursor.lastrowid,
        type=event_type,
        thread_id=thread_id,
        branch_id=branch_id,
        turn_id=turn_id,
        run_id=run_id,
        item_id=item_id,
        timestamp=timestamp,
        payload=event_payload,
    )


def replay_events(
    connection: sqlite3.Connection,
    after_seq: int,
    limit: int,
) -> tuple[list[JournalEvent], int]:
    rows = connection.execute(
        """
        SELECT seq, event_type, thread_id, branch_id, turn_id, run_id, item_id,
               created_at, payload_json
        FROM events
        WHERE seq > ?
        ORDER BY seq
        LIMIT ?
        """,
        (after_seq, limit),
    ).fetchall()
    events = [event_from_row(row) for row in rows]
    return events, latest_sequence(connection)


def latest_sequence(connection: sqlite3.Connection) -> int:
    return int(connection.execute("SELECT COALESCE(MAX(seq), 0) FROM events").fetchone()[0])


def projection_rows(connection: sqlite3.Connection) -> list[sqlite3.Row]:
    return connection.execute(
        "SELECT event_type, created_at, payload_json FROM events ORDER BY seq"
    ).fetchall()


def event_from_row(row: sqlite3.Row) -> JournalEvent:
    return JournalEvent(
        seq=int(row["seq"]),
        type=row["event_type"],
        thread_id=row["thread_id"],
        branch_id=row["branch_id"],
        turn_id=row["turn_id"],
        run_id=row["run_id"],
        item_id=row["item_id"],
        timestamp=row["created_at"],
        payload=json_loads(row["payload_json"]),
    )
