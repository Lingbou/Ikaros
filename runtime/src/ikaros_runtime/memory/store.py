"""SQLite owner for independently durable explicit Memory."""

from __future__ import annotations

import base64
import binascii
import hashlib
import re
import sqlite3
import uuid
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from typing import cast

from ..domain import utc_now
from ..errors import MemoryOperationError
from ..json_codec import dumps as json_dumps
from ..json_codec import loads as json_loads
from .domain import (
    MEMORY_KINDS,
    MEMORY_STATES,
    MemoryKind,
    MemoryListPage,
    MemoryMutationReceipt,
    MemoryProvenance,
    MemoryRecord,
    MemoryScope,
    MemoryScopeType,
    MemorySourceKind,
    MemorySourceSnapshot,
    MemoryState,
)
from .schema import initialize_memory_schema

MEMORY_LIST_DEFAULT_LIMIT = 50
MEMORY_LIST_MAX_LIMIT = 100

_CURSOR_VERSION = 1
_MAX_CURSOR_LENGTH = 1024
_CURSOR_CHARACTERS = re.compile(r"^[A-Za-z0-9_-]+$")
_CANONICAL_TIMESTAMP = re.compile(
    r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$"
)
_INVALID_CURSOR_MESSAGE = "memory.list cursor is invalid"


@dataclass(frozen=True, slots=True)
class _MemoryListFilter:
    scope: MemoryScope | None
    kind: MemoryKind | None
    state: MemoryState

    @property
    def fingerprint(self) -> str:
        return _canonical_sha256(
            {
                "scope": self.scope.to_wire() if self.scope is not None else None,
                "kind": self.kind,
                "state": self.state,
            }
        )


@dataclass(frozen=True, slots=True)
class _MemoryListCursor:
    updated_at: str
    memory_id: str
    filter_sha256: str


