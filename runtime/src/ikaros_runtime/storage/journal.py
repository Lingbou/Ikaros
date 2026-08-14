"""Transaction-neutral access to the append-only Runtime event journal."""

from __future__ import annotations

import sqlite3
from typing import Any

from ..domain import JOURNAL_EVENT_SCHEMA_VERSION, JournalEvent
from ..errors import UnsupportedJournalEventVersionError
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
            schema_version, event_type, thread_id, branch_id, turn_id, run_id, item_id,
            created_at, payload_json
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            JOURNAL_EVENT_SCHEMA_VERSION,
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
        schema_version=JOURNAL_EVENT_SCHEMA_VERSION,
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
        SELECT seq, schema_version, event_type, thread_id, branch_id, turn_id, run_id, item_id,
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


def projection_events(connection: sqlite3.Connection) -> list[JournalEvent]:
    rows = connection.execute(
        """
        SELECT seq, schema_version, event_type, thread_id, branch_id, turn_id, run_id, item_id,
               created_at, payload_json
        FROM events ORDER BY seq
        """
    ).fetchall()
    events = [event_from_row(row) for row in rows]
    for expected_seq, event in enumerate(events, start=1):
        if event.seq != expected_seq:
            raise RuntimeError("journal event sequence is not contiguous")
    if journal_sequence_high_water(connection) != len(events):
        raise RuntimeError("journal event sequence high-water mark is not contiguous")
    return events


def journal_sequence_high_water(connection: sqlite3.Connection) -> int:
    row = connection.execute(
        "SELECT seq FROM sqlite_sequence WHERE name = 'events'"
    ).fetchone()
    return int(row[0]) if row is not None else 0


def event_from_row(row: sqlite3.Row) -> JournalEvent:
    stored_version = _event_schema_version(row["schema_version"])
    decoded_payload = json_loads(row["payload_json"])
    if not isinstance(decoded_payload, dict):
        raise RuntimeError("journal event payload is not an object")
    return JournalEvent(
        seq=int(row["seq"]),
        schema_version=stored_version,
        type=row["event_type"],
        thread_id=row["thread_id"],
        branch_id=row["branch_id"],
        turn_id=row["turn_id"],
        run_id=row["run_id"],
        item_id=row["item_id"],
        timestamp=row["created_at"],
        payload=decoded_payload,
    )


def _event_schema_version(value: object) -> int:
    if not isinstance(value, int) or isinstance(value, bool) or value < 1:
        raise UnsupportedJournalEventVersionError("journal event has an invalid schema version")
    if value != JOURNAL_EVENT_SCHEMA_VERSION:
        raise UnsupportedJournalEventVersionError(
            f"journal event schema version {value} does not match supported version "
            f"{JOURNAL_EVENT_SCHEMA_VERSION}"
        )
    return value
