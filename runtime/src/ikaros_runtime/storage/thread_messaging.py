"""Journal-backed probes for cross-Thread messaging.

A ``thread_send`` Turn records where it came from in its User Item ``data``
(``threadSend``), which is what makes the cross-session hop chain durable
across Runs and restarts. Item data is part of the append-only Journal
projection, so the counter survives a projection rebuild.
"""

from __future__ import annotations

import sqlite3

from ..json_codec import loads as json_loads

THREAD_SEND_DATA_KEY = "threadSend"
_MAX_HOP = 64


def thread_has_active_run(connection: sqlite3.Connection, thread_id: str) -> bool:
    """Whether any Run of the Thread is still queued or running."""

    row = connection.execute(
        """
        SELECT 1
        FROM runs r
        JOIN turns t ON t.id = r.turn_id
        WHERE t.thread_id = ? AND r.status IN ('queued', 'running')
        LIMIT 1
        """,
        (thread_id,),
    ).fetchone()
    return row is not None


def thread_send_hop(connection: sqlite3.Connection, run_id: str) -> int:
    """Read the cross-session hop count recorded for one Run's User Item."""

    row = connection.execute(
        """
        SELECT data_json
        FROM items
        WHERE run_id = ? AND ordinal = 1 AND kind = 'message' AND role = 'user'
        """,
        (run_id,),
    ).fetchone()
    if row is None:
        raise LookupError("run submission item was not found")
    data = json_loads(str(row["data_json"]))
    if not isinstance(data, dict):
        raise RuntimeError("Run submission Item data is invalid")
    record = data.get(THREAD_SEND_DATA_KEY)
    if record is None:
        return 0
    if not isinstance(record, dict):
        raise RuntimeError("Thread message record is invalid")
    hop = record.get("hop")
    if hop is None:
        return 0
    if isinstance(hop, bool) or not isinstance(hop, int) or not 0 <= hop <= _MAX_HOP:
        raise RuntimeError("Thread message hop count is invalid")
    return hop


__all__ = [
    "THREAD_SEND_DATA_KEY",
    "thread_has_active_run",
    "thread_send_hop",
]