class SqliteMemoryStore:
    def __init__(self, database_path: Path) -> None:
        database_path.parent.mkdir(parents=True, exist_ok=True)
        self.database_path = database_path.resolve()
        self._connection = sqlite3.connect(self.database_path)
        self._connection.row_factory = sqlite3.Row
        self._connection.execute("PRAGMA foreign_keys = ON")
        self._connection.execute("PRAGMA secure_delete = ON")
        try:
            initialize_memory_schema(self._connection)
            self._connection.execute("PRAGMA journal_mode = WAL")
            self._connection.execute("PRAGMA synchronous = NORMAL")
        except BaseException:
            self._connection.close()
            raise

    def close(self) -> None:
        self._connection.close()

    def create_memory_once(
        self,
        *,
        kind: MemoryKind,
        scope: MemoryScope,
        content: str,
        client_request_id: str,
        source: MemorySourceSnapshot | None = None,
    ) -> MemoryMutationReceipt:
        fingerprint = _create_fingerprint(kind, scope, source.item_id if source else None)
        content_digest = _canonical_sha256(content)
        self._begin_immediate()
        try:
            existing = self._idempotent_mutation_receipt(
                client_request_id=client_request_id,
                method="memory.create",
                fingerprint=fingerprint,
                content_digest=content_digest,
            )
            if existing is not None:
                self._connection.commit()
                return existing

            memory_id = f"memory_{uuid.uuid4().hex}"
            timestamp = utc_now()
            self._connection.execute(
                """
                INSERT INTO memory_records(
                    id, kind, scope_type, scope_key, current_revision, state,
                    created_at, updated_at, forgotten_at
                ) VALUES (?, ?, ?, ?, 1, 'active', ?, ?, NULL)
                """,
                (memory_id, kind, scope.type, scope.key, timestamp, timestamp),
            )
            self._connection.execute(
                """
                INSERT INTO memory_revisions(
                    memory_id, revision, operation, content, content_digest,
                    content_redacted_at,
                    source_kind, source_thread_id, source_turn_id, source_item_id,
                    source_item_digest, created_at
                ) VALUES (?, 1, 'create', ?, ?, NULL, ?, ?, ?, ?, ?, ?)
                """,
                (
                    memory_id,
                    content,
                    content_digest,
                    "session_item" if source is not None else "user_explicit",
                    source.thread_id if source is not None else None,
                    source.turn_id if source is not None else None,
                    source.item_id if source is not None else None,
                    source.item_digest if source is not None else None,
                    timestamp,
                ),
            )
            self._connection.execute(
                """
                INSERT INTO memory_operations(
                    client_request_id, method, non_content_fingerprint,
                    memory_id, resulting_revision, created_at
                ) VALUES (?, 'memory.create', ?, ?, 1, ?)
                """,
                (client_request_id, fingerprint, memory_id, timestamp),
            )
            self._connection.commit()
            return MemoryMutationReceipt(memory_id, 1, True)
        except BaseException:
            self._connection.rollback()
            raise

    def replay_create_receipt(
        self,
        *,
        kind: MemoryKind,
        scope: MemoryScope,
        content: str,
        client_request_id: str,
        source_item_id: str | None,
    ) -> MemoryMutationReceipt | None:
        """Replay a committed create before resolving its soft Session source."""

        if self._connection.in_transaction:
            raise RuntimeError("Memory receipt reads require an idle SQLite connection")
        return self._idempotent_mutation_receipt(
            client_request_id=client_request_id,
            method="memory.create",
            fingerprint=_create_fingerprint(kind, scope, source_item_id),
            content_digest=_canonical_sha256(content),
        )

    def correct_memory_once(
        self,
        *,
        memory_id: str,
        expected_revision: int,
        content: str,
        client_request_id: str,
    ) -> MemoryMutationReceipt:
        fingerprint = _mutation_fingerprint(memory_id, expected_revision)
        content_digest = _canonical_sha256(content)
        self._begin_immediate()
        try:
            existing = self._idempotent_mutation_receipt(
                client_request_id=client_request_id,
                method="memory.correct",
                fingerprint=fingerprint,
                content_digest=content_digest,
            )
            if existing is not None:
                self._connection.commit()
                return existing
            current = self._current_state(memory_id)
            _require_active_expected_revision(current, expected_revision)
            resulting_revision = expected_revision + 1
            timestamp = utc_now()
            self._connection.execute(
                """
                INSERT INTO memory_revisions(
                    memory_id, revision, operation, content, content_digest,
                    content_redacted_at,
                    source_kind, source_thread_id, source_turn_id, source_item_id,
                    source_item_digest, created_at
                ) VALUES (?, ?, 'correct', ?, ?, NULL,
                          'user_explicit', NULL, NULL, NULL, NULL, ?)
                """,
                (memory_id, resulting_revision, content, content_digest, timestamp),
            )
            updated = self._connection.execute(
                """
                UPDATE memory_records
                SET current_revision = ?, updated_at = ?
                WHERE id = ? AND state = 'active' AND current_revision = ?
                """,
                (resulting_revision, timestamp, memory_id, expected_revision),
            )
            if updated.rowcount != 1:
                raise MemoryOperationError("memory_revision_conflict")
            self._insert_operation(
                client_request_id=client_request_id,
                method="memory.correct",
                fingerprint=fingerprint,
                memory_id=memory_id,
                resulting_revision=resulting_revision,
                timestamp=timestamp,
            )
            self._connection.commit()
            return MemoryMutationReceipt(memory_id, resulting_revision, True)
        except BaseException:
            self._connection.rollback()
            raise

    def forget_memory_once(
        self,
        *,
        memory_id: str,
        expected_revision: int,
        client_request_id: str,
    ) -> MemoryMutationReceipt:
        fingerprint = _mutation_fingerprint(memory_id, expected_revision)
        self._begin_immediate()
        try:
            existing = self._idempotent_mutation_receipt(
                client_request_id=client_request_id,
                method="memory.forget",
                fingerprint=fingerprint,
                content_digest=None,
            )
            if existing is not None:
                self._connection.commit()
                return existing
            current = self._current_state(memory_id)
            _require_active_expected_revision(current, expected_revision)
            resulting_revision = expected_revision + 1
            timestamp = utc_now()
            self._connection.execute(
                """
                INSERT INTO memory_revisions(
                    memory_id, revision, operation, content, content_digest,
                    content_redacted_at,
                    source_kind, source_thread_id, source_turn_id, source_item_id,
                    source_item_digest, created_at
                ) VALUES (?, ?, 'forget', NULL, NULL, NULL, ?, ?, ?, ?, NULL, ?)
                """,
                (
                    memory_id,
                    resulting_revision,
                    current["source_kind"],
                    current["source_thread_id"],
                    current["source_turn_id"],
                    current["source_item_id"],
                    timestamp,
                ),
            )
            self._connection.execute(
                """
                UPDATE memory_revisions
                SET content = NULL,
                    content_digest = NULL,
                    source_item_digest = NULL,
                    content_redacted_at = ?
                WHERE memory_id = ? AND revision < ?
                """,
                (timestamp, memory_id, resulting_revision),
            )
            updated = self._connection.execute(
                """
                UPDATE memory_records
                SET current_revision = ?, state = 'forgotten', updated_at = ?, forgotten_at = ?
                WHERE id = ? AND state = 'active' AND current_revision = ?
                """,
                (
                    resulting_revision,
                    timestamp,
                    timestamp,
                    memory_id,
                    expected_revision,
                ),
            )
            if updated.rowcount != 1:
                raise MemoryOperationError("memory_revision_conflict")
            self._insert_operation(
                client_request_id=client_request_id,
                method="memory.forget",
                fingerprint=fingerprint,
                memory_id=memory_id,
                resulting_revision=resulting_revision,
                timestamp=timestamp,
            )
            self._connection.commit()
            return MemoryMutationReceipt(memory_id, resulting_revision, True)
        except BaseException:
            self._connection.rollback()
            raise

    def get_memory(self, memory_id: str) -> MemoryRecord:
        row = self._connection.execute(
            _CURRENT_MEMORY_SELECT + " WHERE records.id = ?",
            (memory_id,),
        ).fetchone()
        if row is None:
            raise MemoryOperationError("memory_not_found")
        return _record_from_row(row)

    def current_source_snapshot(self, memory_id: str) -> MemorySourceSnapshot | None:
        row = self._connection.execute(
            _CURRENT_MEMORY_SELECT + " WHERE records.id = ?",
            (memory_id,),
        ).fetchone()
        if row is None:
            raise MemoryOperationError("memory_not_found")
        if row["source_kind"] != "session_item" or row["source_item_digest"] is None:
            return None
        return MemorySourceSnapshot(
            thread_id=str(row["source_thread_id"]),
            turn_id=str(row["source_turn_id"]),
            item_id=str(row["source_item_id"]),
            item_digest=str(row["source_item_digest"]),
        )

    def list_memories(
        self,
        *,
        cursor: str | None,
        limit: int,
        scope: MemoryScope | None,
        kind: MemoryKind | None,
        state: MemoryState = "active",
    ) -> MemoryListPage:
        if isinstance(limit, bool) or not isinstance(limit, int):
            raise ValueError("memory.list limit must be an integer")
        if limit < 1 or limit > MEMORY_LIST_MAX_LIMIT:
            raise ValueError(
                f"memory.list limit must be between 1 and {MEMORY_LIST_MAX_LIMIT}"
            )
        if kind is not None and kind not in MEMORY_KINDS:
            raise ValueError("memory.list kind is invalid")
        if state not in MEMORY_STATES:
            raise ValueError("memory.list state is invalid")

        filters = _MemoryListFilter(scope=scope, kind=kind, state=state)
        boundary = (
            _decode_memory_cursor(cursor, filters.fingerprint)
            if cursor is not None
            else None
        )
        clauses = ["records.state = ?"]
        parameters: list[object] = [state]
        if scope is not None:
            clauses.extend(("records.scope_type = ?", "records.scope_key IS ?"))
            parameters.extend((scope.type, scope.key))
        if kind is not None:
            clauses.append("records.kind = ?")
            parameters.append(kind)
        if boundary is not None:
            clauses.append(
                "records.updated_at <= ? AND (records.updated_at < ? OR records.id > ?)"
            )
            parameters.extend(
                (boundary.updated_at, boundary.updated_at, boundary.memory_id)
            )
        parameters.append(limit + 1)

        rows = self._connection.execute(
            _CURRENT_MEMORY_SELECT
            + " WHERE "
            + " AND ".join(f"({clause})" for clause in clauses)
            + " ORDER BY records.updated_at DESC, records.id ASC LIMIT ?",
            tuple(parameters),
        ).fetchall()
        has_more = len(rows) > limit
        returned_rows = rows[:limit]
        summaries = tuple(_record_from_row(row).to_summary() for row in returned_rows)
        next_cursor = None
        if has_more:
            last = returned_rows[-1]
            next_cursor = _encode_memory_cursor(
                _MemoryListCursor(
                    updated_at=str(last["updated_at"]),
                    memory_id=str(last["id"]),
                    filter_sha256=filters.fingerprint,
                )
            )
        return MemoryListPage(summaries, next_cursor, has_more)

    def contains_protected_values(self, protected_values: tuple[str, ...]) -> bool:
        values = tuple(dict.fromkeys(value for value in protected_values if value))
        if not values:
            return False
        cursor = self._connection.execute(
            """
            SELECT records.scope_key,
                   revisions.content,
                   operations.client_request_id
            FROM memory_records AS records
            JOIN memory_revisions AS revisions ON revisions.memory_id = records.id
            LEFT JOIN memory_operations AS operations
              ON operations.memory_id = revisions.memory_id
             AND operations.resulting_revision = revisions.revision
            """
        )
        while rows := cursor.fetchmany(256):
            for row in rows:
                for candidate in row:
                    if isinstance(candidate, str) and any(
                        protected in candidate for protected in values
                    ):
                        return True
        return False

    def _idempotent_mutation_receipt(
        self,
        *,
        client_request_id: str,
        method: str,
        fingerprint: str,
        content_digest: str | None,
    ) -> MemoryMutationReceipt | None:
        row = self._connection.execute(
            """
            SELECT operations.method,
                   operations.non_content_fingerprint,
                   operations.memory_id,
                   operations.resulting_revision,
                   revisions.content_digest,
                   records.state
            FROM memory_operations AS operations
            JOIN memory_revisions AS revisions
              ON revisions.memory_id = operations.memory_id
             AND revisions.revision = operations.resulting_revision
            JOIN memory_records AS records ON records.id = operations.memory_id
            WHERE operations.client_request_id = ?
            """,
            (client_request_id,),
        ).fetchone()
        if row is None:
            return None
        if row["state"] == "forgotten" and row["method"] in {
            "memory.create",
            "memory.correct",
        }:
            raise MemoryOperationError("memory_forgotten")
        if (
            row["method"] != method
            or row["non_content_fingerprint"] != fingerprint
            or (method != "memory.forget" and row["content_digest"] != content_digest)
        ):
            raise MemoryOperationError("memory_idempotency_conflict")
        return MemoryMutationReceipt(
            memory_id=str(row["memory_id"]),
            resulting_revision=int(row["resulting_revision"]),
            created=False,
        )

    def _current_state(self, memory_id: str) -> sqlite3.Row:
        row = self._connection.execute(
            """
            SELECT records.current_revision,
                   records.state,
                   revisions.source_kind,
                   revisions.source_thread_id,
                   revisions.source_turn_id,
                   revisions.source_item_id
            FROM memory_records AS records
            JOIN memory_revisions AS revisions
              ON revisions.memory_id = records.id
             AND revisions.revision = records.current_revision
            WHERE records.id = ?
            """,
            (memory_id,),
        ).fetchone()
        if row is None:
            raise MemoryOperationError("memory_not_found")
        return cast(sqlite3.Row, row)

    def _insert_operation(
        self,
        *,
        client_request_id: str,
        method: str,
        fingerprint: str,
        memory_id: str,
        resulting_revision: int,
        timestamp: str,
    ) -> None:
        self._connection.execute(
            """
            INSERT INTO memory_operations(
                client_request_id, method, non_content_fingerprint,
                memory_id, resulting_revision, created_at
            ) VALUES (?, ?, ?, ?, ?, ?)
            """,
            (
                client_request_id,
                method,
                fingerprint,
                memory_id,
                resulting_revision,
                timestamp,
            ),
        )

    def _begin_immediate(self) -> None:
        if self._connection.in_transaction:
            raise RuntimeError("Memory writes require an idle SQLite connection")
        self._connection.execute("BEGIN IMMEDIATE")


