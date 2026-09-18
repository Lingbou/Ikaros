"""Current Runtime schema. Development databases are reset across breaking changes."""

from __future__ import annotations

import sqlite3

SCHEMA_VERSION = 14
_FILE_CHANGES_SCHEMA = """
CREATE TABLE file_changes (
    tool_call_item_id TEXT PRIMARY KEY REFERENCES items(id) ON DELETE CASCADE,
    record_json TEXT NOT NULL
);
"""
_CANONICAL_SCHEMA = (
    """
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
    started_at TEXT,
    settled_at TEXT,
    reason_code TEXT,
    client_request_id TEXT,
    execution_policy TEXT NOT NULL DEFAULT 'full_access'
);

CREATE UNIQUE INDEX runs_client_request_id_idx
ON runs(client_request_id) WHERE client_request_id IS NOT NULL;
CREATE INDEX runs_turn_history_idx
ON runs(turn_id, created_at ASC, id ASC);

CREATE TABLE run_configs (
    run_id TEXT PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE,
    config_json TEXT NOT NULL
);

CREATE TABLE context_revisions (
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    record_json TEXT NOT NULL,
    PRIMARY KEY(run_id, revision)
);

CREATE TABLE process_sessions (
    process_id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    item_id TEXT NOT NULL REFERENCES items(id) ON DELETE CASCADE,
    record_json TEXT NOT NULL,
    UNIQUE(run_id, item_id)
);

CREATE TABLE model_calls (
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    call_ordinal INTEGER NOT NULL CHECK (call_ordinal >= 1),
    step_ordinal INTEGER CHECK (step_ordinal IS NULL OR step_ordinal >= 1),
    input_json TEXT NOT NULL,
    prepared_at TEXT NOT NULL,
    purpose TEXT NOT NULL DEFAULT 'execution'
        CHECK (purpose IN ('execution', 'compression', 'completion_check')),
    outcome TEXT CHECK (outcome IN ('completed', 'failed', 'cancelled')),
    reason_code TEXT,
    response_model_id TEXT,
    request_id TEXT,
    usage_json TEXT,
    activity_date TEXT,
    finished_at TEXT,
    PRIMARY KEY(run_id, call_ordinal),
    CHECK ((purpose = 'execution' AND step_ordinal IS NOT NULL)
           OR (purpose IN ('compression', 'completion_check') AND step_ordinal IS NULL)),
    CHECK (
        (outcome IS NULL AND reason_code IS NULL AND response_model_id IS NULL
         AND request_id IS NULL AND usage_json IS NULL AND activity_date IS NULL
         AND finished_at IS NULL)
        OR
        (outcome = 'completed' AND reason_code IS NULL AND finished_at IS NOT NULL)
        OR
        (outcome IN ('failed', 'cancelled') AND reason_code IS NOT NULL
         AND finished_at IS NOT NULL)
    ),
    CHECK (
        (usage_json IS NULL AND activity_date IS NULL)
        OR
        (outcome = 'completed' AND usage_json IS NOT NULL AND activity_date IS NOT NULL)
    )
);

CREATE TABLE model_usages (
    thread_id TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    turn_id TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
    run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    call_ordinal INTEGER NOT NULL CHECK (call_ordinal >= 1),
    step_ordinal INTEGER CHECK (step_ordinal IS NULL OR step_ordinal >= 1),
    provider_id TEXT NOT NULL,
    model_id TEXT NOT NULL,
    input_tokens INTEGER NOT NULL CHECK (input_tokens >= 0),
    cached_input_tokens INTEGER CHECK (cached_input_tokens >= 0),
    output_tokens INTEGER NOT NULL CHECK (output_tokens >= 0),
    reasoning_output_tokens INTEGER CHECK (reasoning_output_tokens >= 0),
    total_tokens INTEGER NOT NULL CHECK (total_tokens >= 0),
    activity_date TEXT NOT NULL,
    completed_at TEXT NOT NULL,
    PRIMARY KEY(run_id, call_ordinal),
    FOREIGN KEY(run_id, call_ordinal)
        REFERENCES model_calls(run_id, call_ordinal) ON DELETE CASCADE
);

CREATE INDEX model_usages_completed_at_idx
ON model_usages(completed_at ASC, run_id ASC, call_ordinal ASC);
CREATE INDEX model_usages_activity_date_idx
ON model_usages(activity_date ASC, run_id ASC, call_ordinal ASC);

CREATE UNIQUE INDEX model_calls_execution_step_idx
ON model_calls(run_id, step_ordinal) WHERE purpose = 'execution';

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

CREATE INDEX items_turn_context_idx
ON items(turn_id, ordinal ASC);
"""
    + _FILE_CHANGES_SCHEMA
)

_INCOMPATIBLE_MESSAGE = "state database schema is incompatible; reset required"


def initialize_schema(connection: sqlite3.Connection) -> None:
    version = int(connection.execute("PRAGMA user_version").fetchone()[0])
    if version == 0:
        if _application_objects(connection):
            raise RuntimeError(_INCOMPATIBLE_MESSAGE)
        _create_schema(connection)
        return
    if version != SCHEMA_VERSION:
        raise RuntimeError(_INCOMPATIBLE_MESSAGE)


def validate_existing_schema(connection: sqlite3.Connection) -> None:
    if int(connection.execute("PRAGMA user_version").fetchone()[0]) != SCHEMA_VERSION:
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
        f"BEGIN IMMEDIATE;\n{_CANONICAL_SCHEMA}\nPRAGMA user_version = {SCHEMA_VERSION};\nCOMMIT;"
    )
    try:
        connection.executescript(transaction)
    except BaseException:
        if connection.in_transaction:
            connection.rollback()
        raise


__all__ = ["SCHEMA_VERSION", "initialize_schema", "validate_existing_schema"]
