"""Own connection and schema for messaging-channel state.

Channel state lives in its own SQLite database so adding a channel does not
force a reset of the conversation ``state.db``.
"""

from __future__ import annotations

import sqlite3
from dataclasses import dataclass
from datetime import UTC, datetime, timedelta
from pathlib import Path

from ..domain import utc_now

CHANNEL_SCHEMA_VERSION = 1
CHANNEL_DATABASE_INCOMPATIBLE = "channel database schema is incompatible; reset required"

_SCHEMA = """
CREATE TABLE channel_bindings (
    channel_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    chat_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    branch_id TEXT NOT NULL,
    owner_user_id TEXT,
    owner_username TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY(channel_id, account_id, chat_id)
);

CREATE UNIQUE INDEX channel_bindings_thread_idx
ON channel_bindings(channel_id, thread_id);

CREATE INDEX channel_bindings_account_idx
ON channel_bindings(channel_id, account_id, updated_at DESC, chat_id ASC);

CREATE TABLE channel_cursors (
    channel_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    next_update_id INTEGER NOT NULL CHECK (next_update_id >= 0),
    updated_at TEXT NOT NULL,
    PRIMARY KEY(channel_id, account_id)
);

CREATE TABLE channel_outbox (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    channel_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    chat_id TEXT NOT NULL,
    run_id TEXT,
    text TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'sending', 'sent', 'failed')),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    next_attempt_at TEXT,
    telegram_message_id INTEGER,
    last_error_code TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE UNIQUE INDEX channel_outbox_run_idx
ON channel_outbox(channel_id, account_id, run_id) WHERE run_id IS NOT NULL;

CREATE INDEX channel_outbox_pending_idx
ON channel_outbox(status, next_attempt_at, id);
"""


@dataclass(frozen=True, slots=True)
class ChannelBinding:
    channel_id: str
    account_id: str
    chat_id: str
    thread_id: str
    branch_id: str
    owner_user_id: str | None
    owner_username: str | None
    created_at: str
    updated_at: str


@dataclass(frozen=True, slots=True)
class ChannelOutboxItem:
    id: int
    channel_id: str
    account_id: str
    chat_id: str
    run_id: str | None
    text: str
    status: str
    attempts: int
    next_attempt_at: str | None
    telegram_message_id: int | None
    last_error_code: str | None
    created_at: str
    updated_at: str