_CURRENT_MEMORY_SELECT = """
SELECT records.id,
       records.kind,
       records.scope_type,
       records.scope_key,
       records.current_revision,
       records.state,
       records.created_at,
       records.updated_at,
       records.forgotten_at,
       revisions.content,
       revisions.source_kind,
       revisions.source_thread_id,
       revisions.source_turn_id,
       revisions.source_item_id,
       revisions.source_item_digest
FROM memory_records AS records
JOIN memory_revisions AS revisions
  ON revisions.memory_id = records.id
 AND revisions.revision = records.current_revision
"""


def _record_from_row(row: sqlite3.Row) -> MemoryRecord:
    source_kind = cast(MemorySourceKind, str(row["source_kind"]))
    return MemoryRecord(
        id=str(row["id"]),
        kind=cast(MemoryKind, str(row["kind"])),
        scope=MemoryScope(
            type=cast(MemoryScopeType, str(row["scope_type"])),
            key=cast(str | None, row["scope_key"]),
        ),
        revision=int(row["current_revision"]),
        state=cast(MemoryState, str(row["state"])),
        content=cast(str | None, row["content"]),
        provenance=MemoryProvenance(
            source_kind=source_kind,
            thread_id=cast(str | None, row["source_thread_id"]),
            turn_id=cast(str | None, row["source_turn_id"]),
            item_id=cast(str | None, row["source_item_id"]),
            status="not_applicable" if source_kind == "user_explicit" else "unavailable",
        ),
        created_at=str(row["created_at"]),
        updated_at=str(row["updated_at"]),
        forgotten_at=cast(str | None, row["forgotten_at"]),
    )


