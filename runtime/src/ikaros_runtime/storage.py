from __future__ import annotations

import json
import sqlite3
import uuid
from pathlib import Path
from typing import Any

from .domain import JournalEvent, ThreadSummary, utc_now

_SCHEMA_VERSION = 1
_INITIAL_SCHEMA = """
CREATE TABLE events (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    event_type TEXT NOT NULL,
    thread_id TEXT,
    branch_id TEXT,
    created_at TEXT NOT NULL,
    payload_json TEXT NOT NULL
);

CREATE INDEX events_thread_seq_idx ON events(thread_id, seq);

CREATE TABLE threads (
    id TEXT PRIMARY KEY,
    title TEXT,
    default_branch_id TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE branches (
    id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    is_default INTEGER NOT NULL CHECK (is_default IN (0, 1))
);

CREATE UNIQUE INDEX branches_default_thread_idx
ON branches(thread_id) WHERE is_default = 1;
"""


class SqliteRuntimeStore:
    def __init__(self, database_path: Path) -> None:
        database_path.parent.mkdir(parents=True, exist_ok=True)
        self.database_path = database_path
        self._connection = sqlite3.connect(database_path)
        self._connection.row_factory = sqlite3.Row
        self._connection.execute("PRAGMA foreign_keys = ON")
        self._connection.execute("PRAGMA journal_mode = WAL")
        self._connection.execute("PRAGMA synchronous = NORMAL")
        self._migrate()

    def _migrate(self) -> None:
        version = int(self._connection.execute("PRAGMA user_version").fetchone()[0])
        if version > _SCHEMA_VERSION:
            raise RuntimeError(
                f"state database schema {version} is newer than supported schema {_SCHEMA_VERSION}"
            )
        if version == 0:
            with self._connection:
                self._connection.executescript(_INITIAL_SCHEMA)
                self._connection.execute(f"PRAGMA user_version = {_SCHEMA_VERSION}")

    def close(self) -> None:
        self._connection.close()

    def create_thread(self, title: str | None) -> tuple[ThreadSummary, JournalEvent]:
        thread_id = f"thread_{uuid.uuid4().hex}"
        branch_id = f"branch_{uuid.uuid4().hex}"
        timestamp = utc_now()
        thread = ThreadSummary(
            id=thread_id,
            title=title,
            default_branch_id=branch_id,
            created_at=timestamp,
            updated_at=timestamp,
        )
        payload: dict[str, Any] = {
            "thread": thread.to_wire(),
            "branch": {
                "id": branch_id,
                "threadId": thread_id,
                "createdAt": timestamp,
                "isDefault": True,
            },
        }
        with self._connection:
            self._connection.execute(
                """
                INSERT INTO threads(id, title, default_branch_id, created_at, updated_at)
                VALUES (?, ?, ?, ?, ?)
                """,
                (thread_id, title, branch_id, timestamp, timestamp),
            )
            self._connection.execute(
                """
                INSERT INTO branches(id, thread_id, created_at, is_default)
                VALUES (?, ?, ?, 1)
                """,
                (branch_id, thread_id, timestamp),
            )
            cursor = self._connection.execute(
                """
                INSERT INTO events(event_type, thread_id, branch_id, created_at, payload_json)
                VALUES ('thread.created', ?, ?, ?, ?)
                """,
                (
                    thread_id,
                    branch_id,
                    timestamp,
                    json.dumps(payload, separators=(",", ":"), ensure_ascii=False),
                ),
            )
        if cursor.lastrowid is None:
            raise RuntimeError("SQLite did not return an event sequence")
        event = JournalEvent(
            seq=cursor.lastrowid,
            type="thread.created",
            thread_id=thread_id,
            branch_id=branch_id,
            timestamp=timestamp,
            payload=payload,
        )
        return thread, event

    def list_threads(self) -> list[ThreadSummary]:
        rows = self._connection.execute(
            """
            SELECT id, title, default_branch_id, created_at, updated_at
            FROM threads
            ORDER BY created_at, id
            """
        ).fetchall()
        return [
            ThreadSummary(
                id=row["id"],
                title=row["title"],
                default_branch_id=row["default_branch_id"],
                created_at=row["created_at"],
                updated_at=row["updated_at"],
            )
            for row in rows
        ]

    def replay_events(self, after_seq: int, limit: int) -> tuple[list[JournalEvent], int]:
        rows = self._connection.execute(
            """
            SELECT seq, event_type, thread_id, branch_id, created_at, payload_json
            FROM events
            WHERE seq > ?
            ORDER BY seq
            LIMIT ?
            """,
            (after_seq, limit),
        ).fetchall()
        events = [self._event_from_row(row) for row in rows]
        latest_seq = int(
            self._connection.execute("SELECT COALESCE(MAX(seq), 0) FROM events").fetchone()[0]
        )
        return events, latest_seq

    def rebuild_projections(self) -> None:
        rows = self._connection.execute(
            """
            SELECT payload_json FROM events
            WHERE event_type = 'thread.created'
            ORDER BY seq
            """
        ).fetchall()
        with self._connection:
            self._connection.execute("DELETE FROM branches")
            self._connection.execute("DELETE FROM threads")
            for row in rows:
                payload = json.loads(row["payload_json"])
                thread = payload["thread"]
                branch = payload["branch"]
                self._connection.execute(
                    """
                    INSERT INTO threads(id, title, default_branch_id, created_at, updated_at)
                    VALUES (?, ?, ?, ?, ?)
                    """,
                    (
                        thread["id"],
                        thread["title"],
                        thread["defaultBranchId"],
                        thread["createdAt"],
                        thread["updatedAt"],
                    ),
                )
                self._connection.execute(
                    """
                    INSERT INTO branches(id, thread_id, created_at, is_default)
                    VALUES (?, ?, ?, ?)
                    """,
                    (
                        branch["id"],
                        branch["threadId"],
                        branch["createdAt"],
                        int(branch["isDefault"]),
                    ),
                )

    @staticmethod
    def _event_from_row(row: sqlite3.Row) -> JournalEvent:
        return JournalEvent(
            seq=int(row["seq"]),
            type=row["event_type"],
            thread_id=row["thread_id"],
            branch_id=row["branch_id"],
            timestamp=row["created_at"],
            payload=json.loads(row["payload_json"]),
        )