class ChannelStore:
    """One serialized SQLite connection for channel bindings and delivery."""

    def __init__(self, database_path: Path) -> None:
        database_path.parent.mkdir(parents=True, exist_ok=True)
        self.database_path = database_path
        self._connection = sqlite3.connect(database_path)
        self._connection.row_factory = sqlite3.Row
        self._connection.execute("PRAGMA journal_mode = WAL")
        self._connection.execute("PRAGMA synchronous = NORMAL")
        try:
            self._initialize_schema()
        except BaseException:
            self._connection.close()
            raise

    def close(self) -> None:
        self._connection.close()

    def _initialize_schema(self) -> None:
        version = int(self._connection.execute("PRAGMA user_version").fetchone()[0])
        if version == CHANNEL_SCHEMA_VERSION:
            return
        if version != 0:
            raise RuntimeError(CHANNEL_DATABASE_INCOMPATIBLE)
        with self._connection:
            self._connection.executescript(_SCHEMA)
            self._connection.execute(f"PRAGMA user_version = {CHANNEL_SCHEMA_VERSION}")

    def get_binding(
        self,
        channel_id: str,
        account_id: str,
        chat_id: str,
    ) -> ChannelBinding | None:
        row = self._connection.execute(
            """SELECT * FROM channel_bindings
               WHERE channel_id = ? AND account_id = ? AND chat_id = ?""",
            (channel_id, account_id, chat_id),
        ).fetchone()
        return _binding_from_row(row)

    def list_bindings(self, channel_id: str) -> tuple[ChannelBinding, ...]:
        rows = self._connection.execute(
            """SELECT * FROM channel_bindings
               WHERE channel_id = ?
               ORDER BY updated_at DESC, chat_id ASC""",
            (channel_id,),
        ).fetchall()
        return tuple(
            binding
            for row in rows
            if (binding := _binding_from_row(row)) is not None
        )

    def get_binding_by_thread(
        self,
        channel_id: str,
        thread_id: str,
    ) -> ChannelBinding | None:
        row = self._connection.execute(
            """SELECT * FROM channel_bindings
               WHERE channel_id = ? AND thread_id = ?""",
            (channel_id, thread_id),
        ).fetchone()
        return _binding_from_row(row)

    def upsert_binding(
        self,
        *,
        channel_id: str,
        account_id: str,
        chat_id: str,
        thread_id: str,
        branch_id: str,
        owner_user_id: str | None,
        owner_username: str | None,
    ) -> ChannelBinding:
        timestamp = utc_now()
        with self._connection:
            existing = self._connection.execute(
                """SELECT created_at FROM channel_bindings
                   WHERE channel_id = ? AND account_id = ? AND chat_id = ?""",
                (channel_id, account_id, chat_id),
            ).fetchone()
            created_at = str(existing["created_at"]) if existing is not None else timestamp
            self._connection.execute(
                """INSERT INTO channel_bindings(
                       channel_id, account_id, chat_id, thread_id, branch_id,
                       owner_user_id, owner_username, created_at, updated_at
                   ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
                   ON CONFLICT(channel_id, account_id, chat_id) DO UPDATE SET
                       thread_id = excluded.thread_id,
                       branch_id = excluded.branch_id,
                       owner_user_id = COALESCE(
                           excluded.owner_user_id, channel_bindings.owner_user_id
                       ),
                       owner_username = COALESCE(
                           excluded.owner_username, channel_bindings.owner_username
                       ),
                       updated_at = excluded.updated_at""",
                (
                    channel_id,
                    account_id,
                    chat_id,
                    thread_id,
                    branch_id,
                    owner_user_id,
                    owner_username,
                    created_at,
                    timestamp,
                ),
            )
        binding = self.get_binding(channel_id, account_id, chat_id)
        if binding is None:
            raise RuntimeError("channel binding could not be stored")
        return binding

    def reset_channel(self, channel_id: str) -> None:
        with self._connection:
            self._connection.execute(
                "DELETE FROM channel_outbox WHERE channel_id = ?", (channel_id,)
            )
            self._connection.execute(
                "DELETE FROM channel_cursors WHERE channel_id = ?", (channel_id,)
            )
            self._connection.execute(
                "DELETE FROM channel_bindings WHERE channel_id = ?", (channel_id,)
            )

    def get_cursor(self, channel_id: str, account_id: str) -> int:
        row = self._connection.execute(
            """SELECT next_update_id FROM channel_cursors
               WHERE channel_id = ? AND account_id = ?""",
            (channel_id, account_id),
        ).fetchone()
        return int(row["next_update_id"]) if row is not None else 0

    def set_cursor(self, channel_id: str, account_id: str, next_update_id: int) -> None:
        if next_update_id < 0:
            raise ValueError("channel cursor must be non-negative")
        timestamp = utc_now()
        with self._connection:
            self._connection.execute(
                """INSERT INTO channel_cursors(
                       channel_id, account_id, next_update_id, updated_at
                   ) VALUES (?, ?, ?, ?)
                   ON CONFLICT(channel_id, account_id) DO UPDATE SET
                       next_update_id = excluded.next_update_id,
                       updated_at = excluded.updated_at""",
                (channel_id, account_id, next_update_id, timestamp),
            )

    def enqueue_outbox(
        self,
        *,
        channel_id: str,
        account_id: str,
        chat_id: str,
        text: str,
        run_id: str | None = None,
    ) -> int:
        if not text:
            raise ValueError("channel outbox text must not be empty")
        timestamp = utc_now()
        with self._connection:
            if run_id is not None:
                existing = self._connection.execute(
                    """SELECT id FROM channel_outbox
                       WHERE channel_id = ? AND account_id = ? AND run_id = ?""",
                    (channel_id, account_id, run_id),
                ).fetchone()
                if existing is not None:
                    return int(existing["id"])
            cursor = self._connection.execute(
                """INSERT INTO channel_outbox(
                       channel_id, account_id, chat_id, run_id, text, status,
                       attempts, created_at, updated_at
                   ) VALUES (?, ?, ?, ?, ?, 'pending', 0, ?, ?)""",
                (channel_id, account_id, chat_id, run_id, text, timestamp, timestamp),
            )
            if cursor.lastrowid is None:
                raise RuntimeError("channel outbox insert returned no row id")
            return int(cursor.lastrowid)

    def recover_sending(self) -> None:
        timestamp = utc_now()
        with self._connection:
            self._connection.execute(
                """UPDATE channel_outbox
                   SET status = 'pending', next_attempt_at = NULL, updated_at = ?
                   WHERE status = 'sending'""",
                (timestamp,),
            )

    def claim_outbox(self, *, now: str | None = None) -> ChannelOutboxItem | None:
        timestamp = utc_now() if now is None else now
        with self._connection:
            row = self._connection.execute(
                """SELECT * FROM channel_outbox
                   WHERE status = 'pending'
                     AND (next_attempt_at IS NULL OR next_attempt_at <= ?)
                   ORDER BY id ASC
                   LIMIT 1""",
                (timestamp,),
            ).fetchone()
            if row is None:
                return None
            self._connection.execute(
                "UPDATE channel_outbox SET status = 'sending', updated_at = ? WHERE id = ?",
                (timestamp, int(row["id"])),
            )
        return _outbox_from_row(row)

    def mark_outbox_sent(self, outbox_id: int, telegram_message_id: int) -> None:
        with self._connection:
            updated = self._connection.execute(
                """UPDATE channel_outbox
                   SET status = 'sent', telegram_message_id = ?, next_attempt_at = NULL,
                       last_error_code = NULL, updated_at = ?
                   WHERE id = ?""",
                (telegram_message_id, utc_now(), outbox_id),
            )
            if updated.rowcount != 1:
                raise LookupError("channel outbox item was not found")

    def retry_outbox(self, outbox_id: int, *, delay_seconds: float, error_code: str) -> None:
        timestamp = datetime.now(UTC) + timedelta(seconds=max(0.0, delay_seconds))
        next_attempt = timestamp.isoformat(timespec="milliseconds").replace("+00:00", "Z")
        with self._connection:
            updated = self._connection.execute(
                """UPDATE channel_outbox
                   SET status = 'pending', attempts = attempts + 1,
                       next_attempt_at = ?, last_error_code = ?, updated_at = ?
                   WHERE id = ?""",
                (next_attempt, error_code, utc_now(), outbox_id),
            )
            if updated.rowcount != 1:
                raise LookupError("channel outbox item was not found")

    def fail_outbox(self, outbox_id: int, error_code: str) -> None:
        with self._connection:
            updated = self._connection.execute(
                """UPDATE channel_outbox
                   SET status = 'failed', attempts = attempts + 1,
                       last_error_code = ?, updated_at = ?
                   WHERE id = ?""",
                (error_code, utc_now(), outbox_id),
            )
            if updated.rowcount != 1:
                raise LookupError("channel outbox item was not found")

    def pending_outbox_count(self) -> int:
        row = self._connection.execute(
            "SELECT COUNT(*) FROM channel_outbox WHERE status IN ('pending', 'sending')"
        ).fetchone()
        return int(row[0])