def _require_active_expected_revision(
    current: sqlite3.Row,
    expected_revision: int,
) -> None:
    if current["state"] == "forgotten":
        raise MemoryOperationError("memory_forgotten")
    if int(current["current_revision"]) != expected_revision:
        raise MemoryOperationError("memory_revision_conflict")


def _create_fingerprint(
    kind: MemoryKind,
    scope: MemoryScope,
    source_item_id: str | None,
) -> str:
    if source_item_id is None:
        payload: object = {
            "kind": kind,
            "scope": scope.to_wire(),
            "sourceKind": "user_explicit",
        }
    else:
        payload = {
            "kind": kind,
            "scope": scope.to_wire(),
            "sourceKind": "session_item",
            "sourceItemId": source_item_id,
        }
    return _canonical_sha256(payload)


def _mutation_fingerprint(memory_id: str, expected_revision: int) -> str:
    return _canonical_sha256(
        {
            "memoryId": memory_id,
            "expectedRevision": expected_revision,
        }
    )


def _encode_memory_cursor(cursor: _MemoryListCursor) -> str:
    _validate_cursor(cursor)
    payload = json_dumps(
        {
            "v": _CURSOR_VERSION,
            "updatedAt": cursor.updated_at,
            "id": cursor.memory_id,
            "filterSha256": cursor.filter_sha256,
        },
        ensure_ascii=False,
        separators=(",", ":"),
    ).encode("utf-8")
    return base64.urlsafe_b64encode(payload).rstrip(b"=").decode("ascii")


