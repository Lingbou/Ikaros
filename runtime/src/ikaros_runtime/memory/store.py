"""SQLite owner for independently durable explicit Memory."""

from __future__ import annotations

import base64
import binascii
import hashlib
import re
import sqlite3
import uuid
from collections.abc import Sequence
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from typing import cast

from ..domain import utc_now
from ..errors import MemoryOperationError, MemoryRetrievalError
from ..json_codec import dumps as json_dumps
from ..json_codec import loads as json_loads
from .domain import (
    MEMORY_KINDS,
    MEMORY_STATES,
    MemoryExtractionCursor,
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
from .retrieval import (
    MEMORY_RETRIEVAL_FETCH_LIMIT,
    MEMORY_RETRIEVAL_MAX_CANDIDATES,
    MaterializedMemory,
    MemoryRetrievalCandidate,
    MemorySnapshotReference,
    fts_match_expression,
    fts_token_stream,
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
        source_kind: MemorySourceKind | None = None,
    ) -> MemoryMutationReceipt:
        resolved_source_kind = _resolved_source_kind(
            source=source,
            source_kind=source_kind,
        )
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
                    resolved_source_kind,
                    source.thread_id if source is not None else None,
                    source.turn_id if source is not None else None,
                    source.item_id if source is not None else None,
                    source.item_digest if source is not None else None,
                    timestamp,
                ),
            )
            self._write_memory_index(memory_id=memory_id, content=content)
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
        source: MemorySourceSnapshot | None = None,
        source_kind: MemorySourceKind | None = None,
    ) -> MemoryMutationReceipt:
        fingerprint = _mutation_fingerprint(memory_id, expected_revision)
        content_digest = _canonical_sha256(content)
        resolved_source_kind = _resolved_source_kind(
            source=source,
            source_kind=source_kind,
        )
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
            receipt = self._append_correction(
                current,
                memory_id=memory_id,
                expected_revision=expected_revision,
                content=content,
                content_digest=content_digest,
                client_request_id=client_request_id,
                fingerprint=fingerprint,
                source=source,
                source_kind=resolved_source_kind,
            )
            self._connection.commit()
            return receipt
        except BaseException:
            self._connection.rollback()
            raise

    def correct_memory_latest_once(
        self,
        *,
        memory_id: str,
        content: str,
        client_request_id: str,
        source: MemorySourceSnapshot | None = None,
        source_kind: MemorySourceKind | None = None,
    ) -> MemoryMutationReceipt:
        """Append a correction without requiring the caller to know the revision."""

        fingerprint = _latest_mutation_fingerprint(memory_id)
        content_digest = _canonical_sha256(content)
        resolved_source_kind = _resolved_source_kind(
            source=source,
            source_kind=source_kind,
        )
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
            receipt = self._append_correction(
                current,
                memory_id=memory_id,
                expected_revision=int(current["current_revision"]),
                content=content,
                content_digest=content_digest,
                client_request_id=client_request_id,
                fingerprint=fingerprint,
                source=source,
                source_kind=resolved_source_kind,
            )
            self._connection.commit()
            return receipt
        except BaseException:
            self._connection.rollback()
            raise

    def forget_memory_once(
        self,
        *,
        memory_id: str,
        expected_revision: int,
        client_request_id: str,
        source: MemorySourceSnapshot | None = None,
        source_kind: MemorySourceKind | None = None,
    ) -> MemoryMutationReceipt:
        if source is None and source_kind is not None:
            raise ValueError("forget source kind requires a source snapshot")
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
            receipt = self._append_forget(
                current,
                memory_id=memory_id,
                expected_revision=expected_revision,
                client_request_id=client_request_id,
                fingerprint=fingerprint,
                source=source,
                source_kind=source_kind,
            )
            self._connection.commit()
            return receipt
        except BaseException:
            self._connection.rollback()
            raise

    def forget_memory_latest_once(
        self,
        *,
        memory_id: str,
        client_request_id: str,
        source: MemorySourceSnapshot | None = None,
        source_kind: MemorySourceKind | None = None,
    ) -> MemoryMutationReceipt:
        """Append a forget tombstone without requiring the caller to know the revision."""

        if source is None and source_kind is not None:
            raise ValueError("forget source kind requires a source snapshot")
        fingerprint = _latest_mutation_fingerprint(memory_id)
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
            receipt = self._append_forget(
                current,
                memory_id=memory_id,
                expected_revision=int(current["current_revision"]),
                client_request_id=client_request_id,
                fingerprint=fingerprint,
                source=source,
                source_kind=source_kind,
            )
            self._connection.commit()
            return receipt
        except BaseException:
            self._connection.rollback()
            raise

    def _append_correction(
        self,
        current: sqlite3.Row,
        *,
        memory_id: str,
        expected_revision: int,
        content: str,
        content_digest: str,
        client_request_id: str,
        fingerprint: str,
        source: MemorySourceSnapshot | None,
        source_kind: MemorySourceKind,
    ) -> MemoryMutationReceipt:
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
            ) VALUES (?, ?, 'correct', ?, ?, NULL, ?, ?, ?, ?, ?, ?)
            """,
            (
                memory_id,
                resulting_revision,
                content,
                content_digest,
                source_kind,
                source.thread_id if source is not None else None,
                source.turn_id if source is not None else None,
                source.item_id if source is not None else None,
                source.item_digest if source is not None else None,
                timestamp,
            ),
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
        self._write_memory_index(memory_id=memory_id, content=content)
        self._insert_operation(
            client_request_id=client_request_id,
            method="memory.correct",
            fingerprint=fingerprint,
            memory_id=memory_id,
            resulting_revision=resulting_revision,
            timestamp=timestamp,
        )
        return MemoryMutationReceipt(memory_id, resulting_revision, True)

    def _append_forget(
        self,
        current: sqlite3.Row,
        *,
        memory_id: str,
        expected_revision: int,
        client_request_id: str,
        fingerprint: str,
        source: MemorySourceSnapshot | None,
        source_kind: MemorySourceKind | None,
    ) -> MemoryMutationReceipt:
        _require_active_expected_revision(current, expected_revision)
        if source is None:
            resolved_source_kind = str(current["source_kind"])
            source_thread_id = current["source_thread_id"]
            source_turn_id = current["source_turn_id"]
            source_item_id = current["source_item_id"]
        else:
            resolved_source_kind = _resolved_source_kind(
                source=source,
                source_kind=source_kind,
            )
            source_thread_id = source.thread_id
            source_turn_id = source.turn_id
            source_item_id = source.item_id
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
                resolved_source_kind,
                source_thread_id,
                source_turn_id,
                source_item_id,
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
        self._write_memory_index(memory_id=memory_id, content=None)
        self._insert_operation(
            client_request_id=client_request_id,
            method="memory.forget",
            fingerprint=fingerprint,
            memory_id=memory_id,
            resulting_revision=resulting_revision,
            timestamp=timestamp,
        )
        return MemoryMutationReceipt(memory_id, resulting_revision, True)

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
        if (
            row["source_kind"] not in {"session_item", "auto_extracted"}
            or row["source_item_digest"] is None
        ):
            return None
        return MemorySourceSnapshot(
            thread_id=str(row["source_thread_id"]),
            turn_id=str(row["source_turn_id"]),
            item_id=str(row["source_item_id"]),
            item_digest=str(row["source_item_digest"]),
        )

    def read_active_candidates(
        self,
        *,
        workspace_id: str | None,
    ) -> tuple[MemoryRetrievalCandidate, ...]:
        """Read every scoped active candidate, or fail instead of ranking a prefix."""

        if workspace_id is None:
            scope_clause = "records.scope_type = 'global' AND records.scope_key IS NULL"
            parameters: tuple[object, ...] = (MEMORY_RETRIEVAL_FETCH_LIMIT,)
        else:
            MemoryScope("workspace", workspace_id)
            scope_clause = """
                (records.scope_type = 'global' AND records.scope_key IS NULL)
                OR
                (records.scope_type = 'workspace' AND records.scope_key = ?)
            """
            parameters = (workspace_id, MEMORY_RETRIEVAL_FETCH_LIMIT)
        rows = self._connection.execute(
            """
            SELECT records.id,
                   records.kind,
                   records.scope_type,
                   records.scope_key,
                   records.current_revision,
                   records.updated_at,
                   revisions.content,
                   revisions.content_digest,
                   revisions.content_redacted_at
            FROM memory_records AS records
            JOIN memory_revisions AS revisions
              ON revisions.memory_id = records.id
             AND revisions.revision = records.current_revision
            WHERE records.state = 'active'
              AND ("""
            + scope_clause
            + """)
            ORDER BY records.updated_at DESC, records.id ASC
            LIMIT ?
            """,
            parameters,
        ).fetchall()
        if len(rows) > MEMORY_RETRIEVAL_MAX_CANDIDATES:
            raise MemoryRetrievalError("memory_retrieval_overflow")
        try:
            return tuple(_retrieval_candidate_from_row(row) for row in rows)
        except (TypeError, ValueError):
            raise MemoryRetrievalError("memory_snapshot_unavailable") from None

    def rank_active_memory_ids_by_bm25(
        self,
        *,
        query: str,
        workspace_id: str | None,
        limit: int,
    ) -> tuple[str, ...]:
        """Rank active Memory IDs by FTS5 BM25, best match first, within one scope.

        The MATCH constraint drives the FTS5 index, so even one or two character
        Han queries never fall back to a full-table scan; the join only filters
        scope and state and reads nothing but the Memory ID.  Equal BM25 scores
        fall back to recency and then to the Memory ID, mirroring the keyword
        pool so the fused order degrades to the deterministic recency order
        whenever both pools are tied.
        """

        expression = fts_match_expression(query)
        if expression is None or limit < 1:
            return ()
        if workspace_id is None:
            scope_clause = "records.scope_type = 'global' AND records.scope_key IS NULL"
            parameters: tuple[object, ...] = (expression, limit)
        else:
            MemoryScope("workspace", workspace_id)
            scope_clause = """
                (records.scope_type = 'global' AND records.scope_key IS NULL)
                OR
                (records.scope_type = 'workspace' AND records.scope_key = ?)
            """
            parameters = (expression, workspace_id, limit)
        rows = self._connection.execute(
            """
            SELECT memory_fts.memory_id
            FROM memory_fts
            JOIN memory_records AS records
              ON records.id = memory_fts.memory_id
            WHERE memory_fts MATCH ?
              AND records.state = 'active'
              AND ("""
            + scope_clause
            + """)
            ORDER BY bm25(memory_fts) ASC, records.updated_at DESC, records.id ASC
            LIMIT ?
            """,
            parameters,
        ).fetchall()
        return tuple(str(row["memory_id"]) for row in rows)

    def materialize_memory_revisions(
        self,
        references: Sequence[MemorySnapshotReference],
        *,
        workspace_id: str | None,
    ) -> tuple[MaterializedMemory, ...]:
        """Materialize an ordered frozen revision set in one SQLite read snapshot."""

        frozen = tuple(references)
        if not frozen:
            return ()
        if workspace_id is not None:
            MemoryScope("workspace", workspace_id)
        if len({reference.memory_id for reference in frozen}) != len(frozen):
            raise MemoryRetrievalError("memory_snapshot_unavailable")

        values = ",".join("(?, ?, ?)" for _reference in frozen)
        parameters: list[object] = []
        for ordinal, reference in enumerate(frozen):
            parameters.extend((ordinal, reference.memory_id, reference.revision))
        rows = self._connection.execute(
            f"""
            WITH requested(ordinal, memory_id, revision) AS (
                VALUES {values}
            )
            SELECT requested.ordinal,
                   requested.memory_id AS requested_memory_id,
                   requested.revision AS requested_revision,
                   records.id AS stored_memory_id,
                   records.kind AS stored_kind,
                   records.state,
                   records.scope_type,
                   records.scope_key,
                   revisions.operation,
                   revisions.content,
                   revisions.content_digest,
                   revisions.content_redacted_at
            FROM requested
            LEFT JOIN memory_records AS records
              ON records.id = requested.memory_id
            LEFT JOIN memory_revisions AS revisions
              ON revisions.memory_id = requested.memory_id
             AND revisions.revision = requested.revision
            ORDER BY requested.ordinal ASC
            """,
            tuple(parameters),
        ).fetchall()
        if len(rows) != len(frozen):
            raise MemoryRetrievalError("memory_snapshot_unavailable")

        materialized: list[MaterializedMemory] = []
        try:
            for reference, row in zip(frozen, rows, strict=True):
                if (
                    row["stored_memory_id"] != reference.memory_id
                    or int(row["requested_revision"]) != reference.revision
                    or row["state"] != "active"
                    or row["operation"] not in {"create", "correct"}
                    or row["content"] is None
                    or row["content_digest"] is None
                    or row["content_redacted_at"] is not None
                ):
                    raise ValueError("frozen Memory revision is unavailable")
                if reference.scope == "global":
                    if row["scope_type"] != "global" or row["scope_key"] is not None:
                        raise ValueError("frozen global Memory scope changed")
                elif (
                    workspace_id is None
                    or row["scope_type"] != "workspace"
                    or row["scope_key"] != workspace_id
                ):
                    raise ValueError("frozen workspace Memory scope changed")
                content = str(row["content"])
                if str(row["content_digest"]) != _canonical_sha256(content):
                    raise ValueError("stored Memory content digest does not match")
                kind = str(row["stored_kind"])
                if kind not in MEMORY_KINDS:
                    raise ValueError("frozen Memory kind is invalid")
                materialized.append(
                    MaterializedMemory(
                        reference,
                        cast(MemoryKind, kind),
                        content,
                    )
                )
        except (TypeError, ValueError):
            raise MemoryRetrievalError("memory_snapshot_unavailable") from None
        return tuple(materialized)

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

    def memory_extraction_cursor(self, thread_id: str) -> MemoryExtractionCursor | None:
        """Return the last settled Item a refinement pass consumed, if any."""

        row = self._connection.execute(
            """
            SELECT thread_id,
                   last_turn_ordinal,
                   last_run_created_at,
                   last_run_id,
                   last_item_ordinal,
                   last_item_id,
                   extracted_at,
                   attempted_at
            FROM memory_extraction_cursors
            WHERE thread_id = ?
            """,
            (thread_id,),
        ).fetchone()
        if row is None or row["last_item_id"] is None:
            return None
        return MemoryExtractionCursor(
            thread_id=str(row["thread_id"]),
            turn_ordinal=int(row["last_turn_ordinal"]),
            run_created_at=str(row["last_run_created_at"]),
            run_id=str(row["last_run_id"]),
            item_ordinal=int(row["last_item_ordinal"]),
            item_id=str(row["last_item_id"]),
            extracted_at=str(row["extracted_at"]),
            attempted_at=str(row["attempted_at"]),
        )

    def memory_extraction_attempted_at(self, thread_id: str) -> str | None:
        """Return when this Thread last attempted an extraction call."""

        row = self._connection.execute(
            "SELECT attempted_at FROM memory_extraction_cursors WHERE thread_id = ?",
            (thread_id,),
        ).fetchone()
        return None if row is None else str(row["attempted_at"])

    def record_memory_extraction_attempt(
        self,
        thread_id: str,
        *,
        attempted_at: str,
    ) -> None:
        """Remember an extraction attempt so the hourly budget survives restarts."""

        self._begin_immediate()
        try:
            self._connection.execute(
                """
                INSERT INTO memory_extraction_cursors(thread_id, attempted_at)
                VALUES (?, ?)
                ON CONFLICT(thread_id) DO UPDATE
                SET attempted_at = excluded.attempted_at
                """,
                (thread_id, attempted_at),
            )
            self._connection.commit()
        except BaseException:
            self._connection.rollback()
            raise

    def advance_memory_extraction_cursor(
        self,
        *,
        thread_id: str,
        turn_ordinal: int,
        run_created_at: str,
        run_id: str,
        item_ordinal: int,
        item_id: str,
        extracted_at: str,
        attempted_at: str,
    ) -> None:
        """Advance the Thread cursor to one Item this pass actually consumed."""

        cursor = MemoryExtractionCursor(
            thread_id=thread_id,
            turn_ordinal=turn_ordinal,
            run_created_at=run_created_at,
            run_id=run_id,
            item_ordinal=item_ordinal,
            item_id=item_id,
            extracted_at=extracted_at,
            attempted_at=attempted_at,
        )
        self._begin_immediate()
        try:
            self._connection.execute(
                """
                INSERT INTO memory_extraction_cursors(
                    thread_id, last_turn_ordinal, last_run_created_at, last_run_id,
                    last_item_ordinal, last_item_id, attempted_at, extracted_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(thread_id) DO UPDATE SET
                    last_turn_ordinal = excluded.last_turn_ordinal,
                    last_run_created_at = excluded.last_run_created_at,
                    last_run_id = excluded.last_run_id,
                    last_item_ordinal = excluded.last_item_ordinal,
                    last_item_id = excluded.last_item_id,
                    attempted_at = excluded.attempted_at,
                    extracted_at = excluded.extracted_at
                """,
                (
                    cursor.thread_id,
                    cursor.turn_ordinal,
                    cursor.run_created_at,
                    cursor.run_id,
                    cursor.item_ordinal,
                    cursor.item_id,
                    cursor.attempted_at,
                    cursor.extracted_at,
                ),
            )
            self._connection.commit()
        except BaseException:
            self._connection.rollback()
            raise

    def active_memory_content_exists(
        self,
        *,
        kind: MemoryKind,
        scope: MemoryScope,
        content: str,
    ) -> bool:
        """Report whether an active Memory already holds exactly this content."""

        row = self._connection.execute(
            """
            SELECT 1
            FROM memory_records AS records
            JOIN memory_revisions AS revisions
              ON revisions.memory_id = records.id
             AND revisions.revision = records.current_revision
            WHERE records.state = 'active'
              AND records.kind = ?
              AND records.scope_type = ?
              AND records.scope_key IS ?
              AND revisions.content = ?
            LIMIT 1
            """,
            (kind, scope.type, scope.key, content),
        ).fetchone()
        return row is not None

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

    def _write_memory_index(self, *, memory_id: str, content: str | None) -> None:
        """Replace or drop one Memory's FTS row inside the caller's write transaction.

        The index is a derived projection of the readable revision, so it is
        rewritten with every create/correct and cleared on forget; no other code
        path may write memory_revisions without going through this method.
        """

        self._connection.execute(
            "DELETE FROM memory_fts WHERE memory_id = ?",
            (memory_id,),
        )
        if content is None:
            return
        self._connection.execute(
            "INSERT INTO memory_fts(memory_id, tokens) VALUES (?, ?)",
            (memory_id, fts_token_stream(content)),
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


def _retrieval_candidate_from_row(row: sqlite3.Row) -> MemoryRetrievalCandidate:
    content = row["content"]
    digest = row["content_digest"]
    if content is None or digest is None or row["content_redacted_at"] is not None:
        raise ValueError("active Memory candidate has no readable content")
    content_text = str(content)
    if str(digest) != _canonical_sha256(content_text):
        raise ValueError("active Memory candidate digest does not match")
    return MemoryRetrievalCandidate(
        memory_id=str(row["id"]),
        kind=cast(MemoryKind, str(row["kind"])),
        scope=MemoryScope(
            type=cast(MemoryScopeType, str(row["scope_type"])),
            key=cast(str | None, row["scope_key"]),
        ),
        revision=int(row["current_revision"]),
        content=content_text,
        updated_at=str(row["updated_at"]),
    )


def _require_active_expected_revision(
    current: sqlite3.Row,
    expected_revision: int,
) -> None:
    if current["state"] == "forgotten":
        raise MemoryOperationError("memory_forgotten")
    if int(current["current_revision"]) != expected_revision:
        raise MemoryOperationError("memory_revision_conflict")


def _resolved_source_kind(
    *,
    source: MemorySourceSnapshot | None,
    source_kind: MemorySourceKind | None,
) -> MemorySourceKind:
    if source is None:
        if source_kind not in (None, "user_explicit"):
            raise ValueError("explicit Memory must not declare a Session source kind")
        return "user_explicit"
    if source_kind is None:
        return "session_item"
    if source_kind not in {"session_item", "auto_extracted"}:
        raise ValueError("Session Memory source kind is invalid")
    return source_kind


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

def _latest_mutation_fingerprint(memory_id: str) -> str:
    return _canonical_sha256(
        {
            "memoryId": memory_id,
            "target": "latest-revision",
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