def _binding_from_row(row: sqlite3.Row | None) -> ChannelBinding | None:
    if row is None:
        return None
    return ChannelBinding(
        channel_id=str(row["channel_id"]),
        account_id=str(row["account_id"]),
        chat_id=str(row["chat_id"]),
        thread_id=str(row["thread_id"]),
        branch_id=str(row["branch_id"]),
        owner_user_id=_optional_string(row["owner_user_id"]),
        owner_username=_optional_string(row["owner_username"]),
        created_at=str(row["created_at"]),
        updated_at=str(row["updated_at"]),
    )


def _outbox_from_row(row: sqlite3.Row) -> ChannelOutboxItem:
    return ChannelOutboxItem(
        id=int(row["id"]),
        channel_id=str(row["channel_id"]),
        account_id=str(row["account_id"]),
        chat_id=str(row["chat_id"]),
        run_id=_optional_string(row["run_id"]),
        text=str(row["text"]),
        status=str(row["status"]),
        attempts=int(row["attempts"]),
        next_attempt_at=_optional_string(row["next_attempt_at"]),
        telegram_message_id=(
            int(str(row["telegram_message_id"]))
            if row["telegram_message_id"] is not None
            else None
        ),
        last_error_code=_optional_string(row["last_error_code"]),
        created_at=str(row["created_at"]),
        updated_at=str(row["updated_at"]),
    )


def _optional_string(value: object) -> str | None:
    return str(value) if value is not None else None


__all__ = [
    "CHANNEL_DATABASE_INCOMPATIBLE",
    "CHANNEL_SCHEMA_VERSION",
    "ChannelBinding",
    "ChannelOutboxItem",
    "ChannelStore",
]
