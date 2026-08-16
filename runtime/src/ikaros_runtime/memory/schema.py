"""Canonical schema for the independently durable Memory database."""

from __future__ import annotations

import sqlite3
from functools import cache

from ..errors import MemorySchemaIncompatibleError

MEMORY_SCHEMA_VERSION = 1

_CANONICAL_SCHEMA = """
CREATE TABLE memory_records (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK (kind IN ('fact', 'preference', 'relationship', 'project')),
    scope_type TEXT NOT NULL CHECK (scope_type IN ('global', 'workspace')),
    scope_key TEXT,
    current_revision INTEGER NOT NULL CHECK (current_revision >= 1),
    state TEXT NOT NULL CHECK (state IN ('active', 'forgotten')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    forgotten_at TEXT,
    CHECK (
        (scope_type = 'global' AND scope_key IS NULL)
        OR
        (scope_type = 'workspace' AND scope_key IS NOT NULL AND length(scope_key) BETWEEN 1 AND 200)
    ),
    CHECK (
        (state = 'active' AND forgotten_at IS NULL)
        OR
        (state = 'forgotten' AND forgotten_at IS NOT NULL)
    ),
    FOREIGN KEY(id, current_revision)
        REFERENCES memory_revisions(memory_id, revision)
        DEFERRABLE INITIALLY DEFERRED
);

CREATE TABLE memory_revisions (
    memory_id TEXT NOT NULL REFERENCES memory_records(id) ON DELETE RESTRICT,
    revision INTEGER NOT NULL CHECK (revision >= 1),
    operation TEXT NOT NULL CHECK (operation IN ('create', 'correct', 'forget')),
    content TEXT,
    content_digest TEXT,
    content_redacted_at TEXT,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('user_explicit', 'session_item')),
    source_thread_id TEXT,
    source_turn_id TEXT,
    source_item_id TEXT,
    source_item_digest TEXT,
    created_at TEXT NOT NULL,
    PRIMARY KEY(memory_id, revision),
    CHECK (
        (operation IN ('create', 'correct') AND (
            (content IS NOT NULL AND content_digest IS NOT NULL AND content_redacted_at IS NULL)
            OR
            (content IS NULL AND content_digest IS NULL AND content_redacted_at IS NOT NULL)
        ))
        OR
        (operation = 'forget'
         AND content IS NULL
         AND content_digest IS NULL
         AND content_redacted_at IS NULL)
    ),
    CHECK (
        (source_kind = 'user_explicit'
         AND source_thread_id IS NULL
         AND source_turn_id IS NULL
         AND source_item_id IS NULL
         AND source_item_digest IS NULL)
        OR
        (source_kind = 'session_item'
         AND source_thread_id IS NOT NULL
         AND source_turn_id IS NOT NULL
         AND source_item_id IS NOT NULL
         AND (
             (operation IN ('create', 'correct')
              AND content_redacted_at IS NULL
              AND source_item_digest IS NOT NULL)
             OR
             ((operation = 'forget' OR content_redacted_at IS NOT NULL)
              AND source_item_digest IS NULL)
         ))
    ),
    CHECK (operation != 'forget' OR source_item_digest IS NULL)
);

CREATE TABLE memory_operations (
    client_request_id TEXT PRIMARY KEY,
    method TEXT NOT NULL CHECK (method IN ('memory.create', 'memory.correct', 'memory.forget')),
    non_content_fingerprint TEXT NOT NULL,
    memory_id TEXT NOT NULL,
    resulting_revision INTEGER NOT NULL CHECK (resulting_revision >= 1),
    created_at TEXT NOT NULL,
    UNIQUE(memory_id, resulting_revision),
    FOREIGN KEY(memory_id, resulting_revision)
        REFERENCES memory_revisions(memory_id, revision)
        ON DELETE RESTRICT
);

CREATE INDEX memory_records_state_order_idx
ON memory_records(state, updated_at DESC, id ASC);

CREATE INDEX memory_records_scope_order_idx
ON memory_records(state, scope_type, scope_key, updated_at DESC, id ASC);

CREATE INDEX memory_records_scope_kind_order_idx
ON memory_records(state, scope_type, scope_key, kind, updated_at DESC, id ASC);
"""

_EXPECTED_OBJECTS = frozenset(
    {
        "memory_operations",
        "memory_records",
        "memory_records_scope_kind_order_idx",
        "memory_records_scope_order_idx",
        "memory_records_state_order_idx",
        "memory_revisions",
    }
)
_INCOMPATIBLE_MESSAGE = (
    "memory database schema is incompatible; restore a compatible backup"
)


def initialize_memory_schema(connection: sqlite3.Connection) -> None:
    version = int(connection.execute("PRAGMA user_version").fetchone()[0])
    objects = _application_objects(connection)
    if version == 0:
        if objects:
            raise MemorySchemaIncompatibleError(_INCOMPATIBLE_MESSAGE)
        _create_schema(connection)
        return
    _validate_version_and_schema(connection, version)


def validate_memory_schema(connection: sqlite3.Connection) -> None:
    version = int(connection.execute("PRAGMA user_version").fetchone()[0])
    _validate_version_and_schema(connection, version)


def _validate_version_and_schema(
    connection: sqlite3.Connection,
    version: int,
) -> None:
    if (
        version != MEMORY_SCHEMA_VERSION
        or _application_objects(connection) != _EXPECTED_OBJECTS
        or _application_schema(connection) != _canonical_application_schema()
    ):
        raise MemorySchemaIncompatibleError(_INCOMPATIBLE_MESSAGE)


def _application_objects(connection: sqlite3.Connection) -> frozenset[str]:
    rows = connection.execute(
        """
        SELECT name FROM sqlite_master
        WHERE name NOT LIKE 'sqlite_%'
        """
    ).fetchall()
    return frozenset(str(row[0]) for row in rows)


def _application_schema(
    connection: sqlite3.Connection,
) -> tuple[tuple[str, str, str, str], ...]:
    rows = connection.execute(
        """
        SELECT type, name, tbl_name, sql
        FROM sqlite_master
        WHERE name NOT LIKE 'sqlite_%'
        ORDER BY type, name
        """
    ).fetchall()
    return tuple(
        (
            str(row[0]),
            str(row[1]),
            str(row[2]),
            " ".join(str(row[3]).split()),
        )
        for row in rows
    )


@cache
def _canonical_application_schema() -> tuple[tuple[str, str, str, str], ...]:
    connection = sqlite3.connect(":memory:")
    try:
        connection.executescript(_CANONICAL_SCHEMA)
        return _application_schema(connection)
    finally:
        connection.close()


def _create_schema(connection: sqlite3.Connection) -> None:
    transaction = (
        f"BEGIN IMMEDIATE;\n{_CANONICAL_SCHEMA}\n"
        f"PRAGMA user_version = {MEMORY_SCHEMA_VERSION};\nCOMMIT;"
    )
    try:
        connection.executescript(transaction)
        validate_memory_schema(connection)
    except BaseException:
        if connection.in_transaction:
            connection.rollback()
        raise


__all__ = [
    "MEMORY_SCHEMA_VERSION",
    "initialize_memory_schema",
    "validate_memory_schema",
]
