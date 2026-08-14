"""Canonical SQLite schema for reset-only development Runtime state."""

from __future__ import annotations

import sqlite3

_SCHEMA_VERSION = 3
_CANONICAL_SCHEMA = """
CREATE TABLE events (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    schema_version INTEGER NOT NULL CHECK (schema_version >= 1),
    event_type TEXT NOT NULL,
    thread_id TEXT,
    branch_id TEXT,
    turn_id TEXT,
    run_id TEXT,
    item_id TEXT,
    created_at TEXT NOT NULL,
    payload_json TEXT NOT NULL
);

CREATE INDEX events_thread_seq_idx ON events(thread_id, seq);
CREATE INDEX events_run_seq_idx ON events(run_id, seq);
CREATE UNIQUE INDEX events_one_settled_per_run
ON events(run_id) WHERE event_type = 'run.settled';

CREATE TABLE threads (
    id TEXT PRIMARY KEY,
    title TEXT,
    default_branch_id TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    archived_at TEXT,
    client_request_id TEXT,
    workspace_json TEXT
);

CREATE UNIQUE INDEX threads_client_request_id_idx
ON threads(client_request_id) WHERE client_request_id IS NOT NULL;
CREATE INDEX threads_active_catalog_order_idx
ON threads(updated_at DESC, id ASC) WHERE archived_at IS NULL;
CREATE INDEX threads_archived_catalog_order_idx
ON threads(updated_at DESC, id ASC) WHERE archived_at IS NOT NULL;

CREATE TABLE branches (
    id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    is_default INTEGER NOT NULL CHECK (is_default IN (0, 1))
);

CREATE UNIQUE INDEX branches_default_thread_idx
ON branches(thread_id) WHERE is_default = 1;

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
    settled_at TEXT,
    client_request_id TEXT,
    execution_policy TEXT NOT NULL DEFAULT 'full_access'
);

CREATE UNIQUE INDEX runs_client_request_id_idx
ON runs(client_request_id) WHERE client_request_id IS NOT NULL;
CREATE INDEX runs_turn_history_idx
ON runs(turn_id, created_at ASC, id ASC);

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
    data_json TEXT NOT NULL DEFAULT '{}',
    UNIQUE(run_id, ordinal)
);
"""

_INCOMPATIBLE_MESSAGE = "state database schema is incompatible; reset required"


def initialize_schema(connection: sqlite3.Connection) -> None:
    version = int(connection.execute("PRAGMA user_version").fetchone()[0])
    if version == 0:
        if _application_objects(connection):
            raise RuntimeError(_INCOMPATIBLE_MESSAGE)
        _create_schema(connection)
        return
    if version != _SCHEMA_VERSION:
        raise RuntimeError(_INCOMPATIBLE_MESSAGE)


def _application_objects(connection: sqlite3.Connection) -> tuple[str, ...]:
    rows = connection.execute(
        """
        SELECT name FROM sqlite_master
        WHERE name NOT LIKE 'sqlite_%'
        ORDER BY name
        """
    ).fetchall()
    return tuple(str(row[0]) for row in rows)


def _create_schema(connection: sqlite3.Connection) -> None:
    transaction = (
        f"BEGIN IMMEDIATE;\n{_CANONICAL_SCHEMA}\n"
        f"PRAGMA user_version = {_SCHEMA_VERSION};\nCOMMIT;"
    )
    try:
        connection.executescript(transaction)
    except BaseException:
        if connection.in_transaction:
            connection.rollback()
        raise


__all__ = ["initialize_schema"]
