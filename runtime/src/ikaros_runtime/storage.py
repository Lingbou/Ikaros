from __future__ import annotations

import json
import sqlite3
import uuid
from pathlib import Path
from typing import Any, cast

from .domain import (
    JournalEvent,
    PreparedTurn,
    RecoveryPlan,
    RunDescriptor,
    ThreadSummary,
    utc_now,
)

_SCHEMA_VERSION = 4
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

_MIGRATION_2 = """
ALTER TABLE events ADD COLUMN turn_id TEXT;
ALTER TABLE events ADD COLUMN run_id TEXT;
ALTER TABLE events ADD COLUMN item_id TEXT;

CREATE INDEX events_run_seq_idx ON events(run_id, seq);

CREATE TABLE turns (
    id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    branch_id TEXT NOT NULL REFERENCES branches(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(branch_id, ordinal)
);

CREATE TABLE runs (
    id TEXT PRIMARY KEY,
    turn_id TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
    provider_id TEXT NOT NULL,
    model_id TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL,
    settled_at TEXT
);

CREATE TABLE items (
    id TEXT PRIMARY KEY,
    turn_id TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL,
    kind TEXT NOT NULL,
    role TEXT,
    status TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    UNIQUE(run_id, ordinal)
);
"""

_MIGRATION_3 = """
CREATE UNIQUE INDEX events_one_settled_per_run
ON events(run_id) WHERE event_type = 'run.settled';
"""

_MIGRATION_4 = """
ALTER TABLE threads ADD COLUMN client_request_id TEXT;
ALTER TABLE runs ADD COLUMN client_request_id TEXT;

CREATE UNIQUE INDEX threads_client_request_id_idx
ON threads(client_request_id) WHERE client_request_id IS NOT NULL;

CREATE UNIQUE INDEX runs_client_request_id_idx
ON runs(client_request_id) WHERE client_request_id IS NOT NULL;
"""

_TERMINAL_RUN_STATUSES = frozenset({"completed", "failed", "cancelled"})