def _decode_memory_cursor(value: str, filter_sha256: str) -> _MemoryListCursor:
    if (
        not value
        or len(value) > _MAX_CURSOR_LENGTH
        or _CURSOR_CHARACTERS.fullmatch(value) is None
    ):
        raise ValueError(_INVALID_CURSOR_MESSAGE)
    try:
        padding = "=" * (-len(value) % 4)
        decoded = base64.b64decode(value + padding, altchars=b"-_", validate=True)
        payload = json_loads(decoded)
    except (UnicodeDecodeError, ValueError, binascii.Error):
        raise ValueError(_INVALID_CURSOR_MESSAGE) from None
    if not isinstance(payload, dict) or set(payload) != {
        "v",
        "updatedAt",
        "id",
        "filterSha256",
    }:
        raise ValueError(_INVALID_CURSOR_MESSAGE)
    if payload["v"] != _CURSOR_VERSION or isinstance(payload["v"], bool):
        raise ValueError(_INVALID_CURSOR_MESSAGE)
    if not all(
        isinstance(payload[key], str) for key in ("updatedAt", "id", "filterSha256")
    ):
        raise ValueError(_INVALID_CURSOR_MESSAGE)
    cursor = _MemoryListCursor(
        updated_at=payload["updatedAt"],
        memory_id=payload["id"],
        filter_sha256=payload["filterSha256"],
    )
    try:
        _validate_cursor(cursor)
    except ValueError:
        raise ValueError(_INVALID_CURSOR_MESSAGE) from None
    if cursor.filter_sha256 != filter_sha256 or _encode_memory_cursor(cursor) != value:
        raise ValueError(_INVALID_CURSOR_MESSAGE)
    return cursor


def _validate_cursor(cursor: _MemoryListCursor) -> None:
    if _CANONICAL_TIMESTAMP.fullmatch(cursor.updated_at) is None:
        raise ValueError(_INVALID_CURSOR_MESSAGE)
    try:
        datetime.strptime(cursor.updated_at, "%Y-%m-%dT%H:%M:%S.%fZ")
    except ValueError:
        raise ValueError(_INVALID_CURSOR_MESSAGE) from None
    if not cursor.memory_id.startswith("memory_"):
        raise ValueError(_INVALID_CURSOR_MESSAGE)
    suffix = cursor.memory_id.removeprefix("memory_")
    if len(suffix) != 32 or any(character not in "0123456789abcdef" for character in suffix):
        raise ValueError(_INVALID_CURSOR_MESSAGE)
    if len(cursor.filter_sha256) != 64 or any(
        character not in "0123456789abcdef" for character in cursor.filter_sha256
    ):
        raise ValueError(_INVALID_CURSOR_MESSAGE)


def _canonical_sha256(value: object) -> str:
    canonical = json_dumps(
        value,
        ensure_ascii=False,
        separators=(",", ":"),
        sort_keys=True,
    )
    return hashlib.sha256(canonical.encode("utf-8")).hexdigest()


__all__ = [
    "MEMORY_LIST_DEFAULT_LIMIT",
    "MEMORY_LIST_MAX_LIMIT",
    "SqliteMemoryStore",
]
