"""SQLite schema and atomic migrations for Runtime state."""

from __future__ import annotations

import sqlite3

_SCHEMA_VERSION = 6
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

_MIGRATION_5 = """
ALTER TABLE runs ADD COLUMN execution_policy TEXT NOT NULL DEFAULT 'full_access';
ALTER TABLE items ADD COLUMN data_json TEXT NOT NULL DEFAULT '{}';
"""

_MIGRATION_6 = """
ALTER TABLE threads ADD COLUMN workspace_json TEXT;
"""


def migrate(connection: sqlite3.Connection) -> None:
    version = int(connection.execute("PRAGMA user_version").fetchone()[0])
    if version > _SCHEMA_VERSION:
        raise RuntimeError(
            f"state database schema {version} is newer than supported schema {_SCHEMA_VERSION}"
        )
    migrations = (
        (0, _INITIAL_SCHEMA, 1),
        (1, _MIGRATION_2, 2),
        (2, _MIGRATION_3, 3),
        (3, _MIGRATION_4, 4),
        (4, _MIGRATION_5, 5),
        (5, _MIGRATION_6, 6),
    )
    for source_version, script, target_version in migrations:
        if version == source_version:
            _apply_migration(connection, script, target_version=target_version)
            version = target_version


def _apply_migration(
    connection: sqlite3.Connection,
    script: str,
    *,
    target_version: int,
) -> None:
    transaction = f"BEGIN IMMEDIATE;\n{script}\nPRAGMA user_version = {target_version};\nCOMMIT;"
    try:
        connection.executescript(transaction)
    except BaseException:
        if connection.in_transaction:
            connection.rollback()
        raise