class SqliteRuntimeStore:
    def __init__(self, database_path: Path) -> None:
        database_path.parent.mkdir(parents=True, exist_ok=True)
        self.database_path = database_path
        self._connection = sqlite3.connect(database_path)
        self._connection.row_factory = sqlite3.Row
        self._connection.execute("PRAGMA foreign_keys = ON")
        self._connection.execute("PRAGMA journal_mode = WAL")
        self._connection.execute("PRAGMA synchronous = NORMAL")
        try:
            self._migrate()
        except BaseException:
            self._connection.close()
            raise

    def _migrate(self) -> None:
        version = int(self._connection.execute("PRAGMA user_version").fetchone()[0])
        if version > _SCHEMA_VERSION:
            raise RuntimeError(
                f"state database schema {version} is newer than supported schema {_SCHEMA_VERSION}"
            )
        if version == 0:
            self._apply_migration(_INITIAL_SCHEMA, target_version=1)
            version = 1
        if version == 1:
            self._apply_migration(_MIGRATION_2, target_version=2)
            version = 2
        if version == 2:
            self._apply_migration(_MIGRATION_3, target_version=3)
            version = 3
        if version == 3:
            self._apply_migration(_MIGRATION_4, target_version=4)

    def _apply_migration(self, script: str, *, target_version: int) -> None:
        transaction = (
            "BEGIN IMMEDIATE;\n"
            f"{script}\n"
            f"PRAGMA user_version = {target_version};\n"
            "COMMIT;"
        )
        try:
            self._connection.executescript(transaction)
        except BaseException:
            if self._connection.in_transaction:
                self._connection.rollback()
            raise

    def close(self) -> None:
        self._connection.close()

    def create_thread(self, title: str | None) -> tuple[ThreadSummary, JournalEvent]:
        thread, event, _created = self._create_thread(title, client_request_id=None)
        return thread, event

    def create_thread_once(
        self,
        title: str | None,
        client_request_id: str,
    ) -> tuple[ThreadSummary, JournalEvent, bool]:
        return self._create_thread(title, client_request_id=client_request_id)

    def _create_thread(
        self,
        title: str | None,
        *,
        client_request_id: str | None,
    ) -> tuple[ThreadSummary, JournalEvent, bool]:
        if client_request_id is not None:
            existing = self._connection.execute(
                """
                SELECT id, title, default_branch_id, created_at, updated_at
                FROM threads WHERE client_request_id = ?
                """,
                (client_request_id,),
            ).fetchone()
            if existing is not None:
                if existing["title"] != title:
                    raise LookupError(
                        "clientRequestId was already used with different thread.create parameters"
                    )
                event_row = self._connection.execute(
                    """
                    SELECT seq, event_type, thread_id, branch_id, turn_id, run_id, item_id,
                           created_at, payload_json
                    FROM events
                    WHERE event_type = 'thread.created' AND thread_id = ?
                    ORDER BY seq
                    LIMIT 1
                    """,
                    (existing["id"],),
                ).fetchone()
                if event_row is None:
                    raise RuntimeError("idempotent thread is missing its creation event")
                return (
                    ThreadSummary(
                        id=str(existing["id"]),
                        title=existing["title"],
                        default_branch_id=str(existing["default_branch_id"]),
                        created_at=str(existing["created_at"]),
                        updated_at=str(existing["updated_at"]),
                    ),
                    self._event_from_row(event_row),
                    False,
                )

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
        if client_request_id is not None:
            payload["clientRequestId"] = client_request_id
        with self._connection:
            self._connection.execute(
                """
                INSERT INTO threads(
                    id, title, default_branch_id, created_at, updated_at, client_request_id
                ) VALUES (?, ?, ?, ?, ?, ?)
                """,
                (thread_id, title, branch_id, timestamp, timestamp, client_request_id),
            )
            self._connection.execute(
                """
                INSERT INTO branches(id, thread_id, created_at, is_default)
                VALUES (?, ?, ?, 1)
                """,
                (branch_id, thread_id, timestamp),
            )
            event = self._append_event(
                event_type="thread.created",
                thread_id=thread_id,
                branch_id=branch_id,
                payload=payload,
                timestamp=timestamp,
            )
        return thread, event, True

    def list_threads(self) -> list[ThreadSummary]:
        rows = self._connection.execute(
            """
            SELECT id, title, default_branch_id, created_at, updated_at
            FROM threads
            ORDER BY updated_at DESC, id
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

    def prepare_turn(
        self,
        *,
        thread_id: str,
        branch_id: str,
        content: str,
        provider_id: str,
        model_id: str,
        client_request_id: str | None = None,
    ) -> PreparedTurn:
        if client_request_id is not None:
            existing = self._connection.execute(
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
            if existing is not None:
                expected = (
                    thread_id,
                    branch_id,
                    provider_id,
                    model_id,
                    content,
                )
                actual = (
                    str(existing["thread_id"]),
                    str(existing["branch_id"]),
                    str(existing["provider_id"]),
                    str(existing["model_id"]),
                    str(existing["content"]),
                )
                if actual != expected:
                    raise LookupError(
                        "clientRequestId was already used with different turn.start parameters"
                    )
                return PreparedTurn(
                    turn_id=str(existing["turn_id"]),
                    run_id=str(existing["run_id"]),
                    thread_id=str(existing["thread_id"]),
                    branch_id=str(existing["branch_id"]),
                    initial_events=(),
                    newly_created=False,
                )

        owner = self._connection.execute(
            "SELECT 1 FROM branches WHERE id = ? AND thread_id = ?",
            (branch_id, thread_id),
        ).fetchone()
        if owner is None:
            raise LookupError("thread or branch was not found")

        ordinal = int(
            self._connection.execute(
                "SELECT COALESCE(MAX(ordinal), 0) + 1 FROM turns WHERE branch_id = ?",
                (branch_id,),
            ).fetchone()[0]
        )
        turn_id = f"turn_{uuid.uuid4().hex}"
        run_id = f"run_{uuid.uuid4().hex}"
        user_item_id = f"item_{uuid.uuid4().hex}"
        timestamp = utc_now()
        turn_payload = {
            "id": turn_id,
            "threadId": thread_id,
            "branchId": branch_id,
            "ordinal": ordinal,
            "status": "queued",
            "createdAt": timestamp,
            "updatedAt": timestamp,
        }
        run_payload = {
            "id": run_id,
            "turnId": turn_id,
            "providerId": provider_id,
            "modelId": model_id,
            "status": "queued",
            "createdAt": timestamp,
            "settledAt": None,
        }
        if client_request_id is not None:
            run_payload["clientRequestId"] = client_request_id
        item_payload = self._item_payload(
            item_id=user_item_id,
            turn_id=turn_id,
            run_id=run_id,
            ordinal=1,
            role="user",
            status="completed",
            content=content,
            created_at=timestamp,
            updated_at=timestamp,
        )

        with self._connection:
            self._connection.execute(
                """
                INSERT INTO turns(id, thread_id, branch_id, ordinal, status, created_at, updated_at)
                VALUES (?, ?, ?, ?, 'queued', ?, ?)
                """,
                (turn_id, thread_id, branch_id, ordinal, timestamp, timestamp),
            )
            self._connection.execute(
                """
                INSERT INTO runs(
                    id, turn_id, provider_id, model_id, status, created_at,
                    client_request_id
                ) VALUES (?, ?, ?, ?, 'queued', ?, ?)
                """,
                (
                    run_id,
                    turn_id,
                    provider_id,
                    model_id,
                    timestamp,
                    client_request_id,
                ),
            )
            self._connection.execute(
                """
                INSERT INTO items(
                    id, turn_id, run_id, ordinal, kind, role, status, content,
                    created_at, updated_at
                ) VALUES (?, ?, ?, 1, 'message', 'user', 'completed', ?, ?, ?)
                """,
                (user_item_id, turn_id, run_id, content, timestamp, timestamp),
            )
            self._connection.execute(
                "UPDATE threads SET updated_at = ? WHERE id = ?",
                (timestamp, thread_id),
            )
            user_event = self._append_event(
                event_type="item.completed",
                thread_id=thread_id,
                branch_id=branch_id,
                turn_id=turn_id,
                run_id=run_id,
                item_id=user_item_id,
                timestamp=timestamp,
                payload={
                    "turn": turn_payload,
                    "run": run_payload,
                    "item": item_payload,
                    **(
                        {"clientRequestId": client_request_id}
                        if client_request_id is not None
                        else {}
                    ),
                },
            )
            queued_event = self._append_event(
                event_type="run.state_changed",
                thread_id=thread_id,
                branch_id=branch_id,
                turn_id=turn_id,
                run_id=run_id,
                timestamp=timestamp,
                payload={"status": "queued"},
            )
        return PreparedTurn(
            turn_id=turn_id,
            run_id=run_id,
            thread_id=thread_id,
            branch_id=branch_id,
            initial_events=(user_event, queued_event),
            newly_created=True,
        )

    def get_run(self, run_id: str) -> RunDescriptor:
        row = self._connection.execute(
            """
            SELECT r.id, r.turn_id, r.provider_id, r.model_id, t.thread_id, t.branch_id
            FROM runs r JOIN turns t ON t.id = r.turn_id
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
        )

    def run_status(self, run_id: str) -> str:
        row = self._connection.execute(
            "SELECT status FROM runs WHERE id = ?",
            (run_id,),
        ).fetchone()
        if row is None:
            raise LookupError("run was not found")
        return str(row["status"])

    def context_messages(
        self,
        branch_id: str,
        *,
        through_turn_id: str,
    ) -> list[tuple[str, str]]:
        boundary = self._connection.execute(
            "SELECT ordinal FROM turns WHERE id = ? AND branch_id = ?",
            (through_turn_id, branch_id),
        ).fetchone()
        if boundary is None:
            raise LookupError("context boundary turn was not found")
        rows = self._connection.execute(
            """
            SELECT i.role, i.content
            FROM items i JOIN turns t ON t.id = i.turn_id
            WHERE t.branch_id = ?
              AND t.ordinal <= ?
              AND i.kind = 'message'
              AND i.status = 'completed'
              AND i.role IN ('user', 'assistant')
            ORDER BY t.ordinal, i.ordinal
            """,
            (branch_id, int(boundary["ordinal"])),
        ).fetchall()
        return [(row["role"], row["content"]) for row in rows]

    def mark_run_running(self, run_id: str) -> JournalEvent:
        run = self.get_run(run_id)
        timestamp = utc_now()
        with self._connection:
            updated = self._connection.execute(
                "UPDATE runs SET status = 'running' WHERE id = ? AND status = 'queued'",
                (run_id,),
            )
            if updated.rowcount != 1:
                raise RuntimeError("run cannot transition to running")
            self._connection.execute(
                "UPDATE turns SET status = 'running', updated_at = ? WHERE id = ?",
                (timestamp, run.turn_id),
            )
            return self._append_event(
                event_type="run.state_changed",
                thread_id=run.thread_id,
                branch_id=run.branch_id,
                turn_id=run.turn_id,
                run_id=run.id,
                timestamp=timestamp,
                payload={"status": "running"},
            )

    def create_assistant_item(self, run_id: str) -> tuple[str, JournalEvent]:
        run = self.get_run(run_id)
        item_id = f"item_{uuid.uuid4().hex}"
        timestamp = utc_now()
        ordinal = int(
            self._connection.execute(
                "SELECT COALESCE(MAX(ordinal), 0) + 1 FROM items WHERE run_id = ?",
                (run_id,),
            ).fetchone()[0]
        )
        item = self._item_payload(
            item_id=item_id,
            turn_id=run.turn_id,
            run_id=run.id,
            ordinal=ordinal,
            role="assistant",
            status="streaming",
            content="",
            created_at=timestamp,
            updated_at=timestamp,
        )
        with self._connection:
            self._connection.execute(
                """
                INSERT INTO items(
                    id, turn_id, run_id, ordinal, kind, role, status, content,
                    created_at, updated_at
                ) VALUES (?, ?, ?, ?, 'message', 'assistant', 'streaming', '', ?, ?)
                """,
                (item_id, run.turn_id, run.id, ordinal, timestamp, timestamp),
            )
            event = self._append_event(
                event_type="item.started",
                thread_id=run.thread_id,
                branch_id=run.branch_id,
                turn_id=run.turn_id,
                run_id=run.id,
                item_id=item_id,
                timestamp=timestamp,
                payload={"item": item},
            )
        return item_id, event

    def append_text_delta(self, item_id: str, delta: str) -> JournalEvent:
        location = self._item_location(item_id)
        timestamp = utc_now()
        with self._connection:
            updated = self._connection.execute(
                """
                UPDATE items SET content = content || ?, updated_at = ?
                WHERE id = ? AND status = 'streaming'
                """,
                (delta, timestamp, item_id),
            )
            if updated.rowcount != 1:
                raise RuntimeError("assistant item is not streaming")
            return self._append_event(
                event_type="item.delta",
                thread_id=location["thread_id"],
                branch_id=location["branch_id"],
                turn_id=location["turn_id"],
                run_id=location["run_id"],
                item_id=item_id,
                timestamp=timestamp,
                payload={"delta": delta},
            )

    def terminalize_run(
        self,
        run_id: str,
        status: str,
        *,
        reason_code: str | None = None,
    ) -> tuple[JournalEvent, ...]:
        if status not in _TERMINAL_RUN_STATUSES:
            raise ValueError("run status is not terminal")
        run = self.get_run(run_id)
        timestamp = utc_now()
        with self._connection:
            row = self._connection.execute(
                "SELECT status FROM runs WHERE id = ?",
                (run_id,),
            ).fetchone()
            if row["status"] in _TERMINAL_RUN_STATUSES:
                return ()
            streaming_items = self._connection.execute(
                """
                SELECT id, ordinal, role, content, created_at
                FROM items
                WHERE run_id = ? AND kind = 'message' AND role = 'assistant'
                  AND status = 'streaming'
                ORDER BY ordinal
                """,
                (run_id,),
            ).fetchall()
            events: list[JournalEvent] = []
            for item_row in streaming_items:
                item_id = str(item_row["id"])
                self._connection.execute(
                    "UPDATE items SET status = ?, updated_at = ? WHERE id = ?",
                    (status, timestamp, item_id),
                )
                item = self._item_payload(
                    item_id=item_id,
                    turn_id=run.turn_id,
                    run_id=run.id,
                    ordinal=int(item_row["ordinal"]),
                    role=str(item_row["role"]),
                    status=status,
                    content=str(item_row["content"]),
                    created_at=str(item_row["created_at"]),
                    updated_at=timestamp,
                )
                events.append(
                    self._append_event(
                        event_type="item.completed",
                        thread_id=run.thread_id,
                        branch_id=run.branch_id,
                        turn_id=run.turn_id,
                        run_id=run.id,
                        item_id=item_id,
                        timestamp=timestamp,
                        payload={"item": item},
                    )
                )
            self._connection.execute(
                "UPDATE runs SET status = ?, settled_at = ? WHERE id = ?",
                (status, timestamp, run_id),
            )
            self._connection.execute(
                "UPDATE turns SET status = ?, updated_at = ? WHERE id = ?",
                (status, timestamp, run.turn_id),
            )
            settled_payload: dict[str, Any] = {
                "status": status,
                "settledAt": timestamp,
            }
            if reason_code is not None:
                settled_payload["reasonCode"] = reason_code
            events.append(
                self._append_event(
                    event_type="run.settled",
                    thread_id=run.thread_id,
                    branch_id=run.branch_id,
                    turn_id=run.turn_id,
                    run_id=run.id,
                    timestamp=timestamp,
                    payload=settled_payload,
                )
            )
        return tuple(events)

    def recover_incomplete_runs(self) -> RecoveryPlan:
        running_rows = self._connection.execute(
            "SELECT id FROM runs WHERE status = 'running' ORDER BY created_at, id"
        ).fetchall()
        for row in running_rows:
            self.terminalize_run(
                str(row["id"]),
                "failed",
                reason_code="runtime_interrupted",
            )

        queued_rows = self._connection.execute(
            """
            SELECT r.id, MIN(e.seq) AS first_event_seq
            FROM runs r
            JOIN events e ON e.run_id = r.id
            WHERE r.status = 'queued'
            GROUP BY r.id
            ORDER BY first_event_seq, r.id
            """
        ).fetchall()
        return RecoveryPlan(tuple(str(row["id"]) for row in queued_rows))

    def replay_events(self, after_seq: int, limit: int) -> tuple[list[JournalEvent], int]:
        rows = self._connection.execute(
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
        events = [self._event_from_row(row) for row in rows]
        latest_seq = int(
            self._connection.execute("SELECT COALESCE(MAX(seq), 0) FROM events").fetchone()[0]
        )
        return events, latest_seq

    def latest_sequence(self) -> int:
        return int(
            self._connection.execute("SELECT COALESCE(MAX(seq), 0) FROM events").fetchone()[0]
        )

    def rebuild_projections(self) -> None:
        rows = self._connection.execute(
            "SELECT event_type, created_at, payload_json FROM events ORDER BY seq"
        ).fetchall()
        with self._connection:
            self._connection.execute("DELETE FROM items")
            self._connection.execute("DELETE FROM runs")
            self._connection.execute("DELETE FROM turns")
            self._connection.execute("DELETE FROM branches")
            self._connection.execute("DELETE FROM threads")
            for row in rows:
                self._apply_projection_event(
                    row["event_type"],
                    json.loads(row["payload_json"]),
                    timestamp=row["created_at"],
                )

    def _apply_projection_event(
        self,
        event_type: str,
        payload: dict[str, Any],
        *,
        timestamp: str,
    ) -> None:
        if event_type == "thread.created":
            thread = payload["thread"]
            branch = payload["branch"]
            self._connection.execute(
                """
                INSERT INTO threads(
                    id, title, default_branch_id, created_at, updated_at, client_request_id
                ) VALUES (?, ?, ?, ?, ?, ?)
                """,
                (
                    thread["id"],
                    thread["title"],
                    thread["defaultBranchId"],
                    thread["createdAt"],
                    thread["updatedAt"],
                    payload.get("clientRequestId"),
                ),
            )
            self._connection.execute(
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
            self._connection.execute(
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
            self._connection.execute(
                """
                INSERT INTO runs(
                    id, turn_id, provider_id, model_id, status, created_at, settled_at,
                    client_request_id
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    run["id"],
                    run["turnId"],
                    run["providerId"],
                    run["modelId"],
                    run["status"],
                    run["createdAt"],
                    run["settledAt"],
                    run.get("clientRequestId"),
                ),
            )
            self._insert_projected_item(item)
            self._connection.execute(
                "UPDATE threads SET updated_at = ? WHERE id = ?",
                (turn["updatedAt"], turn["threadId"]),
            )
        elif event_type == "run.state_changed":
            self._connection.execute(
                "UPDATE runs SET status = ? WHERE id = ?",
                (payload["status"], payload.get("runId")),
            )
            self._connection.execute(
                "UPDATE turns SET status = ?, updated_at = ? WHERE id = ?",
                (payload["status"], timestamp, payload.get("turnId")),
            )
        elif event_type == "item.started":
            self._insert_projected_item(payload["item"])
        elif event_type == "item.delta":
            self._connection.execute(
                "UPDATE items SET content = content || ?, updated_at = ? WHERE id = ?",
                (payload["delta"], timestamp, payload.get("itemId")),
            )
        elif event_type == "item.completed":
            item = payload["item"]
            self._connection.execute(
                "UPDATE items SET status = ?, content = ?, updated_at = ? WHERE id = ?",
                (item["status"], item["content"], item["updatedAt"], item["id"]),
            )
        elif event_type == "run.settled":
            self._connection.execute(
                "UPDATE runs SET status = ?, settled_at = ? WHERE id = ?",
                (payload["status"], payload.get("settledAt"), payload.get("runId")),
            )
            self._connection.execute(
                "UPDATE turns SET status = ?, updated_at = ? WHERE id = ?",
                (payload["status"], payload.get("settledAt"), payload.get("turnId")),
            )

    def _insert_projected_item(self, item: dict[str, Any]) -> None:
        self._connection.execute(
            """
            INSERT INTO items(
                id, turn_id, run_id, ordinal, kind, role, status, content, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
            ),
        )

    def _item_location(self, item_id: str) -> sqlite3.Row:
        row = self._connection.execute(
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

    def _append_event(
        self,
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
        cursor = self._connection.execute(
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
                json.dumps(event_payload, separators=(",", ":"), ensure_ascii=False),
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

    @staticmethod
    def _item_payload(
        *,
        item_id: str,
        turn_id: str,
        run_id: str,
        ordinal: int,
        role: str,
        status: str,
        content: str,
        created_at: str,
        updated_at: str,
    ) -> dict[str, Any]:
        return {
            "id": item_id,
            "turnId": turn_id,
            "runId": run_id,
            "ordinal": ordinal,
            "kind": "message",
            "role": role,
            "status": status,
            "content": content,
            "createdAt": created_at,
            "updatedAt": updated_at,
        }

    @staticmethod
    def _event_from_row(row: sqlite3.Row) -> JournalEvent:
        return JournalEvent(
            seq=int(row["seq"]),
            type=row["event_type"],
            thread_id=row["thread_id"],
            branch_id=row["branch_id"],
            turn_id=row["turn_id"],
            run_id=row["run_id"],
            item_id=row["item_id"],
            timestamp=row["created_at"],
            payload=json.loads(row["payload_json"]),
        )
